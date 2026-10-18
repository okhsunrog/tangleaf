//! Backend-only storage workload. The ignored benchmark edits disposable copies of a stable
//! snapshot, never the source. It models serial revision-safe saves after an idle pause, with
//! optional synchronized read/sync or mixed primary-content bursts. It does not simulate the
//! UI, network, physical pen input or power loss.

use notes_core::db::{self, BlockContent};
use notes_core::store::NotesStore;
use notes_core::{
    BlockCreate, BlockMove, BlockSetMarkdown, BlockStyle, Connection, CoreError, Hlc, Op, OpKind,
    OrderKey, ReplicaRole,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[path = "storage_workload/content.rs"]
mod content;

const SEARCH_TOKEN: &str = "storageworkloadfixture";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scenario {
    Quiet,
    Mixed,
    Content,
}

impl Scenario {
    fn name(self) -> &'static str {
        match self {
            Self::Quiet => "quiet",
            Self::Mixed => "mixed",
            Self::Content => "content",
        }
    }

    fn background(self) -> bool {
        self != Self::Quiet
    }
}

#[derive(Clone, Copy)]
struct Workload {
    saves: usize,
    idle: Duration,
    remote_every: usize,
    remote_blocks: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Counts {
    operations: i64,
    outbox: i64,
    undo: i64,
    redo: i64,
}

async fn counts(connection: &Connection) -> Counts {
    connection
        .call(|database| {
            database.query_row(
                "SELECT (SELECT COUNT(*) FROM applied_ops),
                        (SELECT COUNT(*) FROM sync_outbox),
                        (SELECT COUNT(*) FROM history_undo),
                        (SELECT COUNT(*) FROM history_redo)",
                [],
                |row| {
                    Ok(Counts {
                        operations: row.get(0)?,
                        outbox: row.get(1)?,
                        undo: row.get(2)?,
                        redo: row.get(3)?,
                    })
                },
            )
        })
        .await
        .expect("count mutation bookkeeping")
}

struct RemoteFixture {
    workspace: Uuid,
    device: Uuid,
    page: Uuid,
    blocks: Vec<Uuid>,
    clock: Hlc,
    sequence: u64,
}

impl RemoteFixture {
    fn envelope(&mut self, kind: OpKind) -> (u64, Op) {
        self.clock = Hlc::send(
            Some(&self.clock),
            chrono::Utc::now().timestamp_millis().max(0) as u64,
            self.device,
        );
        self.sequence += 1;
        (
            self.sequence,
            Op {
                op_id: Uuid::now_v7(),
                workspace_uuid: self.workspace,
                device_id: self.device,
                hlc: self.clock.clone(),
                format_version: notes_core::operation::FORMAT_VERSION,
                kind,
            },
        )
    }

