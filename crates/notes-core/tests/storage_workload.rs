//! Backend-only storage workload. The ignored benchmark edits disposable copies of a stable
//! snapshot, never the source. It models serial revision-safe saves after an idle pause, with
//! optional synchronized read/sync bursts; it does not simulate the UI, network or power loss.

use notes_core::db::{self, BlockContent};
use notes_core::{
    BlockCreate, BlockMove, BlockSetMarkdown, BlockStyle, Connection, CoreError, Hlc, Op, OpKind,
    OrderKey, ReplicaRole,
};
use std::path::Path;
use std::time::{Duration, Instant};
use uuid::Uuid;

const SEARCH_TOKEN: &str = "storageworkloadfixture";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scenario {
    Quiet,
    Mixed,
}

impl Scenario {
    fn name(self) -> &'static str {
        match self {
            Self::Quiet => "quiet",
            Self::Mixed => "mixed",
        }
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
}

async fn measure(connection: &Connection, scenario: Scenario, workload: Workload) -> Measurement {
    let (mut current, mut remote) = fixture(connection, workload.remote_blocks).await;
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

    for index in 0..workload.saves {
        // Closed-loop pacing: the pause starts after the preceding save and competing
        // burst have completed. Do not interpret this as a fixed-rate arrival model.
        if !workload.idle.is_zero() {
            tokio::time::sleep(workload.idle).await;
        }
        // A save represents a completed typing burst after the idle debounce. The next
        // revision is taken from its acknowledgement; saves never overlap on this block.
        body.push_str(&format!(" Edit {}.", index + 1));
        let content = BlockContent {
            markdown: body.clone(),
        };
        previous_revision = current.markdown_revision.clone();
        let batch = if scenario == Scenario::Mixed && index.is_multiple_of(workload.remote_every) {
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
                content,
                current.markdown_revision.clone(),
            )
            .await;
            (result, started.elapsed().as_secs_f64() * 1_000.0)
        };
        let sync = async {
            if !batch.is_empty() {
                let outcomes = notes_core::apply_sequenced_batch(connection, batch)
                    .await
                    .expect("apply background batch");
                assert_eq!(outcomes.len(), expected_batch_size);
                assert!(outcomes.iter().all(|outcome| outcome.applied));
            }
        };
        let read = async {
            if scenario == Scenario::Mixed {
                let document = db::get_page_document(connection, remote.page)
                    .await
                    .expect("read background page")
                    .expect("background page exists");
                assert_eq!(document.blocks.len(), workload.remote_blocks);
            }
        };
        let search = async {
            if scenario == Scenario::Mixed {
                let hits = db::search_fts(connection, SEARCH_TOKEN.into(), 20)
                    .await
                    .expect("search fixture text");
                assert!(!hits.is_empty(), "fixture remains searchable");
            }
        };
        // Synchronize bursts and alternate producer order to expose both queue contention
        // and saves that arrive first. This does not model real input/network timing.
        let (saved, elapsed_ms) = if index.is_multiple_of(2) {
            let (_, _, _, result) = tokio::join!(biased; sync, read, search, save);
            result
        } else {
            let (result, _, _, _) = tokio::join!(biased; save, sync, read, search);
            result
        };
        let saved = saved.expect("revision-safe autosave");
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

    let after = counts(connection).await;
    assert_eq!(
        after.operations - before.operations,
        (workload.saves + remote_operations) as i64
    );
    assert_eq!(after.outbox - before.outbox, workload.saves as i64);
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
        document_reads: if scenario == Scenario::Mixed {
            workload.saves
        } else {
            0
        },
        searches: if scenario == Scenario::Mixed {
            workload.saves
        } else {
            0
        },
        before,
        after,
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
            let connection = db::open(&path).await.expect("open fixture");
            configure_policy(&connection, synchronous).await;
            let result = measure(
                &connection,
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
            let (busy, _, _) = checkpoint(&connection, false).await;
            assert_eq!(busy, 0);
            connection
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
                .expect("fixture integrity");
        }
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
/// REMOTE_BLOCKS=32 (all names prefixed TANGLEAF_WORKLOAD_). SCENARIO is BOTH/QUIET/MIXED;
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
        _ => panic!("TANGLEAF_WORKLOAD_SCENARIO must be BOTH/QUIET/MIXED"),
    };
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
                let connection = db::open(&path).await.expect("open disposable copy");
                let (actual, journal, autocheckpoint) =
                    configure_policy(&connection, synchronous).await;
                let measured = measure(&connection, scenario, workload).await;
                let wal_before_checkpoint = wal_bytes(&path);
                let checkpoint_started = Instant::now();
                let (busy, frames, checkpointed) = checkpoint(&connection, false).await;
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
                    "run scenario={} mode={mode} run={run} synchronous={actual} journal_mode={journal} wal_autocheckpoint={autocheckpoint} saves={} idle_ms={} remote_blocks={} remote_every={} remote_batches={} remote_operations={} document_reads={} searches={} applied_delta={} outbox_delta={} wal_bytes={wal_before_checkpoint} checkpoint_busy={busy} checkpoint_frames={frames} checkpointed_frames={checkpointed} checkpoint_ms={checkpoint_ms:.3}",
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
                measurements.push((scenario, mode, measured.samples_ms));
            }
        }
    }
    for scenario in scenarios {
        for &(mode, _) in &policies {
            let mut samples = measurements
                .iter()
                .filter(|(which, policy, _)| *which == scenario && *policy == mode)
                .flat_map(|(_, _, values)| values.iter().copied())
                .collect::<Vec<_>>();
            samples.sort_by(f64::total_cmp);
            let count = samples.len();
            let median = (samples[(count - 1) / 2] + samples[count / 2]) / 2.0;
            let p95 = samples[(count * 95).div_ceil(100) - 1];
            println!(
                "summary scenario={} mode={mode} samples={count} median_ms={median:.3} p95_ms={p95:.3}",
                scenario.name()
            );
        }
    }
}
