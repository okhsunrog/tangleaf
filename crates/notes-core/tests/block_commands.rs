use notes_core::db::{self, Block, BlockContent};
use notes_core::store::{CommitNotice, NotesStore};
use notes_core::{BlockStyle, Connection, CoreError, ReorderDirection, TaskState};
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::sync::{Arc, Mutex, mpsc};
use std::task::Poll;

struct Fixture {
    directory: tempfile::TempDir,
    connection: Connection,
    block: Block,
    store: NotesStore,
    notices: Arc<Mutex<Vec<CommitNotice>>>,
}

async fn fixture(style: BlockStyle) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(directory.path().join("notes.db")).await.unwrap();
    let page = db::create_page(&connection, "Block commands".into())
        .await
        .unwrap();
    let block = db::create_block(&connection, page.uuid, None, None, style, "Original".into())
        .await
        .unwrap();
    notes_core::configure_sync(&connection, "https://sync.example")
        .await
        .unwrap();
    connection
        .call(|database| {
            database.execute("DELETE FROM history_undo", [])?;
            database.execute("DELETE FROM history_redo", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let notices = Arc::new(Mutex::new(Vec::new()));
    let observed = notices.clone();
    let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
        observed.lock().unwrap().push(notice.clone());
    });
    Fixture {
        directory,
        connection,
        block,
        store,
        notices,
    }
}

