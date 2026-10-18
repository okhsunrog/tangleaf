use notes_core::db;
use notes_core::store::{CommitNotice, NotesStore};
use notes_core::{
    BlobHash, BlockCreate, BlockSetMarkdown, BlockStyle, Connection, CoreError, Hlc, Op, OpKind,
    OrderKey, Origin, PageCreate, PageKind, PageLayout, PageSetTitle, ReplicaRole,
};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

struct Fixture {
    _directory: tempfile::TempDir,
    connection: Connection,
    workspace: Uuid,
    store: NotesStore,
    notices: Arc<Mutex<Vec<CommitNotice>>>,
}

async fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(directory.path().join("notes.db")).await.unwrap();
    let workspace = db::workspace_uuid(&connection).await.unwrap();
    let notices = Arc::new(Mutex::new(Vec::new()));
    let observed = notices.clone();
    let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
        observed.lock().unwrap().push(notice.clone());
    });
    Fixture {
        _directory: directory,
        connection,
        workspace,
        store,
        notices,
    }
}

impl Fixture {
    fn op(&self, index: u128, kind: OpKind) -> Op {
        let device_id = id(0xD3_71CE);
        Op {
            op_id: id(0x100_0000 + index),
            workspace_uuid: self.workspace,
            device_id,
            hlc: Hlc::new(index as u64 * 10, 0, device_id),
            format_version: notes_core::operation::FORMAT_VERSION,
            kind,
        }
    }
}

fn page(uuid: Uuid, kind: PageKind) -> OpKind {
    OpKind::PageCreate(PageCreate {
        uuid,
        title: Some(format!("Page {uuid}")),
        kind,
        layout: PageLayout::Outline,
        created_at: 1,
    })
}

fn block(uuid: Uuid, page_uuid: Uuid, markdown: &str) -> OpKind {
    OpKind::BlockCreate(BlockCreate {
        uuid,
        page_uuid,
        parent_uuid: None,
        order_key: OrderKey::first(),
        style: BlockStyle::Bullet,
        markdown: markdown.into(),
        created_at: 1,
    })
}

fn publication(page_uuid: Uuid, version_uuid: Uuid, parents: Vec<Uuid>) -> OpKind {
    OpKind::InkPublish(notes_core::ink::Publish {
        page_uuid,
        version_uuid,
        parents,
        root_hash: BlobHash::from_bytes([7; 32]),
        device_name: "Remote".into(),
    })
}

async fn metadata(connection: &Connection, key: &'static str) -> Option<String> {
    connection
        .call(move |database| {
            use rusqlite::OptionalExtension;
            database
                .query_row("SELECT value FROM sync_meta WHERE key = ?1", [key], |row| {
                    row.get(0)
                })
                .optional()
        })
        .await
        .unwrap()
}

