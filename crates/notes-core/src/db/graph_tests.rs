use super::*;
use crate::{BlockCreate, PageCreate};
use std::cell::RefCell;
use std::sync::mpsc;
use std::time::Duration;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn page(uuid: Uuid, title: &str) -> OpKind {
    OpKind::PageCreate(PageCreate {
        uuid,
        title: Some(title.into()),
        kind: PageKind::Note,
        layout: PageLayout::Outline,
        created_at: 10,
    })
}

fn block(uuid: Uuid, page_uuid: Uuid, parent_uuid: Option<Uuid>, markdown: &str) -> OpKind {
    OpKind::BlockCreate(BlockCreate {
        uuid,
        page_uuid,
        parent_uuid,
        order_key: OrderKey::from_ordinal(uuid.as_u128() as usize),
        style: BlockStyle::Paragraph,
        markdown: markdown.into(),
        created_at: 10,
    })
}

async fn fixture(unrelated: usize) -> (tempfile::TempDir, Connection) {
    let directory = tempfile::tempdir().unwrap();
    let connection = open(directory.path().join("notes.db")).await.unwrap();
    let mut kinds = vec![
        page(id(1), "Source"),
        page(id(2), "Target"),
        page(id(3), "Other"),
        block(id(10), id(1), None, "root"),
        block(id(11), id(1), Some(id(10)), "child"),
        block(id(12), id(1), Some(id(11)), "grandchild"),
        block(id(13), id(1), None, "sibling"),
        block(id(20), id(2), None, "target block"),
        block(
            id(30),
            id(1),
            None,
            &format!("[[Target]] (({})) (({}))", id(20), id(40)),
        ),
        block(id(31), id(3), None, &format!("[[Source]] (({}))", id(40))),
    ];
    kinds.extend(
        (100..100 + unrelated as u128).map(|uuid| block(id(uuid), id(1), None, "unrelated")),
    );
    operation::apply_local_kinds(&connection, kinds)
        .await
        .unwrap();
    (directory, connection)
}

#[derive(Debug, Default, Clone)]
struct SqlStats {
    queries: Vec<String>,
    rows: usize,
    writer_committed: bool,
}

struct Pause {
    needle: &'static str,
    start: mpsc::Sender<()>,
    finished: mpsc::Receiver<()>,
}
struct Probe {
    stats: SqlStats,
    pause: Option<Pause>,
}

thread_local! { static PROBE: RefCell<Option<Probe>> = const { RefCell::new(None) }; }

fn trace_graph(event: rusqlite::trace::TraceEvent<'_>) {
    match event {
        rusqlite::trace::TraceEvent::Stmt(statement, _) => {
            let sql = statement.sql();
            if !sql.starts_with("SELECT") && !sql.starts_with("WITH") {
                return;
            }
            let pause = PROBE.with(|probe| {
                let mut probe = probe.borrow_mut();
                let probe = probe.as_mut()?;
                probe.stats.queries.push(sql.to_string());
                if probe
                    .pause
                    .as_ref()
                    .is_some_and(|pause| sql.contains(pause.needle))
                {
                    probe.pause.take()
                } else {
                    None
                }
            });
            if let Some(pause) = pause {
                let committed = pause.start.send(()).is_ok()
                    && pause.finished.recv_timeout(Duration::from_secs(10)).is_ok();
                PROBE.with(|probe| {
                    if let Some(probe) = probe.borrow_mut().as_mut() {
                        probe.stats.writer_committed = committed;
                    }
                });
            }
        }
        rusqlite::trace::TraceEvent::Row(statement) => {
            let sql = statement.sql();
            if sql.starts_with("SELECT") || sql.starts_with("WITH") {
                PROBE.with(|probe| {
                    if let Some(probe) = probe.borrow_mut().as_mut() {
                        probe.stats.rows += 1;
                    }
                });
            }
        }
        _ => {}
    }
}

