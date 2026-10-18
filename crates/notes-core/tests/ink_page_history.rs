//! Page history must recover a drawing independently of native stroke history.

use notes_core::db::{self, HistoryMoveResult};
use notes_core::ink::{InkBackground, InkDraftPatch, InkPoint, InkStroke, Store};
use notes_core::{Connection, configure_sync};
use rusqlite::types::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

struct Database {
    _directory: tempfile::TempDir,
    path: PathBuf,
    connection: Connection,
    ink: Store,
}

async fn database() -> Database {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.db");
    let connection = db::open(&path).await.unwrap();
    configure_sync(&connection, "https://ink-history.example.test")
        .await
        .unwrap();
    let page = db::create_handwritten_note_with_ops(&connection, Some("Drawing".into()))
        .await
        .unwrap()
        .value;
    let ink = Store::new(path.clone(), page.uuid);
    Database {
        _directory: directory,
        path,
        connection,
        ink,
    }
}

fn stroke(index: u128) -> InkStroke {
    InkStroke {
        id: Uuid::from_u128(index),
        width: 3.0,
        points: vec![InkPoint {
            x: 10.0 + index as f64,
            y: 20.0,
            pressure: 0.7,
            tilt_x: 12.0,
            tilt_y: -5.0,
            time: 25.0,
        }],
    }
}

fn draw(ink: &Store) {
    let first = stroke(1);
    let revision = ink
        .patch(
            InkDraftPatch {
                order: vec![first.id],
                upserts: vec![first.clone()],
                background: InkBackground::Plain,
            },
            None,
        )
        .unwrap();
    let second = stroke(2);
    ink.patch(
        InkDraftPatch {
            order: vec![first.id, second.id],
            upserts: vec![second],
            background: InkBackground::Grid,
        },
        Some(revision),
    )
    .unwrap();
}

async fn assert_page_delete_undo_recovers_drawing(published: bool) {
    let Database {
        _directory,
        path,
        connection,
        ink,
    } = database().await;
    draw(&ink);
    let expected_draft = serde_json::to_value(ink.read().unwrap().draft).unwrap();
    let native = ink.open_editor(false).unwrap();
    assert!(
        native.can_undo,
        "the fixture has its own native stroke history"
    );
    assert_eq!(db::history_status(&connection).await.unwrap().undo_count, 0);
    if published {
        let publication = ink.publish("History fixture".into()).unwrap().unwrap();
        let status = ink.status().unwrap();
        assert!(!status.unpublished_changes);
        assert_eq!(status.heads.len(), 1);
        assert_eq!(status.heads[0].version_uuid, publication.version_uuid);
        assert!(status.heads[0].available);
    } else {
        let status = ink.status().unwrap();
        assert!(status.unpublished_changes);
        assert!(status.heads.is_empty());
    }

    let page_uuid = ink.document();
    assert!(
        db::delete_page(&connection, page_uuid)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        db::get_page(&connection, page_uuid)
            .await
            .unwrap()
            .is_none()
    );
    assert!(ink.read().is_err());
    assert_eq!(db::history_status(&connection).await.unwrap().undo_count, 1);
    assert_eq!(
        db::undo_history(&connection).await.unwrap(),
        HistoryMoveResult::Applied
    );
    drop(connection);

    // Reopen through the normal initializer, rather than relying on an old
    // editor object or a surviving in-memory stroke list.
    let reopened = db::open(&path).await.unwrap();
    assert!(db::get_page(&reopened, page_uuid).await.unwrap().is_some());
    let restored = Store::new(path, page_uuid);
    let actual_draft = serde_json::to_value(restored.read().unwrap().draft).unwrap();
    let status = restored.status().unwrap();
    eprintln!(
        "after page Delete/Undo, published={published}: restored strokes={}, heads={}, unpublished={}",
        actual_draft["strokes"].as_array().unwrap().len(),
        status.heads.len(),
        status.unpublished_changes,
    );
    assert_eq!(
        actual_draft, expected_draft,
        "global Undo of a page deletion must recover the complete drawing"
    );
    assert!(
        restored.open_editor(false).unwrap().can_undo,
        "restoring a page must retain its native stroke history"
    );
    if published {
        assert!(
            !status.heads.is_empty() && status.heads.iter().all(|head| head.available),
            "published drawing variants must remain available after recovery"
        );
    }
}