async fn state(connection: &Connection) -> serde_json::Value {
    let content = notes_core::export_sync_snapshot(connection, 0)
        .await
        .unwrap();
    let bookkeeping = connection
        .call(|database| {
            let metadata = database
                .prepare("SELECT key, value FROM sync_meta ORDER BY key")?
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let applied = database
                .prepare("SELECT op_id, seq FROM applied_ops ORDER BY op_id")?
                .query_map([], |row| {
                    Ok((row.get::<_, Uuid>(0)?, row.get::<_, Option<i64>>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let outbox = database
                .prepare("SELECT op_id, envelope FROM sync_outbox ORDER BY op_id")?
                .query_map([], |row| {
                    Ok((row.get::<_, Uuid>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let counts = database.query_row(
                "SELECT (SELECT version FROM store_state), (SELECT COUNT(*) FROM local_device),
                    (SELECT COUNT(*) FROM history_undo), (SELECT COUNT(*) FROM history_redo),
                    (SELECT COUNT(*) FROM ink_documents), (SELECT COUNT(*) FROM ink_versions),
                    (SELECT COUNT(*) FROM ink_version_parents), (SELECT COUNT(*) FROM ink_history)",
                [],
                |row| {
                    (0..8)
                        .map(|column| row.get::<_, i64>(column))
                        .collect::<rusqlite::Result<Vec<_>>>()
                },
            )?;
            Ok((metadata, applied, outbox, counts))
        })
        .await
        .unwrap();
    serde_json::json!([content, bookkeeping])
}

#[tokio::test]
async fn original_local_envelopes_keep_ids_clocks_and_role_based_outbox_policy() {
    for configuration in ["offline", "client", "server"] {
        let f = fixture().await;
        if configuration == "client" {
            notes_core::configure_sync(&f.connection, "https://remote.example")
                .await
                .unwrap();
        }
        if configuration == "server" {
            notes_core::set_replica_role(&f.connection, ReplicaRole::Server)
                .await
                .unwrap();
        }
        let operation = f.op(1, page(id(1), PageKind::Note));
        let clock = metadata(&f.connection, "last_hlc").await;
        let committed = f
            .store
            .apply_batch(std::slice::from_ref(&operation), Origin::Local)
            .await
            .unwrap();
        assert!(committed.value[0].applied);
        assert_eq!(committed.changes.changed_pages, [id(1)]);
        assert!(!committed.changes.history_changed);
        assert_eq!(
            metadata(&f.connection, "last_hlc").await,
            clock,
            "Local envelope API does not receive its own HLC"
        );
        let outbox = notes_core::pending_outbox(&f.connection, 10).await.unwrap();
        if configuration == "offline" {
            assert!(outbox.is_empty());
        } else {
            assert_eq!(outbox.as_slice(), std::slice::from_ref(&operation));
        }
        assert_eq!(
            db::get_page(&f.connection, id(1))
                .await
                .unwrap()
                .unwrap()
                .title_revision
                .to_string(),
            operation.hlc.to_string()
        );
        let before = state(&f.connection).await;
        let duplicate = f
            .store
            .apply_batch(std::slice::from_ref(&operation), Origin::Local)
            .await
            .unwrap();
        assert!(!duplicate.value[0].applied);
        assert_eq!(duplicate.commit_version, None);
        assert!(duplicate.changes.is_empty());
        assert_eq!(state(&f.connection).await, before);
        assert_eq!(f.notices.lock().unwrap().len(), 1);

        let remote = f.op(
            2,
            OpKind::PageSetTitle(PageSetTitle {
                uuid: id(1),
                title: Some("Remote title".into()),
            }),
        );
        f.store
            .apply_batch(std::slice::from_ref(&remote), Origin::Remote)
            .await
            .unwrap();
        let observed = metadata(&f.connection, "last_hlc")
            .await
            .unwrap()
            .parse::<Hlc>()
            .unwrap();
        assert!(observed > remote.hlc);
        assert_eq!(
            notes_core::pending_outbox(&f.connection, 10).await.unwrap(),
            outbox
        );
    }
}

#[tokio::test]
async fn accepted_domain_noop_advances_replication_commit_without_fake_entities() {
    let f = fixture().await;
    let operation = f.op(
        1,
        OpKind::BlockSetMarkdown(BlockSetMarkdown {
            uuid: id(99),
            markdown: "Missing block".into(),
        }),
    );
    let committed = f
        .store
        .apply_batch(std::slice::from_ref(&operation), Origin::Remote)
        .await
        .unwrap();
    assert!(committed.value[0].applied);
    assert_eq!(committed.value[0].affected_uuids, [id(99)]);
    assert!(committed.changes.is_empty());
    assert!(committed.commit_version.is_some());
    assert!(
        db::get_block(&f.connection, id(99))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(f.notices.lock().unwrap().len(), 1);
    assert!(f.notices.lock().unwrap()[0].changes.is_empty());
    let before = state(&f.connection).await;
    let duplicate = f
        .store
        .apply_batch(std::slice::from_ref(&operation), Origin::Remote)
        .await
        .unwrap();
    assert_eq!(duplicate.commit_version, None);
    assert_eq!(state(&f.connection).await, before);
}

#[tokio::test]
async fn sequenced_acknowledgement_advances_cursor_and_commit_without_reapplying_or_observing_clock()
 {
    let f = fixture().await;
    notes_core::configure_sync(&f.connection, "https://remote.example")
        .await
        .unwrap();
    let operation = f.op(1, page(id(1), PageKind::Note));
    let local = f
        .store
        .apply_batch(std::slice::from_ref(&operation), Origin::Local)
        .await
        .unwrap();
    let clock = metadata(&f.connection, "last_hlc").await;
    let ack = f
        .store
        .apply_sequenced_batch(vec![(1, operation.clone())])
        .await
        .unwrap();
    assert!(!ack.value[0].applied);
    assert!(ack.changes.is_empty());
    assert_eq!(
        ack.commit_version,
        local.commit_version.map(|version| version + 1)
    );
    assert_eq!(notes_core::sync_cursor(&f.connection).await.unwrap(), 1);
    assert!(
        notes_core::pending_outbox(&f.connection, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(metadata(&f.connection, "last_hlc").await, clock);
    let before = state(&f.connection).await;
    let duplicate = f
        .store
        .apply_sequenced_batch(vec![(1, operation.clone())])
        .await
        .unwrap();
    assert_eq!(duplicate.commit_version, None);
    assert!(duplicate.changes.is_empty());
    assert_eq!(
        f.store
            .apply_sequenced_batch(Vec::new())
            .await
            .unwrap()
            .commit_version,
        None
    );
    assert_eq!(state(&f.connection).await, before);
    assert_eq!(f.notices.lock().unwrap().len(), 2);

    f.connection
        .call(|database| {
            database
                .execute("DELETE FROM applied_ops WHERE seq = 1", [])
                .map(|_| ())
        })
        .await
        .unwrap();
    let before = state(&f.connection).await;
    let unknown = f.op(2, page(id(2), PageKind::Note));
    let replay = f
        .store
        .apply_sequenced_batch(vec![(1, unknown)])
        .await
        .unwrap();
    assert!(!replay.value[0].applied);
    assert_eq!(replay.commit_version, None);
    assert_eq!(
        state(&f.connection).await,
        before,
        "pruned sequence history remains trusted, not reapplied"
    );
}

#[tokio::test]
async fn cursor_creation_and_normalization_are_progress_but_identical_cursor_is_not() {
    let f = fixture().await;
    let created = f.store.apply_sequenced_batch(Vec::new()).await.unwrap();
    assert!(created.changes.is_empty());
    assert!(created.commit_version.is_some());
    assert_eq!(
        metadata(&f.connection, "last_server_seq").await.as_deref(),
        Some("0")
    );
    assert_eq!(
        f.store
            .apply_sequenced_batch(Vec::new())
            .await
            .unwrap()
            .commit_version,
        None
    );
    f.connection
        .call(|database| {
            database
                .execute(
                    "UPDATE sync_meta SET value = '0000' WHERE key = 'last_server_seq'",
                    [],
                )
                .map(|_| ())
        })
        .await
        .unwrap();
    let canonical = f.store.apply_sequenced_batch(Vec::new()).await.unwrap();
    assert_eq!(
        canonical.commit_version,
        created.commit_version.map(|version| version + 1)
    );
    assert_eq!(
        metadata(&f.connection, "last_server_seq").await.as_deref(),
        Some("0")
    );
    assert_eq!(f.notices.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn explicit_ack_reports_actual_delta_and_late_sequence_conflict_rolls_back_all_acks() {
    let f = fixture().await;
    notes_core::configure_sync(&f.connection, "https://remote.example")
        .await
        .unwrap();
    let first = f.op(1, page(id(1), PageKind::Note));
    let second = f.op(2, page(id(2), PageKind::Note));
    f.store
        .apply_batch(&[first.clone(), second.clone()], Origin::Local)
        .await
        .unwrap();
    let before = state(&f.connection).await;
    let error = f
        .store
        .acknowledge_server_ops(vec![(first.op_id, 7), (second.op_id, 7)])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("UNIQUE"));
    assert_eq!(state(&f.connection).await, before);
    assert_eq!(f.notices.lock().unwrap().len(), 1);
    let ack = f
        .store
        .acknowledge_server_ops(vec![(first.op_id, 7)])
        .await
        .unwrap();
    assert!(ack.changes.is_empty());
    assert!(ack.commit_version.is_some());
    assert_eq!(notes_core::sync_cursor(&f.connection).await.unwrap(), 0);
    let before = state(&f.connection).await;
    assert_eq!(
        f.store
            .acknowledge_server_ops(vec![(first.op_id, 7)])
            .await
            .unwrap()
            .commit_version,
        None
    );
    assert_eq!(
        f.store
            .acknowledge_server_ops(vec![(id(0xAB5E), 8)])
            .await
            .unwrap()
            .commit_version,
        None
    );
    assert_eq!(state(&f.connection).await, before);
    assert_eq!(f.notices.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn overflowing_sequences_are_rejected_before_any_apply_or_ack_write() {
    for overflow in [i64::MAX as u64 + 1, u64::MAX] {
        let f = fixture().await;
        notes_core::configure_sync(&f.connection, "https://remote.example")
            .await
            .unwrap();
        let first = f.op(1, page(id(1), PageKind::Note));
        let second = f.op(2, page(id(2), PageKind::Note));
        let before = state(&f.connection).await;
        let error = f
            .store
            .apply_sequenced_batch(vec![(1, first.clone()), (overflow, second.clone())])
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<CoreError>(),
            Some(CoreError::InvalidInput(_))
        ));
        assert_eq!(state(&f.connection).await, before);
        assert!(f.notices.lock().unwrap().is_empty());
        f.store
            .apply_batch(&[first.clone(), second.clone()], Origin::Local)
            .await
            .unwrap();
        let before = state(&f.connection).await;
        let error = f
            .store
            .acknowledge_server_ops(vec![(first.op_id, 1), (second.op_id, overflow)])
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<CoreError>(),
            Some(CoreError::InvalidInput(_))
        ));
        assert_eq!(state(&f.connection).await, before);
        assert_eq!(f.notices.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn late_gap_conflict_and_foreign_workspace_errors_rollback_events_and_all_progress() {
    for failure in ["gap", "foreign", "invalid"] {
        let f = fixture().await;
        let first = f.op(1, page(id(1), PageKind::Note));
        let mut second = f.op(2, page(id(2), PageKind::Note));
        if failure == "foreign" {
            second.workspace_uuid = id(0xF0_E19);
        }
        if failure == "invalid" {
            second.format_version = 0;
        }
        let before = state(&f.connection).await;
        let error = f
            .store
            .apply_sequenced_batch(vec![
                (1, first),
                (if failure == "gap" { 3 } else { 2 }, second),
            ])
            .await
            .unwrap_err();
        if failure == "gap" {
            assert!(matches!(
                error.downcast_ref::<CoreError>(),
                Some(CoreError::SyncSequenceGap {
                    expected: 2,
                    received: 3
                })
            ));
        }
        assert_eq!(state(&f.connection).await, before, "{failure}");
        assert!(f.notices.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn mixed_text_and_ink_apply_has_exact_final_facts_even_with_unchanged_page_timestamp() {
    let f = fixture().await;
    let ink = f.op(4, publication(id(2), id(10), Vec::new()));
    let operations = vec![
        f.op(1, page(id(1), PageKind::Note)),
        f.op(2, page(id(2), PageKind::Handwriting)),
        f.op(3, block(id(3), id(1), "Text")),
        ink.clone(),
    ];
    let committed = f
        .store
        .apply_batch(&operations, Origin::Remote)
        .await
        .unwrap();
    assert!(committed.value.iter().all(|outcome| outcome.applied));
    assert_eq!(committed.changes.changed_pages, [id(1), id(2)]);
    assert_eq!(committed.changes.handwriting_pages, [id(2)]);
    assert_eq!(committed.changes.blocks.len(), 1);
    assert_eq!(committed.changes.blocks[0].uuid, id(3));
    assert!(!committed.changes.history_changed);
    let unchanged_page = db::get_page(&f.connection, id(2)).await.unwrap().unwrap();
    let next = f.op(5, publication(id(2), id(11), Vec::new()));
    let committed = f.store.apply_batch(&[next], Origin::Remote).await.unwrap();
    assert_eq!(committed.changes.handwriting_pages, [id(2)]);
    assert_eq!(committed.changes.affected_pages, [id(2)]);
    assert!(committed.changes.changed_pages.is_empty());
    assert_eq!(
        db::get_page(&f.connection, id(2)).await.unwrap().unwrap(),
        unchanged_page
    );
    let mut duplicate_version = ink;
    duplicate_version.op_id = id(0xDE_D0);
    let committed = f
        .store
        .apply_batch(&[duplicate_version], Origin::Remote)
        .await
        .unwrap();
    assert!(committed.value[0].applied);
    assert!(committed.changes.is_empty());
    assert!(committed.commit_version.is_some());
    assert_eq!(f.notices.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn late_ink_cycle_rolls_back_mixed_primary_content_and_sequenced_progress() {
    for sequenced in [false, true] {
        let f = fixture().await;
        let ink_page = f.op(1, page(id(2), PageKind::Handwriting));
        f.store
            .apply_batch(&[ink_page], Origin::Remote)
            .await
            .unwrap();
        f.notices.lock().unwrap().clear();
        let before = state(&f.connection).await;
        let operations = vec![
            f.op(2, page(id(1), PageKind::Note)),
            f.op(3, block(id(3), id(1), "Text")),
            f.op(4, publication(id(2), id(10), vec![id(11)])),
            f.op(5, publication(id(2), id(11), vec![id(10)])),
        ];
        let result = if sequenced {
            f.store
                .apply_sequenced_batch(
                    operations
                        .into_iter()
                        .enumerate()
                        .map(|(index, operation)| (index as u64 + 1, operation))
                        .collect(),
                )
                .await
        } else {
            f.store.apply_batch(&operations, Origin::Remote).await
        };
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Cyclic handwriting history")
        );
        assert_eq!(state(&f.connection).await, before);
        assert!(f.notices.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn legacy_per_operation_graph_flags_remain_distinct_from_final_changeset() {
    let f = fixture().await;
    f.store
        .apply_batch(
            &[
                f.op(1, page(id(1), PageKind::Note)),
                f.op(2, block(id(2), id(1), "Initial")),
            ],
            Origin::Remote,
        )
        .await
        .unwrap();
    let committed = f
        .store
        .apply_batch(
            &[
                f.op(
                    3,
                    OpKind::BlockSetMarkdown(BlockSetMarkdown {
                        uuid: id(2),
                        markdown: "[[A]]".into(),
                    }),
                ),
                f.op(
                    4,
                    OpKind::BlockSetMarkdown(BlockSetMarkdown {
                        uuid: id(2),
                        markdown: "[[B]]".into(),
                    }),
                ),
            ],
            Origin::Remote,
        )
        .await
        .unwrap();
    assert!(committed.value[0].graph_changed);
    assert!(!committed.value[1].graph_changed);
    assert_eq!(committed.changes.graph_changed_content, [id(2)]);
    assert_eq!(
        db::get_block(&f.connection, id(2))
            .await
            .unwrap()
            .unwrap()
            .markdown,
        "[[B]]"
    );
}