async fn observe(connection: &Connection, pause: Option<Pause>) {
    connection
        .call(move |db| {
            PROBE.with(|probe| {
                *probe.borrow_mut() = Some(Probe {
                    stats: SqlStats::default(),
                    pause,
                })
            });
            db.trace_v2(
                rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT
                    | rusqlite::trace::TraceEventCodes::SQLITE_TRACE_ROW,
                Some(trace_graph),
            );
            Ok(())
        })
        .await
        .unwrap();
}

async fn observed(connection: &Connection) -> SqlStats {
    connection
        .call(|db| {
            db.trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);
            Ok(PROBE.with(|probe| probe.borrow_mut().take().unwrap().stats))
        })
        .await
        .unwrap()
}

fn writer(
    directory: &tempfile::TempDir,
    needle: &'static str,
) -> (Pause, std::thread::JoinHandle<anyhow::Result<()>>) {
    let mut db = rusqlite::Connection::open(directory.path().join("notes.db")).unwrap();
    db.busy_timeout(Duration::from_secs(5)).unwrap();
    let (start, started) = mpsc::channel();
    let (finished, done) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        started.recv_timeout(Duration::from_secs(10))?;
        let result = (|| -> anyhow::Result<()> {
            let tx = db.transaction()?;
            tx.execute_batch("UPDATE pages SET title = lower('new-' || hex(uuid)), normalized_title = lower('new-' || hex(uuid));
                UPDATE blocks SET markdown = 'new', parent_uuid = NULL;
                DELETE FROM page_links; DELETE FROM block_refs;")?;
            tx.commit()?;
            Ok(())
        })();
        let _ = finished.send(());
        result
    });
    (
        Pause {
            needle,
            start,
            finished: done,
        },
        writer,
    )
}

fn content_ids(contents: Vec<Content>) -> Vec<Uuid> {
    contents.into_iter().map(|content| content.uuid()).collect()
}