#[tokio::test]
async fn undo_page_deletion_recovers_unpublished_drawing_after_reopen() {
    assert_page_delete_undo_recovers_drawing(false).await;
}

#[tokio::test]
async fn undo_page_deletion_recovers_published_drawing_after_reopen() {
    assert_page_delete_undo_recovers_drawing(true).await;
}

#[tokio::test]
async fn redo_page_deletion_preserves_new_strokes_drawn_after_undo() {
    let database = database().await;
    draw(&database.ink);
    let page_uuid = database.ink.document();
    db::delete_page(&database.connection, page_uuid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        db::undo_history(&database.connection).await.unwrap(),
        HistoryMoveResult::Applied
    );

    let previous = database.ink.read().unwrap();
    let new_stroke = stroke(3);
    let mut order = previous
        .draft
        .strokes
        .iter()
        .map(|stroke| stroke.id)
        .collect::<Vec<_>>();
    order.push(new_stroke.id);
    database
        .ink
        .patch(
            InkDraftPatch {
                order,
                upserts: vec![new_stroke],
                background: InkBackground::Grid,
            },
            previous.revision,
        )
        .unwrap();
    let expected_draft = serde_json::to_value(database.ink.read().unwrap().draft).unwrap();

    assert_eq!(
        db::redo_history(&database.connection).await.unwrap(),
        HistoryMoveResult::Skipped,
        "Redo must not erase strokes written since the page was restored"
    );
    assert!(
        db::get_page(&database.connection, page_uuid)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        serde_json::to_value(database.ink.read().unwrap().draft).unwrap(),
        expected_draft
    );
}

async fn rows(connection: &Connection) -> BTreeMap<String, Vec<Vec<Value>>> {
    connection
        .call(|database| {
            let names = database
                .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")?
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut result = BTreeMap::new();
            for name in names {
                let mut statement = database
                    .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))?;
                let columns = statement.column_count();
                let mut entries = statement
                    .query_map([], |row| {
                        (0..columns)
                            .map(|index| row.get(index))
                            .collect::<rusqlite::Result<Vec<Value>>>()
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                entries.sort_by_cached_key(|row| format!("{row:?}"));
                result.insert(name, entries);
            }
            Ok(result)
        })
        .await
        .unwrap()
}