    fn batch(&mut self, generation: usize) -> Vec<(u64, Op)> {
        let blocks = self.blocks.clone();
        let mut batch = Vec::with_capacity(blocks.len() * 2);
        for (index, uuid) in blocks.iter().copied().enumerate() {
            batch.push(self.envelope(OpKind::BlockSetMarkdown(BlockSetMarkdown {
                uuid,
                markdown: remote_markdown(generation, index),
            })));
            batch.push(self.envelope(OpKind::BlockMove(BlockMove {
                uuid,
                page_uuid: self.page,
                parent_uuid: None,
                order_key: remote_order(generation, index, blocks.len()),
            })));
        }
        batch
    }
}

fn remote_markdown(generation: usize, index: usize) -> String {
    format!("{SEARCH_TOKEN} background block {index}, revision {generation}")
}

fn remote_order(generation: usize, index: usize, count: usize) -> OrderKey {
    OrderKey::from_ordinal(if generation.is_multiple_of(2) {
        index + 1
    } else {
        count - index
    })
}

async fn fixture(connection: &Connection, remote_blocks: usize) -> (db::Block, RemoteFixture) {
    // These settings belong to the disposable fixture only. No transport is started, but
    // local saves exercise the real client outbox path even for an unconfigured source.
    notes_core::set_replica_role(connection, ReplicaRole::Client)
        .await
        .expect("configure fixture replica role");
    notes_core::configure_sync(connection, "https://storage-workload.invalid")
        .await
        .expect("configure fixture outbox");
    let note = db::create_note(connection, None)
        .await
        .expect("create autosave fixture")
        .into_created()
        .expect("untitled fixture is new");
    let background = db::create_note(connection, None)
        .await
        .expect("create background fixture")
        .into_created()
        .expect("untitled fixture is new");
    let content = format!(
        "{SEARCH_TOKEN}\n\n{}",
        "A short editing sentence. ".repeat(80)
    );
    let initial = db::set_block_content_if_revision_with_ops(
        connection,
        note.initial_block.uuid,
        BlockContent { markdown: content },
        note.initial_block.markdown_revision,
    )
    .await
    .expect("seed autosave fixture")
    .value
    .0;
    let clock = connection
        .call(|database| {
            database.query_row(
                "SELECT value FROM sync_meta WHERE key = 'last_hlc'",
                [],
                |row| row.get::<_, String>(0),
            )
        })
        .await
        .expect("read fixture clock")
        .parse()
        .expect("valid persisted clock");
    let mut remote = RemoteFixture {
        workspace: db::workspace_uuid(connection).await.expect("workspace"),
        device: Uuid::now_v7(),
        page: background.page.uuid,
        blocks: vec![background.initial_block.uuid],
        clock,
        sequence: notes_core::sync_cursor(connection).await.expect("cursor"),
    };
    let mut setup = vec![remote.envelope(OpKind::BlockSetMarkdown(BlockSetMarkdown {
        uuid: background.initial_block.uuid,
        markdown: remote_markdown(0, 0),
    }))];
    for index in 1..remote_blocks {
        let uuid = Uuid::now_v7();
        remote.blocks.push(uuid);
        setup.push(remote.envelope(OpKind::BlockCreate(BlockCreate {
            uuid,
            page_uuid: background.page.uuid,
            parent_uuid: None,
            order_key: remote_order(0, index, remote_blocks),
            style: BlockStyle::Paragraph,
            markdown: remote_markdown(0, index),
            created_at: chrono::Utc::now().timestamp(),
        })));
    }
    let outcomes = notes_core::apply_sequenced_batch(connection, setup)
        .await
        .expect("seed remote fixture");
    assert!(outcomes.iter().all(|outcome| outcome.applied));
    (initial, remote)
}

async fn checkpoint(connection: &Connection, truncate: bool) -> (i64, i64, i64) {
    connection
        .call(move |database| {
            database.query_row(
                if truncate {
                    "PRAGMA wal_checkpoint(TRUNCATE)"
                } else {
                    "PRAGMA wal_checkpoint(PASSIVE)"
                },
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
        })
        .await
        .expect("checkpoint disposable database")
}

struct Measurement {
    samples_ms: Vec<f64>,
    remote_batches: usize,
    remote_operations: usize,
    document_reads: usize,
    searches: usize,
    before: Counts,
    after: Counts,
    phases_ms: BTreeMap<&'static str, Vec<f64>>,
    final_block: db::Block,
    remote: RemoteFixture,
    content: Option<content::Verification>,
}

async fn measure(
    store: &NotesStore,
    path: &Path,
    scenario: Scenario,
    workload: Workload,
) -> Measurement {
    let connection = store.write_connection();
    let reader = store.read_connection();
    let (mut current, mut remote) = fixture(connection, workload.remote_blocks).await;
    let mut content = if scenario == Scenario::Content {
        Some(content::Workload::new(store, path, current.uuid).await)
    } else {
        None
    };
    let (busy, _, _) = checkpoint(connection, true).await;
    assert_eq!(
        busy, 0,
        "fixture checkpoint must complete before measurement"
    );
    let before = counts(connection).await;
    let mut body = current.markdown.clone();
    let mut samples_ms = Vec::with_capacity(workload.saves);
    let mut remote_batches = 0;
    let mut remote_operations = 0;
    let mut previous_revision = current.markdown_revision.clone();
    let mut phases_ms = BTreeMap::<_, Vec<_>>::new();

    for index in 0..workload.saves {
        // Closed-loop pacing: the pause starts after the preceding save and competing
        // burst have completed. Do not interpret this as a fixed-rate arrival model.
        if !workload.idle.is_zero() {
            tokio::time::sleep(workload.idle).await;
        }
        // A save represents a completed typing burst after the idle debounce. The next
        // revision is taken from its acknowledgement; saves never overlap on this block.
        body.push_str(&format!(" Edit {}.", index + 1));
        let text_content = BlockContent {
            markdown: body.clone(),
        };
        previous_revision = current.markdown_revision.clone();
        let batch = if scenario.background() && index.is_multiple_of(workload.remote_every) {
            remote_batches += 1;
            let batch = remote.batch(remote_batches);
            remote_operations += batch.len();
            batch
        } else {
            Vec::new()
        };
        let expected_batch_size = batch.len();
        let save = async {
            let started = Instant::now();
            let result = db::set_block_content_if_revision_with_ops(
                connection,
                current.uuid,
                text_content,
                current.markdown_revision.clone(),
            )
            .await;
            (result, started.elapsed().as_secs_f64() * 1_000.0)
        };
        let sync = async {
            if !batch.is_empty() {
                let started = Instant::now();
                let outcomes = notes_core::apply_sequenced_batch(connection, batch)
                    .await
                    .unwrap_or_else(|error| {
                        panic!(
                            "apply background batch: {error:#}; sqlite={:?}; domain={:?}",
                            error.downcast_ref::<rusqlite::Error>(),
                            error.downcast_ref::<CoreError>()
                        )
                    });
                assert_eq!(outcomes.len(), expected_batch_size);
                assert!(outcomes.iter().all(|outcome| outcome.applied));
                Some(started.elapsed().as_secs_f64() * 1_000.0)
            } else {
                None
            }
        };
        let read = async {
            if scenario.background() {
                let started = Instant::now();
                let document = db::get_page_document(reader, remote.page)
                    .await
                    .expect("read background page")
                    .expect("background page exists");
                assert_eq!(document.blocks.len(), workload.remote_blocks);
                Some(started.elapsed().as_secs_f64() * 1_000.0)
            } else {
                None
            }
        };
        let search = async {
            if scenario.background() {
                let started = Instant::now();
                let hits = db::search_fts(reader, SEARCH_TOKEN.into(), 20)
                    .await
                    .expect("search fixture text");
                assert!(!hits.is_empty(), "fixture remains searchable");
                Some(started.elapsed().as_secs_f64() * 1_000.0)
            } else {
                None
            }
        };
        let primary_content = async {
            if let Some(content) = &mut content {
                content
                    .tick(store, index, index + 1 == workload.saves)
                    .await;
            }
        };
        // Synchronize bursts and alternate producer order to expose both queue contention
        // and saves that arrive first. This does not model real input/network timing.
        let burst_started = Instant::now();
        let (saved, synced, read, searched) = if index.is_multiple_of(2) {
            let (synced, read, searched, _, saved) =
                tokio::join!(biased; sync, read, search, primary_content, save);
            (saved, synced, read, searched)
        } else {
            let (saved, _, synced, read, searched) =
                tokio::join!(biased; save, primary_content, sync, read, search);
            (saved, synced, read, searched)
        };
        phases_ms
            .entry("burst")
            .or_default()
            .push(burst_started.elapsed().as_secs_f64() * 1_000.0);
        for (phase, sample) in [
            ("sync", synced),
            ("document_read", read),
            ("search", searched),
        ] {
            if let Some(sample) = sample {
                phases_ms.entry(phase).or_default().push(sample);
            }
        }
        let (saved, elapsed_ms) = saved;
        let saved = saved.unwrap_or_else(|error| {
            panic!(
                "revision-safe autosave: {error:#}; sqlite={:?}; domain={:?}",
                error.downcast_ref::<rusqlite::Error>(),
                error.downcast_ref::<CoreError>()
            )
        });
        assert_eq!(saved.operations.len(), 1);
        assert!(
            saved.value.0.markdown == body,
            "saved fixture text must match"
        );
        assert_eq!(saved.value.0.page_uuid, current.page_uuid);
        assert_eq!(saved.value.0.parent_uuid, current.parent_uuid);
        current = saved.value.0;
        samples_ms.push(elapsed_ms);
    }

    let content = if let Some(content) = content {
        let verification = content.finish(store, workload.saves, &mut phases_ms).await;
        Some(verification)
    } else {
        None
    };
    let extra_ops = content.as_ref().map_or(0, |v| v.authored_operations());
    let after = counts(connection).await;
    assert_eq!(
        after.operations - before.operations,
        (workload.saves + remote_operations + extra_ops) as i64
    );
    assert_eq!(
        after.outbox - before.outbox,
        (workload.saves + extra_ops) as i64
    );
    assert_eq!((after.undo, after.redo), (before.undo, before.redo));
    assert_eq!(
        notes_core::sync_cursor(connection)
            .await
            .expect("final cursor"),
        remote.sequence
    );
    let restored = db::get_block(connection, current.uuid)
        .await
        .expect("read saved block")
        .expect("saved block exists");
    assert!(
        restored.markdown == body,
        "final persisted content must match"
    );
    assert_eq!(restored.markdown_revision, current.markdown_revision);
    if remote_batches > 0 {
        let blocks = db::get_blocks(connection, remote.blocks.clone())
            .await
            .expect("read remote results");
        assert_eq!(blocks.len(), remote.blocks.len());
        for (index, uuid) in remote.blocks.iter().enumerate() {
            let block = blocks
                .iter()
                .find(|block| block.uuid == *uuid)
                .expect("remote block exists");
            assert!(
                block.markdown == remote_markdown(remote_batches, index),
                "remote content must match"
            );
            assert_eq!(block.page_uuid, remote.page);
            assert_eq!(block.parent_uuid, None);
            assert_eq!(
                block.order_key,
                remote_order(remote_batches, index, remote.blocks.len())
            );
        }
    }

    // Validate stale-ack idempotence and conflict rejection outside measured samples.
    let duplicate = db::set_block_content_if_revision_with_ops(
        connection,
        current.uuid,
        BlockContent {
            markdown: body.clone(),
        },
        previous_revision.clone(),
    )
    .await
    .expect("equal stale retry succeeds");
    assert!(duplicate.operations.is_empty());
    assert_eq!(
        duplicate.value.0.markdown_revision,
        current.markdown_revision
    );
    let error = db::set_block_content_if_revision_with_ops(
        connection,
        current.uuid,
        BlockContent {
            markdown: "rejected stale fixture edit".into(),
        },
        previous_revision,
    )
    .await
    .expect_err("different stale write must conflict");
    assert!(matches!(
        error.downcast_ref::<CoreError>(),
        Some(CoreError::Conflict(_))
    ));
    assert_eq!(
        counts(connection).await,
        after,
        "retry/conflict must not add mutations"
    );

    Measurement {
        samples_ms,
        remote_batches,
        remote_operations,
        document_reads: if scenario.background() {
            workload.saves
        } else {
            0
        },
        searches: if scenario.background() {
            workload.saves
        } else {
            0
        },
        before,
        after,
        phases_ms,
        final_block: current,
        remote,
        content,
    }
}

impl Measurement {
    async fn verify_reopened(&self, store: &NotesStore, path: &Path) {
        let reader = store.read_connection();
        assert_eq!(
            counts(reader).await,
            self.after,
            "bookkeeping survives reopen"
        );
        let block = db::get_block(reader, self.final_block.uuid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(block, self.final_block, "text/revision survives reopen");
        assert_eq!(
            notes_core::sync_cursor(reader).await.unwrap(),
            self.remote.sequence
        );
        let blocks = db::get_blocks(reader, self.remote.blocks.clone())
            .await
            .unwrap();
        assert_eq!(blocks.len(), self.remote.blocks.len());
        for (index, uuid) in self.remote.blocks.iter().enumerate() {
            let block = blocks.iter().find(|block| block.uuid == *uuid).unwrap();
            assert_eq!(block.markdown, remote_markdown(self.remote_batches, index));
            assert_eq!(block.page_uuid, self.remote.page);
            assert_eq!(block.parent_uuid, None);
            // The initial first block predates the remote fixture's explicit moves.
            if self.remote_batches > 0 || index > 0 {
                assert_eq!(
                    block.order_key,
                    remote_order(self.remote_batches, index, blocks.len())
                );
            }
        }
        if let Some(content) = &self.content {
            content.verify(store, path).await;
        }
        reader
            .call(|database| {
                let integrity: String =
                    database.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
                assert_eq!(integrity, "ok");
                let foreign_key_errors: i64 = database.query_row(
                    "SELECT COUNT(*) FROM pragma_foreign_key_check",
                    [],
                    |row| row.get(0),
                )?;
                assert_eq!(foreign_key_errors, 0);
                Ok(())
            })
            .await
            .expect("reopened fixture integrity");
    }
}

async fn configure_policy(connection: &Connection, synchronous: i64) -> (i64, String, i64) {
    let values = connection
        .call(move |database| {
            database.pragma_update(None, "synchronous", synchronous)?;
            Ok((
                database.pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))?,
                database.pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))?,
                database
                    .pragma_query_value(None, "wal_autocheckpoint", |row| row.get::<_, i64>(0))?,
            ))
        })
        .await
        .expect("configure disposable writer");
    assert_eq!(values.0, synchronous);
    assert!(values.1.eq_ignore_ascii_case("wal"));
    values
}

