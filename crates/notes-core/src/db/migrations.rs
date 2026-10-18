use crate::sqlite::Connection;
use anyhow::{Context, Result};
use rusqlite_migration::{M, Migrations};

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(include_str!("migrations/V001__initial.sql")),
        M::up(include_str!("migrations/V002__stem_page_titles.sql")),
        M::up(include_str!("migrations/V003__attachment_image_cache.sql")),
        M::up(include_str!("migrations/V004__handwriting.sql")),
        M::up(include_str!("migrations/V005__outbox_quarantine.sql")),
        M::up(include_str!("migrations/V006__store_commit_clock.sql")),
        M::up(include_str!("migrations/V007__structure_dependencies.sql")),
    ])
}

pub(super) async fn migrate(connection: &Connection) -> Result<()> {
    connection
        .call(|database| {
            migrations()
                .to_latest(database)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;

            // V002 cannot run Snowball inside SQLite. Backfill after the schema
            // transaction; the empty-value predicate makes retry after a crash safe.
            let titles = {
                let mut statement = database.prepare(
                    "SELECT rowid, title FROM pages
                      WHERE title IS NOT NULL AND title_stemmed = ''",
                )?;
                statement
                    .query_map([], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            let transaction = database.transaction()?;
            for (rowid, title) in titles {
                transaction.execute(
                    "UPDATE pages SET title_stemmed = ?2 WHERE rowid = ?1",
                    rusqlite::params![rowid, crate::stem::stem(&title)],
                )?;
            }
            transaction.commit()
        })
        .await
        .context("applying embedded database migrations")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn baseline_migration_is_valid() {
        migrations().validate().expect("valid migrations");
    }

    #[tokio::test]
    async fn v002_backfills_existing_titles_into_the_stemmed_index() {
        let file = tempfile::NamedTempFile::new().expect("temporary database");
        let page_uuid = uuid::Uuid::from_u128(1);
        {
            let mut database = rusqlite::Connection::open(file.path()).expect("open baseline");
            Migrations::new(vec![M::up(include_str!("migrations/V001__initial.sql"))])
                .to_latest(&mut database)
                .expect("apply baseline");
            database
                .execute(
                    "INSERT INTO page_identities(page_uuid, page_kind) VALUES (?1, 'note')",
                    [page_uuid],
                )
                .expect("insert page identity");
            database
                .execute(
                    "INSERT INTO pages(
                       uuid, title, normalized_title, layout, existence_hlc, created_at, updated_at
                     ) VALUES (?1, 'Проекты', 'проекты', 'outline', '1', 1, 1)",
                    [page_uuid],
                )
                .expect("insert legacy page");
        }

        let connection = Connection::open(file.path())
            .await
            .expect("reopen database");
        migrate(&connection).await.expect("apply V002");
        let (stored_stem, matches): (String, i64) = connection
            .call(move |database| {
                Ok((
                    database.query_row(
                        "SELECT title_stemmed FROM pages WHERE uuid = ?1",
                        [page_uuid],
                        |row| row.get(0),
                    )?,
                    database.query_row(
                        "SELECT count(*) FROM pages_fts WHERE pages_fts MATCH '\"проект\"*'",
                        [],
                        |row| row.get(0),
                    )?,
                ))
            })
            .await
            .expect("inspect migrated index");
        assert_eq!(stored_stem, crate::stem::stem("Проекты"));
        assert_eq!(matches, 1);
    }

    fn assert_dependency_query_plan(
        database: &rusqlite::Connection,
        sql: &str,
        parameters: impl rusqlite::Params,
        index: &str,
        covering: bool,
    ) {
        let plan = database
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map(parameters, |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join("\n");
        let expected = if covering {
            format!("USING COVERING INDEX {index}")
        } else {
            format!("INDEX {index}")
        };
        assert!(
            plan.contains("SEARCH ") && plan.contains(&expected),
            "{sql}: {plan}"
        );
        assert!(!plan.contains("SCAN block_structure_lww"), "{sql}: {plan}");
    }

    #[test]
    fn v007_dependency_queries_use_parent_and_page_indexes() {
        let mut database = rusqlite::Connection::open_in_memory().unwrap();
        migrations().to_latest(&mut database).unwrap();
        let first = uuid::Uuid::from_u128(1);
        let second = uuid::Uuid::from_u128(2);
        assert_dependency_query_plan(
            &database,
            "SELECT block_uuid, parent_uuid FROM block_structure_lww WHERE parent_uuid IN (?1, ?2)",
            [first, second],
            "idx_structure_parent",
            true,
        );
        assert_dependency_query_plan(
            &database,
            "SELECT block_uuid, parent_uuid FROM block_structure_lww WHERE page_uuid IN (?1, ?2)",
            [first, second],
            "idx_structure_page",
            false,
        );
        assert_dependency_query_plan(
            &database,
            "SELECT block_uuid FROM block_structure_lww WHERE page_uuid IN (?1, ?2)",
            [first, second],
            "idx_structure_page",
            true,
        );
        assert_dependency_query_plan(
            &database,
            "SELECT block_uuid FROM block_structure_lww WHERE parent_uuid = ?1",
            [first],
            "idx_structure_parent",
            true,
        );
        assert_dependency_query_plan(
            &database,
            "SELECT block_uuid FROM block_structure_lww WHERE page_uuid = ?1",
            [first],
            "idx_structure_page",
            true,
        );
    }

    type DatabaseRows = BTreeMap<String, Vec<Vec<rusqlite::types::Value>>>;

    async fn persistent_rows(connection: &Connection) -> DatabaseRows {
        connection
            .call(|database| {
                let tables = database
                    .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")?
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut image = BTreeMap::new();
                for table in tables {
                    let identifier = table.replace('"', "\"\"");
                    let columns = database
                        .prepare(&format!("SELECT * FROM \"{identifier}\""))?
                        .column_count();
                    let ordering = (1..=columns)
                        .map(|column| column.to_string())
                        .collect::<Vec<_>>()
                        .join(",");
                    let rows = database
                        .prepare(&format!(
                            "SELECT * FROM \"{identifier}\" ORDER BY {ordering}"
                        ))?
                        .query_map([], |row| {
                            (0..columns)
                                .map(|column| row.get(column))
                                .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    image.insert(table, rows);
                }
                Ok(image)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn v007_upgrade_preserves_all_v006_rows_and_database_integrity() {
        use crate::db::{self, BlockContent};
        use crate::{AttachmentOwner, BlobHash, BlockStyle};

        let file = tempfile::NamedTempFile::new().unwrap();
        let connection = Connection::open(file.path()).await.unwrap();
        connection
            .call(|database| {
                database.pragma_update(None, "foreign_keys", "ON")?;
                migrations()
                    .to_version(database, 6)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
                assert_eq!(
                    database
                        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))?,
                    6
                );
                Ok(())
            })
            .await
            .unwrap();
        db::workspace::initialize_workspace(&connection)
            .await
            .unwrap();
        crate::configure_sync(&connection, "https://migration.example.test")
            .await
            .unwrap();
        let target = db::create_note(&connection, Some("Migration target".into()))
            .await
            .unwrap()
            .into_created()
            .unwrap();
        let source = db::create_note(&connection, Some("Version six note".into()))
            .await
            .unwrap()
            .into_created()
            .unwrap();
        db::create_block(
            &connection,
            source.page.uuid,
            Some(source.initial_block.uuid),
            None,
            BlockStyle::Bullet,
            format!("Nested (({}))", target.initial_block.uuid),
        )
        .await
        .unwrap();
        db::create_attachment(
            &connection,
            AttachmentOwner::Block(source.initial_block.uuid),
            BlobHash::from_bytes([0x71; 32]),
            "kept.bin".into(),
            "application/octet-stream".into(),
            13,
        )
        .await
        .unwrap();
        db::split_block(
            &connection,
            source.initial_block.uuid,
            ["First [[Migration target]]", "Second"]
                .into_iter()
                .map(|markdown| BlockContent {
                    markdown: markdown.into(),
                })
                .collect(),
            source.initial_block.markdown_revision,
        )
        .await
        .unwrap();
        assert_eq!(
            db::undo_history(&connection).await.unwrap(),
            db::HistoryMoveResult::Applied
        );
        let ink_page =
            db::create_handwritten_note_with_ops(&connection, Some("Existing ink".into()))
                .await
                .unwrap()
                .value
                .uuid;
        connection
            .call(move |database| {
                let root = uuid::Uuid::from_u128(0x7001);
                database.execute(
                    "INSERT INTO ink_records(id, data, hash) VALUES (?1, ?2, ?3)",
                    rusqlite::params![root, vec![0x80_u8], vec![0x72_u8; 32]],
                )?;
                database.execute(
                    "INSERT INTO ink_documents(page_uuid, root_id, revision, cursor, dirty)
                VALUES (?1, ?2, 'existing-revision', 7, 1)",
                    rusqlite::params![ink_page, root],
                )?;
                database.execute(
                    "UPDATE sync_outbox SET quarantined_at = 7, quarantine_reason = 'preserved'
                WHERE rowid = (SELECT MIN(rowid) FROM sync_outbox)",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        let before = persistent_rows(&connection).await;
        for table in [
            "blocks",
            "block_structure_lww",
            "history_undo",
            "history_redo",
            "sync_outbox",
            "applied_ops",
            "attachments",
            "ink_records",
            "ink_documents",
        ] {
            assert!(!before[table].is_empty(), "fixture must populate {table}");
        }
        migrate(&connection).await.unwrap();
        assert_eq!(persistent_rows(&connection).await, before);
        connection
            .call(|database| {
                assert_eq!(
                    database
                        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))?,
                    7
                );
                let foreign_key_errors = database
                    .prepare("PRAGMA foreign_key_check")?
                    .query_map([], |_| Ok(()))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                assert!(foreign_key_errors.is_empty());
                let integrity = database
                    .prepare("PRAGMA integrity_check")?
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                assert_eq!(integrity, ["ok"]);
                Ok(())
            })
            .await
            .unwrap();
        migrate(&connection).await.unwrap();
        assert_eq!(persistent_rows(&connection).await, before);
    }
}
