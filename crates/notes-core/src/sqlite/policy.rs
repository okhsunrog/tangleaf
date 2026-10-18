//! Shared connection settings and durability for persistent application state.
//! Primary writers use WAL/FULL, including files mixing primary and derived data.
//! Explicit profiles remain available for disposable fixtures and measurements.

use anyhow::{Result, ensure};
use rusqlite::Connection;
use std::time::Duration;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterDurability {
    Normal,
    Full,
}

const PRIMARY_WRITER_DURABILITY: WriterDurability = WriterDurability::Full;

/// The common durability contract for primary content and persistent state.
pub fn configure_primary_writer(connection: &Connection) -> Result<()> {
    configure_writer(connection, PRIMARY_WRITER_DURABILITY)
}

/// Apply the same contract to a short-lived connection without reassigning the
/// initialized database's persistent journal mode while other writers use it.
pub fn configure_existing_primary_writer(connection: &Connection) -> Result<()> {
    configure_existing_wal_writer(connection, PRIMARY_WRITER_DURABILITY)
}

impl WriterDurability {
    fn pragma(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Full => "FULL",
        }
    }
}

pub fn configure_writer(connection: &Connection, durability: WriterDurability) -> Result<()> {
    ensure!(
        !connection.is_readonly("main")?,
        "SQLite writer connection is read-only"
    );
    configure_common(connection)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    let journal: String = connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    // SQLite cannot use WAL for an in-memory database. This exception supports
    // existing isolated tests; every file-backed application writer uses WAL.
    ensure!(
        journal == "wal" || (journal == "memory" && connection.path() == Some("")),
        "SQLite writer requires WAL, got journal_mode={journal}"
    );
    configure_writer_settings(connection, durability)
}

/// Configure a short-lived writer to an already initialized WAL database.
/// In particular, opening the native ink adapter must not repeatedly execute a
/// persistent journal-mode assignment while other connections are using it.
pub fn configure_existing_wal_writer(
    connection: &Connection,
    durability: WriterDurability,
) -> Result<()> {
    ensure!(
        !connection.is_readonly("main")?,
        "SQLite writer connection is read-only"
    );
    let journal: String = connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    ensure!(
        journal == "wal",
        "SQLite writer requires an existing WAL database, got journal_mode={journal}"
    );
    configure_common(connection)?;
    configure_writer_settings(connection, durability)
}

fn configure_writer_settings(connection: &Connection, durability: WriterDurability) -> Result<()> {
    connection.pragma_update(None, "query_only", "OFF")?;
    connection.pragma_update(None, "synchronous", durability.pragma())?;
    Ok(())
}

pub(super) fn configure_read_only(connection: &Connection) -> Result<()> {
    ensure!(
        connection.is_readonly("main")?,
        "SQLite reader must be opened with read-only flags"
    );
    configure_common(connection)?;
    connection.pragma_update(None, "query_only", "ON")?;
    Ok(())
}

