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
        M::up(include_str!("migrations/V008__store_projection_state.sql")),
        M::up(include_str!("migrations/V009__history_ink_retention.sql")),
        M::up(include_str!("migrations/V010__workspace_epoch.sql")),
        M::up_with_hook(
            include_str!("migrations/V011__ink_outbox_roots.sql"),
            |transaction| {
                crate::ink::outbox_retention::backfill(transaction)
                    .map_err(|error| rusqlite_migration::HookError::Hook(error.to_string()))
            },
        ),
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
    async fn v010_adds_zero_replacement_epoch_without_rewriting_existing_data() {
        let (_file, connection) = populated_v006_database().await;
        migrate_to_version(&connection, 9).await;
        connection
            .call(|database| {
                database.execute(
                    "UPDATE store_state SET version=73,structure_projection_version=1",
                    [],
                )?;
                let action = uuid::Uuid::from_u128(0x10_001);
                let page = uuid::Uuid::from_u128(0x6003);
                database.execute(
                    "INSERT INTO history_ink_snapshots VALUES(?1,?2,?3,?4)",
                    rusqlite::params![action, page, "preserved recovery metadata", vec![7u8; 32]],
                )?;
                assert_eq!(
                    database.execute(
                        "INSERT INTO history_ink_roots SELECT ?1,?2,id FROM ink_records LIMIT 1",
                        (action, page)
                    )?,
                    1
                );
                Ok(())
            })
            .await
            .unwrap();
        let before = persistent_rows(&connection).await;
        migrate_to_version(&connection, 10).await;
        let mut after = persistent_rows(&connection).await;
        for row in after.get_mut("store_state").unwrap() {
            assert_eq!(row.pop(), Some(rusqlite::types::Value::Integer(0)));
        }
        assert_eq!(after, before);
        connection
            .call(|database| {
                assert_eq!(
                    database
                        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))?,
                    10
                );
                assert_eq!(
                    database
                        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?,
                    "ok"
                );
                assert!(
                    database
                        .prepare("PRAGMA foreign_key_check")?
                        .query_map([], |_| Ok(()))?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                        .is_empty()
                );
                Ok(())
            })
            .await
            .unwrap();
        let once = persistent_rows(&connection).await;
        migrate_to_version(&connection, 10).await;
        assert_eq!(persistent_rows(&connection).await, once);
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

    async fn populated_v006_database() -> (tempfile::NamedTempFile, Connection) {
        use crate::db;
        use crate::operation::{
            self, AttachmentAdd, BlockCreate, BlockSetMarkdown, OpKind, PageCreate,
            ProjectionPolicy, StructureProjectionMode,
        };
        use crate::{
            AttachmentOwner, BlobHash, BlockStyle, CoreResult, OrderKey, PageKind, PageLayout,
        };

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
        // Populate historical metadata directly: current Store commands need
        // columns and the projection marker added after this V006 fixture.
        connection.call(|database| {
            database.execute("INSERT INTO sync_meta(key,value) VALUES('server_url','https://migration.example.test')", [])?;
            Ok(())
        }).await.unwrap();
        let ink_page = uuid::Uuid::from_u128(0x6003);
        connection
            .call_domain(move |database| -> CoreResult<()> {
                let transaction = database.transaction()?;
                let target = uuid::Uuid::from_u128(0x6001);
                let source = uuid::Uuid::from_u128(0x6002);
                let target_block = uuid::Uuid::from_u128(0x6011);
                let source_block = uuid::Uuid::from_u128(0x6012);
                let child = uuid::Uuid::from_u128(0x6013);
                let create_page = |uuid, kind, title: &str| {
                    OpKind::PageCreate(PageCreate {
                        uuid,
                        kind,
                        title: Some(title.into()),
                        layout: PageLayout::Outline,
                        created_at: 1,
                    })
                };
                let create_block = |uuid, page_uuid, parent_uuid, markdown: String| {
                    OpKind::BlockCreate(BlockCreate {
                        uuid,
                        page_uuid,
                        parent_uuid,
                        order_key: OrderKey::first(),
                        style: BlockStyle::Bullet,
                        markdown,
                        created_at: 1,
                    })
                };
                // Historical migration fixtures deliberately populate the old
                // schema through the explicit full projector. Normal current
                // commands require the canonical marker introduced in V008.
                let kinds = vec![
                    create_page(target, PageKind::Note, "Migration target"),
                    create_page(source, PageKind::Note, "Version six note"),
                    create_page(ink_page, PageKind::Handwriting, "Existing ink"),
                    create_block(target_block, target, None, String::new()),
                    create_block(source_block, source, None, String::new()),
                    create_block(
                        child,
                        source,
                        Some(source_block),
                        format!("Nested (({target_block}))"),
                    ),
                    OpKind::AttachmentAdd(AttachmentAdd {
                        owner: AttachmentOwner::Block(source_block),
                        blob_hash: BlobHash::from_bytes([0x71; 32]),
                        filename: "kept.bin".into(),
                        mime: "application/octet-stream".into(),
                        size: 13,
                    }),
                ];
                let inverse = db::history::capture_inverse_kinds(&transaction, &kinds)?;
                operation::apply_local_kinds_deferred_in_transaction(
                    &transaction,
                    kinds.clone(),
                    StructureProjectionMode::Full,
                )?;
                db::history::record_action(
                    &transaction,
                    "Fixture create",
                    kinds,
                    inverse,
                    ProjectionPolicy::Deferred,
                )?;
                let kinds = vec![OpKind::BlockSetMarkdown(BlockSetMarkdown {
                    uuid: source_block,
                    markdown: "First [[Migration target]]\n\nSecond".into(),
                })];
                let inverse = db::history::capture_inverse_kinds(&transaction, &kinds)?;
                operation::apply_local_kinds_deferred_in_transaction(
                    &transaction,
                    kinds.clone(),
                    StructureProjectionMode::Full,
                )?;
                db::history::record_action(
                    &transaction,
                    "Fixture edit",
                    kinds,
                    inverse,
                    ProjectionPolicy::Deferred,
                )?;
                transaction.commit()?;
                Ok(())
            })
            .await
            .unwrap();
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
                // Populate both old-schema stacks without invoking commands
                // whose current executor now requires store_state from V008.
                database.execute_batch(
                    "INSERT INTO history_redo(action_uuid, action, forward_json, inverse_json, created_at)
                     SELECT action_uuid, action, forward_json, inverse_json, created_at
                       FROM history_undo ORDER BY id DESC LIMIT 1;
                     DELETE FROM history_undo WHERE id = (SELECT MAX(id) FROM history_undo);",
                )?;
                Ok(())
            })
            .await
            .unwrap();
        (file, connection)
    }

    async fn migrate_to_version(connection: &Connection, version: usize) {
        connection
            .call(move |database| {
                migrations()
                    .to_version(database, version)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn v007_upgrade_preserves_all_v006_rows_and_database_integrity() {
        let (_file, connection) = populated_v006_database().await;
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
        migrate_to_version(&connection, 7).await;
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
        migrate_to_version(&connection, 7).await;
        assert_eq!(persistent_rows(&connection).await, before);
    }

    #[tokio::test]
    async fn v008_preserves_commit_order_and_content_and_requires_runtime_projection_activation() {
        use rusqlite::types::Value;

        let (_file, connection) = populated_v006_database().await;
        migrate_to_version(&connection, 7).await;
        connection
            .call(|database| {
                database.execute(
                    "UPDATE store_commit_clock SET version = 37 WHERE singleton = 1",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let mut before = persistent_rows(&connection).await;
        assert_eq!(
            before.remove("store_commit_clock").unwrap(),
            [vec![Value::Integer(1), Value::Integer(37)]]
        );

        migrate_to_version(&connection, 8).await;

        let mut after = persistent_rows(&connection).await;
        assert!(!after.contains_key("store_commit_clock"));
        assert_eq!(
            after.remove("store_state").unwrap(),
            [vec![
                Value::Integer(1),
                Value::Integer(37),
                Value::Integer(0)
            ]]
        );
        assert_eq!(
            after, before,
            "all other persistent rows must survive unchanged"
        );
        connection
            .call(|database| {
                assert_eq!(
                    database
                        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))?,
                    8
                );
                assert!(
                    database
                        .execute(
                            "UPDATE store_state SET structure_projection_version = -1",
                            []
                        )
                        .is_err()
                );
                assert_eq!(
                    database.query_row(
                        "SELECT structure_projection_version FROM store_state WHERE singleton = 1",
                        [],
                        |row| row.get::<_, i64>(0)
                    )?,
                    0
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
        let once = persistent_rows(&connection).await;
        migrate_to_version(&connection, 8).await;
        assert_eq!(persistent_rows(&connection).await, once);
    }

    #[tokio::test]
    async fn v009_adds_empty_history_retention_without_changing_existing_data() {
        let (_file, connection) = populated_v006_database().await;
        migrate_to_version(&connection, 8).await;
        connection
            .call(|database| {
                database.execute(
                    "UPDATE store_state SET version=73,structure_projection_version=1",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let before = persistent_rows(&connection).await;
        migrate_to_version(&connection, 9).await;
        let mut after = persistent_rows(&connection).await;
        assert!(after.remove("history_ink_snapshots").unwrap().is_empty());
        assert!(after.remove("history_ink_roots").unwrap().is_empty());
        assert_eq!(after, before);
        connection
            .call(|database| {
                assert_eq!(
                    database
                        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))?,
                    9
                );
                assert!(
                    database
                        .prepare("PRAGMA foreign_key_check")?
                        .query_map([], |_| Ok(()))?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                        .is_empty()
                );
                assert_eq!(
                    database
                        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?,
                    "ok"
                );
                Ok(())
            })
            .await
            .unwrap();
        let once = persistent_rows(&connection).await;
        migrate_to_version(&connection, 9).await;
        assert_eq!(persistent_rows(&connection).await, once);
    }
}