#[tokio::test]
async fn indexed_neighbors_match_legacy_depth_order_and_dangling_reference_traversal() {
    let (_directory, connection) = fixture(0).await;
    for start in [id(1), id(10), id(12), id(30), id(40), id(99)] {
        for depth in 0..=4 {
            let expected = connection.call(move |db| {
                let tx = db.transaction()?;
                let uuids = tx.prepare("WITH RECURSIVE adjacent(source, target) AS (
                    SELECT source_block_uuid, target_page_uuid FROM page_links WHERE target_page_uuid IS NOT NULL
                    UNION SELECT target_page_uuid, source_block_uuid FROM page_links WHERE target_page_uuid IS NOT NULL
                    UNION SELECT source_block_uuid, target_block_uuid FROM block_refs
                    UNION SELECT target_block_uuid, source_block_uuid FROM block_refs
                    UNION SELECT uuid, page_uuid FROM blocks WHERE parent_uuid IS NULL
                    UNION SELECT page_uuid, uuid FROM blocks WHERE parent_uuid IS NULL
                    UNION SELECT uuid, parent_uuid FROM blocks WHERE parent_uuid IS NOT NULL
                    UNION SELECT parent_uuid, uuid FROM blocks WHERE parent_uuid IS NOT NULL
                  ), reachable(uuid, depth) AS (
                    SELECT ?1, 0 UNION SELECT adjacent.target, reachable.depth + 1 FROM reachable
                    JOIN adjacent ON adjacent.source = reachable.uuid WHERE reachable.depth < ?2
                  ) SELECT DISTINCT uuid FROM reachable WHERE uuid != ?1 ORDER BY depth, uuid")?
                    .query_map(rusqlite::params![start, depth], |row| row.get::<_, Uuid>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                get_contents_in_transaction(&tx, &uuids)
            }).await.unwrap();
            assert_eq!(
                neighbors(&connection, start, depth).await.unwrap(),
                expected,
                "start {start}, depth {depth}"
            );
        }
    }
    assert!(content_ids(neighbors(&connection, id(30), 2).await.unwrap()).contains(&id(31)));
}

#[tokio::test]
async fn hierarchy_reads_preserve_order_and_existing_depth_semantics() {
    let (_directory, connection) = fixture(0).await;
    assert_eq!(
        content_ids(read_ancestors(&connection, id(12)).await.unwrap()),
        [1, 10, 11, 12].map(id)
    );
    assert_eq!(
        content_ids(read_ancestors(&connection, id(1)).await.unwrap()),
        [id(1)]
    );
    assert!(
        read_ancestors(&connection, id(99))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        content_ids(read_subtree(&connection, id(1), 1).await.unwrap()),
        [10, 13, 30].map(id)
    );
    assert_eq!(
        content_ids(read_subtree(&connection, id(1), 0).await.unwrap()),
        [10, 13, 30].map(id)
    );
    assert_eq!(
        content_ids(read_subtree(&connection, id(10), 2).await.unwrap()),
        [11, 12].map(id)
    );
    assert_eq!(
        content_ids(read_subtree(&connection, id(1), 3).await.unwrap()),
        [10, 11, 12, 13, 30].map(id)
    );
}

#[tokio::test]
async fn focused_alias_edges_preserve_duplicates_while_workspace_edges_collapse_them() {
    let (_directory, connection) = fixture(0).await;
    operation::apply_local_kinds(
        &connection,
        vec![
            OpKind::PageAliasSet(crate::PageAliasSet {
                uuid: id(2),
                alias: "Target alias".parse().unwrap(),
                present: true,
            }),
            OpKind::BlockSetMarkdown(crate::BlockSetMarkdown {
                uuid: id(30),
                markdown: "[[Target]] [[Target alias]]".into(),
            }),
        ],
    )
    .await
    .unwrap();
    let focused = graph_snapshot(&connection, Some(id(30))).await.unwrap();
    assert_eq!(
        focused
            .edges
            .iter()
            .filter(|edge| edge.source_uuid == id(30)
                && edge.target_uuid == id(2)
                && edge.relation == GraphRelation::PageLink)
            .count(),
        2
    );
    let workspace = graph_snapshot(&connection, None).await.unwrap();
    assert_eq!(
        workspace
            .edges
            .iter()
            .filter(|edge| edge.source_uuid == id(1)
                && edge.target_uuid == id(2)
                && edge.relation == GraphRelation::PageLink)
            .count(),
        1
    );
}

#[tokio::test]
async fn focused_graph_queries_and_rows_stay_bounded_with_unrelated_roots() {
    let mut counts = Vec::new();
    for unrelated in [1_000, 10_000] {
        let (_directory, connection) = fixture(unrelated).await;
        observe(&connection, None).await;
        let graph = graph_snapshot(&connection, Some(id(30))).await.unwrap();
        let stats = observed(&connection).await;
        assert_eq!(
            graph.items.iter().map(|item| item.uuid).collect::<Vec<_>>(),
            [30, 1, 2, 20].map(id)
        );
        assert_eq!(graph.edges.len(), 4);
        for query in &stats.queries {
            assert!(query.contains(" IN ("), "unbounded focused query: {query}");
            assert!(query.matches('?').count() <= UUID_READ_CHUNK_SIZE);
        }
        counts.push((stats.queries.len(), stats.rows));
    }
    assert_eq!(counts[0], counts[1]);
    assert_eq!(counts[0].0, 12);
}

#[tokio::test]
async fn neighbors_discovery_and_hydration_share_one_wal_snapshot() {
    let (directory, connection) = fixture(0).await;
    let (pause, writer) = writer(&directory, "FROM pages WHERE uuid IN (");
    observe(&connection, Some(pause)).await;
    let result = neighbors(&connection, id(30), 1).await.unwrap();
    let stats = observed(&connection).await;
    writer.join().unwrap().unwrap();
    assert!(stats.writer_committed);
    assert_eq!(content_ids(result.clone()), [1, 2, 20].map(id));
    assert!(
        result
            .iter()
            .any(|content| content.text() == "target block")
    );
}

#[tokio::test]
async fn backlinks_discovery_and_hydration_share_one_wal_snapshot() {
    let (directory, connection) = fixture(0).await;
    let (pause, writer) = writer(&directory, "FROM pages WHERE uuid IN (");
    observe(&connection, Some(pause)).await;
    let result = find_backlinks(&connection, id(2)).await.unwrap();
    let stats = observed(&connection).await;
    writer.join().unwrap().unwrap();
    assert!(stats.writer_committed);
    assert_eq!(content_ids(result.clone()), [id(30)]);
    assert!(result[0].text().starts_with("[[Target]]"));
    assert!(find_backlinks(&connection, id(2)).await.unwrap().is_empty());
}

#[tokio::test]
async fn backlinks_snapshot_preserves_source_order_reference_scope_and_distinct_page_kinds() {
    let (_directory, connection) = fixture(0).await;
    let date = "2026-10-18".parse::<crate::JournalDate>().unwrap();
    let journal_uuid = crate::journal_page_uuid(workspace_uuid(&connection).await.unwrap(), &date);
    operation::apply_local_kinds(
        &connection,
        vec![
            OpKind::PageAliasSet(crate::PageAliasSet {
                uuid: id(2),
                alias: crate::PageAlias::new("Target alias").unwrap(),
                present: true,
            }),
            block(
                id(32),
                id(1),
                Some(id(10)),
                &format!("[[Target]] [[Target alias]] (({})) (({}))", id(20), id(20)),
            ),
            block(id(33), id(3), None, "[[Target alias]]"),
            OpKind::PageCreate(PageCreate {
                uuid: journal_uuid,
                title: None,
                kind: PageKind::Journal { date },
                layout: PageLayout::Document,
                created_at: 10,
            }),
            block(id(34), journal_uuid, None, "[[Target]]"),
        ],
    )
    .await
    .unwrap();

    for (target, expected) in [
        (id(2), vec![id(30), id(32), id(33), id(34)]),
        (id(20), vec![id(30), id(32)]),
    ] {
        let snapshot = find_backlinks_snapshot(&connection, target).await.unwrap();
        assert_eq!(
            snapshot
                .blocks
                .iter()
                .map(|block| block.uuid)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            find_backlinks(&connection, target).await.unwrap(),
            snapshot
                .blocks
                .iter()
                .cloned()
                .map(Content::Block)
                .collect::<Vec<_>>()
        );
        let owner_uuids = snapshot
            .blocks
            .iter()
            .map(|block| block.page_uuid)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            snapshot
                .pages
                .iter()
                .map(|page| page.uuid)
                .collect::<Vec<_>>(),
            owner_uuids.into_iter().collect::<Vec<_>>()
        );
        if target == id(2) {
            assert!(
                snapshot
                    .pages
                    .iter()
                    .any(|page| page.uuid == journal_uuid && page.kind.is_journal())
            );
        }
    }
}

