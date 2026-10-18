//! Disposable-file durability workload; synthetic vectors and extraction, no model or network.

use super::*;
use notes_core::db::{self, BlockContent};
use notes_core::store::NotesStore;
use std::time::{Duration, Instant};

const DIMENSIONS: usize = 768;
const BLOCKS: usize = 32;
const DOCUMENTS: u64 = BLOCKS as u64 + 1;
const IDENTITY: &str = "synthetic-durability-workload-v1";

#[tokio::test]
async fn failed_migration_closes_the_unreturned_ai_connection() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("future.db");
    let connection =
        Connection::open_writer(&path, notes_core::sqlite::policy::WriterDurability::Normal)
            .await
            .unwrap();
    connection
        .call(|database| {
            database.pragma_update(None, "user_version", i32::MAX)?;
            database.execute_batch(
                "CREATE TABLE preserved(value TEXT); INSERT INTO preserved VALUES('kept')",
            )
        })
        .await
        .unwrap();
    let retained = connection.clone();
    let result = AiStore::initialize_connection(
        connection,
        "future".into(),
        768,
        false,
        AiControl::default(),
    )
    .await;
    assert!(result.is_err());
    assert!(
        retained.call(|_| Ok(())).await.is_err(),
        "failed startup closes admission before returning"
    );
    retained.close().await.unwrap();
    let raw = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        raw.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        i64::from(i32::MAX)
    );
    assert_eq!(
        raw.query_row("SELECT value FROM preserved", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "kept"
    );
}

#[tokio::test]
async fn failed_vector_table_closes_the_unreturned_ai_connection_and_allows_retry() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vectors.db");
    register_sqlite_vec();
    let connection =
        Connection::open_writer(&path, notes_core::sqlite::policy::WriterDurability::Normal)
            .await
            .unwrap();
    let retained = connection.clone();
    // sqlite-vec's positive dimension limit is 8192. Reach the vector-table
    // stage, after migrations and generation insertion, rather than validation.
    let error = AiStore::initialize_connection(
        connection,
        "invalid-vector".into(),
        8193,
        true,
        AiControl::default(),
    )
    .await
    .err()
    .expect("vec0 rejects unsupported dimensions");
    assert!(format!("{error:#}").contains("creating generation vector table"));
    assert!(retained.call(|_| Ok(())).await.is_err());
    retained.close().await.unwrap();
    let store = AiStore::open(path, "supported-vector".into(), 768)
        .await
        .unwrap();
    assert_eq!(store.control().await.unwrap(), AiControl::default());
    store.connection.close().await.unwrap();
}

#[tokio::test]
async fn failed_initial_control_write_also_closes_the_constructed_store() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("control.db");
    register_sqlite_vec();
    let connection =
        Connection::open_writer(&path, notes_core::sqlite::policy::WriterDurability::Normal)
            .await
            .unwrap();
    connection.call(|database| {
        migrations().to_latest(database).unwrap();
        database.execute_batch("CREATE TRIGGER reject_control BEFORE UPDATE ON index_control BEGIN SELECT RAISE(ABORT,'control fixture'); END;")
    }).await.unwrap();
    let retained = connection.clone();
    let error = AiStore::initialize_connection(
        connection,
        "control-failure".into(),
        DIMENSIONS,
        true,
        AiControl {
            automatic_embeddings: true,
            entity_extraction: true,
            query_rewriting: false,
        },
    )
    .await
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("control fixture"));
    assert!(retained.call(|_| Ok(())).await.is_err());
    retained.close().await.unwrap();
    let raw = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        raw.query_row(
            "SELECT automatic_embeddings,entity_extraction,query_rewriting FROM index_control",
            [],
            |row| Ok((
                row.get::<_, bool>(0)?,
                row.get::<_, bool>(1)?,
                row.get::<_, bool>(2)?
            ))
        )
        .unwrap(),
        (false, false, true)
    );
}

