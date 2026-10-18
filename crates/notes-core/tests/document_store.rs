use notes_core::db::{self, DocumentUnitDraft, PageDocumentSnapshot};
use notes_core::store::{CommitNotice, NotesStore};
use notes_core::{BlockStyle, Connection, CoreError};
use std::sync::{Arc, Mutex};

struct Fixture {
    _directory: tempfile::TempDir,
    connection: Connection,
    store: NotesStore,
    before: PageDocumentSnapshot,
    notices: Arc<Mutex<Vec<CommitNotice>>>,
}

async fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(directory.path().join("notes.db")).await.unwrap();
    let page = db::create_page(&connection, "Document".into())
        .await
        .unwrap();
    let parent = db::create_block(
        &connection,
        page.uuid,
        None,
        None,
        BlockStyle::Paragraph,
        "Parent".into(),
    )
    .await
    .unwrap();
    db::create_block(
        &connection,
        page.uuid,
        Some(parent.uuid),
        None,
        BlockStyle::Bullet,
        "Child".into(),
    )
    .await
    .unwrap();
    db::create_block(
        &connection,
        page.uuid,
        None,
        Some(parent.uuid),
        BlockStyle::Quote,
        "Remove".into(),
    )
    .await
    .unwrap();
    notes_core::configure_sync(&connection, "https://sync.example")
        .await
        .unwrap();
    let before = db::get_page_document(&connection, page.uuid)
        .await
        .unwrap()
        .unwrap();
    let notices = Arc::new(Mutex::new(Vec::new()));
    let observed = notices.clone();
    let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
        observed.lock().unwrap().push(notice.clone());
    });
    Fixture {
        _directory: directory,
        connection,
        store,
        before,
        notices,
    }
}