#[tokio::test]
async fn backlinks_snapshot_keeps_a_missing_owner_explicit_without_losing_its_source() {
    let (_directory, connection) = fixture(0).await;
    let empty = find_backlinks_snapshot(&connection, id(99)).await.unwrap();
    assert!(empty.blocks.is_empty() && empty.pages.is_empty());
    // A healthy workspace cannot contain this state because of the FK. Keep
    // the read model useful if an imported/externally altered file does.
    connection
        .call(|database| {
            database.pragma_update(None, "foreign_keys", false)?;
            database.execute("DELETE FROM pages WHERE uuid = ?1", [id(1)])?;
            database.pragma_update(None, "foreign_keys", true)?;
            Ok(())
        })
        .await
        .unwrap();
    let snapshot = find_backlinks_snapshot(&connection, id(2)).await.unwrap();
    assert_eq!(
        snapshot
            .blocks
            .iter()
            .map(|block| block.uuid)
            .collect::<Vec<_>>(),
        [id(30)]
    );
    assert!(snapshot.pages.is_empty());
}

#[tokio::test]
async fn backlinks_snapshot_source_blocks_and_owner_pages_share_one_wal_snapshot() {
    let (directory, connection) = fixture(0).await;
    let (pause, writer) = writer(&directory, "FROM pages WHERE uuid IN (");
    observe(&connection, Some(pause)).await;
    let result = find_backlinks_snapshot(&connection, id(2)).await.unwrap();
    let stats = observed(&connection).await;
    writer.join().unwrap().unwrap();
    assert!(stats.writer_committed);
    assert_eq!(result.blocks.len(), 1);
    assert!(result.blocks[0].markdown.starts_with("[[Target]]"));
    assert_eq!(result.pages.len(), 1);
    assert_eq!(result.pages[0].uuid, result.blocks[0].page_uuid);
    assert_eq!(result.pages[0].title.as_deref(), Some("Source"));
    assert!(
        find_backlinks_snapshot(&connection, id(2))
            .await
            .unwrap()
            .blocks
            .is_empty()
    );
}