fn input(revision: usize, index: usize) -> String {
    format!(
        "Revision {revision}, source {index}. Alice describes the synchronization change. {}",
        "A short technical explanation. ".repeat(8)
    )
}

fn control(index: usize) -> AiControl {
    let enabled = index.is_multiple_of(2);
    AiControl {
        automatic_embeddings: enabled,
        entity_extraction: enabled,
        query_rewriting: !enabled,
    }
}

fn vector(slots: &HashMap<uuid::Uuid, usize>, uuid: uuid::Uuid) -> Vec<f32> {
    let mut values = vec![0.0; DIMENSIONS];
    values[slots[&uuid]] = 1.0;
    values
}

fn embeddings(jobs: &[IndexJob], slots: &HashMap<uuid::Uuid, usize>) -> Vec<EmbeddedDocument> {
    jobs.iter()
        .map(|job| EmbeddedDocument {
            content_uuid: job.content_uuid,
            input_hash: job.input_hash.clone(),
            chunks: vec![EmbeddedChunk {
                chunk_index: 0,
                input_text: job.input_text.clone(),
                embedding: vector(slots, job.content_uuid),
            }],
        })
        .collect()
}

async fn finish_job(store: &AiStore, job: &IndexJob, entity: uuid::Uuid) {
    store
        .finish_extraction(
            job,
            vec![ExtractedEntityRecord {
                uuid: entity,
                name: "Alice".into(),
                normalized_name: "alice".into(),
                description: "Synthetic workload entity".into(),
            }],
            vec![ExtractedEdge {
                src_uuid: job.content_uuid,
                dst_uuid: entity,
                kind: "mentions".into(),
            }],
        )
        .await
        .unwrap();
}

#[derive(Debug, PartialEq, Eq)]
struct Counts {
    vectors: i64,
    raw_vectors: i64,
    embedding_jobs: i64,
    extraction_jobs: i64,
    extraction_state: i64,
    entities: i64,
    edges: i64,
}

async fn counts(store: &AiStore) -> Counts {
    let table = store.table_name.clone();
    store
        .connection
        .call(move |database| {
            database.query_row(
                &format!(
                    "SELECT (SELECT COUNT(*) FROM generation_vectors),
             (SELECT COUNT(*) FROM {table}), (SELECT COUNT(*) FROM embedding_jobs),
             (SELECT COUNT(*) FROM extraction_jobs), (SELECT COUNT(*) FROM extraction_state),
             (SELECT COUNT(*) FROM entities), (SELECT COUNT(*) FROM extraction_edges)"
                ),
                [],
                |row| {
                    Ok(Counts {
                        vectors: row.get(0)?,
                        raw_vectors: row.get(1)?,
                        embedding_jobs: row.get(2)?,
                        extraction_jobs: row.get(3)?,
                        extraction_state: row.get(4)?,
                        entities: row.get(5)?,
                        edges: row.get(6)?,
                    })
                },
            )
        })
        .await
        .unwrap()
}