fn content(markdown: &str) -> BlockContent {
    BlockContent {
        markdown: markdown.into(),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Bookkeeping {
    applied: i64,
    outbox: i64,
    undo: i64,
    redo: i64,
    version: i64,
    hlc: String,
}

async fn bookkeeping(connection: &Connection) -> Bookkeeping {
    connection
        .call(|database| {
            database.query_row(
                "SELECT (SELECT COUNT(*) FROM applied_ops),
                (SELECT COUNT(*) FROM sync_outbox),
                (SELECT COUNT(*) FROM history_undo),
                (SELECT COUNT(*) FROM history_redo),
                (SELECT version FROM store_state),
                (SELECT value FROM sync_meta WHERE key = 'last_hlc')",
                [],
                |row| {
                    Ok(Bookkeeping {
                        applied: row.get(0)?,
                        outbox: row.get(1)?,
                        undo: row.get(2)?,
                        redo: row.get(3)?,
                        version: row.get(4)?,
                        hlc: row.get(5)?,
                    })
                },
            )
        })
        .await
        .unwrap()
}

/// Hold the worker while command futures submit, so their admission order is
/// deterministic and the test does not depend on Tokio scheduling or sleeps.
async fn block_worker(connection: &Connection) -> (mpsc::Sender<()>, tokio::task::JoinHandle<()>) {
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let connection = connection.clone();
    let blocker = tokio::spawn(async move {
        connection
            .call(move |_| {
                let _ = entered_tx.send(());
                let _ = release_rx.recv();
                Ok(())
            })
            .await
            .unwrap();
    });
    entered_rx.await.unwrap();
    (release_tx, blocker)
}

async fn enqueue<F: Future>(mut future: Pin<&mut F>) {
    poll_fn(|context| {
        assert!(future.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn competing_guarded_saves_have_one_winner_and_stale_equal_retry_is_silent() {
    let f = fixture(BlockStyle::Paragraph).await;
    let before = bookkeeping(&f.connection).await;
    let (release, blocker) = block_worker(&f.connection).await;
    let mut first = std::pin::pin!(f.store.set_block_content_if_revision(
        f.block.uuid,
        content("First [[Target]]"),
        f.block.markdown_revision.clone(),
    ));
    let mut second = std::pin::pin!(f.store.set_block_content_if_revision(
        f.block.uuid,
        content("Second"),
        f.block.markdown_revision.clone(),
    ));
    enqueue(first.as_mut()).await;
    enqueue(second.as_mut()).await;
    release.send(()).unwrap();
    blocker.await.unwrap();
    let (first, second) = tokio::join!(first, second);
    let committed = first.unwrap();
    assert!(matches!(
        second.unwrap_err().downcast_ref::<CoreError>(),
        Some(CoreError::Conflict(_))
    ));
    assert_eq!(committed.value.0.markdown, "First [[Target]]");
    assert!(committed.value.1);
    assert_eq!(
        db::get_block(&f.connection, f.block.uuid)
            .await
            .unwrap()
            .unwrap(),
        committed.value.0
    );
    assert!(committed.changes.changed_pages.is_empty());
    assert_eq!(committed.changes.affected_pages, [f.block.page_uuid]);
    assert_eq!(committed.changes.graph_changed_content, [f.block.uuid]);
    assert!(!committed.changes.history_changed);
    assert_eq!(committed.changes.blocks.len(), 1);
    let change = &committed.changes.blocks[0];
    assert_eq!(change.uuid, f.block.uuid);
    assert_eq!(change.before, change.after);
    let after = bookkeeping(&f.connection).await;
    assert_eq!(after.applied, before.applied + 1);
    assert_eq!(after.outbox, before.outbox + 1);
    assert_eq!((after.undo, after.redo), (0, 0));
    assert_eq!(after.version, before.version + 1);
    let notices = f.notices.lock().unwrap().clone();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].changes, committed.changes);
    assert_eq!(Some(notices[0].commit_version), committed.commit_version);

    let retry = f
        .store
        .set_block_content_if_revision(
            f.block.uuid,
            content("First [[Target]]"),
            f.block.markdown_revision,
        )
        .await
        .unwrap();
    assert_eq!(retry.value.0, committed.value.0);
    assert!(!retry.value.1);
    assert!(retry.changes.is_empty());
    assert_eq!(retry.commit_version, None);
    assert_eq!(bookkeeping(&f.connection).await, after);
    assert_eq!(f.notices.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn queued_unchecked_saves_return_their_own_committed_rows() {
    let f = fixture(BlockStyle::Paragraph).await;
    let (release, blocker) = block_worker(&f.connection).await;
    let mut first = std::pin::pin!(f.store.set_block_content(f.block.uuid, content("First")));
    let mut second = std::pin::pin!(f.store.set_block_content(f.block.uuid, content("Second")));
    enqueue(first.as_mut()).await;
    enqueue(second.as_mut()).await;
    release.send(()).unwrap();
    blocker.await.unwrap();
    let (first, second) = tokio::join!(first, second);
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.value.0.markdown, "First");
    assert_eq!(second.value.0.markdown, "Second");
    assert_ne!(
        first.value.0.markdown_revision,
        second.value.0.markdown_revision
    );
    // Legacy reference-change booleans stay narrow, while graph label changes
    // are represented by the exact store facts.
    assert!(!first.value.1);
    assert!(!second.value.1);
    assert_eq!(first.changes.graph_changed_content, [f.block.uuid]);
    assert_eq!(second.changes.graph_changed_content, [f.block.uuid]);
    assert_eq!(
        second.commit_version,
        first.commit_version.map(|version| version + 1)
    );
    assert_eq!(f.notices.lock().unwrap().len(), 2);
    assert_eq!(
        db::get_block(&f.connection, f.block.uuid)
            .await
            .unwrap()
            .unwrap(),
        second.value.0
    );
    assert_eq!(
        db::history_status(&f.connection).await.unwrap().undo_count,
        0
    );
}

#[tokio::test]
async fn task_precondition_observes_the_preceding_style_command_and_undo_uses_its_inverse() {
    let original_style = BlockStyle::task(TaskState::Todo);
    let f = fixture(original_style).await;
    let before = bookkeeping(&f.connection).await;
    let (release, blocker) = block_worker(&f.connection).await;
    let mut style = std::pin::pin!(f.store.set_block_style(f.block.uuid, BlockStyle::Paragraph));
    let mut task = std::pin::pin!(f.store.set_task_state(f.block.uuid, TaskState::Done));
    enqueue(style.as_mut()).await;
    enqueue(task.as_mut()).await;
    release.send(()).unwrap();
    blocker.await.unwrap();
    let (style, task) = tokio::join!(style, task);
    let style = style.unwrap();
    assert_eq!(style.value.style, BlockStyle::Paragraph);
    assert!(style.changes.history_changed);
    assert_eq!(style.changes.blocks.len(), 1);
    assert!(matches!(
        task.unwrap_err().downcast_ref::<CoreError>(),
        Some(CoreError::InvalidInput(_))
    ));
    let after = bookkeeping(&f.connection).await;
    assert_eq!(after.outbox, before.outbox + 1);
    assert_eq!(after.undo, 1);
    assert_eq!(after.version, before.version + 1);
    assert_eq!(f.notices.lock().unwrap().len(), 1);
    assert_eq!(
        f.store.undo().await.unwrap().value,
        db::HistoryMoveResult::Applied
    );
    assert_eq!(
        db::get_block(&f.connection, f.block.uuid)
            .await
            .unwrap()
            .unwrap()
            .style,
        original_style
    );
}

#[tokio::test]
async fn style_and_task_noops_and_missing_block_errors_do_not_publish_or_write() {
    let style = BlockStyle::task(TaskState::Todo);
    let f = fixture(style).await;
    let before = bookkeeping(&f.connection).await;
    let same_style = f.store.set_block_style(f.block.uuid, style).await.unwrap();
    let same_task = f
        .store
        .set_task_state(f.block.uuid, TaskState::Todo)
        .await
        .unwrap();
    for no_op in [same_style, same_task] {
        assert_eq!(no_op.value, f.block);
        assert!(no_op.changes.is_empty());
        assert_eq!(no_op.commit_version, None);
    }
    let missing = uuid::Uuid::now_v7();
    let errors = [
        f.store
            .set_block_content(missing, content("Missing"))
            .await
            .unwrap_err(),
        f.store
            .set_block_content_if_revision(
                missing,
                content("Missing"),
                f.block.markdown_revision.clone(),
            )
            .await
            .unwrap_err(),
        f.store
            .set_block_style(missing, BlockStyle::Bullet)
            .await
            .unwrap_err(),
        f.store
            .set_task_state(missing, TaskState::Done)
            .await
            .unwrap_err(),
    ];
    for error in errors {
        assert!(matches!(
            error.downcast_ref::<CoreError>(),
            Some(CoreError::NotFound(_))
        ));
    }
    assert_eq!(bookkeeping(&f.connection).await, before);
    assert!(f.notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn late_history_failure_rolls_back_style_outbox_clock_and_notification() {
    let f = fixture(BlockStyle::Paragraph).await;
    f.connection
        .call(|database| {
            database.execute_batch(
                "CREATE TRIGGER reject_block_history BEFORE INSERT ON history_undo
         BEGIN SELECT RAISE(ABORT, 'injected history failure'); END;",
            )
        })
        .await
        .unwrap();
    let before = bookkeeping(&f.connection).await;
    let error = f
        .store
        .set_block_style(f.block.uuid, BlockStyle::task(TaskState::Done))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("injected history failure"));
    assert_eq!(
        db::get_block(&f.connection, f.block.uuid)
            .await
            .unwrap()
            .unwrap(),
        f.block
    );
    assert_eq!(bookkeeping(&f.connection).await, before);
    assert!(f.notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn late_outbox_failure_rolls_back_markdown_links_and_notification() {
    let f = fixture(BlockStyle::Paragraph).await;
    f.connection
        .call(|database| {
            database.execute_batch(
                "CREATE TRIGGER reject_block_outbox BEFORE INSERT ON sync_outbox
         BEGIN SELECT RAISE(ABORT, 'injected outbox failure'); END;",
            )
        })
        .await
        .unwrap();
    let before = bookkeeping(&f.connection).await;
    let error = f
        .store
        .set_block_content_if_revision(
            f.block.uuid,
            content("[[Target]]"),
            f.block.markdown_revision.clone(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("injected outbox failure"));
    assert_eq!(
        db::get_block(&f.connection, f.block.uuid)
            .await
            .unwrap()
            .unwrap(),
        f.block
    );
    assert_eq!(bookkeeping(&f.connection).await, before);
    assert_eq!(
        f.connection
            .call(|database| database
                .query_row("SELECT COUNT(*) FROM page_links", [], |row| row
                    .get::<_, i64>(0),))
            .await
            .unwrap(),
        0
    );
    assert!(f.notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn observer_reads_final_rows_outbox_and_history_from_another_connection() {
    let f = fixture(BlockStyle::Paragraph).await;
    let path = f.directory.path().join("notes.db");
    let uuid = f.block.uuid;
    let observed = Arc::new(Mutex::new(Vec::new()));
    let observed_rows = observed.clone();
    let store = NotesStore::new(f.connection.clone()).with_commit_observer(move |notice| {
        let reader = rusqlite::Connection::open(&path).unwrap();
        let row = reader
            .query_row(
                "SELECT markdown, style, (SELECT COUNT(*) FROM sync_outbox),
                    (SELECT COUNT(*) FROM history_undo), (SELECT version FROM store_state)
             FROM blocks WHERE uuid = ?1",
                [uuid],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, BlockStyle>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .unwrap();
        observed_rows.lock().unwrap().push((notice.clone(), row));
    });
    let saved = store
        .set_block_content(f.block.uuid, content("Saved"))
        .await
        .unwrap();
    let styled = store.set_task_state(f.block.uuid, TaskState::Done).await;
    assert!(styled.is_err());
    let styled = store
        .set_block_style(f.block.uuid, BlockStyle::task(TaskState::Todo))
        .await
        .unwrap();
    let task = store
        .set_task_state(f.block.uuid, TaskState::Done)
        .await
        .unwrap();
    let observed = observed.lock().unwrap();
    assert_eq!(observed.len(), 3);
    let (notice, row) = &observed[0];
    assert_eq!(notice.changes, saved.changes);
    assert_eq!(
        row,
        &(
            "Saved".into(),
            BlockStyle::Paragraph,
            1,
            0,
            saved.commit_version.unwrap()
        )
    );
    let (notice, row) = &observed[1];
    assert_eq!(notice.changes, styled.changes);
    assert_eq!(
        row,
        &(
            "Saved".into(),
            BlockStyle::task(TaskState::Todo),
            2,
            1,
            styled.commit_version.unwrap()
        )
    );
    let (notice, row) = &observed[2];
    assert_eq!(notice.changes, task.changes);
    assert_eq!(
        row,
        &(
            "Saved".into(),
            BlockStyle::task(TaskState::Done),
            3,
            2,
            task.commit_version.unwrap()
        )
    );
}

#[tokio::test]
async fn legacy_adapters_keep_operations_and_reference_change_flags_without_observer() {
    let f = fixture(BlockStyle::Paragraph).await;
    let saved = db::set_block_content_if_revision_with_ops(
        &f.connection,
        f.block.uuid,
        content("Plain text"),
        f.block.markdown_revision.clone(),
    )
    .await
    .unwrap();
    assert_eq!(saved.operations.len(), 1);
    assert!(!saved.value.1);
    let repeated = db::set_block_content_if_revision_with_ops(
        &f.connection,
        f.block.uuid,
        content("Plain text"),
        f.block.markdown_revision,
    )
    .await
    .unwrap();
    assert!(repeated.operations.is_empty());
    assert!(!repeated.value.1);
    let (saved, graph_changed) =
        db::set_block_content(&f.connection, f.block.uuid, content("[[Target]]"))
            .await
            .unwrap();
    assert_eq!(saved.markdown, "[[Target]]");
    assert!(graph_changed);
    let style = db::set_block_style_with_ops(
        &f.connection,
        f.block.uuid,
        BlockStyle::task(TaskState::Todo),
    )
    .await
    .unwrap();
    assert_eq!(style.operations.len(), 1);
    let task = db::set_task_state_with_ops(&f.connection, f.block.uuid, TaskState::Done)
        .await
        .unwrap();
    assert_eq!(task.operations.len(), 1);
    let repeated = db::set_task_state_with_ops(&f.connection, f.block.uuid, TaskState::Done)
        .await
        .unwrap();
    assert!(repeated.operations.is_empty());
    assert!(f.notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn queued_appends_plan_against_the_previous_commit_and_publish_only_created_blocks() {
    let f = fixture(BlockStyle::Paragraph).await;
    let before = bookkeeping(&f.connection).await;
    let (release, blocker) = block_worker(&f.connection).await;
    let mut first = std::pin::pin!(f.store.create_block(
        f.block.page_uuid,
        None,
        None,
        BlockStyle::Bullet,
        "First".into(),
    ));
    let mut second = std::pin::pin!(f.store.create_block(
        f.block.page_uuid,
        None,
        None,
        BlockStyle::Bullet,
        "Second".into(),
    ));
    enqueue(first.as_mut()).await;
    enqueue(second.as_mut()).await;
    release.send(()).unwrap();
    blocker.await.unwrap();
    let (first, second) = tokio::join!(first, second);
    let first = first.unwrap();
    let second = second.unwrap();
    let blocks = db::list_block_children(&f.connection, f.block.page_uuid, None)
        .await
        .unwrap();
    assert_eq!(
        blocks.iter().map(|block| block.uuid).collect::<Vec<_>>(),
        [f.block.uuid, first.value.uuid, second.value.uuid]
    );
    assert!(
        blocks
            .windows(2)
            .all(|pair| pair[0].order_key < pair[1].order_key)
    );
    for committed in [&first, &second] {
        assert_eq!(committed.changes.blocks.len(), 1);
        assert_eq!(committed.changes.blocks[0].uuid, committed.value.uuid);
        assert!(committed.changes.blocks[0].before.is_none());
        assert!(committed.changes.blocks[0].after.is_some());
        assert_eq!(committed.changes.affected_pages, [f.block.page_uuid]);
        assert!(committed.changes.changed_pages.is_empty());
        assert!(committed.changes.history_changed);
    }
    assert_eq!(f.notices.lock().unwrap().len(), 2);
    let after = bookkeeping(&f.connection).await;
    assert_eq!(after.applied, before.applied + 2);
    assert_eq!(after.outbox, before.outbox + 2);
    assert_eq!(after.undo, 2);
    assert_eq!(after.version, before.version + 2);
    for uuid in [second.value.uuid, first.value.uuid] {
        assert_eq!(
            f.store.undo().await.unwrap().value,
            db::HistoryMoveResult::Applied
        );
        assert!(db::get_block(&f.connection, uuid).await.unwrap().is_none());
    }
}

#[tokio::test]
async fn create_then_delete_parent_checks_leaf_guard_after_the_child_commit() {
    let f = fixture(BlockStyle::Paragraph).await;
    let before = bookkeeping(&f.connection).await;
    let (release, blocker) = block_worker(&f.connection).await;
    let mut create = std::pin::pin!(f.store.create_block(
        f.block.page_uuid,
        Some(f.block.uuid),
        None,
        BlockStyle::Bullet,
        "Child".into(),
    ));
    let mut delete = std::pin::pin!(f.store.delete_block(f.block.uuid));
    enqueue(create.as_mut()).await;
    enqueue(delete.as_mut()).await;
    release.send(()).unwrap();
    blocker.await.unwrap();
    let (create, delete) = tokio::join!(create, delete);
    let child = create.unwrap();
    let delete = delete.unwrap();
    assert!(!delete.value);
    assert!(delete.changes.is_empty());
    assert_eq!(delete.commit_version, None);
    assert_eq!(
        db::get_block(&f.connection, child.value.uuid)
            .await
            .unwrap()
            .unwrap()
            .parent_uuid,
        Some(f.block.uuid)
    );
    assert_eq!(
        db::get_block(&f.connection, f.block.uuid)
            .await
            .unwrap()
            .unwrap(),
        f.block
    );
    let after = bookkeeping(&f.connection).await;
    assert_eq!(after.outbox, before.outbox + 1);
    assert_eq!(after.undo, before.undo + 1);
    assert_eq!(after.version, before.version + 1);
    assert_eq!(f.notices.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn delete_then_create_under_removed_parent_is_typed_and_does_not_resurrect_content() {
    let f = fixture(BlockStyle::Paragraph).await;
    let before = bookkeeping(&f.connection).await;
    let (release, blocker) = block_worker(&f.connection).await;
    let mut delete = std::pin::pin!(f.store.delete_block(f.block.uuid));
    let mut create = std::pin::pin!(f.store.create_block(
        f.block.page_uuid,
        Some(f.block.uuid),
        None,
        BlockStyle::Bullet,
        "Child".into(),
    ));
    enqueue(delete.as_mut()).await;
    enqueue(create.as_mut()).await;
    release.send(()).unwrap();
    blocker.await.unwrap();
    let (delete, create) = tokio::join!(delete, create);
    let delete = delete.unwrap();
    assert!(delete.value);
    let error = create.unwrap_err();
    assert!(
        matches!(error.downcast_ref::<CoreError>(), Some(CoreError::NotFound(message)) if message == "parent block was not found")
    );
    assert_eq!(delete.changes.blocks.len(), 1);
    assert_eq!(delete.changes.blocks[0].uuid, f.block.uuid);
    assert!(delete.changes.blocks[0].before.is_some());
    assert!(delete.changes.blocks[0].after.is_none());
    assert_eq!(delete.changes.affected_pages, [f.block.page_uuid]);
    assert!(delete.changes.history_changed);
    assert!(
        db::list_block_children(&f.connection, f.block.page_uuid, None)
            .await
            .unwrap()
            .is_empty()
    );
    let after = bookkeeping(&f.connection).await;
    assert_eq!(after.outbox, before.outbox + 1);
    assert_eq!(after.undo, 1);
    assert_eq!(after.version, before.version + 1);
    assert_eq!(f.notices.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn create_validation_and_absent_delete_leave_every_mutation_surface_unchanged() {
    let f = fixture(BlockStyle::Paragraph).await;
    let foreign_page = db::create_page(&f.connection, "Foreign".into())
        .await
        .unwrap();
    let foreign = db::create_block(
        &f.connection,
        foreign_page.uuid,
        None,
        None,
        BlockStyle::Paragraph,
        "Foreign".into(),
    )
    .await
    .unwrap();
    let before = bookkeeping(&f.connection).await;
    let missing = uuid::Uuid::now_v7();
    for (page, parent, after, expected_kind) in [
        (missing, None, None, "not_found"),
        (f.block.page_uuid, Some(missing), None, "not_found"),
        (f.block.page_uuid, Some(foreign.uuid), None, "conflict"),
        (f.block.page_uuid, None, Some(foreign.uuid), "invalid"),
    ] {
        let error = f
            .store
            .create_block(page, parent, after, BlockStyle::Bullet, "Rejected".into())
            .await
            .unwrap_err();
        assert!(matches!(
            (error.downcast_ref::<CoreError>(), expected_kind),
            (Some(CoreError::NotFound(_)), "not_found")
                | (Some(CoreError::Conflict(_)), "conflict")
                | (Some(CoreError::InvalidInput(_)), "invalid")
        ));
    }
    let absent = f.store.delete_block(missing).await.unwrap();
    assert!(!absent.value);
    assert_eq!(absent.commit_version, None);
    assert!(absent.changes.is_empty());
    assert_eq!(bookkeeping(&f.connection).await, before);
    assert!(f.notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn create_and_delete_history_failures_roll_back_content_and_bookkeeping() {
    for deleting in [false, true] {
        let f = fixture(BlockStyle::Paragraph).await;
        f.connection
            .call(|database| {
                database.execute_batch(
                    "CREATE TRIGGER reject_structure_history BEFORE INSERT ON history_undo
             BEGIN SELECT RAISE(ABORT, 'injected structure history failure'); END;",
                )
            })
            .await
            .unwrap();
        let before = bookkeeping(&f.connection).await;
        let error = if deleting {
            f.store.delete_block(f.block.uuid).await.unwrap_err()
        } else {
            f.store
                .create_block(
                    f.block.page_uuid,
                    None,
                    None,
                    BlockStyle::Bullet,
                    "Rejected".into(),
                )
                .await
                .unwrap_err()
        };
        assert!(
            error
                .to_string()
                .contains("injected structure history failure")
        );
        assert_eq!(
            db::list_block_children(&f.connection, f.block.page_uuid, None)
                .await
                .unwrap(),
            [f.block]
        );
        assert_eq!(bookkeeping(&f.connection).await, before);
        assert!(f.notices.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn middle_insertion_keeps_neighbor_keys_and_immediate_history_policy() {
    let f = fixture(BlockStyle::Paragraph).await;
    let second = db::create_block(
        &f.connection,
        f.block.page_uuid,
        None,
        None,
        BlockStyle::Bullet,
        "Second".into(),
    )
    .await
    .unwrap();
    let inserted = db::create_block_with_ops(
        &f.connection,
        f.block.page_uuid,
        None,
        Some(f.block.uuid),
        BlockStyle::Bullet,
        "Middle".into(),
    )
    .await
    .unwrap();
    assert_eq!(inserted.operations.len(), 1);
    let blocks = db::list_block_children(&f.connection, f.block.page_uuid, None)
        .await
        .unwrap();
    assert_eq!(
        blocks.iter().map(|block| block.uuid).collect::<Vec<_>>(),
        [f.block.uuid, inserted.value.uuid, second.uuid]
    );
    assert_eq!(blocks[0], f.block);
    assert_eq!(blocks[2], second);
    assert!(blocks[0].order_key < blocks[1].order_key);
    assert!(blocks[1].order_key < blocks[2].order_key);
    let payload = f
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
    let payload: serde_json::Value =
        notes_core::operation::decode_persisted_envelope(&payload).unwrap();
    assert_eq!(payload["apply_policy"], "immediate");
    assert!(f.notices.lock().unwrap().is_empty());
    assert!(
        db::delete_block_with_ops(&f.connection, inserted.value.uuid)
            .await
            .unwrap()
            .value
    );
}

async fn child(f: &Fixture, parent: Option<uuid::Uuid>, markdown: &str) -> Block {
    db::create_block(
        &f.connection,
        f.block.page_uuid,
        parent,
        None,
        BlockStyle::Bullet,
        markdown.into(),
    )
    .await
    .unwrap()
}

async fn root_uuids(f: &Fixture) -> Vec<uuid::Uuid> {
    db::list_block_children(&f.connection, f.block.page_uuid, None)
        .await
        .unwrap()
        .into_iter()
        .map(|block| block.uuid)
        .collect()
}

#[tokio::test]
async fn queued_reorders_use_current_sibling_order_and_return_each_intermediate_position() {
    let f = fixture(BlockStyle::Paragraph).await;
    let second = child(&f, None, "Second").await;
    let third = child(&f, None, "Third").await;
    let before = bookkeeping(&f.connection).await;
    let (release, blocker) = block_worker(&f.connection).await;
    let mut first = std::pin::pin!(f.store.reorder_block(third.uuid, ReorderDirection::Up));
    let mut next = std::pin::pin!(
        f.store
            .move_block_in_direction(third.uuid, ReorderDirection::Up)
    );
    enqueue(first.as_mut()).await;
    enqueue(next.as_mut()).await;
    release.send(()).unwrap();
    blocker.await.unwrap();
    let (first, next) = tokio::join!(first, next);
    let first = first.unwrap();
    let next = next.unwrap();
    assert!(f.block.order_key < first.value.order_key);
    assert!(first.value.order_key < second.order_key);
    assert!(next.value.order_key < f.block.order_key);
    assert_eq!(
        root_uuids(&f).await,
        [third.uuid, f.block.uuid, second.uuid]
    );
    for committed in [&first, &next] {
        assert!(committed.changes.history_changed);
        assert_eq!(committed.changes.affected_pages, [f.block.page_uuid]);
        assert!(committed.changes.changed_pages.is_empty());
        assert!(committed.changes.graph_changed_content.is_empty());
        let source = committed
            .changes
            .blocks
            .iter()
            .find(|change| change.uuid == third.uuid)
            .unwrap();
        assert_ne!(source.before, source.after);
        assert_eq!(
            source.after.as_ref().unwrap().order_key,
            committed.value.order_key
        );
    }
    let after = bookkeeping(&f.connection).await;
    assert_eq!(after.outbox, before.outbox + 2);
    assert_eq!(after.undo, before.undo + 2);
    assert_eq!(after.version, before.version + 2);
    assert_eq!(f.notices.lock().unwrap().len(), 2);
    assert_eq!(
        f.store.undo().await.unwrap().value,
        db::HistoryMoveResult::Applied
    );
    assert_eq!(
        root_uuids(&f).await,
        [f.block.uuid, third.uuid, second.uuid]
    );
    assert_eq!(
        f.store.undo().await.unwrap().value,
        db::HistoryMoveResult::Applied
    );
    assert_eq!(
        root_uuids(&f).await,
        [f.block.uuid, second.uuid, third.uuid]
    );
}

#[tokio::test]
async fn repeated_indent_rechecks_its_new_siblings_and_outdent_reports_both_placements() {
    let f = fixture(BlockStyle::Paragraph).await;
    let second = child(&f, None, "Second").await;
    let before = bookkeeping(&f.connection).await;
    let (release, blocker) = block_worker(&f.connection).await;
    let mut first = std::pin::pin!(f.store.indent_block(second.uuid));
    let mut next = std::pin::pin!(f.store.indent_block(second.uuid));
    enqueue(first.as_mut()).await;
    enqueue(next.as_mut()).await;
    release.send(()).unwrap();
    blocker.await.unwrap();
    let (first, next) = tokio::join!(first, next);
    let first = first.unwrap();
    let next = next.unwrap();
    assert_eq!(first.value.parent_uuid, Some(f.block.uuid));
    assert_eq!(next.value, first.value);
    assert_eq!(next.commit_version, None);
    assert!(next.changes.is_empty());
    assert_eq!(first.changes.graph_changed_content, [second.uuid]);
    let moved = first
        .changes
        .blocks
        .iter()
        .find(|change| change.uuid == second.uuid)
        .unwrap();
    assert_eq!(moved.before.as_ref().unwrap().parent_uuid, None);
    assert_eq!(
        moved.after.as_ref().unwrap().parent_uuid,
        Some(f.block.uuid)
    );
    let after = bookkeeping(&f.connection).await;
    assert_eq!(after.outbox, before.outbox + 1);
    assert_eq!(after.undo, before.undo + 1);
    assert_eq!(after.version, before.version + 1);
    assert_eq!(f.notices.lock().unwrap().len(), 1);

    let outdented = f.store.outdent_block(second.uuid).await.unwrap();
    assert_eq!(outdented.value.parent_uuid, None);
    assert_eq!(root_uuids(&f).await, [f.block.uuid, second.uuid]);
    let moved = outdented
        .changes
        .blocks
        .iter()
        .find(|change| change.uuid == second.uuid)
        .unwrap();
    assert_eq!(
        moved.before.as_ref().unwrap().parent_uuid,
        Some(f.block.uuid)
    );
    assert_eq!(moved.after.as_ref().unwrap().parent_uuid, None);
    assert_eq!(outdented.changes.affected_pages, [f.block.page_uuid]);
    assert!(outdented.changes.history_changed);
}

#[tokio::test]
async fn queued_outdents_plan_from_the_latest_parent_and_undo_restores_each_level() {
    let f = fixture(BlockStyle::Paragraph).await;
    let parent = child(&f, Some(f.block.uuid), "Parent").await;
    let nested = child(&f, Some(parent.uuid), "Nested").await;
    let (release, blocker) = block_worker(&f.connection).await;
    let mut first = std::pin::pin!(f.store.outdent_block(nested.uuid));
    let mut next = std::pin::pin!(f.store.outdent_block(nested.uuid));
    enqueue(first.as_mut()).await;
    enqueue(next.as_mut()).await;
    release.send(()).unwrap();
    blocker.await.unwrap();
    let (first, next) = tokio::join!(first, next);
    let first = first.unwrap();
    let next = next.unwrap();
    assert_eq!(first.value.parent_uuid, Some(f.block.uuid));
    assert_eq!(next.value.parent_uuid, None);
    assert_eq!(root_uuids(&f).await, [f.block.uuid, nested.uuid]);
    assert_eq!(f.notices.lock().unwrap().len(), 2);
    for expected_parent in [Some(f.block.uuid), Some(parent.uuid)] {
        assert_eq!(
            f.store.undo().await.unwrap().value,
            db::HistoryMoveResult::Applied
        );
        assert_eq!(
            db::get_block(&f.connection, nested.uuid)
                .await
                .unwrap()
                .unwrap()
                .parent_uuid,
            expected_parent
        );
    }
}

#[tokio::test]
async fn a_second_competing_move_cannot_create_a_cycle_after_the_first_move_commits() {
    let f = fixture(BlockStyle::Paragraph).await;
    let second = child(&f, None, "Second").await;
    let before = bookkeeping(&f.connection).await;
    let (release, blocker) = block_worker(&f.connection).await;
    let mut first = std::pin::pin!(f.store.move_block(f.block.uuid, Some(second.uuid), None));
    let mut next = std::pin::pin!(f.store.move_block(second.uuid, Some(f.block.uuid), None));
    enqueue(first.as_mut()).await;
    enqueue(next.as_mut()).await;
    release.send(()).unwrap();
    blocker.await.unwrap();
    let (first, next) = tokio::join!(first, next);
    let first = first.unwrap();
    assert_eq!(first.value.parent_uuid, Some(second.uuid));
    assert!(matches!(
        next.unwrap_err().downcast_ref::<CoreError>(),
        Some(CoreError::Conflict(_))
    ));
    assert_eq!(root_uuids(&f).await, [second.uuid]);
    assert_eq!(
        db::get_block(&f.connection, second.uuid)
            .await
            .unwrap()
            .unwrap(),
        second
    );
    let after = bookkeeping(&f.connection).await;
    assert_eq!(after.outbox, before.outbox + 1);
    assert_eq!(after.undo, before.undo + 1);
    assert_eq!(after.version, before.version + 1);
    assert_eq!(f.notices.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn structural_boundary_noops_and_typed_guards_leave_all_bookkeeping_unchanged() {
    let f = fixture(BlockStyle::Paragraph).await;
    let second = child(&f, None, "Second").await;
    let nested = child(&f, Some(second.uuid), "Nested").await;
    let foreign_page = db::create_page(&f.connection, "Foreign".into())
        .await
        .unwrap();
    let foreign = db::create_block(
        &f.connection,
        foreign_page.uuid,
        None,
        None,
        BlockStyle::Bullet,
        "Foreign".into(),
    )
    .await
    .unwrap();
    let before = bookkeeping(&f.connection).await;
    for no_op in [
        f.store
            .reorder_block(f.block.uuid, ReorderDirection::Up)
            .await
            .unwrap(),
        f.store
            .reorder_block(second.uuid, ReorderDirection::Down)
            .await
            .unwrap(),
        f.store.indent_block(f.block.uuid).await.unwrap(),
    ] {
        assert!(no_op.changes.is_empty());
        assert_eq!(no_op.commit_version, None);
    }
    let missing = uuid::Uuid::now_v7();
    for (uuid, parent, after, expected_kind) in [
        (f.block.uuid, Some(f.block.uuid), None, "conflict"),
        (second.uuid, Some(nested.uuid), None, "conflict"),
        (f.block.uuid, Some(foreign.uuid), None, "conflict"),
        (f.block.uuid, Some(missing), None, "not_found"),
        (f.block.uuid, None, Some(nested.uuid), "invalid"),
        (missing, None, None, "not_found"),
    ] {
        let error = f.store.move_block(uuid, parent, after).await.unwrap_err();
        assert!(matches!(
            (error.downcast_ref::<CoreError>(), expected_kind),
            (Some(CoreError::NotFound(_)), "not_found")
                | (Some(CoreError::Conflict(_)), "conflict")
                | (Some(CoreError::InvalidInput(_)), "invalid")
        ));
    }
    assert!(matches!(
        f.store
            .outdent_block(f.block.uuid)
            .await
            .unwrap_err()
            .downcast_ref::<CoreError>(),
        Some(CoreError::Conflict(_))
    ));
    for error in [
        f.store
            .reorder_block(missing, ReorderDirection::Up)
            .await
            .unwrap_err(),
        f.store.indent_block(missing).await.unwrap_err(),
        f.store.outdent_block(missing).await.unwrap_err(),
    ] {
        assert!(matches!(
            error.downcast_ref::<CoreError>(),
            Some(CoreError::NotFound(_))
        ));
    }
    assert_eq!(bookkeeping(&f.connection).await, before);
    assert!(f.notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn every_structural_command_rolls_back_all_rows_after_a_late_history_failure() {
    for command in ["move", "reorder", "indent", "outdent"] {
        let f = fixture(BlockStyle::Paragraph).await;
        let second = child(&f, None, "Second").await;
        let nested = child(&f, Some(second.uuid), "Nested").await;
        f.connection
            .call(|database| {
                database.execute_batch(
                    "CREATE TRIGGER reject_movement_history BEFORE INSERT ON history_undo
             BEGIN SELECT RAISE(ABORT, 'injected movement history failure'); END;",
                )
            })
            .await
            .unwrap();
        let before = bookkeeping(&f.connection).await;
        let uuids = vec![f.block.uuid, second.uuid, nested.uuid];
        let before_blocks = db::get_blocks(&f.connection, uuids.clone()).await.unwrap();
        let error = match command {
            "move" => f
                .store
                .move_block(f.block.uuid, Some(second.uuid), None)
                .await
                .unwrap_err(),
            "reorder" => f
                .store
                .reorder_block(second.uuid, ReorderDirection::Up)
                .await
                .unwrap_err(),
            "indent" => f.store.indent_block(second.uuid).await.unwrap_err(),
            "outdent" => f.store.outdent_block(nested.uuid).await.unwrap_err(),
            _ => unreachable!(),
        };
        assert!(
            error
                .to_string()
                .contains("injected movement history failure"),
            "{command}: {error:#}"
        );
        assert_eq!(
            db::get_blocks(&f.connection, uuids).await.unwrap(),
            before_blocks,
            "{command}"
        );
        assert_eq!(bookkeeping(&f.connection).await, before, "{command}");
        assert!(f.notices.lock().unwrap().is_empty(), "{command}");
    }
}

#[tokio::test]
async fn legacy_structural_adapters_move_one_block_and_skip_boundary_operations() {
    let f = fixture(BlockStyle::Paragraph).await;
    let second = child(&f, None, "Second").await;
    let reordered = db::reorder_block_with_ops(&f.connection, second.uuid, ReorderDirection::Up)
        .await
        .unwrap();
    assert_eq!(reordered.operations.len(), 1);
    assert_eq!(root_uuids(&f).await, [second.uuid, f.block.uuid]);
    let moved = db::move_block_with_ops(&f.connection, f.block.uuid, Some(second.uuid), None)
        .await
        .unwrap();
    assert_eq!(moved.operations.len(), 1);
    assert_eq!(moved.value.parent_uuid, Some(second.uuid));
    let outdented = db::outdent_block_with_ops(&f.connection, f.block.uuid)
        .await
        .unwrap();
    assert_eq!(outdented.operations.len(), 1);
    assert_eq!(root_uuids(&f).await, [second.uuid, f.block.uuid]);
    let indented = db::indent_block_with_ops(&f.connection, f.block.uuid)
        .await
        .unwrap();
    assert_eq!(indented.operations.len(), 1);
    let no_op =
        db::move_block_in_direction_with_ops(&f.connection, f.block.uuid, ReorderDirection::Up)
            .await
            .unwrap();
    assert!(no_op.operations.is_empty());
    let no_op = db::indent_block_with_ops(&f.connection, f.block.uuid)
        .await
        .unwrap();
    assert!(no_op.operations.is_empty());
    assert!(f.notices.lock().unwrap().is_empty());
}