#[tokio::test]
async fn backlinks_snapshot_chunked_queries_stay_bounded_with_unrelated_roots() {
    let mut counts = Vec::new();
    for unrelated in [1_000, 10_000] {
        let (_directory, connection) = fixture(unrelated).await;
        let mut kinds = vec![page(id(90), "Batch target")];
        for offset in 0..=UUID_READ_CHUNK_SIZE {
            let owner = id(1_000_000 + offset as u128);
            kinds.push(page(owner, &format!("Batch owner {offset}")));
            kinds.push(block(
                id(50_000 + offset as u128),
                owner,
                None,
                "[[Batch target]]",
            ));
        }
        kinds.push(block(id(60_000), id(1_000_000), None, "[[Batch target]]"));
        kinds.push(block(id(60_001), id(1_000_000), None, "[[Batch target]]"));
        operation::apply_local_kinds(&connection, kinds)
            .await
            .unwrap();
        observe(&connection, None).await;
        let snapshot = find_backlinks_snapshot(&connection, id(90)).await.unwrap();
        let stats = observed(&connection).await;
        assert_eq!(snapshot.blocks.len(), UUID_READ_CHUNK_SIZE + 3);
        assert_eq!(snapshot.pages.len(), UUID_READ_CHUNK_SIZE + 1);
        assert!(
            snapshot
                .blocks
                .windows(2)
                .all(|pair| pair[0].uuid < pair[1].uuid)
        );
        assert!(
            snapshot
                .pages
                .windows(2)
                .all(|pair| pair[0].uuid < pair[1].uuid)
        );
        assert_eq!(stats.queries.len(), 5); // discovery, two block chunks, two page chunks
        for query in &stats.queries {
            assert!(query.matches('?').count() <= UUID_READ_CHUNK_SIZE);
            assert!(
                query.contains(" IN (")
                    || query
                        .contains("source_block_uuid FROM page_links WHERE target_page_uuid = ?1")
            );
        }
        assert_eq!(stats.rows, 2 * snapshot.blocks.len() + snapshot.pages.len());
        counts.push((stats.queries.len(), stats.rows));
    }
    assert_eq!(counts[0], counts[1]);
}

#[tokio::test]
async fn ancestor_target_and_traversal_share_one_wal_snapshot() {
    let (directory, connection) = fixture(0).await;
    let (pause, writer) = writer(&directory, "WITH RECURSIVE ancestors");
    observe(&connection, Some(pause)).await;
    let result = read_ancestors(&connection, id(12)).await.unwrap();
    let stats = observed(&connection).await;
    writer.join().unwrap().unwrap();
    assert!(stats.writer_committed);
    assert_eq!(content_ids(result), [1, 10, 11, 12].map(id));
    assert_eq!(
        content_ids(read_ancestors(&connection, id(12)).await.unwrap()),
        [1, 12].map(id)
    );
}

#[tokio::test]
async fn subtree_target_and_traversal_share_one_wal_snapshot() {
    let (directory, connection) = fixture(0).await;
    let (pause, writer) = writer(&directory, "WITH RECURSIVE subtree");
    observe(&connection, Some(pause)).await;
    let result = read_subtree(&connection, id(1), 1).await.unwrap();
    let stats = observed(&connection).await;
    writer.join().unwrap().unwrap();
    assert!(stats.writer_committed);
    assert_eq!(content_ids(result), [10, 13, 30].map(id));
    assert_eq!(read_subtree(&connection, id(1), 1).await.unwrap().len(), 5);
}

