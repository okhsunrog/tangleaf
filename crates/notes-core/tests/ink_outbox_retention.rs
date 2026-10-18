use notes_core::db;
use notes_core::ink::{InkBackground, InkDraftPatch, InkPoint, InkStroke, Store, transfer};
use notes_core::{Connection, Hlc, Op, OpKind, Origin, PageDelete};
use std::path::Path;

fn draw(ink: &Store, ids: &[u128]) {
    let revision = ink.read().unwrap().revision;
    ink.patch(
        InkDraftPatch {
            order: ids.iter().copied().map(uuid::Uuid::from_u128).collect(),
            upserts: ids
                .iter()
                .map(|id| InkStroke {
                    id: uuid::Uuid::from_u128(*id),
                    width: 2.0,
                    points: vec![InkPoint {
                        x: *id as f64,
                        y: 2.0,
                        pressure: 0.7,
                        tilt_x: 0.0,
                        tilt_y: 0.0,
                        time: 1.0,
                    }],
                })
                .collect(),
            background: InkBackground::Grid,
        },
        revision,
    )
    .unwrap();
}

async fn assert_pending_graphs_are_exportable(connection: &Connection, pending: &[Op]) {
    for operation in pending {
        if let OpKind::InkPublish(publication) = &operation.kind {
            let graph = transfer::export_graph(connection, publication.root_hash)
                .await
                .unwrap_or_else(|error| {
                    panic!(
                        "pending publication {} lost its graph {}: {error:#}",
                        publication.version_uuid, publication.root_hash
                    )
                });
            transfer::validate_graph(publication.root_hash, &graph).unwrap();
        }
    }
}

async fn restore_with_pending_publications(path: &Path) -> (uuid::Uuid, Vec<Op>) {
    let connection = db::open(path).await.unwrap();
    notes_core::configure_sync(&connection, "https://sync.example")
        .await
        .unwrap();
    let page = db::create_handwritten_note_with_ops(&connection, Some("Drawing".into()))
        .await
        .unwrap()
        .value;
    let ink = Store::new(path, page.uuid);
    draw(&ink, &[1]);
    let first = ink.publish("First drawing".into()).unwrap().unwrap();
    let archive = db::export_archive(&connection).await.unwrap();
    draw(&ink, &[1, 2]);
    let second = ink.publish("Second drawing".into()).unwrap().unwrap();
    assert_ne!(first.root_hash, second.root_hash);
    let pending = notes_core::pending_outbox(&connection, 100).await.unwrap();
    assert_eq!(
        pending
            .iter()
            .filter(|operation| matches!(operation.kind, OpKind::InkPublish(_)))
            .count(),
        2
    );
    assert_pending_graphs_are_exportable(&connection, &pending).await;
    db::import_archive(&connection, archive).await.unwrap();
    let after = notes_core::pending_outbox(&connection, 100).await.unwrap();
    for operation in &pending {
        assert!(after.iter().any(|candidate| candidate == operation));
    }
    assert_eq!(ink.read().unwrap().draft.strokes.len(), 1);
    connection.close().await.unwrap();
    (page.uuid, after)
}

#[tokio::test]
async fn archive_restore_preserves_every_pending_drawing_graph_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.db");
    let (page_uuid, pending) = restore_with_pending_publications(&path).await;
    let connection = db::open(&path).await.unwrap();
    assert_pending_graphs_are_exportable(&connection, &pending).await;
    let ink = Store::new(&path, page_uuid);
    assert_eq!(
        ink.versions().unwrap().len(),
        1,
        "retention must not expose old versions as active heads"
    );
    assert_eq!(ink.read().unwrap().draft.strokes.len(), 1);
}

async fn deleted_drawing_with_restored_sibling(path: &Path) -> (Connection, Op) {
    let connection = db::open(path).await.unwrap();
    notes_core::configure_sync(&connection, "https://sync.example")
        .await
        .unwrap();
    let deleted = db::create_handwritten_note_with_ops(&connection, Some("Deleted".into()))
        .await
        .unwrap()
        .value;
    let ink = Store::new(path, deleted.uuid);
    draw(&ink, &[11]);
    let publication = ink.publish("Deleted drawing".into()).unwrap().unwrap();
    let sibling = db::create_handwritten_note_with_ops(&connection, Some("Sibling".into()))
        .await
        .unwrap()
        .value;
    let sibling_ink = Store::new(path, sibling.uuid);
    draw(&sibling_ink, &[22]);
    sibling_ink
        .publish("Sibling drawing".into())
        .unwrap()
        .unwrap();
    db::delete_page(&connection, deleted.uuid)
        .await
        .unwrap()
        .unwrap();
    let archive = db::export_archive(&connection).await.unwrap();
    db::import_archive(&connection, archive).await.unwrap();
    let history_pins: i64 = connection
        .call(|database| {
            database.query_row("SELECT COUNT(*) FROM history_ink_roots", [], |row| {
                row.get(0)
            })
        })
        .await
        .unwrap();
    assert_eq!(
        history_pins, 0,
        "sibling restore must clear the deletion's history owner"
    );
    assert!(
        db::get_page(&connection, deleted.uuid)
            .await
            .unwrap()
            .is_none()
    );
    let operation = notes_core::pending_outbox(&connection, 100).await.unwrap().into_iter().find(|operation| {
        matches!(&operation.kind, OpKind::InkPublish(p) if p.version_uuid == publication.version_uuid)
    }).unwrap();
    (connection, operation)
}