async fn retained_pages(connection: &Connection) -> i64 {
    connection
        .call(|database| {
            database.query_row("SELECT COUNT(*) FROM history_ink_snapshots", [], |row| {
                row.get(0)
            })
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn page_history_recovers_native_stroke_undo_and_redo() {
    let database = database().await;
    draw(&database.ink);
    let expected = serde_json::to_value(database.ink.read().unwrap().draft).unwrap();
    db::delete_page(&database.connection, database.ink.document())
        .await
        .unwrap()
        .unwrap();
    db::undo_history(&database.connection).await.unwrap();
    let revision = database.ink.read().unwrap().revision;
    database.ink.history(Some(false), revision).unwrap();
    let prior = database.ink.read().unwrap();
    assert_eq!(prior.draft.strokes.len(), 1);
    assert!(matches!(prior.draft.background, InkBackground::Plain));
    database.ink.history(Some(true), prior.revision).unwrap();
    assert_eq!(
        serde_json::to_value(database.ink.read().unwrap().draft).unwrap(),
        expected
    );
}

#[tokio::test]
async fn repeated_page_undo_redo_retains_drawing_and_publishes_fresh_versions() {
    let database = database().await;
    draw(&database.ink);
    let original = database.ink.publish("Original".into()).unwrap().unwrap();
    let expected = serde_json::to_value(database.ink.read().unwrap().draft).unwrap();
    db::delete_page(&database.connection, database.ink.document())
        .await
        .unwrap()
        .unwrap();
    let mut versions = std::collections::HashSet::from([original.version_uuid]);
    for _ in 0..3 {
        assert_eq!(
            db::undo_history(&database.connection).await.unwrap(),
            HistoryMoveResult::Applied
        );
        assert_eq!(
            serde_json::to_value(database.ink.read().unwrap().draft).unwrap(),
            expected
        );
        let status = database.ink.status().unwrap();
        assert_eq!(status.heads.len(), 1);
        assert_eq!(status.heads[0].root_hash, original.root_hash.to_string());
        assert!(versions.insert(status.heads[0].version_uuid));
        assert_eq!(retained_pages(&database.connection).await, 1);
        assert_eq!(
            db::redo_history(&database.connection).await.unwrap(),
            HistoryMoveResult::Applied
        );
        assert!(
            db::get_page(&database.connection, database.ink.document())
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn missing_or_corrupt_retention_refuses_undo_without_any_writes() {
    for damage in [
        "DELETE FROM history_ink_snapshots",
        "UPDATE history_ink_snapshots SET metadata_json='{}'",
        "DELETE FROM history_ink_roots",
    ] {
        let database = database().await;
        draw(&database.ink);
        db::delete_page(&database.connection, database.ink.document())
            .await
            .unwrap()
            .unwrap();
        database
            .connection
            .call(move |database| database.execute_batch(damage))
            .await
            .unwrap();
        let before = rows(&database.connection).await;
        assert!(
            db::undo_history(&database.connection).await.is_err(),
            "{damage}"
        );
        assert_eq!(rows(&database.connection).await, before, "{damage}");
    }
}

#[tokio::test]
async fn failure_after_pinning_rolls_back_the_drawing_history_and_outbox() {
    let database = database().await;
    draw(&database.ink);
    database.connection.call(|database| database.execute_batch("CREATE TRIGGER fail_page_history BEFORE INSERT ON history_undo BEGIN SELECT RAISE(ABORT,'history write failed'); END")).await.unwrap();
    let before = rows(&database.connection).await;
    assert!(
        db::delete_page(&database.connection, database.ink.document())
            .await
            .is_err()
    );
    assert_eq!(rows(&database.connection).await, before);
    assert_eq!(database.ink.read().unwrap().draft.strokes.len(), 2);
}

#[tokio::test]
async fn pruning_or_clearing_history_releases_pins_and_collects_unowned_bodies() {
    let database = database().await;
    draw(&database.ink);
    db::delete_page(&database.connection, database.ink.document())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained_pages(&database.connection).await, 1);
    for _ in 0..50 {
        db::create_note(&database.connection, None).await.unwrap();
    }
    assert_eq!(retained_pages(&database.connection).await, 0);
    let ink_rows = database
        .connection
        .call(|database| {
            database.query_row("SELECT COUNT(*) FROM ink_records", [], |row| {
                row.get::<_, i64>(0)
            })
        })
        .await
        .unwrap();
    assert_eq!(ink_rows, 0);

    let another = db::create_handwritten_note_with_ops(&database.connection, None)
        .await
        .unwrap()
        .value;
    let ink = Store::new(database.path.clone(), another.uuid);
    draw(&ink);
    db::delete_page(&database.connection, another.uuid)
        .await
        .unwrap()
        .unwrap();
    db::undo_history(&database.connection).await.unwrap();
    assert_eq!(retained_pages(&database.connection).await, 1);
    db::create_note(&database.connection, None).await.unwrap();
    assert_eq!(retained_pages(&database.connection).await, 0);
    assert_eq!(
        ink.read().unwrap().draft.strokes.len(),
        2,
        "live roots must survive pin release"
    );
}

#[tokio::test]
async fn replacing_workspace_releases_history_pins() {
    for archive in [false, true] {
        let database = database().await;
        draw(&database.ink);
        db::delete_page(&database.connection, database.ink.document())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retained_pages(&database.connection).await, 1);
        if archive {
            let snapshot = db::export_archive(&database.connection).await.unwrap();
            db::import_archive(&database.connection, snapshot)
                .await
                .unwrap();
        } else {
            let snapshot = notes_core::export_sync_snapshot(&database.connection, 0)
                .await
                .unwrap();
            notes_core::import_sync_snapshot(&database.connection, snapshot)
                .await
                .unwrap();
        }
        assert_eq!(retained_pages(&database.connection).await, 0);
    }
}

#[tokio::test]
async fn a_corrupt_non_current_native_history_graph_refuses_restoration() {
    let database = database().await;
    draw(&database.ink);
    let current = database.ink.read().unwrap();
    database
        .ink
        .patch(
            InkDraftPatch {
                order: vec![current.draft.strokes[0].id],
                upserts: vec![],
                background: InkBackground::Plain,
            },
            current.revision,
        )
        .unwrap();
    let page = database.ink.document();
    let current_root = database
        .connection
        .call(move |database| {
            database.query_row(
                "SELECT root_id FROM ink_documents WHERE page_uuid=?1",
                [page],
                |row| row.get::<_, Vec<u8>>(0),
            )
        })
        .await
        .unwrap();
    db::delete_page(&database.connection, page)
        .await
        .unwrap()
        .unwrap();
    database.connection.call(move |database| {
        let chunk: Vec<u8> = database.query_row("SELECT object_id FROM ink_root_refs WHERE kind=1 AND root_id IN (SELECT root_id FROM history_ink_roots) AND object_id NOT IN (SELECT object_id FROM ink_root_refs WHERE root_id=?1 AND kind=1) LIMIT 1", [current_root], |row| row.get(0))?;
        database.execute("DELETE FROM ink_chunks WHERE id=?1", [chunk])?;
        Ok(())
    }).await.unwrap();
    let before = rows(&database.connection).await;
    assert!(db::undo_history(&database.connection).await.is_err());
    assert_eq!(
        rows(&database.connection).await,
        before,
        "no PageCreate may commit after corrupt native history"
    );
}

#[tokio::test]
async fn retained_native_roots_survive_compaction_and_shared_body_collection() {
    let database = database().await;
    draw(&database.ink);
    let current = database.ink.read().unwrap();
    let third = stroke(3);
    let mut order = current
        .draft
        .strokes
        .iter()
        .map(|stroke| stroke.id)
        .collect::<Vec<_>>();
    order.push(third.id);
    let revision = database
        .ink
        .patch(
            InkDraftPatch {
                order,
                upserts: vec![third],
                background: InkBackground::Grid,
            },
            current.revision,
        )
        .unwrap();
    database
        .ink
        .patch(
            InkDraftPatch {
                order: vec![stroke(1).id],
                upserts: vec![],
                background: InkBackground::Plain,
            },
            Some(revision),
        )
        .unwrap();
    let expected = serde_json::to_value(database.ink.read().unwrap().draft).unwrap();
    let shared_page = db::create_handwritten_note_with_ops(&database.connection, None)
        .await
        .unwrap()
        .value;
    let shared = Store::new(database.path.clone(), shared_page.uuid);
    draw(&shared);
    let shared_expected = serde_json::to_value(shared.read().unwrap().draft).unwrap();
    db::delete_page(&database.connection, database.ink.document())
        .await
        .unwrap()
        .unwrap();
    db::undo_history(&database.connection).await.unwrap();
    let revision = database.ink.read().unwrap().revision;
    assert!(
        database
            .ink
            .compact_with_limits(2, 16 * 1024 * 1024)
            .unwrap(),
        "fixture must perform real compaction"
    );
    assert_eq!(database.ink.read().unwrap().revision, revision);
    assert_eq!(
        serde_json::to_value(shared.read().unwrap().draft).unwrap(),
        shared_expected
    );
    assert_eq!(
        db::redo_history(&database.connection).await.unwrap(),
        HistoryMoveResult::Applied,
        "physical compaction must not invalidate semantic history guards"
    );
    assert_eq!(
        db::undo_history(&database.connection).await.unwrap(),
        HistoryMoveResult::Applied
    );
    assert_eq!(
        serde_json::to_value(database.ink.read().unwrap().draft).unwrap(),
        expected
    );
    let revision = database.ink.read().unwrap().revision;
    database.ink.history(Some(false), revision).unwrap();
    assert_eq!(database.ink.read().unwrap().draft.strokes.len(), 3);
    let revision = database.ink.read().unwrap().revision;
    database.ink.history(Some(false), revision).unwrap();
    assert_eq!(database.ink.read().unwrap().draft.strokes.len(), 2);
    assert_eq!(
        serde_json::to_value(shared.read().unwrap().draft).unwrap(),
        shared_expected
    );
}

// Freeze the pre-v3 decoder's explicit gate while deliberately ignoring extra
// fields: rejection must rely on the format version, not an unknown ink field.
fn decode_with_v2_gate(json: &str) -> Result<(), String> {
    let payload: serde_json::Value = notes_core::operation::decode_persisted_envelope(json)
        .map_err(|error| error.to_string())?;
    if payload.is_array() || payload["history_format_version"] == 2 {
        Ok(())
    } else {
        Err("unsupported history payload format version".into())
    }
}

#[tokio::test]
async fn ink_history_is_version_gated_while_text_history_remains_v2() {
    let database = database().await;
    draw(&database.ink);
    db::delete_page(&database.connection, database.ink.document())
        .await
        .unwrap()
        .unwrap();
    let (forward, inverse) = database
        .connection
        .call(|database| {
            database.query_row(
                "SELECT forward_json,inverse_json FROM history_undo ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
        })
        .await
        .unwrap();
    for json in [forward, inverse] {
        assert!(decode_with_v2_gate(&json).is_err());
        let value: serde_json::Value =
            notes_core::operation::decode_persisted_envelope(&json).unwrap();
        assert_eq!(value["history_format_version"], 3);
    }
    db::create_note(&database.connection, None).await.unwrap();
    let json = database
        .connection
        .call(|database| {
            database.query_row(
                "SELECT inverse_json FROM history_undo ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
        })
        .await
        .unwrap();
    decode_with_v2_gate(&json).unwrap();
    let value: serde_json::Value = notes_core::operation::decode_persisted_envelope(&json).unwrap();
    assert_eq!(value["history_format_version"], 2);
}

#[tokio::test]
async fn already_collected_legacy_drawing_history_requires_a_backup() {
    let database = database().await;
    draw(&database.ink);
    db::delete_page(&database.connection, database.ink.document())
        .await
        .unwrap()
        .unwrap();
    database.connection.call(|database| {
        let (forward, inverse): (String,String) = database.query_row("SELECT forward_json,inverse_json FROM history_undo ORDER BY id DESC LIMIT 1", [], |row| Ok((row.get(0)?,row.get(1)?)))?;
        let downgrade = |json: String| {
            let mut value: serde_json::Value = notes_core::operation::decode_persisted_envelope(&json).unwrap();
            value["history_format_version"] = 2.into();
            value["scope_guards"] = serde_json::json!([]);
            value.as_object_mut().unwrap().remove("ink_restore_pages");
            notes_core::operation::encode_persisted_envelope(&value).unwrap()
        };
        database.execute("UPDATE history_undo SET forward_json=?1,inverse_json=?2", rusqlite::params![downgrade(forward),downgrade(inverse)])?;
        database.execute_batch("DELETE FROM history_ink_snapshots; DELETE FROM ink_root_refs; DELETE FROM ink_records; DELETE FROM ink_chunks;")?;
        Ok(())
    }).await.unwrap();
    let before = rows(&database.connection).await;
    let error = db::undo_history(&database.connection).await.unwrap_err();
    assert!(error.to_string().contains("restore a backup"), "{error:#}");
    assert_eq!(rows(&database.connection).await, before);
}

#[tokio::test]
async fn recovered_publication_reaches_another_replica_after_delete() {
    let database = database().await;
    draw(&database.ink);
    database.ink.publish("Original".into()).unwrap().unwrap();
    let expected = serde_json::to_value(database.ink.read().unwrap().draft).unwrap();
    db::delete_page(&database.connection, database.ink.document())
        .await
        .unwrap()
        .unwrap();
    db::undo_history(&database.connection).await.unwrap();

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("replica.db");
    let replica = db::open(&path).await.unwrap();
    let mut empty = notes_core::export_sync_snapshot(&replica, 0).await.unwrap();
    empty.workspace_uuid = db::workspace_uuid(&database.connection).await.unwrap();
    notes_core::import_sync_snapshot(&replica, empty)
        .await
        .unwrap();
    let operations = notes_core::pending_outbox(&database.connection, 100)
        .await
        .unwrap();
    let mut downloaded_publications = 0;
    for (index, operation) in operations.iter().enumerate() {
        if let notes_core::OpKind::InkPublish(publication) = &operation.kind {
            let blobs = notes_core::ink::transfer::export_graph(
                &database.connection,
                publication.root_hash,
            )
            .await
            .unwrap();
            notes_core::ink::transfer::stage_graph(&replica, publication.root_hash, blobs)
                .await
                .unwrap();
            downloaded_publications += 1;
        }
        notes_core::apply_sequenced(&replica, index as u64 + 1, operation)
            .await
            .unwrap();
    }
    assert_eq!(
        downloaded_publications, 2,
        "both original and recovered operations carry available graph bytes"
    );
    let restored = Store::new(path, database.ink.document());
    assert_eq!(
        serde_json::to_value(restored.read().unwrap().draft).unwrap(),
        expected
    );
    assert_eq!(
        notes_core::export_sync_snapshot(&replica, 0).await.unwrap(),
        notes_core::export_sync_snapshot(&database.connection, 0)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn recovery_preserves_unpublished_work_and_a_concurrent_published_variant() {
    let database = database().await;
    draw(&database.ink);
    let base = database.ink.publish("Base".into()).unwrap().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("other.db");
    let other = db::open(&path).await.unwrap();
    let snapshot = notes_core::export_sync_snapshot(&database.connection, 0)
        .await
        .unwrap();
    notes_core::import_sync_snapshot(&other, snapshot)
        .await
        .unwrap();
    configure_sync(&other, "https://ink-history.example.test")
        .await
        .unwrap();
    let blobs = notes_core::ink::transfer::export_graph(&database.connection, base.root_hash)
        .await
        .unwrap();
    notes_core::ink::transfer::stage_graph(&other, base.root_hash, blobs)
        .await
        .unwrap();
    let other_ink = Store::new(path, database.ink.document());
    let append = |ink: &Store, index| {
        let before = ink.read().unwrap();
        let next = stroke(index);
        let mut order = before
            .draft
            .strokes
            .iter()
            .map(|stroke| stroke.id)
            .collect::<Vec<_>>();
        order.push(next.id);
        ink.patch(
            InkDraftPatch {
                order,
                upserts: vec![next],
                background: InkBackground::Grid,
            },
            before.revision,
        )
        .unwrap();
    };
    append(&database.ink, 3);
    append(&other_ink, 4);
    let publication = other_ink.publish("Other device".into()).unwrap().unwrap();
    let remote_draft = serde_json::to_value(other_ink.read().unwrap().draft).unwrap();
    let working_draft = serde_json::to_value(database.ink.read().unwrap().draft).unwrap();
    let blobs = notes_core::ink::transfer::export_graph(&other, publication.root_hash)
        .await
        .unwrap();
    notes_core::ink::transfer::stage_graph(&database.connection, publication.root_hash, blobs)
        .await
        .unwrap();
    let operation = notes_core::pending_outbox(&other, 100).await.unwrap().into_iter().find(|operation| matches!(&operation.kind, notes_core::OpKind::InkPublish(value) if value.version_uuid == publication.version_uuid)).unwrap();
    notes_core::apply(&database.connection, &operation, notes_core::Origin::Remote)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(database.ink.read().unwrap().draft).unwrap(),
        working_draft
    );
    db::delete_page(&database.connection, database.ink.document())
        .await
        .unwrap()
        .unwrap();
    db::undo_history(&database.connection).await.unwrap();
    assert_eq!(
        serde_json::to_value(database.ink.read().unwrap().draft).unwrap(),
        working_draft
    );
    let status = database.ink.status().unwrap();
    assert_eq!(status.heads.len(), 2);
    let actual = status
        .heads
        .iter()
        .map(|head| {
            serde_json::to_string(
                &serde_json::to_value(database.ink.preview(head.version_uuid).unwrap()).unwrap(),
            )
            .unwrap()
        })
        .collect::<std::collections::BTreeSet<_>>();
    let expected = [working_draft, remote_draft]
        .into_iter()
        .map(|draft| serde_json::to_string(&draft).unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(actual, expected);
}
