use super::*;
use crate::db;
use rusqlite::types::Value;
use std::cell::Cell;
use std::collections::BTreeMap;

thread_local! { static FULL_READS: Cell<usize> = const { Cell::new(0) }; }

async fn full_reads(conn: &Connection, start: bool) -> usize {
    conn.call(move |database| {
        if start {
            FULL_READS.set(0);
            database.trace_v2(
                rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
                Some(|event| {
                    if let rusqlite::trace::TraceEvent::Stmt(statement, _) = event
                        && statement
                            .sql()
                            .starts_with("SELECT uuid, existence_hlc, page_uuid")
                        && statement.sql().trim_end().ends_with("FROM blocks")
                    {
                        FULL_READS.set(FULL_READS.get() + 1);
                    }
                }),
            );
        } else {
            database.trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);
        }
        Ok(FULL_READS.get())
    })
    .await
    .unwrap()
}

async fn immutable_tables(conn: &Connection) -> BTreeMap<String, Vec<Vec<Value>>> {
    conn.call(|database| {
        let names = database
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table'
             AND name NOT IN ('blocks', 'store_state', 'sqlite_sequence') ORDER BY name",
            )?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut tables = BTreeMap::new();
        for name in names {
            let sql = format!("SELECT * FROM \"{}\"", name.replace('"', "\"\""));
            let mut query = database.prepare(&sql)?;
            let columns = query.column_count();
            let mut rows = query
                .query_map([], |row| {
                    (0..columns)
                        .map(|column| row.get::<_, Value>(column))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows.sort_by_cached_key(|row| format!("{row:?}"));
            tables.insert(name, rows);
        }
        Ok(tables)
    })
    .await
    .unwrap()
}

async fn stale_fixture() -> (tempfile::TempDir, Connection, db::Block, uuid::Uuid) {
    let directory = tempfile::tempdir().unwrap();
    let conn = db::open(directory.path().join("notes.db")).await.unwrap();
    configure_sync(&conn, "https://sync.example").await.unwrap();
    let original_page = db::create_page(&conn, "Intended page".into())
        .await
        .unwrap();
    let stale_page = db::create_page(&conn, "Stale materialization".into())
        .await
        .unwrap();
    let block = db::create_block(
        &conn,
        original_page.uuid,
        None,
        None,
        crate::BlockStyle::Bullet,
        "Preserve **text** and [[Intended page]]".into(),
    )
    .await
    .unwrap();
    let uuid = block.uuid;
    conn.call(move |database| {
        database.execute(
            "UPDATE blocks SET page_uuid = ?2 WHERE uuid = ?1",
            rusqlite::params![uuid, stale_page.uuid],
        )?;
        database.execute(
            "UPDATE store_state SET structure_projection_version = 0, version = 42",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    (directory, conn, block, stale_page.uuid)
}

#[tokio::test]
async fn structure_activation_repairs_legacy_baseline_once_without_authored_mutations() {
    let (directory, conn, original, _) = stale_fixture().await;
    let before = immutable_tables(&conn).await;
    full_reads(&conn, true).await;
    assert!(initialize_structure_projection(&conn).await.unwrap());
    assert_eq!(full_reads(&conn, false).await, 1);
    assert_eq!(
        db::get_block(&conn, original.uuid).await.unwrap(),
        Some(original.clone())
    );
    assert_eq!(immutable_tables(&conn).await, before);
    let state = conn
        .call(|database| {
            database.query_row(
                "SELECT version, structure_projection_version FROM store_state",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(state, (42, STRUCTURE_PROJECTION_VERSION));
    full_reads(&conn, true).await;
    assert!(!initialize_structure_projection(&conn).await.unwrap());
    assert_eq!(full_reads(&conn, false).await, 0);
    drop(conn);
    let reopened = db::open(directory.path().join("notes.db")).await.unwrap();
    assert_eq!(
        db::get_block(&reopened, original.uuid).await.unwrap(),
        Some(original)
    );
    assert_eq!(immutable_tables(&reopened).await, before);
}

#[tokio::test]
async fn structure_activation_failure_rolls_back_repair_and_retries_on_open() {
    let (directory, conn, original, stale_page) = stale_fixture().await;
    let before = immutable_tables(&conn).await;
    conn.call(|database| database.execute_batch(
        "CREATE TRIGGER reject_projection_marker BEFORE UPDATE OF structure_projection_version ON store_state
         BEGIN SELECT RAISE(ABORT, 'injected projection marker failure'); END;",
    )).await.unwrap();
    assert!(initialize_structure_projection(&conn).await.is_err());
    assert_eq!(
        db::get_block(&conn, original.uuid)
            .await
            .unwrap()
            .unwrap()
            .page_uuid,
        stale_page
    );
    assert_eq!(immutable_tables(&conn).await, before);
    let marker = conn
        .call(|database| {
            database.query_row(
                "SELECT structure_projection_version FROM store_state",
                [],
                |row| row.get::<_, i64>(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(marker, 0);
    drop(conn);
    assert!(db::open(directory.path().join("notes.db")).await.is_err());
    {
        let raw = rusqlite::Connection::open(directory.path().join("notes.db")).unwrap();
        raw.execute_batch("DROP TRIGGER reject_projection_marker")
            .unwrap();
    }
    let reopened = db::open(directory.path().join("notes.db")).await.unwrap();
    assert_eq!(
        db::get_block(&reopened, original.uuid).await.unwrap(),
        Some(original)
    );
    assert_eq!(immutable_tables(&reopened).await, before);
}

#[tokio::test]
async fn structure_activation_rejects_future_projection_without_rewriting_it() {
    let (directory, conn, original, stale_page) = stale_fixture().await;
    conn.call(|database| {
        database.execute(
            "UPDATE store_state SET structure_projection_version = 2",
            [],
        )
    })
    .await
    .unwrap();
    let before = immutable_tables(&conn).await;
    let error = initialize_structure_projection(&conn).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unsupported structural projection version 2")
    );
    assert_eq!(
        db::get_block(&conn, original.uuid)
            .await
            .unwrap()
            .unwrap()
            .page_uuid,
        stale_page
    );
    assert_eq!(immutable_tables(&conn).await, before);
    drop(conn);
    assert!(db::open(directory.path().join("notes.db")).await.is_err());
}
