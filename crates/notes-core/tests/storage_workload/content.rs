//! Real native ink and filesystem attachment work competing with the main writer.

use super::*;
use notes_blob::{BlobHash, BlobStore, InstallOutcome};
use notes_core::AttachmentOwner;
use notes_core::ink::runtime::{Session, WorkspaceRuntime};
use notes_core::ink::{self, InkBackground, InkDraft, InkDraftPatch, InkPoint, InkStroke};
use std::io::Cursor;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const ATTACHMENT_BYTES: usize = 256 * 1024;
const CONTENT_EVERY: usize = 3;
const POINTS_PER_STROKE: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PrimaryCounts {
    pages: i64,
    blocks: i64,
    attachments: i64,
}

async fn primary_counts(connection: &Connection) -> PrimaryCounts {
    connection
        .call(|database| {
            database.query_row(
                "SELECT (SELECT COUNT(*) FROM pages),
                                   (SELECT COUNT(*) FROM blocks),
                                   (SELECT COUNT(*) FROM attachments)",
                [],
                |row| {
                    Ok(PrimaryCounts {
                        pages: row.get(0)?,
                        blocks: row.get(1)?,
                        attachments: row.get(2)?,
                    })
                },
            )
        })
        .await
        .expect("count primary content")
}

pub(super) struct Workload {
    runtime: WorkspaceRuntime,
    session: Session,
    revision: Option<String>,
    draft: InkDraft,
    blobs: BlobStore,
    owner: AttachmentOwner,
    attachments: Vec<(db::Attachment, Vec<u8>)>,
    publications: Arc<AtomicUsize>,
    phases: BTreeMap<&'static str, Vec<f64>>,
    before: PrimaryCounts,
    salt: Uuid,
    path: std::path::PathBuf,
}

impl Workload {
    pub(super) async fn new(store: &NotesStore, path: &Path, owner: Uuid) -> Self {
        let page = store
            .create_handwritten_note(None)
            .await
            .expect("create ink fixture")
            .value;
        let runtime = WorkspaceRuntime::default();
        let publications = Arc::new(AtomicUsize::new(0));
        let published = publications.clone();
        let session = Session::new_in_runtime(
            ink::Store::new(path, page.uuid),
            runtime.clone(),
            "Storage workload".into(),
            move |_| {
                published.fetch_add(1, Ordering::Relaxed);
            },
        )
        .expect("create ink session");
        let initial = session.open(true).await.expect("open native ink editor");
        assert!(initial.snapshot.draft.strokes.is_empty());
        Self {
            runtime,
            session,
            revision: initial.snapshot.revision,
            draft: initial.snapshot.draft,
            blobs: BlobStore::new(
                path.parent()
                    .expect("fixture directory")
                    .join("content-blobs"),
            ),
            owner: AttachmentOwner::Block(owner),
            attachments: Vec::new(),
            publications,
            phases: BTreeMap::new(),
            before: primary_counts(store.read_connection()).await,
            salt: Uuid::now_v7(),
            path: path.to_owned(),
        }
    }

    fn phase(&mut self, phase: &'static str, start: Instant) {
        self.phases
            .entry(phase)
            .or_default()
            .push(start.elapsed().as_secs_f64() * 1_000.0);
    }