async fn retained_rows(connection: &Connection, operation: &Op) -> (i64, i64) {
    let OpKind::InkPublish(publication) = &operation.kind else {
        panic!("an ink publication")
    };
    let op_id = operation.op_id;
    let hash = publication.root_hash;
    connection
        .call(move |database| {
            database.query_row(
                "SELECT (SELECT COUNT(*) FROM ink_outbox_roots WHERE op_id=?1),
         (SELECT COUNT(*) FROM ink_records WHERE hash=?2)",
                rusqlite::params![op_id, hash.as_bytes().as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn deleted_drawing_survives_sibling_restore_and_quarantine_until_its_acknowledgement() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.db");
    let (connection, operation) = deleted_drawing_with_restored_sibling(&path).await;
    notes_core::quarantine_outbox_op(&connection, operation.op_id, "retry later".into(), 12)
        .await
        .unwrap();
    assert_eq!(retained_rows(&connection, &operation).await, (1, 1));
    connection.close().await.unwrap();
    let connection = db::open(&path).await.unwrap();
    assert_pending_graphs_are_exportable(&connection, std::slice::from_ref(&operation)).await;
    assert!(
        !notes_core::pending_outbox(&connection, 100)
            .await
            .unwrap()
            .iter()
            .any(|pending| pending.op_id == operation.op_id)
    );
    assert_eq!(
        notes_core::release_quarantined_outbox(&connection)
            .await
            .unwrap(),
        1
    );
    assert_pending_graphs_are_exportable(&connection, std::slice::from_ref(&operation)).await;
    notes_core::acknowledge_server_op(&connection, operation.op_id, 600)
        .await
        .unwrap();
    assert_eq!(retained_rows(&connection, &operation).await, (0, 0));
    let archived = db::export_archive(&connection).await.unwrap();
    assert_eq!(
        archived.ink.documents.len(),
        1,
        "held publications must never recreate a visible deleted drawing"
    );
}

#[tokio::test]
async fn sequenced_acknowledgements_release_only_unowned_drawing_bodies() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.db");
    let (connection, deleted) = deleted_drawing_with_restored_sibling(&path).await;
    let pending = notes_core::pending_outbox(&connection, 100).await.unwrap();
    let expected_cursor = pending.len() as u64;
    notes_core::apply_sequenced_batch(
        &connection,
        pending
            .into_iter()
            .enumerate()
            .map(|(index, operation)| (index as u64 + 1, operation))
            .collect(),
    )
    .await
    .unwrap();
    assert_eq!(retained_rows(&connection, &deleted).await, (0, 0));
    assert_eq!(
        notes_core::sync_cursor(&connection).await.unwrap(),
        expected_cursor
    );
    assert!(
        notes_core::pending_outbox(&connection, 100)
            .await
            .unwrap()
            .is_empty()
    );
    let snapshot = notes_core::export_sync_snapshot(&connection, expected_cursor)
        .await
        .unwrap();
    assert_eq!(snapshot.ink_versions.len(), 1);
    let graph = transfer::export_graph(&connection, snapshot.ink_versions[0].publication.root_hash)
        .await
        .unwrap();
    transfer::validate_graph(snapshot.ink_versions[0].publication.root_hash, &graph).unwrap();
}

#[tokio::test]
async fn failed_collection_rolls_back_acknowledgement_and_its_ink_owner() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.db");
    let (connection, operation) = deleted_drawing_with_restored_sibling(&path).await;
    let OpKind::InkPublish(publication) = &operation.kind else {
        panic!("publication")
    };
    let hash = publication.root_hash;
    connection
        .call(move |database| {
            database.execute_batch(&format!(
        "CREATE TEMP TRIGGER fail_outbox_collection BEFORE DELETE ON ink_records WHEN OLD.hash=X'{}'
         BEGIN SELECT RAISE(ABORT, 'outbox collection failure'); END;", hash
    ))
        })
        .await
        .unwrap();
    let before = notes_core::pending_outbox(&connection, 100).await.unwrap();
    let error = notes_core::acknowledge_server_op(&connection, operation.op_id, 700)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("outbox collection failure"));
    assert_eq!(
        notes_core::pending_outbox(&connection, 100).await.unwrap(),
        before
    );
    assert_eq!(retained_rows(&connection, &operation).await, (1, 1));
    assert_pending_graphs_are_exportable(&connection, std::slice::from_ref(&operation)).await;
}

#[tokio::test]
async fn a_missing_queued_root_can_arrive_after_the_visible_page_was_deleted() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.db");
    let source = db::open(&source_path).await.unwrap();
    notes_core::configure_sync(&source, "https://sync.example")
        .await
        .unwrap();
    let page = db::create_handwritten_note_with_ops(&source, Some("Drawing".into()))
        .await
        .unwrap()
        .value;
    let empty = notes_core::export_sync_snapshot(&source, 0).await.unwrap();
    let ink = Store::new(&source_path, page.uuid);
    draw(&ink, &[33]);
    let publication = ink.publish("Incoming graph".into()).unwrap().unwrap();
    let graph = transfer::export_graph(&source, publication.root_hash)
        .await
        .unwrap();
    let operation = notes_core::pending_outbox(&source, 100)
        .await
        .unwrap()
        .into_iter()
        .find(|operation| matches!(&operation.kind, OpKind::InkPublish(_)))
        .unwrap();
    let target = db::open(directory.path().join("target.db")).await.unwrap();
    notes_core::import_sync_snapshot(&target, empty)
        .await
        .unwrap();
    notes_core::configure_sync(&target, "https://sync.example")
        .await
        .unwrap();
    notes_core::apply(&target, &operation, Origin::Local)
        .await
        .unwrap();
    let op_id = operation.op_id;
    let root: Option<Vec<u8>> = target
        .call(move |database| {
            database.query_row(
                "SELECT root_id FROM ink_outbox_roots WHERE op_id=?1",
                [op_id],
                |row| row.get(0),
            )
        })
        .await
        .unwrap();
    assert!(root.is_none());
    let device_id = uuid::Uuid::now_v7();
    notes_core::apply(
        &target,
        &Op {
            op_id: uuid::Uuid::now_v7(),
            workspace_uuid: operation.workspace_uuid,
            device_id,
            hlc: Hlc::new(operation.hlc.wall_ms() + 10_000, 0, device_id),
            format_version: notes_core::operation::FORMAT_VERSION,
            kind: OpKind::PageDelete(PageDelete { uuid: page.uuid }),
        },
        Origin::Remote,
    )
    .await
    .unwrap();
    transfer::stage_graph(&target, publication.root_hash, graph)
        .await
        .unwrap();
    assert!(
        transfer::has_graph(&target, publication.root_hash)
            .await
            .unwrap()
    );
    assert_pending_graphs_are_exportable(&target, std::slice::from_ref(&operation)).await;
    let staged: i64 = target
        .call(|database| {
            database.query_row("SELECT COUNT(*) FROM ink_staged_roots", [], |row| {
                row.get(0)
            })
        })
        .await
        .unwrap();
    assert_eq!(
        staged, 1,
        "validated staging still owns a graph whose next publication may arrive later"
    );
    notes_core::acknowledge_server_op(&target, operation.op_id, 1)
        .await
        .unwrap();
    assert_eq!(retained_rows(&target, &operation).await, (0, 1));
    assert!(
        transfer::has_graph(&target, publication.root_hash)
            .await
            .unwrap()
    );
    assert!(db::get_page(&target, page.uuid).await.unwrap().is_none());
}

