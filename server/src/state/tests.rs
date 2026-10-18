//! Durable boundaries between the authored notes transaction and publication.
use super::*;
use notes_ai::agent::GraphWriter;
use notes_core::{OpKind, db};

async fn open_user(directory: &Path) -> (UserState, CancellationToken) {
    let stop = CancellationToken::new();
    // Open only the actual user replica/actor. There is no registry sweeper,
    // AI, OAuth or blob-ownership worker racing this explicit fault boundary.
    let user = UserState::open(
        &UserConfig {
            id: "recovery".into(),
            admin: true,
            token_sha256: None,
            token: None,
            tokens: Vec::new(),
        },
        directory,
        0,
        stop.clone(),
    )
    .await
    .unwrap();
    (user, stop)
}

async fn close_user(user: UserState, stop: CancellationToken) {
    stop.cancel();
    // Explicitly drain/join both SQL workers before opening a new generation;
    // dropping a handle or an actor is not proof that its files are closed.
    user.notes.close().await.unwrap();
    user.oplog.close_for_test().await.unwrap();
    assert!(user.notes.call(|_| Ok(())).await.is_err());
    assert!(user.oplog.latest_seq().await.is_err());
}

fn publication_fault(directory: &Path, enabled: bool) {
    let database = rusqlite::Connection::open_with_flags(
        directory.join("users/recovery/oplog.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
    )
    .unwrap();
    notes_core::sqlite::policy::configure_existing_primary_writer(&database).unwrap();
    database
        .execute_batch(if enabled {
            "CREATE TRIGGER reject_second_publication BEFORE INSERT ON oplog
             WHEN NEW.seq=2 BEGIN
               SELECT RAISE(ABORT, 'injected postcommit publication failure');
             END;"
        } else {
            "DROP TRIGGER reject_second_publication;"
        })
        .unwrap();
    database.close().unwrap();
}

async fn assert_recovery(partial_oplog_tail: bool) {
    let directory = tempfile::tempdir().unwrap();
    let (user, stop) = open_user(directory.path()).await;
    publication_fault(directory.path(), true);
    let error = GraphWriter::create_page(
        &user,
        "Committed before publication".into(),
        "The exact **authored** paragraph\nwith a second line.".into(),
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("injected postcommit publication failure"),
        "{error:#}"
    );
    // seq1 was appended before seq2 failed. Compensation must remove only
    // the new oplog tail, while the committed notes transaction stays durable.
    assert_eq!(user.oplog.latest_seq().await.unwrap(), 0);
    user.oplog.assert_gapless().await.unwrap();
    assert_eq!(sync_cursor(&user.notes).await.unwrap(), 0);
    let authored = notes_core::pending_outbox(&user.notes, 256).await.unwrap();
    assert_eq!(authored.len(), 2);
    let page_uuid = match &authored[0].kind {
        OpKind::PageCreate(page) => page.uuid,
        other => panic!("expected a page creation, got {other:?}"),
    };
    let block_uuid = match &authored[1].kind {
        OpKind::BlockCreate(block) => {
            assert_eq!(block.page_uuid, page_uuid);
            block.uuid
        }
        other => panic!("expected a paragraph creation, got {other:?}"),
    };
    let page = db::get_page(&user.notes, page_uuid).await.unwrap().unwrap();
    let block = db::get_block(&user.notes, block_uuid)
        .await
        .unwrap()
        .unwrap();
    let history = db::history_status(&user.notes).await.unwrap();
    assert_eq!((history.undo_count, history.redo_count), (1, 0));
    audit_unpublished_ops(&user.notes, &user.id).await.unwrap();
    publication_fault(directory.path(), false);
    if partial_oplog_tail {
        // Model process death after the first oplog commit and before the
        // separate notes acknowledgment transaction. Preserve its real IDs,
        // payload and HLC rather than reconstructing a new operation.
        let persisted = user.oplog.append(authored[0].clone()).await.unwrap();
        assert!(persisted.inserted);
        assert_eq!(persisted.operation.seq, 1);
        assert_eq!(sync_cursor(&user.notes).await.unwrap(), 0);
        assert_eq!(
            notes_core::pending_outbox(&user.notes, 256).await.unwrap(),
            authored
        );
    }
    close_user(user, stop).await;

    let (reopened, stop) = open_user(directory.path()).await;
    let replayed = u64::from(partial_oplog_tail);
    assert_eq!(sync_cursor(&reopened.notes).await.unwrap(), replayed);
    assert_eq!(
        notes_core::pending_outbox(&reopened.notes, 256)
            .await
            .unwrap(),
        authored[usize::from(partial_oplog_tail)..]
    );
    assert_eq!(
        db::get_page(&reopened.notes, page_uuid)
            .await
            .unwrap()
            .unwrap(),
        page
    );
    assert_eq!(
        db::get_block(&reopened.notes, block_uuid)
            .await
            .unwrap()
            .unwrap(),
        block
    );
    assert_eq!(db::history_status(&reopened.notes).await.unwrap(), history);
    reopened.publish_authored_ops().await.unwrap();
    let sequenced = reopened.oplog.ops_since(0, 256).await.unwrap();
    assert_eq!(
        sequenced.iter().map(|op| op.seq).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(
        sequenced
            .iter()
            .map(|op| op.envelope.clone())
            .collect::<Vec<_>>(),
        authored
    );
    assert_eq!(sync_cursor(&reopened.notes).await.unwrap(), 2);
    assert!(
        notes_core::pending_outbox(&reopened.notes, 256)
            .await
            .unwrap()
            .is_empty()
    );
    reopened.oplog.assert_gapless().await.unwrap();
    audit_unpublished_ops(&reopened.notes, &reopened.id)
        .await
        .unwrap();

    // Retrying the original envelopes cannot allocate new sequence IDs or
    // duplicate the page/paragraph. A genuinely new write uses the next seq.
    let retried = reopened.ingest(authored.clone()).await.unwrap();
    assert_eq!(retried, sequenced);
    assert_eq!(reopened.oplog.latest_seq().await.unwrap(), 2);
    assert_eq!(
        db::get_page(&reopened.notes, page_uuid)
            .await
            .unwrap()
            .unwrap(),
        page
    );
    assert_eq!(
        db::get_block(&reopened.notes, block_uuid)
            .await
            .unwrap()
            .unwrap(),
        block
    );
    assert_eq!(db::history_status(&reopened.notes).await.unwrap(), history);
    let captured =
        GraphWriter::create_page(&reopened, page.title.unwrap(), "Next paragraph".into())
            .await
            .unwrap();
    assert_eq!(captured.uuid, page_uuid);
    assert_eq!(reopened.oplog.latest_seq().await.unwrap(), 3);
    reopened.oplog.assert_gapless().await.unwrap();
    let blocks = db::list_block_children(&reopened.notes, page_uuid, None)
        .await
        .unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0], block);
    assert_eq!(blocks[1].markdown, "Next paragraph");
    assert!(
        notes_core::pending_outbox(&reopened.notes, 256)
            .await
            .unwrap()
            .is_empty()
    );
    let applied: i64 = reopened
        .notes
        .call(|database| {
            database.query_row("SELECT count(*) FROM applied_ops", [], |row| row.get(0))
        })
        .await
        .unwrap();
    assert_eq!(applied, 3);
    close_user(reopened, stop).await;
}

#[tokio::test]
async fn failed_postcommit_publication_recovers_the_original_authored_operations_after_reopen() {
    assert_recovery(false).await;
}

#[tokio::test]
async fn partial_durable_oplog_tail_replays_then_publishes_remaining_outbox_exactly_once() {
    assert_recovery(true).await;
}