async fn verify(
    store: &AiStore,
    expected_control: AiControl,
    block: uuid::Uuid,
    body: &str,
    slots: &HashMap<uuid::Uuid, usize>,
) {
    assert_eq!(store.control().await.unwrap(), expected_control);
    let status = store.status(DOCUMENTS).await.unwrap();
    assert_eq!(status.generation_status, GenerationStatus::Active);
    assert_eq!(status.indexed, DOCUMENTS);
    assert_eq!(status.source_documents, DOCUMENTS);
    assert_eq!(
        (
            status.pending,
            status.failed,
            status.extraction_pending,
            status.extraction_failed
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(
        counts(store).await,
        Counts {
            vectors: DOCUMENTS as i64,
            raw_vectors: DOCUMENTS as i64,
            embedding_jobs: 0,
            extraction_jobs: 0,
            extraction_state: DOCUMENTS as i64,
            entities: 1,
            edges: DOCUMENTS as i64
        }
    );
    let matches = store.search(vector(slots, block), 4).await.unwrap();
    assert_eq!(matches.len(), 4);
    assert_eq!(matches[0].content_uuid, block);
    assert_eq!(matches[0].distance, 0.0);
    assert!(matches[0].chunk_text.ends_with(body.trim()));
    assert_eq!(
        store.cached_source_document_count().await.unwrap(),
        DOCUMENTS
    );
    let entities = store.list_entities(10).await.unwrap();
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "Alice");
    assert_eq!(entities[0].mention_count, DOCUMENTS);
}

async fn policy(connection: &Connection, synchronous: i64) {
    connection
        .call(move |database| {
            database.pragma_update(None, "synchronous", synchronous)?;
            assert_eq!(
                database.pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))?,
                synchronous
            );
            assert_eq!(
                database.pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))?,
                "wal"
            );
            Ok(())
        })
        .await
        .unwrap();
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
        .unwrap()
}

fn sample(phases: &mut BTreeMap<&'static str, Vec<f64>>, phase: &'static str, started: Instant) {
    phases
        .entry(phase)
        .or_default()
        .push(started.elapsed().as_secs_f64() * 1000.0);
}

struct Measurement {
    phases: BTreeMap<&'static str, Vec<f64>>,
    checkpoint: (i64, i64, i64),
    checkpoint_ms: f64,
    wal_bytes: u64,
    embedding_jobs: usize,
    extraction_jobs: usize,
}