async fn counters(connection: &Connection) -> (i64, i64, i64, i64, i64) {
    connection
        .call(|database| {
            database.query_row(
                "SELECT (SELECT COUNT(*) FROM applied_ops), (SELECT COUNT(*) FROM sync_outbox),
                (SELECT COUNT(*) FROM history_undo), (SELECT COUNT(*) FROM history_redo),
                (SELECT version FROM store_state WHERE singleton = 1)",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
        })
        .await
        .unwrap()
}

fn drafts(snapshot: &PageDocumentSnapshot) -> Vec<DocumentUnitDraft> {
    snapshot
        .blocks
        .iter()
        .map(|block| DocumentUnitDraft {
            previous_uuid: Some(block.uuid),
            parent_index: block.parent_uuid.map(|parent| {
                snapshot
                    .blocks
                    .iter()
                    .position(|block| block.uuid == parent)
                    .unwrap() as u32
            }),
            style: block.style,
            markdown: block.markdown.clone(),
        })
        .collect()
}

fn replacement(snapshot: &PageDocumentSnapshot) -> Vec<DocumentUnitDraft> {
    vec![
        DocumentUnitDraft {
            previous_uuid: Some(snapshot.blocks[1].uuid),
            parent_index: None,
            style: BlockStyle::Numbered,
            markdown: "Changed child".into(),
        },
        DocumentUnitDraft {
            previous_uuid: None,
            parent_index: None,
            style: BlockStyle::Quote,
            markdown: "New parent".into(),
        },
        DocumentUnitDraft {
            previous_uuid: None,
            parent_index: Some(1),
            style: BlockStyle::Bullet,
            markdown: "New child".into(),
        },
    ]
}

#[tokio::test]
async fn replacement_captures_final_snapshot_notice_and_map_facts_in_one_history_action() {
    let fixture = fixture().await;
    let before_counts = counters(&fixture.connection).await;
    let committed = fixture
        .store
        .replace_page_document(
            fixture.before.page_uuid,
            fixture.before.revision.clone(),
            replacement(&fixture.before),
        )
        .await
        .unwrap();
    assert_eq!(
        committed.value,
        db::get_page_document(&fixture.connection, fixture.before.page_uuid)
            .await
            .unwrap()
            .unwrap()
    );
    assert_eq!(committed.value.blocks.len(), 3);
    assert_eq!(
        committed.value.blocks[0].uuid,
        fixture.before.blocks[1].uuid
    );
    assert!(committed.value.blocks[0].parent_uuid.is_none());
    assert_eq!(
        committed.value.blocks[2].parent_uuid,
        Some(committed.value.blocks[1].uuid)
    );
    assert!(committed.changes.changed_pages.is_empty());
    assert_eq!(
        committed.changes.affected_pages,
        vec![fixture.before.page_uuid]
    );
    assert_eq!(committed.changes.blocks.len(), 5);
    for deleted in [fixture.before.blocks[0].uuid, fixture.before.blocks[2].uuid] {
        assert!(
            committed
                .changes
                .blocks
                .iter()
                .any(|block| block.uuid == deleted
                    && block.before.is_some()
                    && block.after.is_none())
        );
    }
    assert!(committed.changes.history_changed);
    assert_eq!(committed.commit_version, Some(before_counts.4 + 1));
    let after_counts = counters(&fixture.connection).await;
    assert!(after_counts.0 > before_counts.0);
    assert_eq!(
        after_counts.1 - before_counts.1,
        after_counts.0 - before_counts.0
    );
    assert_eq!(after_counts.2, before_counts.2 + 1);
    assert_eq!(fixture.notices.lock().unwrap().len(), 1);
    assert_eq!(
        fixture.notices.lock().unwrap()[0].changes,
        committed.changes
    );
    assert_eq!(
        fixture.notices.lock().unwrap()[0].commit_version,
        committed.commit_version.unwrap()
    );

    let changes = committed.changes.clone();
    let version = committed.commit_version;
    let mapped = committed.map_value(|snapshot| snapshot.blocks.len());
    assert_eq!(mapped.value, 3);
    assert_eq!(mapped.changes, changes);
    assert_eq!(mapped.commit_version, version);
}

#[tokio::test]
async fn exact_document_replacement_has_no_operations_history_commit_or_notice() {
    let fixture = fixture().await;
    let before_counts = counters(&fixture.connection).await;
    let committed = fixture
        .store
        .replace_page_document(
            fixture.before.page_uuid,
            fixture.before.revision.clone(),
            drafts(&fixture.before),
        )
        .await
        .unwrap();
    assert_eq!(committed.value, fixture.before);
    assert!(committed.changes.is_empty());
    assert_eq!(committed.commit_version, None);
    assert_eq!(counters(&fixture.connection).await, before_counts);
    assert!(fixture.notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn competing_documents_revalidate_the_revision_inside_the_writer() {
    let fixture = fixture().await;
    let before_counts = counters(&fixture.connection).await;
    let mut left = drafts(&fixture.before);
    let mut right = left.clone();
    left[0].markdown = "Left edit".into();
    right[0].markdown = "Right edit".into();
    let (left, right) = tokio::join!(
        fixture.store.replace_page_document(
            fixture.before.page_uuid,
            fixture.before.revision.clone(),
            left
        ),
        fixture.store.replace_page_document(
            fixture.before.page_uuid,
            fixture.before.revision.clone(),
            right
        ),
    );
    let (winner, rejected) = match (left, right) {
        (Ok(winner), Err(rejected)) | (Err(rejected), Ok(winner)) => (winner, rejected),
        outcomes => panic!("expected one commit and one conflict: {outcomes:?}"),
    };
    assert!(matches!(
        rejected.downcast_ref::<CoreError>(),
        Some(CoreError::Conflict(_))
    ));
    assert_eq!(
        winner.value,
        db::get_page_document(&fixture.connection, fixture.before.page_uuid)
            .await
            .unwrap()
            .unwrap()
    );
    let after_counts = counters(&fixture.connection).await;
    assert_eq!(
        after_counts,
        (
            before_counts.0 + 1,
            before_counts.1 + 1,
            before_counts.2 + 1,
            before_counts.3,
            before_counts.4 + 1
        )
    );
    assert_eq!(fixture.notices.lock().unwrap().len(), 1);

    let counts_before_retry = counters(&fixture.connection).await;
    let retry = fixture
        .store
        .replace_page_document(
            winner.value.page_uuid,
            fixture.before.revision,
            drafts(&winner.value),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            retry.downcast_ref::<CoreError>(),
            Some(CoreError::Conflict(_))
        ),
        "document retries require the current revision even for identical content"
    );
    assert_eq!(counters(&fixture.connection).await, counts_before_retry);
    assert_eq!(fixture.notices.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn failures_mid_apply_and_after_result_capture_roll_back_every_mutation_surface() {
    for stage in ["during apply", "after result"] {
        let fixture = fixture().await;
        let before_counts = counters(&fixture.connection).await;
        let before = serde_json::to_value(
            notes_core::export_sync_snapshot(&fixture.connection, 0)
                .await
                .unwrap(),
        )
        .unwrap();
        fixture
            .connection
            .call(move |database| {
                database.execute_batch(if stage == "during apply" {
            "CREATE TEMP TRIGGER reject_document_insert BEFORE INSERT ON blocks
             WHEN NEW.markdown = 'Reject child'
             BEGIN SELECT RAISE(ABORT, 'injected document failure'); END;"
        } else {
            "CREATE TEMP TRIGGER reject_document_commit BEFORE UPDATE OF version ON store_state
             BEGIN SELECT RAISE(ABORT, 'injected document failure'); END;"
        })
            })
            .await
            .unwrap();
        let mut units = replacement(&fixture.before);
        if stage == "during apply" {
            units[2].markdown = "Reject child".into();
        }
        let error = fixture
            .store
            .replace_page_document(
                fixture.before.page_uuid,
                fixture.before.revision.clone(),
                units,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("injected document failure"));
        assert_eq!(counters(&fixture.connection).await, before_counts);
        assert_eq!(
            serde_json::to_value(
                notes_core::export_sync_snapshot(&fixture.connection, 0)
                    .await
                    .unwrap()
            )
            .unwrap(),
            before
        );
        assert_eq!(
            db::get_page_document(&fixture.connection, fixture.before.page_uuid)
                .await
                .unwrap(),
            Some(fixture.before)
        );
        assert!(fixture.notices.lock().unwrap().is_empty());
    }
}