fn simulate_v010(path: &Path, alter: impl FnOnce(&rusqlite::Transaction<'_>)) {
    let mut database = rusqlite::Connection::open(path).unwrap();
    database.pragma_update(None, "foreign_keys", "ON").unwrap();
    let tx = database.transaction().unwrap();
    tx.execute_batch("DROP TABLE ink_outbox_roots; PRAGMA user_version=10;")
        .unwrap();
    alter(&tx);
    tx.commit().unwrap();
}

fn outbox_rows(path: &Path) -> Vec<(uuid::Uuid, String, Option<i64>, Option<String>)> {
    rusqlite::Connection::open(path).unwrap().prepare(
        "SELECT op_id,envelope,quarantined_at,quarantine_reason FROM sync_outbox ORDER BY rowid"
    ).unwrap().query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).unwrap()
        .collect::<rusqlite::Result<Vec<_>>>().unwrap()
}

#[tokio::test]
async fn v011_backfills_legacy_v6_and_current_v7_owners_without_rewriting_envelopes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.db");
    let (_, pending) = restore_with_pending_publications(&path).await;
    let mut legacy = pending
        .iter()
        .find(|operation| matches!(operation.kind, OpKind::InkPublish(_)))
        .unwrap()
        .clone();
    legacy.format_version = 6;
    let op_id = legacy.op_id;
    let bare = serde_json::to_string(&legacy).unwrap();
    simulate_v010(&path, |tx| {
        tx.execute("UPDATE sync_outbox SET envelope=?2,quarantined_at=17,quarantine_reason='preserved' WHERE op_id=?1", rusqlite::params![op_id, bare]).unwrap();
    });
    let before = outbox_rows(&path);
    let connection = db::open(&path).await.unwrap();
    assert_eq!(outbox_rows(&path), before);
    let state = connection
        .call(|database| {
            database.query_row(
                "SELECT (SELECT COUNT(*) FROM ink_outbox_roots),
         (SELECT COUNT(*) FROM ink_outbox_roots WHERE root_id IS NULL),
         (SELECT user_version FROM pragma_user_version)",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(state, (3, 0, 11));
    assert_pending_graphs_are_exportable(&connection, &pending).await;
    assert_eq!(
        notes_core::quarantined_outbox(&connection, 100)
            .await
            .unwrap()
            .len(),
        1
    );
    connection.close().await.unwrap();
    let connection = db::open(&path).await.unwrap();
    assert_eq!(outbox_rows(&path), before);
    assert_pending_graphs_are_exportable(&connection, &pending).await;
}

#[tokio::test]
async fn v011_records_already_missing_bytes_as_unavailable_without_inventing_a_drawing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.db");
    let (_, pending) = restore_with_pending_publications(&path).await;
    simulate_v010(&path, |tx| {
        tx.execute_batch(
            "DELETE FROM ink_versions; DELETE FROM ink_history; DELETE FROM ink_documents;
             DELETE FROM ink_staged_roots; DELETE FROM ink_root_refs;
             DELETE FROM ink_records; DELETE FROM ink_chunks;",
        )
        .unwrap();
    });
    let before = outbox_rows(&path);
    let connection = db::open(&path).await.unwrap();
    assert_eq!(outbox_rows(&path), before);
    let (pins, missing, bodies) = connection
        .call(|database| {
            database.query_row(
                "SELECT (SELECT COUNT(*) FROM ink_outbox_roots),
         (SELECT COUNT(*) FROM ink_outbox_roots WHERE root_id IS NULL),
         (SELECT COUNT(*) FROM ink_records)",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
        })
        .await
        .unwrap();
    assert_eq!((pins, missing, bodies), (3, 3, 0));
    for operation in pending {
        if let OpKind::InkPublish(publication) = operation.kind {
            assert!(
                transfer::export_graph(&connection, publication.root_hash)
                    .await
                    .is_err()
            );
            assert!(
                !transfer::has_graph(&connection, publication.root_hash)
                    .await
                    .unwrap()
            );
        }
    }
}

#[tokio::test]
async fn malformed_outbox_rolls_back_v011_schema_and_all_earlier_backfill_rows() {
    for corruption in ["json", "storage version", "operation ID"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.db");
        let (_, pending) = restore_with_pending_publications(&path).await;
        let bad_id = uuid::Uuid::now_v7();
        let envelope = match corruption {
            "json" => "{".into(),
            "storage version" => {
                serde_json::json!({ "format_version": 99, "payload": pending[0] }).to_string()
            }
            "operation ID" => notes_core::encode_persisted_envelope(&pending[0]).unwrap(),
            _ => unreachable!(),
        };
        simulate_v010(&path, |tx| {
            tx.execute(
                "INSERT INTO sync_outbox(op_id,envelope,created_at) VALUES(?1,?2,0)",
                rusqlite::params![bad_id, envelope],
            )
            .unwrap();
            tx.execute("DELETE FROM ink_sealed_chunks", []).unwrap();
        });
        let before = outbox_rows(&path);
        let error = db::open(&path)
            .await
            .err()
            .expect("invalid envelope must fail migration");
        assert!(
            format!("{error:#}").contains("outbox envelope"),
            "{corruption}: {error:#}"
        );
        assert_eq!(outbox_rows(&path), before);
        let database = rusqlite::Connection::open(&path).unwrap();
        let state = database
            .query_row(
                "SELECT (SELECT user_version FROM pragma_user_version),
             (SELECT COUNT(*) FROM sqlite_schema WHERE name='ink_outbox_roots'),
             (SELECT COUNT(*) FROM ink_sealed_chunks)",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(state, (10, 0, 0));
        database
            .execute("DELETE FROM sync_outbox WHERE op_id=?1", [bad_id])
            .unwrap();
        drop(database);
        let connection = db::open(&path).await.unwrap();
        assert_pending_graphs_are_exportable(&connection, &pending).await;
    }
}
