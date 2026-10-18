use super::*;
use crate::db;
use rusqlite::types::Value;
use std::sync::{Arc, Mutex, OnceLock};
use tracing::instrument::WithSubscriber;
use tracing_subscriber::prelude::*;
use uuid::Uuid;

const A: Uuid = Uuid::from_u128(1);
const B: Uuid = Uuid::from_u128(2);
const C: Uuid = Uuid::from_u128(3);
const ROOT: Uuid = Uuid::from_u128(10);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ProjectionSample {
    mode: String,
    blocks_read: u64,
    intents_read: u64,
    pages_read: u64,
    vertices_visited: u64,
    seed_rows_read: u64,
}

#[derive(Default)]
struct ProjectionFields {
    sample: ProjectionSample,
    found: bool,
}

impl tracing::field::Visit for ProjectionFields {
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "blocks_read" => {
                self.sample.blocks_read = value;
                self.found = true;
            }
            "intents_read" => self.sample.intents_read = value,
            "pages_read" => self.sample.pages_read = value,
            "vertices_visited" => self.sample.vertices_visited = value,
            "seed_rows_read" => self.sample.seed_rows_read = value,
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "mode" {
            self.sample.mode = format!("{value:?}");
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct ProjectionCapture(Arc<Mutex<Vec<ProjectionSample>>>);

impl ProjectionCapture {
    pub(crate) fn dispatch(&self) -> tracing::Dispatch {
        // Prevent a concurrent untraced worker from caching Interest::never
        // through tracing's single-dispatch fast path.
        static INTEREST_ANCHOR: OnceLock<tracing::Dispatch> = OnceLock::new();
        INTEREST_ANCHOR
            .get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
        tracing::Dispatch::new(tracing_subscriber::registry().with(self.clone()))
    }

    fn snapshot(&self) -> Vec<ProjectionSample> {
        self.0.lock().unwrap().clone()
    }

    pub(crate) fn assert_full(&self) {
        let samples = self.snapshot();
        assert!(
            !samples.is_empty(),
            "bulk operation must exercise structural projection"
        );
        assert!(
            samples.iter().all(|sample| sample.mode == "Full"),
            "{samples:?}"
        );
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for ProjectionCapture {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let mut fields = ProjectionFields::default();
        event.record(&mut fields);
        if fields.found {
            self.0.lock().unwrap().push(fields.sample);
        }
    }
}

fn page(uuid: Uuid, title: &str) -> OpKind {
    OpKind::PageCreate(PageCreate {
        uuid,
        kind: PageKind::Note,
        title: Some(title.into()),
        layout: PageLayout::Outline,
        created_at: 1,
    })
}

fn block(uuid: Uuid, page_uuid: Uuid, ordinal: usize) -> OpKind {
    OpKind::BlockCreate(BlockCreate {
        uuid,
        page_uuid,
        parent_uuid: None,
        order_key: OrderKey::from_ordinal(ordinal),
        style: BlockStyle::Bullet,
        markdown: format!("Block {ordinal}"),
        created_at: 1,
    })
}

async fn fixture(unrelated: usize) -> (Connection, db::Block) {
    let connection = db::open_in_memory().await.unwrap();
    connection
        .call_domain(move |database| -> CoreResult<()> {
            let transaction = database.transaction()?;
            let mut kinds = vec![page(A, "A"), page(B, "B"), page(C, "C"), block(ROOT, A, 1)];
            kinds.extend(
                (0..unrelated)
                    .map(|index| block(Uuid::from_u128(0x1000 + index as u128), C, index + 1)),
            );
            apply_local_kinds_deferred_in_transaction(
                &transaction,
                kinds,
                StructureProjectionMode::Full,
            )?;
            transaction.commit()?;
            Ok(())
        })
        .await
        .unwrap();
    let source = db::get_block(&connection, ROOT).await.unwrap().unwrap();
    (connection, source)
}

fn incoming(workspace: Uuid, index: u128, wall: u64, kind: OpKind) -> Op {
    let device = Uuid::from_u128(0xD3A1CE);
    Op {
        op_id: Uuid::from_u128(0x9000 + index),
        workspace_uuid: workspace,
        device_id: device,
        hlc: Hlc::new(wall, 0, device),
        format_version: FORMAT_VERSION,
        kind,
    }
}

#[tokio::test]
async fn public_structure_commands_read_only_their_components_with_unrelated_workspace_growth() {
    let mut reference = None;
    for unrelated in [0, 2_048] {
        let (connection, _) = fixture(unrelated).await;
        let workspace = db::workspace_uuid(&connection).await.unwrap();
        let capture = ProjectionCapture::default();
        async {
            let child = db::create_block(
                &connection,
                A,
                None,
                Some(ROOT),
                BlockStyle::Bullet,
                "Local".into(),
            )
            .await
            .unwrap();
            db::move_block(&connection, child.uuid, Some(ROOT), None)
                .await
                .unwrap();
            let wall = chrono::Utc::now().timestamp_millis().max(0) as u64 + 60_000;
            let remote = incoming(
                workspace,
                1,
                wall,
                OpKind::BlockMove(BlockMove {
                    uuid: ROOT,
                    page_uuid: B,
                    parent_uuid: None,
                    order_key: OrderKey::first(),
                }),
            );
            assert!(
                apply(&connection, &remote, Origin::Remote)
                    .await
                    .unwrap()
                    .applied
            );
            let sequenced = incoming(
                workspace,
                2,
                wall + 1,
                OpKind::BlockMove(BlockMove {
                    uuid: child.uuid,
                    page_uuid: B,
                    parent_uuid: Some(ROOT),
                    order_key: OrderKey::first(),
                }),
            );
            assert!(
                apply_sequenced(&connection, 1, &sequenced)
                    .await
                    .unwrap()
                    .applied
            );
            assert_eq!(
                db::get_block(&connection, ROOT)
                    .await
                    .unwrap()
                    .unwrap()
                    .page_uuid,
                B
            );
            assert_eq!(
                db::get_block(&connection, child.uuid)
                    .await
                    .unwrap()
                    .unwrap()
                    .parent_uuid,
                Some(ROOT)
            );
        }
        .with_subscriber(capture.dispatch())
        .await;
        let samples = capture.snapshot();
        assert!(
            samples.len() >= 4,
            "all public mutation paths must project: {samples:?}"
        );
        assert!(
            samples.iter().all(|sample| sample.mode == "Components"
                && sample.blocks_read <= 8
                && sample.intents_read <= 8
                && sample.pages_read <= 2
                && sample.vertices_visited <= 2),
            "{samples:?}"
        );
        eprintln!("public component rows with {unrelated} unrelated blocks: {samples:?}");
        if let Some(reference) = &reference {
            assert_eq!(&samples, reference);
        } else {
            reference = Some(samples);
        }
    }
}

type DatabaseRows = BTreeMap<String, Vec<Vec<Value>>>;

async fn persistent_rows(connection: &Connection) -> DatabaseRows {
    connection
        .call(|database| {
            let names = database
                .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")?
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut image = BTreeMap::new();
            for name in names {
                let mut statement = database
                    .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))?;
                let columns = statement.column_count();
                let mut rows = statement
                    .query_map([], |row| {
                        (0..columns)
                            .map(|index| row.get::<_, Value>(index))
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows.sort_by_cached_key(|row| format!("{row:?}"));
                image.insert(name, rows);
            }
            Ok(image)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn public_component_apply_rejects_uninitialized_or_future_projection_without_any_writes() {
    for version in [0, 2] {
        let (connection, _) = fixture(0).await;
        configure_sync(&connection, "https://guard.example.test")
            .await
            .unwrap();
        let note = db::create_note(&connection, Some("Pending local operations".into()))
            .await
            .unwrap()
            .into_created()
            .unwrap();
        let pending = pending_outbox(&connection, 10).await.unwrap();
        assert!(!pending.is_empty());
        let workspace = db::workspace_uuid(&connection).await.unwrap();
        let incoming = incoming(
            workspace,
            1,
            chrono::Utc::now().timestamp_millis().max(0) as u64 + 60_000,
            OpKind::BlockMove(BlockMove {
                uuid: note.initial_block.uuid,
                page_uuid: B,
                parent_uuid: None,
                order_key: OrderKey::first(),
            }),
        );
        connection
            .call(move |database| {
                database
                    .execute(
                        "UPDATE store_state SET structure_projection_version = ?1",
                        [version],
                    )
                    .map(|_| ())
            })
            .await
            .unwrap();
        let before = persistent_rows(&connection).await;

        let errors = [
            db::create_block(
                &connection,
                A,
                None,
                None,
                BlockStyle::Bullet,
                "Must not save".into(),
            )
            .await
            .unwrap_err(),
            apply_batch(&connection, std::slice::from_ref(&incoming), Origin::Local)
                .await
                .unwrap_err(),
            apply_batch(&connection, std::slice::from_ref(&incoming), Origin::Remote)
                .await
                .unwrap_err(),
            // The first envelope would acknowledge/delete a pending outbox row
            // even without a new structural operation if the guard ran too late.
            apply_sequenced_batch(&connection, vec![(1, pending[0].clone()), (2, incoming)])
                .await
                .unwrap_err(),
        ];
        for error in errors {
            assert!(
                error
                    .to_string()
                    .contains(&format!("structural projection version {version}")),
                "{error:#}"
            );
        }
        assert_eq!(persistent_rows(&connection).await, before);
    }
}

#[tokio::test]
async fn bulk_snapshot_and_archive_keep_explicit_full_projection() {
    let (source, _) = fixture(16).await;
    let snapshot = export_sync_snapshot(&source, 0).await.unwrap();
    let target = db::open_in_memory().await.unwrap();
    let snapshot_capture = ProjectionCapture::default();
    import_sync_snapshot(&target, snapshot.clone())
        .with_subscriber(snapshot_capture.dispatch())
        .await
        .unwrap();
    snapshot_capture.assert_full();
    assert_eq!(export_sync_snapshot(&target, 0).await.unwrap(), snapshot);

    let tail = incoming(
        snapshot.workspace_uuid,
        1,
        chrono::Utc::now().timestamp_millis().max(0) as u64 + 60_000,
        OpKind::BlockMove(BlockMove {
            uuid: ROOT,
            page_uuid: B,
            parent_uuid: None,
            order_key: OrderKey::first(),
        }),
    );
    let tail_capture = ProjectionCapture::default();
    let projected = project_sync_snapshot(snapshot, &[tail])
        .with_subscriber(tail_capture.dispatch())
        .await
        .unwrap();
    tail_capture.assert_full();
    assert_eq!(
        projected
            .blocks
            .iter()
            .find(|block| block.uuid == ROOT)
            .unwrap()
            .page_uuid,
        B
    );

    let archive = db::export_archive(&source).await.unwrap();
    let target = db::open_in_memory().await.unwrap();
    let archive_capture = ProjectionCapture::default();
    db::import_archive(&target, archive)
        .with_subscriber(archive_capture.dispatch())
        .await
        .unwrap();
    archive_capture.assert_full();
    assert_eq!(
        db::get_page_document(&source, A)
            .await
            .unwrap()
            .unwrap()
            .blocks
            .len(),
        db::get_page_document(&target, A)
            .await
            .unwrap()
            .unwrap()
            .blocks
            .len()
    );
}