async fn assert_policy(connection: &Connection, synchronous: i64) {
    let actual = connection
        .call(|database| {
            database.pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))
        })
        .await
        .expect("read measured writer durability");
    assert_eq!(
        actual, synchronous,
        "native writers must not alter main writer policy"
    );
}

fn wal_bytes(path: &Path) -> u64 {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    match std::fs::metadata(wal) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(_) => panic!("inspect disposable WAL size"),
    }
}

#[tokio::test]
async fn storage_workload_preserves_revisions_replication_and_outbox() {
    for synchronous in [1, 2] {
        for scenario in [Scenario::Quiet, Scenario::Mixed] {
            let directory = tempfile::tempdir().expect("temporary directory");
            let path = directory.path().join("workload.db");
            let store = NotesStore::open(&path).await.expect("open fixture");
            let connection = store.write_connection();
            configure_policy(connection, synchronous).await;
            let result = measure(
                &store,
                &path,
                scenario,
                Workload {
                    saves: 3,
                    idle: Duration::ZERO,
                    remote_every: 2,
                    remote_blocks: 4,
                },
            )
            .await;
            assert_eq!(result.samples_ms.len(), 3);
            assert_eq!(
                result.remote_batches,
                if scenario == Scenario::Mixed { 2 } else { 0 }
            );
            let (busy, _, _) = checkpoint(connection, false).await;
            assert_eq!(busy, 0);
            store.close().await.expect("close fixture workers");
            let reopened = NotesStore::open(&path).await.expect("reopen fixture");
            result.verify_reopened(&reopened, &path).await;
            reopened.close().await.expect("close reopened fixture");
        }
    }
}