fn configure_common(connection: &Connection) -> Result<()> {
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::Connection as AsyncConnection;
    use std::cell::RefCell;

    fn settings(connection: &Connection) -> (String, i64, i64, i64, i64, i64) {
        (
            connection
                .pragma_query_value(None, "journal_mode", |row| row.get(0))
                .unwrap(),
            connection
                .pragma_query_value(None, "synchronous", |row| row.get(0))
                .unwrap(),
            connection
                .pragma_query_value(None, "foreign_keys", |row| row.get(0))
                .unwrap(),
            connection
                .pragma_query_value(None, "temp_store", |row| row.get(0))
                .unwrap(),
            connection
                .pragma_query_value(None, "busy_timeout", |row| row.get(0))
                .unwrap(),
            connection
                .pragma_query_value(None, "query_only", |row| row.get(0))
                .unwrap(),
        )
    }

    #[tokio::test]
    async fn async_writer_profiles_set_the_requested_durability_and_shared_settings() {
        let directory = tempfile::tempdir().unwrap();
        for (durability, synchronous) in
            [(WriterDurability::Normal, 1), (WriterDurability::Full, 2)]
        {
            let path = directory.path().join(format!("{durability:?}.db"));
            for _ in 0..2 {
                let connection = AsyncConnection::open_writer(&path, durability)
                    .await
                    .unwrap();
                let actual = connection.call(|raw| Ok(settings(raw))).await.unwrap();
                assert_eq!(actual, ("wal".into(), synchronous, 1, 2, 5000, 0));
                connection.close().await.unwrap();
            }
        }
    }

    #[tokio::test]
    async fn main_database_open_uses_primary_full_and_the_same_shared_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.db");
        for _ in 0..2 {
            let connection = crate::db::open(&path).await.unwrap();
            assert_eq!(
                connection.call(|raw| Ok(settings(raw))).await.unwrap(),
                ("wal".into(), 2, 1, 2, 5000, 0)
            );
            connection.close().await.unwrap();
        }
        let memory = crate::db::open_in_memory().await.unwrap();
        assert_eq!(
            memory.call(|raw| Ok(settings(raw))).await.unwrap(),
            ("memory".into(), 2, 1, 2, 5000, 0)
        );
        memory.close().await.unwrap();
    }

    #[tokio::test]
    async fn primary_writer_opener_uses_full_and_shared_settings_on_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("primary.db");
        for _ in 0..2 {
            let connection = AsyncConnection::open_primary_writer(&path).await.unwrap();
            assert_eq!(
                connection.call(|raw| Ok(settings(raw))).await.unwrap(),
                ("wal".into(), 2, 1, 2, 5000, 0)
            );
            connection.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn read_only_open_never_creates_a_missing_database() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.db");
        assert!(AsyncConnection::open_read_only(&missing).await.is_err());
        assert!(!missing.exists());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn read_only_open_preserves_schema_and_journal_and_refuses_permanent_and_temporary_writes()
     {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.db");
        let writer = AsyncConnection::open_writer(&path, WriterDurability::Normal)
            .await
            .unwrap();
        writer.call(|raw| raw.execute_batch("CREATE TABLE primary_content(value TEXT); INSERT INTO primary_content VALUES ('saved');")).await.unwrap();
        let reader = AsyncConnection::open_read_only(&path).await.unwrap();
        let actual = reader.call(|raw| Ok(settings(raw))).await.unwrap();
        assert_eq!(
            (actual.0, actual.2, actual.3, actual.4, actual.5),
            ("wal".into(), 1, 2, 5000, 1)
        );
        assert_eq!(
            reader
                .call(
                    |raw| raw.query_row("SELECT value FROM primary_content", [], |row| row
                        .get::<_, String>(0))
                )
                .await
                .unwrap(),
            "saved"
        );
        for statement in [
            "INSERT INTO primary_content VALUES ('lost')",
            "CREATE TABLE forbidden(value)",
            "CREATE TEMP TABLE forbidden_temp(value)",
        ] {
            let error = reader
                .call(move |raw| raw.execute_batch(statement))
                .await
                .unwrap_err();
            assert!(
                matches!(error.downcast_ref::<rusqlite::Error>(), Some(rusqlite::Error::SqliteFailure(error, _)) if error.code == rusqlite::ErrorCode::ReadOnly)
            );
        }
        // query_only is a second guard; disabling it cannot bypass the actual
        // read-only open flags for the persistent database.
        reader
            .call(|raw| raw.pragma_update(None, "query_only", "OFF"))
            .await
            .unwrap();
        assert!(
            reader
                .call(|raw| raw.execute("DELETE FROM primary_content", []))
                .await
                .is_err()
        );
        let tables = reader
            .call(|raw| {
                raw.prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")?
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .unwrap();
        assert_eq!(tables, vec!["primary_content"]);
        writer
            .call(|raw| raw.execute("INSERT INTO primary_content VALUES ('second commit')", []))
            .await
            .unwrap();
        assert_eq!(
            reader
                .call(
                    |raw| raw.query_row("SELECT COUNT(*) FROM primary_content", [], |row| row
                        .get::<_, i64>(0))
                )
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            writer.call(|raw| Ok(settings(raw))).await.unwrap(),
            ("wal".into(), 1, 1, 2, 5000, 0)
        );
    }

    #[tokio::test]
    async fn read_only_open_does_not_upgrade_a_legacy_journal_mode() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.db");
        {
            let raw = Connection::open(&path).unwrap();
            raw.execute_batch("CREATE TABLE legacy(value); INSERT INTO legacy VALUES (1);")
                .unwrap();
            assert_eq!(settings(&raw).0, "delete");
        }
        let reader = AsyncConnection::open_read_only(&path).await.unwrap();
        assert_eq!(
            reader.call(|raw| Ok(settings(raw).0)).await.unwrap(),
            "delete"
        );
        assert_eq!(
            reader
                .call(
                    |raw| raw.query_row("SELECT value FROM legacy", [], |row| row.get::<_, i64>(0))
                )
                .await
                .unwrap(),
            1
        );
    }

    #[test]
    fn existing_writer_requires_wal_without_changing_the_persistent_mode() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.db");
        let raw = Connection::open(&path).unwrap();
        assert_eq!(settings(&raw).0, "delete");
        let error = configure_existing_primary_writer(&raw).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("requires an existing WAL database")
        );
        assert_eq!(settings(&raw).0, "delete");
    }

    #[test]
    fn existing_wal_configuration_does_not_assign_journal_mode() {
        thread_local! { static STATEMENTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) }; }
        let directory = tempfile::tempdir().unwrap();
        let raw = Connection::open(directory.path().join("notes.db")).unwrap();
        configure_writer(&raw, WriterDurability::Normal).unwrap();
        raw.trace_v2(
            rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
            Some(|event| {
                if let rusqlite::trace::TraceEvent::Stmt(statement, _) = event {
                    STATEMENTS.with_borrow_mut(|statements| {
                        statements.push(statement.sql().into_owned())
                    });
                }
            }),
        );
        configure_existing_primary_writer(&raw).unwrap();
        assert_eq!(settings(&raw), ("wal".into(), 2, 1, 2, 5000, 0));
        STATEMENTS.with_borrow(|statements| {
            assert!(statements.iter().any(|statement| {
                statement
                    .to_ascii_lowercase()
                    .contains("pragma journal_mode")
            }));
            assert!(!statements.iter().any(|statement| {
                statement
                    .to_ascii_lowercase()
                    .replace(' ', "")
                    .contains("journal_mode=")
            }));
        });
    }
}
