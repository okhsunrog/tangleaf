use notes_core::db::{self, Block, BlockContent};
use notes_core::store::{CommitNotice, NotesStore};
use notes_core::{BlockStyle, Connection, CoreError, TaskState};
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