    pub(super) async fn tick(&mut self, store: &NotesStore, index: usize, last: bool) {
        // Completing closes the editor. The next burst reopens the same native draft,
        // retaining its acknowledged revision and history rather than replacing it.
        if index > 0 && index.is_multiple_of(CONTENT_EVERY) {
            let start = Instant::now();
            let opened = self.session.open(true).await.expect("reopen native editor");
            assert_eq!(opened.snapshot.revision, self.revision);
            assert_draft(&opened.snapshot.draft, &self.draft);
            self.phase("ink_open", start);
        }
        let stroke = InkStroke {
            id: Uuid::now_v7(),
            width: 2.0,
            points: (0..POINTS_PER_STROKE)
                .map(|point| InkPoint {
                    x: 20.0 + point as f64 * 2.0,
                    y: 20.0 + (index % 60) as f64 * 20.0 + (point as f64 * 0.05).sin() * 8.0,
                    pressure: 0.5 + (point as f64 * 0.1).sin() * 0.2,
                    tilt_x: 5.0,
                    tilt_y: -3.0,
                    time: (index * POINTS_PER_STROKE + point) as f64,
                })
                .collect(),
        };
        self.draft.strokes.push(stroke.clone());
        self.draft.background = InkBackground::Grid;
        let start = Instant::now();
        self.revision = Some(
            self.session
                .patch(
                    InkDraftPatch {
                        order: self.draft.strokes.iter().map(|stroke| stroke.id).collect(),
                        upserts: vec![stroke],
                        background: InkBackground::Grid,
                    },
                    self.revision.clone(),
                )
                .await
                .expect("revision-safe native ink patch"),
        );
        self.phase("ink_patch", start);

        if (index + 1).is_multiple_of(CONTENT_EVERY) {
            let mut bytes = vec![0u8; ATTACHMENT_BYTES];
            for (offset, byte) in bytes.iter_mut().enumerate() {
                *byte = ((offset * 31 + index * 17) % 251) as u8;
            }
            bytes[..16].copy_from_slice(self.salt.as_bytes());
            bytes[16..24].copy_from_slice(&(index as u64).to_le_bytes());
            let hash = BlobHash::digest(&bytes);
            let blobs = self.blobs.clone();
            let input = bytes.clone();
            let start = Instant::now();
            let installed = tokio::task::spawn_blocking(move || {
                blobs.install_reader(Cursor::new(input), hash, ATTACHMENT_BYTES as u64)
            })
            .await
            .expect("join blob install")
            .expect("install and fsync blob");
            assert_eq!(installed.outcome, InstallOutcome::Installed);
            assert_eq!(installed.blob.hash, hash);
            assert_eq!(installed.blob.size, ATTACHMENT_BYTES as u64);
            self.phase("blob_install", start);
            let start = Instant::now();
            let attachment = store
                .create_attachment(
                    self.owner,
                    hash,
                    format!("workload-{}.bin", index + 1),
                    "application/octet-stream".into(),
                    ATTACHMENT_BYTES as u64,
                )
                .await
                .expect("commit installed attachment metadata");
            assert_eq!(
                attachment.changes.attachment_owners,
                vec![self.owner.uuid()]
            );
            assert_eq!(attachment.value.owner, self.owner);
            assert_eq!(attachment.value.blob_hash, hash);
            assert_eq!(attachment.value.size, ATTACHMENT_BYTES as u64);
            self.attachments.push((attachment.value, bytes));
            self.phase("attachment_metadata", start);
        }
        if (index + 1).is_multiple_of(CONTENT_EVERY) || last {
            let start = Instant::now();
            self.session
                .complete()
                .await
                .expect("complete native publication");
            assert!(!self.session.needs_completion());
            self.phase("ink_complete", start);
        }
    }

    pub(super) async fn finish(
        self,
        store: &NotesStore,
        saves: usize,
        phases: &mut BTreeMap<&'static str, Vec<f64>>,
    ) -> Verification {
        // Drain every admitted native job before closing/reopening the main workers.
        // This is outside measured bursts; shutdown must not create another publication.
        self.runtime
            .shutdown(vec![self.session.clone()])
            .await
            .expect("drain native fixture");
        assert_eq!(
            self.publications.load(Ordering::Relaxed),
            saves.div_ceil(CONTENT_EVERY)
        );
        assert_eq!(self.attachments.len(), saves / CONTENT_EVERY);
        for (phase, samples) in self.phases {
            phases.insert(phase, samples);
        }
        let result = Verification {
            page: self.session.page(),
            revision: self.revision,
            draft: self.draft,
            owner: self.owner,
            attachments: self.attachments,
            publications: saves.div_ceil(CONTENT_EVERY),
            saves,
            before: self.before,
        };
        result.verify(store, &self.path).await;
        result
    }
}

pub(super) struct Verification {
    page: Uuid,
    revision: Option<String>,
    draft: InkDraft,
    owner: AttachmentOwner,
    attachments: Vec<(db::Attachment, Vec<u8>)>,
    publications: usize,
    saves: usize,
    before: PrimaryCounts,
}

impl Verification {
    pub(super) fn authored_operations(&self) -> usize {
        self.publications + self.attachments.len()
    }

    pub(super) fn counts(&self) -> (usize, usize, usize) {
        (self.saves, self.publications, self.attachments.len())
    }

