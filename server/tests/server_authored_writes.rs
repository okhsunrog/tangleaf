//! The server authors operations of its own — today from the chat agent, next
//! from the MCP tools. Those writes take a different route into the oplog than a
//! client's `POST /v1/ops`, and for a while they took no route at all: they
//! landed in the server's materialized database and stopped there, invisible to
//! every device. These tests pin the route down.

mod common;

use common::TOKEN;
use notes_ai::agent::GraphWriter;

#[tokio::test]
async fn a_server_authored_page_is_sequenced_before_the_write_returns() {
    let harness = common::start_server().await;
    let user = harness
        .state
        .registry
        .authenticate(TOKEN)
        .expect("authenticated user");

    // A device that has already joined the workspace, as one would be when the
    // assistant writes: bootstrapped from the server, following the oplog.
    let replica_directory = tempfile::tempdir().expect("replica directory");
    let replica = notes_core::db::open(replica_directory.path().join("notes.db"))
        .await
        .expect("replica database");
    let transport = harness.sync_transport();
    let snapshot = transport.snapshot().await.expect("server snapshot");
    let cursor = snapshot.seq;
    notes_core::import_sync_snapshot(&replica, snapshot)
        .await
        .expect("joining the workspace");

    let page = GraphWriter::create_page(
        &*user,
        "Captured by the assistant".into(),
        "the first paragraph".into(),
    )
    .await
    .expect("server-authored write");

    // No sleep, no waiting on the sweeper: the write is only complete once its
    // operations carry a sequence number, so a device polling the instant it
    // returns already sees them.
    let operations = transport
        .ops_since(cursor, 256)
        .await
        .expect("pulling operations");
    assert_eq!(
        operations.len(),
        2,
        "expected the page and its paragraph to be sequenced, got {operations:?}"
    );
    assert_eq!(operations[0].seq, cursor + 1);
    assert_eq!(operations[1].seq, cursor + 2);
    assert!(
        matches!(&operations[0].envelope.kind, notes_core::OpKind::PageCreate(create) if create.uuid == page.uuid)
    );
    assert!(
        matches!(&operations[1].envelope.kind, notes_core::OpKind::BlockCreate(create)
        if create.page_uuid == page.uuid && create.markdown == "the first paragraph"
        && create.style == notes_core::BlockStyle::Paragraph)
    );

    for operation in &operations {
        notes_core::apply_sequenced(&replica, operation.seq, &operation.envelope)
            .await
            .expect("applying to the replica");
    }

    let mirrored = notes_core::db::get_page(&replica, page.uuid)
        .await
        .expect("reading the replica")
        .expect("the page reached the replica");
    assert_eq!(mirrored.title.as_deref(), Some("Captured by the assistant"));
    let blocks = notes_core::db::list_block_children(&replica, page.uuid, None)
        .await
        .unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].markdown, "the first paragraph");
    assert_eq!(authored_state(&user.notes).await, (1, 1, 2, 0, 1, 0));
}

