use super::*;
use crate::{BlockCreate, PageCreate};
use std::cell::RefCell;
use std::sync::mpsc;
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ReadStats {
    page_queries: usize,
    block_queries: usize,
    max_parameters: usize,
    writer_committed_during_read: bool,
}

struct Pause {
    at_block_query: usize,
    start_writer: mpsc::Sender<()>,
    writer_finished: mpsc::Receiver<()>,
}

struct ReadProbe {
    stats: ReadStats,
    pause: Option<Pause>,
}

thread_local! {
    static READ_PROBE: RefCell<Option<ReadProbe>> = const { RefCell::new(None) };
}

fn trace_reads(event: rusqlite::trace::TraceEvent<'_>) {
    let rusqlite::trace::TraceEvent::Stmt(statement, _) = event else {
        return;
    };
    let sql = statement.sql();
    let page = sql.contains("FROM pages WHERE uuid IN (");
    let block = sql.contains("FROM blocks WHERE uuid IN (");
    if !page && !block {
        return;
    }
    let pause = READ_PROBE.with(|probe| {
        let mut probe = probe.borrow_mut();
        let probe = probe.as_mut()?;
        probe.stats.page_queries += usize::from(page);
        probe.stats.block_queries += usize::from(block);
        probe.stats.max_parameters = probe.stats.max_parameters.max(sql.matches('?').count());
        if block
            && probe
                .pause
                .as_ref()
                .is_some_and(|pause| pause.at_block_query == probe.stats.block_queries)
        {
            probe.pause.take()
        } else {
            None
        }
    });
    if let Some(pause) = pause {
        // A separate WAL writer commits between SELECTs. Channels make this
        // schedule deterministic; the callback never unwinds through SQLite.
        let committed = pause.start_writer.send(()).is_ok()
            && pause
                .writer_finished
                .recv_timeout(Duration::from_secs(10))
                .is_ok();
        READ_PROBE.with(|probe| {
            if let Some(probe) = probe.borrow_mut().as_mut() {
                probe.stats.writer_committed_during_read = committed;
            }
        });
    }
}

async fn observe(connection: &Connection, pause: Option<Pause>) {
    connection
        .call(move |database| {
            READ_PROBE.with(|probe| {
                *probe.borrow_mut() = Some(ReadProbe {
                    stats: ReadStats::default(),
                    pause,
                })
            });
            database.trace_v2(
                rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
                Some(trace_reads),
            );
            Ok(())
        })
        .await
        .unwrap();
}

async fn finish_observing(connection: &Connection) -> ReadStats {
    connection
        .call(|database| {
            database.trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);
            Ok(READ_PROBE.with(|probe| probe.borrow_mut().take().unwrap().stats))
        })
        .await
        .unwrap()
}

async fn fixture(count: usize) -> (tempfile::TempDir, Connection, Uuid, Vec<Uuid>) {
    let directory = tempfile::tempdir().unwrap();
    let connection = open(directory.path().join("notes.db")).await.unwrap();
    let page = Uuid::from_u128(1);
    let blocks = (2..=count + 1)
        .map(|id| Uuid::from_u128(id as u128))
        .collect::<Vec<_>>();
    let mut kinds = vec![OpKind::PageCreate(PageCreate {
        uuid: page,
        title: Some("old page".into()),
        kind: PageKind::Note,
        layout: PageLayout::Outline,
        created_at: 10,
    })];
    kinds.extend(blocks.iter().enumerate().map(|(index, uuid)| {
        OpKind::BlockCreate(BlockCreate {
            uuid: *uuid,
            page_uuid: page,
            parent_uuid: None,
            order_key: OrderKey::from_ordinal(index + 1),
            style: BlockStyle::Paragraph,
            markdown: "old".into(),
            created_at: 10,
        })
    }));
    operation::apply_local_kinds(&connection, kinds)
        .await
        .unwrap();
    (directory, connection, page, blocks)
}

fn concurrent_writer(
    directory: &tempfile::TempDir,
    at_block_query: usize,
) -> (Pause, std::thread::JoinHandle<anyhow::Result<()>>) {
    let mut writer = rusqlite::Connection::open(directory.path().join("notes.db")).unwrap();
    writer.busy_timeout(Duration::from_secs(5)).unwrap();
    let (start_writer, started) = mpsc::channel();
    let (finished, writer_finished) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        started.recv_timeout(Duration::from_secs(10))?;
        let result = (|| -> anyhow::Result<()> {
            let tx = writer.transaction()?;
            tx.execute("UPDATE pages SET title = 'new page'", [])?;
            tx.execute("UPDATE blocks SET markdown = 'new'", [])?;
            tx.commit()?;
            Ok(())
        })();
        let _ = finished.send(());
        result
    });
    (
        Pause {
            at_block_query,
            start_writer,
            writer_finished,
        },
        writer,
    )
}

