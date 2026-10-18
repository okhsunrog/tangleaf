//! Abrupt application termination at transaction boundaries, not power-loss
//! testing: the OS and its filesystem cache remain running in these tests.

use notes_core::db::{self, BlockContent};
use notes_core::{
    BlockCreate, BlockSetMarkdown, BlockStyle, Connection, Hlc, Op, OpKind, OrderKey,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{BufRead, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use uuid::Uuid;

const READY: &str = "TANGLEAF_STORAGE_RECOVERY_READY";
const ORIGINAL: &str = "Original paragraph\n\nSecond original paragraph";
const REMOTE_BLOCK: Uuid = Uuid::from_u128(0xABCD_0042);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
enum SqlValue {
    Null,
    Integer(i64),
    RealBits(u64),
    Text(String),
    Blob(Vec<u8>),
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct DatabaseImage {
    user_version: i64,
    tables: BTreeMap<String, Vec<Vec<SqlValue>>>,
}

impl DatabaseImage {
    fn count(&self, table: &str) -> usize {
        self.tables[table].len()
    }
}

// Include all persistent tables, including FTS shadow tables, raw structure
// intents, history payloads, applied operation IDs, outbox and sync metadata.
// Counts alone would miss a partially updated row or a changed history guard.
async fn database_image(connection: &Connection) -> DatabaseImage {
    connection
        .call(|database| {
            let tables = database
                .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")?
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut image = BTreeMap::new();
            for table in tables {
                let identifier = table.replace('"', "\"\"");
                let mut statement = database.prepare(&format!("SELECT * FROM \"{identifier}\""))?;
                let columns = statement.column_count();
                let mut rows = statement
                    .query_map([], |row| {
                        (0..columns)
                            .map(|index| {
                                Ok(match row.get::<_, rusqlite::types::Value>(index)? {
                                    rusqlite::types::Value::Null => SqlValue::Null,
                                    rusqlite::types::Value::Integer(value) => {
                                        SqlValue::Integer(value)
                                    }
                                    rusqlite::types::Value::Real(value) => {
                                        SqlValue::RealBits(value.to_bits())
                                    }
                                    rusqlite::types::Value::Text(value) => SqlValue::Text(value),
                                    rusqlite::types::Value::Blob(value) => SqlValue::Blob(value),
                                })
                            })
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows.sort();
                image.insert(table, rows);
            }
            Ok(DatabaseImage {
                user_version: database
                    .pragma_query_value(None, "user_version", |row| row.get(0))?,
                tables: image,
            })
        })
        .await
        .expect("snapshot all persistent rows")
}

#[derive(Debug, Serialize, Deserialize)]
struct Fixture {
    source: db::Block,
    child: db::Block,
    target_page: Uuid,
    target_block: Uuid,
    workspace: Uuid,
    outbox: Vec<Op>,
    baseline: DatabaseImage,
}

impl Fixture {
    fn autosave_text(&self) -> String {
        format!("Saved [[Recovery target]] and (({})).", self.target_block)
    }

    fn split_parts(&self) -> Vec<BlockContent> {
        [
            "First [[Recovery target]]".to_owned(),
            format!("Second (({}))", self.target_block),
            "Third paragraph".to_owned(),
        ]
        .into_iter()
        .map(|markdown| BlockContent { markdown })
        .collect()
    }

    fn sequenced_ops(&self) -> Vec<(u64, Op)> {
        assert_eq!(self.outbox.len(), 1);
        let device = Uuid::from_u128(0xABCD_0010);
        let next_wall = self.outbox[0].hlc.wall_ms() + 1;
        let remote = |index, kind| Op {
            op_id: Uuid::from_u128(0xABCD_0020 + u128::from(index)),
            workspace_uuid: self.workspace,
            device_id: device,
            hlc: Hlc::new(next_wall, index, device),
            format_version: notes_core::operation::FORMAT_VERSION,
            kind,
        };
        vec![
            // A server echo must remove this local outbox entry in the same
            // transaction that assigns its sequence and advances the cursor.
            (1, self.outbox[0].clone()),
            (
                2,
                remote(
                    1,
                    OpKind::BlockSetMarkdown(BlockSetMarkdown {
                        uuid: self.source.uuid,
                        markdown: "Remote [[Recovery target]]".into(),
                    }),
                ),
            ),
            (
                3,
                remote(
                    2,
                    OpKind::BlockCreate(BlockCreate {
                        uuid: REMOTE_BLOCK,
                        page_uuid: self.source.page_uuid,
                        parent_uuid: None,
                        order_key: OrderKey::from_ordinal(20),
                        style: BlockStyle::Paragraph,
                        markdown: format!("Remote child link (({}))", self.target_block),
                        created_at: 1,
                    }),
                ),
            ),
        ]
    }
}

async fn fixture(connection: &Connection) -> Fixture {
    let target = db::create_note(connection, Some("Recovery target".into()))
        .await
        .unwrap()
        .into_created()
        .unwrap();
    let source = db::create_note(connection, Some("Initial source title".into()))
        .await
        .unwrap()
        .into_created()
        .unwrap();
    let (source_block, _, _) = db::set_block_content_if_revision(
        connection,
        source.initial_block.uuid,
        BlockContent {
            markdown: ORIGINAL.into(),
        },
        source.initial_block.markdown_revision,
    )
    .await
    .unwrap();
    let child = db::create_block(
        connection,
        source.page.uuid,
        Some(source_block.uuid),
        None,
        BlockStyle::Bullet,
        "Nested child survives".into(),
    )
    .await
    .unwrap();
    notes_core::configure_sync(connection, "https://recovery.example.test")
        .await
        .unwrap();
    db::rename_page_if_revision(
        connection,
        source.page.uuid,
        Some("Recovery source".into()),
        source.page.title_revision,
    )
    .await
    .unwrap();
    Fixture {
        source: source_block,
        child,
        target_page: target.page.uuid,
        target_block: target.initial_block.uuid,
        workspace: db::workspace_uuid(connection).await.unwrap(),
        outbox: notes_core::pending_outbox(connection, 100).await.unwrap(),
        baseline: database_image(connection).await,
    }
}

fn save_json(path: &Path, value: &impl Serialize) {
    std::fs::write(path, serde_json::to_vec(value).expect("serialize fixture"))
        .expect("write fixture");
}

fn load_json<T: serde::de::DeserializeOwned>(path: &Path) -> T {
    serde_json::from_slice(&std::fs::read(path).expect("read child fixture"))
        .expect("decode child fixture")
}

// The parent owns stdin and never sends anything. A closed pipe is a failure,
// not permission for a transaction paused before COMMIT to continue.
fn ready_and_wait(phase: &str) -> ! {
    if writeln!(std::io::stdout().lock(), "{READY} {phase}").is_err()
        || std::io::stdout().flush().is_err()
    {
        std::process::exit(90);
    }
    let _ = std::io::stdin().read_exact(&mut [0_u8]);
    std::process::exit(91);
}

fn before_commit(event: rusqlite::trace::TraceEvent<'_>) {
    if let rusqlite::trace::TraceEvent::Stmt(statement, _) = event
        && statement.sql().trim().eq_ignore_ascii_case("COMMIT")
    {
        // SQLITE_TRACE_STMT runs before the statement executes. All mutation
        // work preceding this command's COMMIT has already run on this worker.
        ready_and_wait("before-commit");
    }
}

#[test]
#[ignore = "subprocess helper, invoked only by the recovery tests with an isolated path"]
fn recovery_child_process() {
    let directory = std::env::var_os("TANGLEAF_RECOVERY_DIRECTORY")
        .expect("only the parent recovery test may invoke this helper");
    let scenario = std::env::var("TANGLEAF_RECOVERY_SCENARIO").unwrap();
    let phase = std::env::var("TANGLEAF_RECOVERY_PHASE").unwrap();
    let mode = std::env::var("TANGLEAF_RECOVERY_SYNCHRONOUS").unwrap();
    assert!(matches!(mode.as_str(), "NORMAL" | "FULL"));
    assert!(matches!(phase.as_str(), "before-commit" | "acknowledged"));
    let directory = Path::new(&directory);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let connection = db::open(directory.join("notes.db")).await.unwrap();
            connection
                .call(move |database| {
                    database.pragma_update(None, "synchronous", &mode)?;
                    let actual: i64 =
                        database.pragma_query_value(None, "synchronous", |row| row.get(0))?;
                    assert_eq!(actual, if mode == "FULL" { 2 } else { 1 });
                    let journal: String =
                        database.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
                    assert_eq!(journal, "wal");
                    Ok(())
                })
                .await
                .unwrap();
            let fixture = fixture(&connection).await;
            save_json(&directory.join("fixture.json"), &fixture);
            if phase == "before-commit" {
                // The hook exists only on this disposable child's writer, after
                // initialization and the fixture have committed. No production
                // fault hook or feature is needed.
                connection
                    .call(|database| {
                        database.trace_v2(
                            rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
                            Some(before_commit),
                        );
                        Ok(())
                    })
                    .await
                    .unwrap();
            }
            match scenario.as_str() {
                "autosave" => {
                    let (block, changed, _) = db::set_block_content_if_revision(
                        &connection,
                        fixture.source.uuid,
                        BlockContent {
                            markdown: fixture.autosave_text(),
                        },
                        fixture.source.markdown_revision.clone(),
                    )
                    .await
                    .unwrap();
                    assert!(changed);
                    assert_eq!(block.markdown, fixture.autosave_text());
                }
                "split" => {
                    let blocks = db::split_block(
                        &connection,
                        fixture.source.uuid,
                        fixture.split_parts(),
                        fixture.source.markdown_revision.clone(),
                    )
                    .await
                    .unwrap();
                    assert_eq!(blocks.len(), 3);
                }
                "sequenced" => {
                    let outcomes =
                        notes_core::apply_sequenced_batch(&connection, fixture.sequenced_ops())
                            .await
                            .unwrap();
                    assert_eq!(outcomes.len(), 3);
                    assert!(!outcomes[0].applied);
                    assert!(outcomes[1..].iter().all(|outcome| outcome.applied));
                }
                _ => panic!("unknown recovery scenario"),
            }
            assert_eq!(
                phase, "acknowledged",
                "the COMMIT barrier must have stopped the other child"
            );
            // The public command has returned success. Preserve its entire logical
            // state as well as checking its independent invariants in the parent.
            save_json(
                &directory.join("committed.json"),
                &database_image(&connection).await,
            );
            ready_and_wait("acknowledged");
        });
}

struct KillOnDrop(Option<Child>);

impl KillOnDrop {
    fn terminate_and_wait(&mut self) {
        let Some(mut child) = self.0.take() else {
            return;
        };
        let _ = child.kill();
        let status = child.wait().expect("wait for terminated recovery child");
        assert!(!status.success(), "child unexpectedly exited normally");
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn crash_child(directory: &Path, scenario: &str, phase: &str, mode: &str) {
    let stderr_path = directory.join("child-stderr.log");
    let stderr = std::fs::File::create(&stderr_path).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "recovery_child_process",
            "--ignored",
            "--nocapture",
        ])
        .env("TANGLEAF_RECOVERY_DIRECTORY", directory)
        .env("TANGLEAF_RECOVERY_SCENARIO", scenario)
        .env("TANGLEAF_RECOVERY_PHASE", phase)
        .env("TANGLEAF_RECOVERY_SYNCHRONOUS", mode)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .spawn()
        .expect("spawn isolated recovery helper");
    let mut guard = KillOnDrop(Some(child));
    let stdout = guard.0.as_mut().unwrap().stdout.take().unwrap();
    let expected = format!("{READY} {phase}");
    let (ready_tx, ready_rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut output = String::new();
        for line in std::io::BufReader::new(stdout).lines() {
            let line = line.expect("read helper output");
            output.push_str(&line);
            output.push('\n');
            if line.ends_with(&expected) {
                let _ = ready_tx.send(Ok(()));
                return;
            }
        }
        let _ = ready_tx.send(Err(output));
    });
    // A liveness timeout, not a performance assertion or timed crash guess.
    let readiness = ready_rx.recv_timeout(Duration::from_secs(30));
    guard.terminate_and_wait();
    reader.join().expect("join readiness reader");
    assert!(
        matches!(readiness, Ok(Ok(()))),
        "{scenario}/{phase}/{mode}: helper did not reach boundary: {readiness:?}; stderr: {}",
        std::fs::read_to_string(stderr_path).unwrap_or_default()
    );
}

async fn check_integrity(connection: &Connection) {
    connection
        .call(|database| {
            let integrity = database
                .prepare("PRAGMA integrity_check")?
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            assert_eq!(integrity, ["ok"]);
            assert!(!database.prepare("PRAGMA foreign_key_check")?.exists([])?);
            Ok(())
        })
        .await
        .unwrap();
}

fn block_semantics(block: &db::Block) -> (Uuid, Uuid, Option<Uuid>, &OrderKey, &BlockStyle, &str) {
    (
        block.uuid,
        block.page_uuid,
        block.parent_uuid,
        &block.order_key,
        &block.style,
        &block.markdown,
    )
}

async fn assert_committed(connection: &Connection, fixture: &Fixture, scenario: &str) {
    let image = database_image(connection).await;
    let block = db::get_block(connection, fixture.source.uuid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        db::get_block(connection, fixture.child.uuid).await.unwrap(),
        Some(fixture.child.clone())
    );
    assert_ne!(block.markdown_revision, fixture.source.markdown_revision);
    assert_eq!(
        image.tables["history_redo"],
        fixture.baseline.tables["history_redo"]
    );
    let outbox = notes_core::pending_outbox(connection, 100).await.unwrap();
    match scenario {
        "autosave" => {
            assert_eq!(block.markdown, fixture.autosave_text());
            assert_eq!(image.count("blocks"), fixture.baseline.count("blocks"));
            // Autosave currently does not add an undo action.
            assert_eq!(
                image.tables["history_undo"],
                fixture.baseline.tables["history_undo"]
            );
            assert_eq!(outbox.len(), fixture.outbox.len() + 1);
            assert_eq!(&outbox[..fixture.outbox.len()], &fixture.outbox);
            assert_eq!(
                image.count("applied_ops"),
                fixture.baseline.count("applied_ops") + 1
            );
            assert_eq!(notes_core::sync_cursor(connection).await.unwrap(), 0);
            assert_eq!(
                outbox.last().unwrap().kind,
                OpKind::BlockSetMarkdown(BlockSetMarkdown {
                    uuid: fixture.source.uuid,
                    markdown: fixture.autosave_text(),
                })
            );
        }
        "split" => {
            let parts = fixture.split_parts();
            let blocks = db::list_block_children(connection, fixture.source.page_uuid, None)
                .await
                .unwrap();
            assert_eq!(blocks.len(), parts.len());
            assert_eq!(blocks[0].uuid, fixture.source.uuid);
            assert_eq!(blocks[0].order_key, fixture.source.order_key);
            assert_eq!(
                blocks
                    .iter()
                    .map(|block| &block.markdown)
                    .collect::<Vec<_>>(),
                parts.iter().map(|part| &part.markdown).collect::<Vec<_>>()
            );
            assert!(
                blocks
                    .iter()
                    .all(|block| block.style == fixture.source.style)
            );
            assert_eq!(image.count("blocks"), fixture.baseline.count("blocks") + 2);
            assert_eq!(
                image.count("history_undo"),
                fixture.baseline.count("history_undo") + 1
            );
            assert_eq!(outbox.len(), fixture.outbox.len() + 3);
            assert_eq!(&outbox[..fixture.outbox.len()], &fixture.outbox);
            let expected_kinds = std::iter::once(OpKind::BlockSetMarkdown(BlockSetMarkdown {
                uuid: blocks[0].uuid,
                markdown: blocks[0].markdown.clone(),
            }))
            .chain(blocks.iter().skip(1).map(|block| {
                OpKind::BlockCreate(BlockCreate {
                    uuid: block.uuid,
                    page_uuid: block.page_uuid,
                    parent_uuid: block.parent_uuid,
                    order_key: block.order_key.clone(),
                    style: block.style,
                    markdown: block.markdown.clone(),
                    created_at: block.created_at,
                })
            }))
            .collect::<Vec<_>>();
            assert_eq!(
                outbox[fixture.outbox.len()..]
                    .iter()
                    .map(|operation| operation.kind.clone())
                    .collect::<Vec<_>>(),
                expected_kinds
            );
            assert_eq!(
                image.count("applied_ops"),
                fixture.baseline.count("applied_ops") + 3
            );
            assert_eq!(notes_core::sync_cursor(connection).await.unwrap(), 0);
            assert_eq!(
                db::undo_history(connection).await.unwrap(),
                db::HistoryMoveResult::Applied
            );
            let restored = db::get_block(connection, fixture.source.uuid)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(block_semantics(&restored), block_semantics(&fixture.source));
            assert_eq!(
                db::history_status(connection).await.unwrap(),
                db::HistoryStatus {
                    undo_count: fixture.baseline.count("history_undo") as i64,
                    redo_count: 1,
                }
            );
            assert_eq!(
                db::list_block_children(connection, fixture.source.page_uuid, None)
                    .await
                    .unwrap()
                    .len(),
                1
            );
            for created in blocks.iter().skip(1) {
                assert!(
                    db::get_block(connection, created.uuid)
                        .await
                        .unwrap()
                        .is_none()
                );
            }
            assert_eq!(
                db::get_block(connection, fixture.child.uuid).await.unwrap(),
                Some(fixture.child.clone())
            );
            assert_eq!(
                db::redo_history(connection).await.unwrap(),
                db::HistoryMoveResult::Applied
            );
            let redone = db::list_block_children(connection, fixture.source.page_uuid, None)
                .await
                .unwrap();
            assert_eq!(
                redone.iter().map(block_semantics).collect::<Vec<_>>(),
                blocks.iter().map(block_semantics).collect::<Vec<_>>()
            );
            assert_eq!(
                db::history_status(connection).await.unwrap(),
                db::HistoryStatus {
                    undo_count: fixture.baseline.count("history_undo") as i64 + 1,
                    redo_count: 0,
                }
            );
        }
        "sequenced" => {
            assert_eq!(block.markdown, "Remote [[Recovery target]]");
            assert_eq!(
                db::get_block(connection, REMOTE_BLOCK)
                    .await
                    .unwrap()
                    .unwrap()
                    .markdown,
                format!("Remote child link (({}))", fixture.target_block)
            );
            assert_eq!(image.count("blocks"), fixture.baseline.count("blocks") + 1);
            assert_eq!(
                image.tables["history_undo"],
                fixture.baseline.tables["history_undo"]
            );
            assert!(outbox.is_empty());
            assert_eq!(
                image.count("applied_ops"),
                fixture.baseline.count("applied_ops") + 2
            );
            assert_eq!(notes_core::sync_cursor(connection).await.unwrap(), 3);
            let operation_ids = fixture
                .sequenced_ops()
                .into_iter()
                .map(|(_, operation)| operation.op_id)
                .collect::<Vec<_>>();
            let sequences = connection
                .call(move |database| {
                    operation_ids
                        .into_iter()
                        .map(|id| {
                            database.query_row(
                                "SELECT seq FROM applied_ops WHERE op_id = ?1",
                                [id],
                                |row| row.get::<_, i64>(0),
                            )
                        })
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .await
                .unwrap();
            assert_eq!(sequences, [1, 2, 3]);
            // Replay after reopening must not duplicate content or lose a cursor.
            notes_core::apply_sequenced_batch(connection, fixture.sequenced_ops())
                .await
                .unwrap();
            assert_eq!(database_image(connection).await, image);
        }
        _ => unreachable!(),
    }
    assert_eq!(
        db::find_backlinks(connection, fixture.target_page)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db::find_backlinks(connection, fixture.target_block)
            .await
            .unwrap()
            .len(),
        1
    );
}

async fn recovery_scenarios(scenario: &str) {
    for mode in ["NORMAL", "FULL"] {
        for phase in ["before-commit", "acknowledged"] {
            let directory = tempfile::Builder::new()
                .prefix("storage-recovery-")
                .tempdir()
                .unwrap();
            crash_child(directory.path(), scenario, phase, mode);
            let fixture: Fixture = load_json(&directory.path().join("fixture.json"));
            let connection = db::open(directory.path().join("notes.db")).await.unwrap();
            check_integrity(&connection).await;
            if phase == "before-commit" {
                assert_eq!(
                    database_image(&connection).await,
                    fixture.baseline,
                    "{scenario}/{mode}: the interrupted transaction must leave every persistent row unchanged"
                );
            } else {
                let committed: DatabaseImage = load_json(&directory.path().join("committed.json"));
                assert_eq!(
                    database_image(&connection).await,
                    committed,
                    "{scenario}/{mode}: every acknowledged row must survive abrupt termination"
                );
                assert_committed(&connection, &fixture, scenario).await;
                check_integrity(&connection).await;
            }
        }
    }
}

#[tokio::test]
async fn autosave_recovers_at_commit_boundaries() {
    recovery_scenarios("autosave").await;
}

#[tokio::test]
async fn split_history_and_outbox_recover_at_commit_boundaries() {
    recovery_scenarios("split").await;
}

#[tokio::test]
async fn sequenced_batch_outbox_and_cursor_recover_at_commit_boundaries() {
    recovery_scenarios("sequenced").await;
}