async fn authored_state(connection: &notes_core::Connection) -> (i64, i64, i64, i64, i64, i64) {
    connection
        .call(|database| {
            database.query_row(
                "SELECT (SELECT COUNT(*) FROM pages), (SELECT COUNT(*) FROM blocks),
        (SELECT COUNT(*) FROM applied_ops), (SELECT COUNT(*) FROM sync_outbox),
        (SELECT COUNT(*) FROM history_undo),
        (SELECT COUNT(*) FROM applied_ops WHERE seq IS NULL)",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn failed_paragraph_does_not_publish_or_leave_a_partial_page() {
    let harness = common::start_server().await;
    let user = harness.state.registry.authenticate(TOKEN).unwrap();
    let before_seq = user.oplog.latest_seq().await.unwrap();
    let before = authored_state(&user.notes).await;
    user.notes
        .call(|database| {
            database.execute_batch(
                "CREATE TEMP TRIGGER reject_capture BEFORE INSERT ON blocks BEGIN
         SELECT RAISE(ABORT, 'injected paragraph failure'); END;",
            )
        })
        .await
        .unwrap();
    let error = GraphWriter::create_page(&*user, "Partial page".into(), "rejected".into())
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("injected paragraph failure"),
        "{error:#}"
    );
    assert_eq!(authored_state(&user.notes).await, before);
    assert_eq!(user.oplog.latest_seq().await.unwrap(), before_seq);
    assert!(
        notes_core::db::get_page_by_title(&user.notes, "Partial page".into())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        harness
            .sync_transport()
            .ops_since(before_seq, 256)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn an_alias_capture_appends_one_sequenced_paragraph_to_the_existing_page() {
    let harness = common::start_server().await;
    let user = harness.state.registry.authenticate(TOKEN).unwrap();
    let page = GraphWriter::create_page(&*user, "Current title".into(), "original".into())
        .await
        .unwrap();
    let device_id = uuid::Uuid::now_v7();
    user.ingest(vec![notes_core::Op {
        op_id: uuid::Uuid::now_v7(),
        workspace_uuid: notes_core::db::workspace_uuid(&user.notes).await.unwrap(),
        device_id,
        hlc: notes_core::Hlc::new(1, 0, device_id),
        format_version: notes_core::operation::FORMAT_VERSION,
        kind: notes_core::OpKind::PageAliasSet(notes_core::PageAliasSet {
            uuid: page.uuid,
            alias: notes_core::PageAlias::new("Former title").unwrap(),
            present: true,
        }),
    }])
    .await
    .unwrap();
    let cursor = user.oplog.latest_seq().await.unwrap();
    let captured = GraphWriter::create_page(&*user, " FORMER TITLE ".into(), " appended\n".into())
        .await
        .unwrap();
    assert_eq!(captured.uuid, page.uuid);
    assert_eq!(captured.title, page.title);
    let operations = harness
        .sync_transport()
        .ops_since(cursor, 256)
        .await
        .unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].seq, cursor + 1);
    assert!(
        matches!(&operations[0].envelope.kind, notes_core::OpKind::BlockCreate(create)
        if create.page_uuid == page.uuid && create.markdown == " appended\n")
    );
    let blocks = notes_core::db::list_block_children(&user.notes, page.uuid, None)
        .await
        .unwrap();
    assert_eq!(
        blocks
            .iter()
            .map(|block| block.markdown.as_str())
            .collect::<Vec<_>>(),
        vec!["original", " appended\n"]
    );
    assert_eq!(authored_state(&user.notes).await, (1, 2, 4, 0, 2, 0));
}

#[tokio::test]
async fn concurrent_captures_publish_each_exact_paragraph_once_with_contiguous_sequences() {
    let harness = common::start_server().await;
    let user = harness.state.registry.authenticate(TOKEN).unwrap();
    let cursor = user.oplog.latest_seq().await.unwrap();
    let (first, second, third) = tokio::join!(
        GraphWriter::create_page(&*user, " Shared ".into(), "first\n".into()),
        GraphWriter::create_page(&*user, "SHARED".into(), " second".into()),
        GraphWriter::create_page(&*user, "shared".into(), "third **paragraph**".into()),
    );
    let pages = [first.unwrap(), second.unwrap(), third.unwrap()];
    assert!(pages.iter().all(|page| page.uuid == pages[0].uuid));
    let operations = harness
        .sync_transport()
        .ops_since(cursor, 256)
        .await
        .unwrap();
    assert_eq!(operations.len(), 4);
    assert_eq!(
        operations.iter().map(|op| op.seq).collect::<Vec<_>>(),
        (cursor + 1..=cursor + 4).collect::<Vec<_>>()
    );
    assert!(
        matches!(&operations[0].envelope.kind, notes_core::OpKind::PageCreate(create) if create.uuid == pages[0].uuid)
    );
    let mut markdown = operations
        .iter()
        .filter_map(|operation| match &operation.envelope.kind {
            notes_core::OpKind::BlockCreate(create) => {
                assert_eq!(create.page_uuid, pages[0].uuid);
                Some(create.markdown.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    markdown.sort_unstable();
    assert_eq!(markdown, vec![" second", "first\n", "third **paragraph**"]);
    let blocks = notes_core::db::list_block_children(&user.notes, pages[0].uuid, None)
        .await
        .unwrap();
    assert_eq!(blocks.len(), 3);
    assert!(
        blocks
            .windows(2)
            .all(|pair| pair[0].order_key < pair[1].order_key)
    );
    assert_eq!(authored_state(&user.notes).await, (1, 3, 4, 0, 3, 0));
}

#[tokio::test]
async fn server_authored_operations_leave_nothing_stranded() {
    let harness = common::start_server().await;
    let user = harness
        .state
        .registry
        .authenticate(TOKEN)
        .expect("authenticated user");

    GraphWriter::create_page(&*user, "Stranding check".into(), String::new())
        .await
        .expect("server-authored write");

    // The invariant the server audits on every open: an applied operation is
    // either queued for publication or already sequenced. Anything else is a
    // write the materialized state remembers and the log does not, and it
    // cannot be reconstructed — only the envelope carries the HLC.
    let stranded: i64 = user
        .notes
        .call(|database| {
            database.query_row(
                "SELECT COUNT(*) FROM applied_ops
                 WHERE seq IS NULL AND op_id NOT IN (SELECT op_id FROM sync_outbox)",
                [],
                |row| row.get(0),
            )
        })
        .await
        .expect("auditing the replica");
    assert_eq!(stranded, 0, "server-authored operations were stranded");
}