#[tokio::test]
async fn storage_content_workload_preserves_ink_blobs_and_pending_uploads_after_reopen() {
    for synchronous in [1, 2] {
        let directory = tempfile::tempdir().expect("temporary content directory");
        let path = directory.path().join("workload.db");
        let store = NotesStore::open(&path).await.expect("open content fixture");
        configure_policy(store.write_connection(), synchronous).await;
        let result = measure(
            &store,
            &path,
            Scenario::Content,
            Workload {
                saves: 7,
                idle: Duration::ZERO,
                remote_every: 3,
                remote_blocks: 32,
            },
        )
        .await;
        assert_eq!(result.samples_ms.len(), 7);
        assert_eq!(result.remote_batches, 3);
        assert_eq!(result.remote_operations, 3 * 64);
        assert_eq!(result.content.as_ref().unwrap().counts(), (7, 3, 2));
        for (phase, count) in [
            ("ink_patch", 7),
            ("ink_complete", 3),
            ("ink_open", 2),
            ("blob_install", 2),
            ("attachment_metadata", 2),
            ("document_read", 7),
            ("search", 7),
            ("sync", 3),
            ("burst", 7),
        ] {
            assert_eq!(
                result.phases_ms.get(phase).unwrap().len(),
                count,
                "{phase} samples"
            );
        }
        assert_eq!(
            result.after.operations - result.before.operations,
            7 + 192 + 3 + 2
        );
        assert_eq!(result.after.outbox - result.before.outbox, 7 + 3 + 2);
        assert_eq!(
            (result.after.undo, result.after.redo),
            (result.before.undo, result.before.redo)
        );
        assert_policy(store.write_connection(), synchronous).await;
        store.close().await.expect("close content fixture");
        let reopened = NotesStore::open(&path)
            .await
            .expect("reopen content fixture");
        result.verify_reopened(&reopened, &path).await;
        reopened
            .close()
            .await
            .expect("close reopened content fixture");
    }
}