async fn measure(
    directory: &Path,
    synchronous: i64,
    iterations: usize,
    idle: Duration,
) -> Measurement {
    let notes_path = directory.join("notes.db");
    let ai_path = directory.join("ai.db");
    let notes = NotesStore::open(&notes_path).await.unwrap();
    // Isolate the AI policy comparison: source writes use the same NORMAL
    // policy in both cases, even if the production default later changes.
    policy(notes.write_connection(), 1).await;
    let page = notes.create_page("AI workload".into()).await.unwrap().value;
    let mut blocks = Vec::new();
    let mut previous = None;
    for index in 0..BLOCKS {
        let block = db::create_block(
            notes.write_connection(),
            page.uuid,
            None,
            previous,
            BlockStyle::Paragraph,
            input(0, index),
        )
        .await
        .unwrap();
        previous = Some(block.uuid);
        blocks.push(block);
    }
    let slots = std::iter::once(page.uuid)
        .chain(blocks.iter().map(|b| b.uuid))
        .enumerate()
        .map(|(index, uuid)| (uuid, index))
        .collect::<HashMap<_, _>>();
    let store = AiStore::open(&ai_path, IDENTITY.into(), DIMENSIONS)
        .await
        .unwrap();
    policy(&store.connection, synchronous).await;
    assert_eq!(
        store.reconcile(notes.read_connection()).await.unwrap(),
        DOCUMENTS
    );
    let jobs = store.take_jobs(128).await.unwrap();
    assert_eq!(jobs.len(), DOCUMENTS as usize);
    store
        .write_embeddings(DOCUMENTS, embeddings(&jobs, &slots))
        .await
        .unwrap();
    assert!(
        store
            .reconcile_extractions(notes.read_connection())
            .await
            .unwrap()
            .is_empty()
    );
    let jobs = store.take_extraction_jobs(128).await.unwrap();
    assert_eq!(jobs.len(), DOCUMENTS as usize);
    let entity = uuid::Uuid::now_v7();
    for job in jobs {
        finish_job(&store, &job, entity).await;
    }
    verify(
        &store,
        AiControl::default(),
        blocks[0].uuid,
        &input(0, 0),
        &slots,
    )
    .await;
    assert_eq!(checkpoint(&store.connection, true).await.0, 0);
    let mut phases = BTreeMap::new();
    let mut current = blocks.remove(0);
    let mut embedding_jobs = 0;
    let mut extraction_jobs = 0;
    for index in 0..iterations {
        if !idle.is_zero() {
            tokio::time::sleep(idle).await;
        }
        let cycle = Instant::now();
        let started = Instant::now();
        current = db::set_block_content_if_revision_with_ops(
            notes.write_connection(),
            current.uuid,
            BlockContent {
                markdown: input(index + 1, 0),
            },
            current.markdown_revision,
        )
        .await
        .unwrap()
        .value
        .0;
        sample(&mut phases, "source_edit", started);
        let started = Instant::now();
        assert_eq!(
            store.set_control(control(index)).await.unwrap(),
            control(index)
        );
        sample(&mut phases, "control", started);
        let started = Instant::now();
        assert_eq!(
            store.reconcile(notes.read_connection()).await.unwrap(),
            DOCUMENTS
        );
        sample(&mut phases, "reconcile", started);
        let started = Instant::now();
        let jobs = store.take_jobs(128).await.unwrap();
        sample(&mut phases, "take_jobs", started);
        assert_eq!(
            jobs.iter()
                .map(|job| job.content_uuid)
                .collect::<HashSet<_>>(),
            HashSet::from([page.uuid, current.uuid])
        );
        embedding_jobs += jobs.len();
        let documents = embeddings(&jobs, &slots);
        let started = Instant::now();
        store.write_embeddings(DOCUMENTS, documents).await.unwrap();
        sample(&mut phases, "write_embeddings", started);
        let started = Instant::now();
        assert!(
            store
                .reconcile_extractions(notes.read_connection())
                .await
                .unwrap()
                .is_empty()
        );
        sample(&mut phases, "reconcile_extractions", started);
        let started = Instant::now();
        let jobs = store.take_extraction_jobs(128).await.unwrap();
        sample(&mut phases, "take_extraction_jobs", started);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].content_uuid, current.uuid);
        assert!(
            store
                .extraction_job_is_current(notes.read_connection(), &jobs[0])
                .await
                .unwrap()
        );
        extraction_jobs += jobs.len();
        let started = Instant::now();
        finish_job(&store, &jobs[0], entity).await;
        sample(&mut phases, "finish_extraction", started);
        let started = Instant::now();
        verify(
            &store,
            control(index),
            current.uuid,
            &current.markdown,
            &slots,
        )
        .await;
        sample(&mut phases, "read_verify", started);
        sample(&mut phases, "cycle", cycle);
    }
    assert_eq!(embedding_jobs, iterations * 2);
    assert_eq!(extraction_jobs, iterations);
    assert!(store.take_jobs(128).await.unwrap().is_empty());
    assert!(store.take_extraction_jobs(128).await.unwrap().is_empty());
    let generation = store.generation_id;
    let wal_bytes = std::fs::metadata(directory.join("ai.db-wal"))
        .map(|m| m.len())
        .unwrap_or(0);
    let started = Instant::now();
    let checkpoint = checkpoint(&store.connection, false).await;
    let checkpoint_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(checkpoint.0, 0);
    assert_eq!(checkpoint.1, checkpoint.2);
    store.connection.close().await.unwrap();
    notes.close().await.unwrap();
    // A different default must not overwrite the persistent settings on reopen.
    let reopened =
        AiStore::open_with_control(&ai_path, IDENTITY.into(), DIMENSIONS, control(iterations))
            .await
            .unwrap();
    assert_eq!(reopened.generation_id, generation);
    verify(
        &reopened,
        control(iterations - 1),
        current.uuid,
        &current.markdown,
        &slots,
    )
    .await;
    reopened
        .connection
        .call(|database| {
            assert_eq!(
                database.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))?,
                "ok"
            );
            assert_eq!(
                database.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get::<_, i64>(0)
                })?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();
    reopened.connection.close().await.unwrap();
    let notes = NotesStore::open(notes_path).await.unwrap();
    assert_eq!(
        db::get_block(notes.read_connection(), current.uuid)
            .await
            .unwrap()
            .unwrap(),
        current
    );
    notes.close().await.unwrap();
    Measurement {
        phases,
        checkpoint,
        checkpoint_ms,
        wal_bytes,
        embedding_jobs,
        extraction_jobs,
    }
}