#[tokio::test]
async fn mixed_content_reads_preserve_order_duplicates_and_missing_objects_in_bounded_chunks() {
    let (_directory, connection, page, blocks) = fixture(513).await;
    let missing = Uuid::from_u128(10_000);
    let mut requested = blocks.iter().rev().copied().collect::<Vec<_>>();
    requested.insert(12, missing);
    requested.insert(4, page);
    requested.extend([blocks[0], page]);
    observe(&connection, None).await;
    let contents = get_contents(&connection, requested.clone()).await.unwrap();
    let stats = finish_observing(&connection).await;
    assert_eq!(
        contents.iter().map(Content::uuid).collect::<Vec<_>>(),
        requested
            .iter()
            .copied()
            .filter(|uuid| *uuid != missing)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        contents
            .iter()
            .filter(|content| matches!(content, Content::Page(_)))
            .count(),
        2
    );
    assert_eq!(stats.page_queries, 3);
    assert_eq!(stats.block_queries, 3);
    assert_eq!(stats.max_parameters, UUID_READ_CHUNK_SIZE);
    assert_eq!(
        get_content(&connection, page).await.unwrap(),
        Some(Content::Page(
            get_page(&connection, page).await.unwrap().unwrap()
        ))
    );
    assert_eq!(
        get_content(&connection, blocks[0]).await.unwrap(),
        Some(Content::Block(
            get_block(&connection, blocks[0]).await.unwrap().unwrap()
        ))
    );
    assert!(get_content(&connection, missing).await.unwrap().is_none());
}

#[tokio::test]
async fn block_reads_preserve_order_duplicates_and_missing_objects_in_bounded_chunks() {
    let (_directory, connection, page, blocks) = fixture(513).await;
    let missing = Uuid::from_u128(10_000);
    let mut requested = blocks.iter().rev().copied().collect::<Vec<_>>();
    requested.extend([missing, page, blocks[0], blocks[0]]);
    observe(&connection, None).await;
    let result = get_blocks(&connection, requested.clone()).await.unwrap();
    let stats = finish_observing(&connection).await;
    assert_eq!(
        result.iter().map(|block| block.uuid).collect::<Vec<_>>(),
        requested
            .into_iter()
            .filter(|uuid| *uuid != missing && *uuid != page)
            .collect::<Vec<_>>()
    );
    assert_eq!(stats.page_queries, 0);
    assert_eq!(stats.block_queries, 3);
    assert_eq!(stats.max_parameters, UUID_READ_CHUNK_SIZE);
}

#[tokio::test]
async fn empty_reads_execute_no_queries() {
    let (_directory, connection, _page, _blocks) = fixture(0).await;
    observe(&connection, None).await;
    assert!(
        get_contents(&connection, Vec::new())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        get_blocks(&connection, Vec::new())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(finish_observing(&connection).await, ReadStats::default());
}

#[tokio::test]
async fn block_chunks_share_one_snapshot_while_another_writer_commits() {
    let (directory, connection, _page, blocks) = fixture(513).await;
    let (pause, writer) = concurrent_writer(&directory, 2);
    observe(&connection, Some(pause)).await;
    let result = get_blocks(&connection, blocks.clone()).await.unwrap();
    let stats = finish_observing(&connection).await;
    writer.join().unwrap().unwrap();
    assert!(stats.writer_committed_during_read);
    assert_eq!(result.len(), 513);
    assert!(result.iter().all(|block| block.markdown == "old"));
    assert!(
        get_blocks(&connection, blocks)
            .await
            .unwrap()
            .iter()
            .all(|block| block.markdown == "new")
    );
}

#[tokio::test]
async fn pages_and_blocks_share_one_snapshot_while_another_writer_commits() {
    let (directory, connection, page, blocks) = fixture(513).await;
    let (pause, writer) = concurrent_writer(&directory, 1);
    let requested = std::iter::once(page).chain(blocks).collect::<Vec<_>>();
    observe(&connection, Some(pause)).await;
    let result = get_contents(&connection, requested.clone()).await.unwrap();
    let stats = finish_observing(&connection).await;
    writer.join().unwrap().unwrap();
    assert!(stats.writer_committed_during_read);
    assert_eq!(result.len(), 514);
    assert!(result.iter().all(|content| match content {
        Content::Page(page) => page.title.as_deref() == Some("old page"),
        Content::Block(block) => block.markdown == "old",
    }));
    assert!(
        get_contents(&connection, requested)
            .await
            .unwrap()
            .iter()
            .all(|content| match content {
                Content::Page(page) => page.title.as_deref() == Some("new page"),
                Content::Block(block) => block.markdown == "new",
            })
    );
}