fn setting(name: &str, default: usize, allow_zero: bool) -> usize {
    let value = std::env::var(name)
        .map(|value| value.parse::<usize>().expect("numeric workload setting"))
        .unwrap_or(default);
    assert!(allow_zero || value > 0, "workload count must be positive");
    value
}

/// Opt in with TANGLEAF_BENCH_DATABASE pointing to a stable snapshot and run with --release.
/// Defaults: TANGLEAF_WORKLOAD_RUNS=3, SAVES=10, IDLE_MS=400, REMOTE_EVERY=3,
/// REMOTE_BLOCKS=32 (all names prefixed TANGLEAF_WORKLOAD_). SCENARIO is BOTH/QUIET/MIXED/CONTENT;
/// BOTH preserves the original QUIET+MIXED scenarios. CONTENT additionally writes native ink
/// (256 points/save, completion every third/final save) and installs/fsyncs a unique 256 KiB
/// blob plus attachment metadata every third save. CONTENT requires at least 30 saves/policy.
/// TANGLEAF_BENCH_SYNCHRONOUS is BOTH/NORMAL/FULL. Each run uses a fresh VACUUM INTO copy.
/// Leave RUST_LOG unset for comparisons; enable it only for separate phase profiling.
/// Pacing is closed-loop: idle follows completion of the preceding complete burst.
/// TMPDIR must point to the task's disk-backed artifact directory. No network is contacted.
#[tokio::test]
#[ignore = "requires a stable snapshot; writes disposable copies only"]
async fn benchmark_storage_workload_on_database_copies() {
    if let Ok(filter) = std::env::var("RUST_LOG") {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_new(filter).expect("valid RUST_LOG"),
            )
            .with_test_writer()
            .try_init()
            .expect("install workload profiling subscriber");
    }
    let source = rusqlite::Connection::open_with_flags(
        std::env::var("TANGLEAF_BENCH_DATABASE").expect("snapshot path"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open read-only snapshot");
    let runs = setting("TANGLEAF_WORKLOAD_RUNS", 3, false);
    let workload = Workload {
        saves: setting("TANGLEAF_WORKLOAD_SAVES", 10, false),
        idle: Duration::from_millis(setting("TANGLEAF_WORKLOAD_IDLE_MS", 400, true) as u64),
        remote_every: setting("TANGLEAF_WORKLOAD_REMOTE_EVERY", 3, false),
        remote_blocks: setting("TANGLEAF_WORKLOAD_REMOTE_BLOCKS", 32, false),
    };
    let policies = match std::env::var("TANGLEAF_BENCH_SYNCHRONOUS")
        .unwrap_or_else(|_| "BOTH".into())
        .to_ascii_uppercase()
        .as_str()
    {
        "BOTH" => vec![("NORMAL", 1), ("FULL", 2)],
        "NORMAL" => vec![("NORMAL", 1)],
        "FULL" => vec![("FULL", 2)],
        _ => panic!("TANGLEAF_BENCH_SYNCHRONOUS must be BOTH/NORMAL/FULL"),
    };
    let scenarios = match std::env::var("TANGLEAF_WORKLOAD_SCENARIO")
        .unwrap_or_else(|_| "BOTH".into())
        .to_ascii_uppercase()
        .as_str()
    {
        "BOTH" => vec![Scenario::Quiet, Scenario::Mixed],
        "QUIET" => vec![Scenario::Quiet],
        "MIXED" => vec![Scenario::Mixed],
        "CONTENT" => vec![Scenario::Content],
        _ => panic!("TANGLEAF_WORKLOAD_SCENARIO must be BOTH/QUIET/MIXED/CONTENT"),
    };
    if scenarios.contains(&Scenario::Content) {
        assert!(
            runs * workload.saves >= 30,
            "CONTENT needs at least 30 saves per policy"
        );
        assert!(
            workload.saves <= 150_000 / 256,
            "CONTENT stroke fixture must fit the supported point limit"
        );
    }
    let mut measurements = Vec::new();
    for run in 1..=runs {
        let mut order = policies.clone();
        if run.is_multiple_of(2) {
            order.reverse();
        }
        for &scenario in &scenarios {
            for &(mode, synchronous) in &order {
                let directory = tempfile::tempdir().expect("disposable directory");
                let path = directory.path().join("workload.db");
                source
                    .execute("VACUUM INTO ?1", [path.to_str().expect("copy path")])
                    .expect("copy stable snapshot");
                let store = NotesStore::open(&path).await.expect("open disposable copy");
                let connection = store.write_connection();
                let (actual, journal, autocheckpoint) =
                    configure_policy(connection, synchronous).await;
                let measured = measure(&store, &path, scenario, workload).await;
                assert_policy(connection, synchronous).await;
                let wal_before_checkpoint = wal_bytes(&path);
                let checkpoint_started = Instant::now();
                let (busy, frames, checkpointed) = checkpoint(connection, false).await;
                let checkpoint_ms = checkpoint_started.elapsed().as_secs_f64() * 1000.0;
                for (sample, elapsed) in measured.samples_ms.iter().enumerate() {
                    let order = if scenario == Scenario::Quiet {
                        "save_only"
                    } else if sample.is_multiple_of(2) {
                        "background_first"
                    } else {
                        "save_first"
                    };
                    println!(
                        "sample scenario={} mode={mode} run={run} save={} order={order} autosave_ms={elapsed:.3}",
                        scenario.name(),
                        sample + 1
                    );
                }
                println!(
                    "run scenario={} mode={mode} run={run} read_workers=1 synchronous={actual} journal_mode={journal} wal_autocheckpoint={autocheckpoint} saves={} idle_ms={} remote_blocks={} remote_every={} remote_batches={} remote_operations={} document_reads={} searches={} applied_delta={} outbox_delta={} wal_bytes={wal_before_checkpoint} checkpoint_busy={busy} checkpoint_frames={frames} checkpointed_frames={checkpointed} checkpoint_ms={checkpoint_ms:.3}",
                    scenario.name(),
                    workload.saves,
                    workload.idle.as_millis(),
                    workload.remote_blocks,
                    workload.remote_every,
                    measured.remote_batches,
                    measured.remote_operations,
                    measured.document_reads,
                    measured.searches,
                    measured.after.operations - measured.before.operations,
                    measured.after.outbox - measured.before.outbox
                );
                if let Some(content) = &measured.content {
                    let (patches, publications, attachments) = content.counts();
                    println!(
                        "content mode={mode} run={run} ink_patches={patches} ink_completions={publications} attachment_installs={attachments} attachment_bytes={} native_ink_policy=FULL undo_delta={} redo_delta={}",
                        attachments * 256 * 1024,
                        measured.after.undo - measured.before.undo,
                        measured.after.redo - measured.before.redo
                    );
                }
                for (phase, samples) in &measured.phases_ms {
                    for (index, elapsed) in samples.iter().enumerate() {
                        println!(
                            "phase scenario={} mode={mode} run={run} phase={phase} sample={} elapsed_ms={elapsed:.3}",
                            scenario.name(),
                            index + 1
                        );
                    }
                }
                store.close().await.expect("close disposable workers");
                let reopened = NotesStore::open(&path).await.expect("reopen measured copy");
                measured.verify_reopened(&reopened, &path).await;
                reopened.close().await.expect("close verified copy");
                measurements.push((scenario, mode, measured));
            }
        }
    }
    for scenario in scenarios {
        for &(mode, _) in &policies {
            let mut samples = measurements
                .iter()
                .filter(|(which, policy, _)| *which == scenario && *policy == mode)
                .flat_map(|(_, _, measurement)| measurement.samples_ms.iter().copied())
                .collect::<Vec<_>>();
            samples.sort_by(f64::total_cmp);
            let count = samples.len();
            let median = (samples[(count - 1) / 2] + samples[count / 2]) / 2.0;
            let p95 = samples[(count * 95).div_ceil(100) - 1];
            println!(
                "summary scenario={} mode={mode} samples={count} min_ms={:.3} p50_ms={median:.3} median_ms={median:.3} p95_ms={p95:.3}",
                scenario.name(),
                samples[0]
            );
            let mut phases = BTreeMap::<_, Vec<f64>>::new();
            for (_, _, measurement) in measurements
                .iter()
                .filter(|(which, policy, _)| *which == scenario && *policy == mode)
            {
                for (phase, samples) in &measurement.phases_ms {
                    phases.entry(*phase).or_default().extend(samples);
                }
            }
            for (phase, mut samples) in phases {
                samples.sort_by(f64::total_cmp);
                let count = samples.len();
                let p50 = (samples[(count - 1) / 2] + samples[count / 2]) / 2.0;
                let p95 = samples[(count * 95).div_ceil(100) - 1];
                println!(
                    "phase_summary scenario={} mode={mode} phase={phase} samples={count} min_ms={:.3} p50_ms={p50:.3} p95_ms={p95:.3}",
                    scenario.name(),
                    samples[0]
                );
            }
        }
    }
}