#[tokio::test]
async fn ai_workload_preserves_control_vectors_and_extractions_after_reopen() {
    for synchronous in [1, 2] {
        let directory = tempfile::tempdir().unwrap();
        let result = measure(directory.path(), synchronous, 4, Duration::ZERO).await;
        assert_eq!((result.embedding_jobs, result.extraction_jobs), (8, 4));
        assert_eq!(result.phases.len(), 10);
        assert!(result.phases.values().all(|samples| samples.len() == 4));
    }
}

/// Opt-in backend comparison: 768-dimensional synthetic vectors and synthetic
/// extraction results; no network/model. TANGLEAF_AI_WORKLOAD_RUNS/SAMPLES/IDLE_MS
/// default to 3/10/400. Only the disposable AI writer's policy changes.
#[tokio::test]
#[ignore = "release durability workload; run in a quiet window on disposable files"]
async fn benchmark_ai_store_durability() {
    fn setting(name: &str, default: usize) -> usize {
        std::env::var(name)
            .map(|s| s.parse().unwrap())
            .unwrap_or(default)
    }
    let runs = setting("TANGLEAF_AI_WORKLOAD_RUNS", 3);
    let iterations = setting("TANGLEAF_AI_WORKLOAD_SAMPLES", 10);
    let idle = Duration::from_millis(setting("TANGLEAF_AI_WORKLOAD_IDLE_MS", 400) as u64);
    assert!(runs > 0 && iterations > 0 && runs.saturating_mul(iterations) >= 30);
    let mut summaries = BTreeMap::<_, Vec<f64>>::new();
    for run in 1..=runs {
        let policies = if run.is_multiple_of(2) {
            [("FULL", 2), ("NORMAL", 1)]
        } else {
            [("NORMAL", 1), ("FULL", 2)]
        };
        for (mode, synchronous) in policies {
            let directory = tempfile::tempdir().unwrap();
            let result = measure(directory.path(), synchronous, iterations, idle).await;
            println!(
                "ai_run mode={mode} run={run} dimensions={DIMENSIONS} source_blocks={BLOCKS} documents={DOCUMENTS} controls={iterations} embedding_jobs={} extraction_jobs={} final_vectors={DOCUMENTS} final_entities=1 final_edges={DOCUMENTS} source_policy=NORMAL idle_ms={} wal_bytes={} checkpoint_busy={} checkpoint_frames={} checkpointed_frames={} checkpoint_ms={:.3}",
                result.embedding_jobs,
                result.extraction_jobs,
                idle.as_millis(),
                result.wal_bytes,
                result.checkpoint.0,
                result.checkpoint.1,
                result.checkpoint.2,
                result.checkpoint_ms
            );
            for (phase, samples) in result.phases {
                for (index, elapsed) in samples.iter().enumerate() {
                    println!(
                        "ai_sample mode={mode} run={run} phase={phase} sample={} elapsed_ms={elapsed:.3}",
                        index + 1
                    );
                }
                summaries.entry((mode, phase)).or_default().extend(samples);
            }
        }
    }
    for ((mode, phase), mut samples) in summaries {
        samples.sort_by(f64::total_cmp);
        let count = samples.len();
        let p50 = (samples[(count - 1) / 2] + samples[count / 2]) / 2.0;
        let p95 = samples[(count * 95).div_ceil(100) - 1];
        println!(
            "ai_summary mode={mode} phase={phase} samples={count} min_ms={:.3} p50_ms={p50:.3} p95_ms={p95:.3}",
            samples[0]
        );
    }
}
