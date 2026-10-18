use super::*;
use std::cell::RefCell;
use std::path::PathBuf;

struct Probe {
    path: Option<PathBuf>,
    before_sql: &'static str,
    writer_result: Option<Result<(), String>>,
}

thread_local! {
    static PROBE: RefCell<Option<Probe>> = const { RefCell::new(None) };
}

fn write_between_search_steps(event: rusqlite::trace::TraceEvent<'_>) {
    let rusqlite::trace::TraceEvent::Stmt(statement, _) = event else {
        return;
    };
    let path = PROBE.with(|probe| {
        let mut probe = probe.borrow_mut();
        let probe = probe.as_mut()?;
        statement
            .sql()
            .contains(probe.before_sql)
            .then(|| probe.path.take())
            .flatten()
    });
    let Some(path) = path else { return };
    // No unwind crosses the SQLite callback. Join a real WAL writer before the
    // next SELECT executes; the read must retain its already-established view.
    let result = std::thread::spawn(move || -> anyhow::Result<()> {
        let mut writer = rusqlite::Connection::open(path)?;
        writer.busy_timeout(std::time::Duration::from_secs(5))?;
        let tx = writer.transaction()?;
        tx.execute(
            "UPDATE pages SET title = 'needle new', normalized_title = 'needle new'",
            [],
        )?;
        tx.execute(
            "UPDATE blocks SET markdown = 'needle new', body_stemmed = 'needl new'",
            [],
        )?;
        tx.commit()?;
        Ok(())
    })
    .join()
    .map_err(|_| "writer panicked".to_owned())
    .and_then(|result| result.map_err(|error| error.to_string()));
    PROBE.with(|probe| probe.borrow_mut().as_mut().unwrap().writer_result = Some(result));
}

async fn fixture() -> (tempfile::TempDir, Connection) {
    let directory = tempfile::tempdir().unwrap();
    let connection = open(directory.path().join("notes.db")).await.unwrap();
    let page = create_page(&connection, "needle old".into()).await.unwrap();
    create_block(
        &connection,
        page.uuid,
        None,
        None,
        BlockStyle::Paragraph,
        "needle old".into(),
    )
    .await
    .unwrap();
    (directory, connection)
}

async fn arm(connection: &Connection, path: PathBuf, before_sql: &'static str) {
    connection
        .call(move |database| {
            PROBE.with(|probe| {
                *probe.borrow_mut() = Some(Probe {
                    path: Some(path),
                    before_sql,
                    writer_result: None,
                })
            });
            database.trace_v2(
                rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
                Some(write_between_search_steps),
            );
            Ok(())
        })
        .await
        .unwrap();
}

async fn finish(connection: &Connection) {
    connection
        .call(|database| {
            database.trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);
            PROBE.with(|probe| {
                assert_eq!(
                    probe.borrow_mut().take().unwrap().writer_result,
                    Some(Ok(()))
                )
            });
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn combined_search_keeps_titles_and_blocks_in_one_wal_snapshot() {
    let (directory, connection) = fixture().await;
    arm(
        &connection,
        directory.path().join("notes.db"),
        "JOIN blocks_fts",
    )
    .await;
    let hits = search_fts(&connection, "needle".into(), 10).await.unwrap();
    finish(&connection).await;
    assert_eq!(hits.len(), 2);
    assert!(matches!(&hits[0].content, Content::Block(block) if block.markdown == "needle old"));
    assert!(
        matches!(&hits[1].content, Content::Page(page) if page.title.as_deref() == Some("needle old"))
    );
    let after = search_fts(&connection, "needle".into(), 10).await.unwrap();
    assert!(matches!(&after[0].content, Content::Block(block) if block.markdown == "needle new"));
    assert!(
        matches!(&after[1].content, Content::Page(page) if page.title.as_deref() == Some("needle new"))
    );
}

#[tokio::test]
async fn title_fallback_keeps_the_snapshot_of_its_initial_fts_attempt() {
    let (directory, connection) = fixture().await;
    arm(
        &connection,
        directory.path().join("notes.db"),
        "instr(normalized_title",
    )
    .await;
    let pages = search_pages_by_title(&connection, "ee".into(), 10)
        .await
        .unwrap();
    finish(&connection).await;
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].title.as_deref(), Some("needle old"));
    let after = search_pages_by_title(&connection, "ee".into(), 10)
        .await
        .unwrap();
    assert_eq!(after[0].title.as_deref(), Some("needle new"));
}