#[tokio::test]
async fn focused_graph_labels_and_edges_share_one_wal_snapshot() {
    let (directory, connection) = fixture(0).await;
    let (pause, writer) = writer(
        &directory,
        "SELECT source_block_uuid, target_page_uuid FROM page_links WHERE source_block_uuid IN (",
    );
    observe(&connection, Some(pause)).await;
    let graph = graph_snapshot(&connection, Some(id(30))).await.unwrap();
    let stats = observed(&connection).await;
    writer.join().unwrap().unwrap();
    assert!(stats.writer_committed);
    assert!(
        graph
            .items
            .iter()
            .any(|item| item.uuid == id(2) && item.label == "Target")
    );
    assert!(graph.edges.iter().any(|edge| edge.source_uuid == id(30)
        && edge.target_uuid == id(2)
        && edge.relation == GraphRelation::PageLink));
}

#[tokio::test]
async fn workspace_graph_labels_and_edges_share_one_wal_snapshot() {
    let (directory, connection) = fixture(0).await;
    let (pause, writer) = writer(
        &directory,
        "SELECT DISTINCT source.page_uuid, link.target_page_uuid",
    );
    observe(&connection, Some(pause)).await;
    let graph = graph_snapshot(&connection, None).await.unwrap();
    let stats = observed(&connection).await;
    writer.join().unwrap().unwrap();
    assert!(stats.writer_committed);
    assert!(
        graph
            .items
            .iter()
            .any(|item| item.uuid == id(2) && item.label == "Target")
    );
    assert!(graph.edges.iter().any(|edge| edge.source_uuid == id(1)
        && edge.target_uuid == id(2)
        && edge.relation == GraphRelation::PageLink));
    assert!(
        graph_snapshot(&connection, None)
            .await
            .unwrap()
            .edges
            .is_empty()
    );
}

#[tokio::test]
async fn alias_resolution_and_page_hydration_share_one_wal_snapshot() {
    let (directory, connection) = fixture(0).await;
    let (pause, writer) = writer(&directory, "FROM pages WHERE uuid = ?1");
    observe(&connection, Some(pause)).await;
    let page = get_page_by_title(&connection, "Target".into())
        .await
        .unwrap()
        .unwrap();
    let stats = observed(&connection).await;
    writer.join().unwrap().unwrap();
    assert!(stats.writer_committed);
    assert_eq!(page.title.as_deref(), Some("Target"));
    assert!(
        get_page_by_title(&connection, "Target".into())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn workspace_scope_includes_handwriting_and_preserves_focused_journals() {
    let (_directory, connection) = fixture(0).await;
    let date: crate::JournalDate = "2026-10-18".parse().unwrap();
    let workspace = workspace_uuid(&connection).await.unwrap();
    let journal = crate::journal_page_uuid(workspace, &date);
    operation::apply_local_kinds(
        &connection,
        vec![
            OpKind::PageCreate(PageCreate {
                uuid: id(50),
                kind: PageKind::Handwriting,
                title: Some("Drawing".into()),
                layout: PageLayout::Outline,
                created_at: 10,
            }),
            OpKind::PageCreate(PageCreate {
                uuid: journal,
                kind: PageKind::Journal { date },
                title: None,
                layout: PageLayout::Outline,
                created_at: 10,
            }),
        ],
    )
    .await
    .unwrap();
    let graph = graph_snapshot(&connection, None).await.unwrap();
    assert_eq!(graph.items.len(), 4);
    assert!(graph.items.iter().any(|item| item.uuid == id(50)));
    assert!(!graph.items.iter().any(|item| item.uuid == journal));
    assert!(
        graph_snapshot(&connection, Some(journal))
            .await
            .unwrap()
            .items
            .iter()
            .any(|item| item.uuid == journal && item.label == "2026-10-18")
    );
}