    pub(super) async fn verify(&self, store: &NotesStore, path: &Path) {
        let reader = store.read_connection();
        let counts = primary_counts(reader).await;
        assert_eq!(counts.pages, self.before.pages);
        assert_eq!(counts.blocks, self.before.blocks);
        assert_eq!(
            counts.attachments - self.before.attachments,
            self.attachments.len() as i64
        );
        let attachments = db::list_attachments(reader, self.owner)
            .await
            .expect("read fixture attachments");
        assert_eq!(attachments.len(), self.attachments.len());
        for (expected, bytes) in &self.attachments {
            assert!(
                attachments.contains(expected),
                "attachment metadata survives"
            );
            let hash = expected.blob_hash;
            let blobs = BlobStore::new(path.parent().unwrap().join("content-blobs"));
            let actual = tokio::task::spawn_blocking(move || {
                blobs.read_verified(hash, ATTACHMENT_BYTES as u64)
            })
            .await
            .unwrap()
            .expect("read and hash installed blob");
            assert_eq!(&actual, bytes, "exact blob bytes survive");
        }
        let page = self.page;
        let native = reader
            .call(move |database| {
                database.query_row(
                    "SELECT cursor, dirty, editing, publication_requested,
                  (SELECT COUNT(*) FROM ink_history WHERE page_uuid=?1),
                  (SELECT MIN(seq) FROM ink_history WHERE page_uuid=?1),
                  (SELECT MAX(seq) FROM ink_history WHERE page_uuid=?1),
                  (SELECT COUNT(*) FROM ink_versions WHERE page_uuid=?1),
                  (SELECT COUNT(*) FROM ink_versions WHERE page_uuid=?1 AND root_id IS NOT NULL)
                FROM ink_documents WHERE page_uuid=?1",
                    [page],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, bool>(1)?,
                            row.get::<_, bool>(2)?,
                            row.get::<_, bool>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, i64>(5)?,
                            row.get::<_, i64>(6)?,
                            row.get::<_, i64>(7)?,
                            row.get::<_, i64>(8)?,
                        ))
                    },
                )
            })
            .await
            .expect("native bookkeeping");
        assert_eq!(
            native,
            (
                self.saves as i64,
                false,
                false,
                false,
                (self.saves + 1).min(51) as i64,
                self.saves.saturating_sub(50) as i64,
                self.saves as i64,
                self.publications as i64,
                self.publications as i64
            )
        );
        let path = path.to_owned();
        let (snapshot, status) = tokio::task::spawn_blocking(move || {
            let ink = ink::Store::new(path, page);
            (
                ink.read().expect("read native draft"),
                ink.status().expect("read native heads"),
            )
        })
        .await
        .expect("join native verification");
        assert_eq!(snapshot.revision, self.revision);
        assert_draft(&snapshot.draft, &self.draft);
        assert_eq!(status.revision, self.revision);
        assert!(!status.unpublished_changes && !status.publication_requested);
        assert_eq!(status.heads.len(), 1);
        assert!(status.heads[0].available);
        let pending = notes_core::pending_outbox(reader, u32::MAX)
            .await
            .expect("pending uploads");
        let mut fixture_publications = 0;
        for operation in pending {
            if let OpKind::InkPublish(publication) = operation.kind {
                let graph = ink::transfer::export_graph(reader, publication.root_hash)
                    .await
                    .expect("every pending ink upload retains complete graph");
                ink::transfer::validate_graph(publication.root_hash, &graph)
                    .expect("valid queued graph");
                fixture_publications += usize::from(publication.page_uuid == self.page);
            }
        }
        assert_eq!(fixture_publications, self.publications);
    }
}

fn assert_draft(actual: &InkDraft, expected: &InkDraft) {
    assert_eq!(
        (actual.version, actual.width, actual.height),
        (expected.version, expected.width, expected.height)
    );
    // Geometry is stored in float32; compare against its defined round-trip precision.
    assert_eq!(actual.strokes.len(), expected.strokes.len());
    assert_eq!(
        serde_json::to_value(&actual.background).unwrap(),
        serde_json::to_value(&expected.background).unwrap()
    );
    for (actual, expected) in actual.strokes.iter().zip(&expected.strokes) {
        assert_eq!(actual.id, expected.id);
        assert_eq!(actual.width, expected.width);
        assert_eq!(actual.points.len(), expected.points.len());
        for (a, e) in actual.points.iter().zip(&expected.points) {
            for (a, e) in [
                (a.x, e.x),
                (a.y, e.y),
                (a.pressure, e.pressure),
                (a.tilt_x, e.tilt_x),
                (a.tilt_y, e.tilt_y),
                (a.time, e.time),
            ] {
                assert!(
                    (a - e).abs() <= 0.0001,
                    "native point round-trip differs: {a} vs {e}"
                );
            }
        }
    }
}
