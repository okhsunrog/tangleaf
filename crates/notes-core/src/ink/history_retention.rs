//! Local page-history retention. Native input and immutable geometry stay in
//! their existing stores; an action owns only metadata and root pins here.

use super::*;
use crate::OpKind;
use ink_format::Id;
use ink_format::model::Body;
use notes_blob::BlobHash;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SemanticState {
    revision: Option<String>,
    cursor: i64,
    dirty: bool,
    heads: Vec<(Uuid, BlobHash)>,
}

pub(crate) fn semantic_state(conn: &Connection, page: Uuid) -> CommandResult<SemanticState> {
    let (revision, cursor, dirty) = conn
        .query_row(
            "SELECT revision,cursor,dirty FROM ink_documents WHERE page_uuid=?1",
            [page],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .unwrap_or((None, 0, false));
    let mut heads = Vec::new();
    for id in versions::head_ids(conn, page)? {
        let version =
            versions::get(conn, id)?.ok_or_else(|| CommandError::not_found("Missing ink head"))?;
        heads.push((id, version.publication.root_hash));
    }
    Ok(SemanticState {
        revision,
        cursor,
        dirty,
        heads,
    })
}

pub(crate) fn is_ink_page(conn: &Connection, page: Uuid) -> CommandResult<bool> {
    Ok(conn
        .query_row(
            "SELECT content_type='ink' FROM page_identities WHERE page_uuid=?1",
            [page],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

pub(crate) fn is_untouched_empty(conn: &Connection, page: Uuid) -> CommandResult<bool> {
    let state = semantic_state(conn, page)?;
    Ok(state.revision.is_none() && state.cursor == 0 && !state.dirty && state.heads.is_empty())
}

pub(crate) fn has_retention(conn: &Connection, action: Uuid, page: Uuid) -> CommandResult<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM history_ink_snapshots WHERE action_uuid=?1 AND page_uuid=?2)",
        params![action, page],
        |row| row.get(0),
    )?)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Variant {
    root: Id,
    hash: BlobHash,
    device_name: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedPage {
    version: u32,
    page: Uuid,
    working_root: Option<Id>,
    cursor: i64,
    history: Vec<(i64, Id)>,
    previous_heads: Vec<Uuid>,
    variants: Vec<Variant>,
}

fn root_id(bytes: Vec<u8>) -> CommandResult<Id> {
    bytes
        .try_into()
        .map_err(|_| CommandError::invalid("Invalid retained ink root ID"))
}

fn retained_root(
    conn: &Connection,
    id: Id,
) -> CommandResult<(ink_format::snapshot::Snapshot, BlobHash)> {
    let root = storage::get_record(conn, id, None)?;
    let bytes = root.encode().map_err(err)?;
    let snapshot = storage::load_root(conn, root)?;
    Ok((snapshot, BlobHash::digest(&bytes)))
}

fn validate_root(conn: &Connection, id: Id) -> CommandResult<BlobHash> {
    let (snapshot, hash) = retained_root(conn, id)?;
    snapshot.validate(&[1, 2, 3, 4]).map_err(err)?;
    storage::validate_adapter(&snapshot)?;
    Ok(hash)
}

fn validate_recovery_root(conn: &Connection, id: Id) -> CommandResult<BlobHash> {
    let (snapshot, hash) = retained_root(conn, id)?;
    // Apply the transfer decoder's aggregate budget before allocating decoded
    // columns from a recovery file, including roots outside the current draft.
    let decoded = snapshot.chunks.values().try_fold(0u64, |total, bytes| {
        total
            .checked_add(ink_format::chunk::Chunk::decoded_value_bytes(bytes).map_err(err)? as u64)
            .ok_or_else(|| CommandError::invalid("Decoded retained handwriting size overflow"))
    })?;
    if decoded > 128 * 1024 * 1024 {
        return Err(CommandError::invalid(
            "Retained handwriting graph exceeds the supported recovery decoded budget",
        ));
    }
    // This performs complete catalog/DAG/hash validation and the same numeric
    // and profile conversion as opening the native editor, without any writes.
    storage::to_draft(&snapshot)?;
    Ok(hash)
}

/// Validate the immutable graph of every retained owner without opening a
/// native writer, changing pins or materializing a currently unavailable root.
pub(crate) fn validate_retained_roots(conn: &Connection) -> CommandResult<()> {
    let mut statement = conn.prepare(
        "SELECT root_id, NULL AS expected_hash FROM ink_history
         UNION SELECT root_id, NULL FROM ink_documents WHERE root_id IS NOT NULL
         UNION SELECT root_id, NULL FROM history_ink_roots
         UNION SELECT root_id, root_hash FROM ink_versions WHERE root_id IS NOT NULL
         UNION SELECT root_id, root_hash FROM ink_staged_roots
         UNION SELECT root_id, root_hash FROM ink_outbox_roots WHERE root_id IS NOT NULL
         ORDER BY root_id",
    )?;
    let mut rows = statement.query([])?;
    let mut last: Option<(Id, BlobHash)> = None;
    while let Some(row) = rows.next()? {
        let root = root_id(row.get(0)?)?;
        let hash = match last {
            Some((previous, hash)) if previous == root => hash,
            _ => {
                let hash = validate_recovery_root(conn, root)?;
                last = Some((root, hash));
                hash
            }
        };
        if let Some(expected) = row.get::<_, Option<Vec<u8>>>(1)? {
            let expected: [u8; 32] = expected
                .try_into()
                .map_err(|_| CommandError::invalid("Invalid retained handwriting root hash"))?;
            if hash != BlobHash::from_bytes(expected) {
                return Err(CommandError::invalid(
                    "Retained handwriting owner hash mismatch",
                ));
            }
        }
    }
    Ok(())
}

impl RetainedPage {
    fn validate_metadata(&self) -> CommandResult<()> {
        if self.version != 1
            || self.page.is_nil()
            || self.cursor < 0
            || self.previous_heads.len() > 128
            || self.previous_heads.iter().any(Uuid::is_nil)
            || self.previous_heads.iter().collect::<BTreeSet<_>>().len()
                != self.previous_heads.len()
            || self.history.iter().any(|(seq, _)| *seq < 0)
            || self.history.windows(2).any(|pair| pair[0].0 >= pair[1].0)
            || (!self.history.is_empty()
                && !self
                    .history
                    .iter()
                    .any(|(seq, root)| *seq == self.cursor && Some(*root) == self.working_root))
            || (self.working_root.is_none() && !self.history.is_empty())
            || self
                .variants
                .iter()
                .map(|variant| variant.hash)
                .collect::<BTreeSet<_>>()
                .len()
                != self.variants.len()
        {
            return Err(CommandError::invalid(
                "Invalid drawing recovery metadata or history cursor",
            ));
        }
        for variant in &self.variants {
            Publish {
                version_uuid: Uuid::now_v7(),
                page_uuid: self.page,
                parents: self.previous_heads.clone(),
                root_hash: variant.hash,
                device_name: variant.device_name.clone(),
            }
            .validate()?;
        }
        Ok(())
    }

    fn roots(&self) -> BTreeSet<Id> {
        self.working_root
            .into_iter()
            .chain(self.history.iter().map(|(_, root)| *root))
            .chain(self.variants.iter().map(|variant| variant.root))
            .collect()
    }
}

/// Capture before PageDelete runs its GC, in the deletion's own transaction.
pub(crate) fn capture(conn: &Connection, action: Uuid, page: Uuid) -> CommandResult<()> {
    let state = conn
        .query_row(
            "SELECT root_id,cursor,dirty,base_version FROM ink_documents WHERE page_uuid=?1",
            [page],
            |row| {
                Ok((
                    row.get::<_, Option<Vec<u8>>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, Option<Uuid>>(3)?,
                ))
            },
        )
        .optional()?;
    let working_root = state
        .as_ref()
        .and_then(|state| state.0.clone())
        .map(root_id)
        .transpose()?;
    let dirty = state.as_ref().is_some_and(|state| state.2);
    let base = state.as_ref().and_then(|state| state.3);
    let history = conn
        .prepare("SELECT seq,root_id FROM ink_history WHERE page_uuid=?1 ORDER BY seq")?
        .query_map([page], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?
        .map(|row| {
            let (seq, root) = row?;
            Ok((seq, root_id(root)?))
        })
        .collect::<CommandResult<Vec<_>>>()?;
    let heads = versions::head_ids(conn, page)?;
    let mut variants = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(root) = working_root
        && (dirty || heads.is_empty())
    {
        let hash = validate_root(conn, root)?;
        seen.insert(hash);
        variants.push(Variant {
            root,
            hash,
            device_name: "Recovered working copy".into(),
        });
    }
    for head in &heads {
        if dirty && base == Some(*head) {
            continue;
        }
        let version = versions::get(conn, *head)?
            .ok_or_else(|| CommandError::not_found("Missing ink version"))?;
        if !seen.insert(version.publication.root_hash) {
            continue;
        }
        let root = conn
            .query_row(
                "SELECT root_id FROM ink_versions WHERE version_uuid=?1",
                [head],
                |row| row.get::<_, Option<Vec<u8>>>(0),
            )?
            .ok_or_else(|| {
                CommandError::not_found(
                    "Download all handwriting versions before deleting the page",
                )
            })?;
        let root = root_id(root)?;
        if validate_root(conn, root)? != version.publication.root_hash {
            return Err(CommandError::invalid(
                "Retained handwriting version hash mismatch",
            ));
        }
        variants.push(Variant {
            root,
            hash: version.publication.root_hash,
            device_name: version.publication.device_name,
        });
    }
    if let Some(root) = working_root {
        let hash = validate_root(conn, root)?;
        if let Some(index) = variants.iter().position(|variant| variant.hash == hash) {
            variants.swap(0, index);
        }
    }
    let retained = RetainedPage {
        version: 1,
        page,
        working_root,
        cursor: state.as_ref().map_or(0, |state| state.1),
        history,
        previous_heads: heads,
        variants,
    };
    retained.validate_metadata()?;
    let json = serde_json::to_string(&retained).map_err(err)?;
    conn.execute("INSERT INTO history_ink_snapshots(action_uuid,page_uuid,metadata_json,metadata_hash) VALUES(?1,?2,?3,?4)", params![action, page, json, Sha256::digest(json.as_bytes()).as_slice()])?;
    for root in retained.roots() {
        // Verify even native redo roots before retaining their existing graph.
        let record = storage::get_record(conn, root, None)?;
        validate_root(conn, root)?;
        let document = ink_format::model::Document::read(&record).map_err(err)?;
        storage::history::index_root(conn, root, &document)?;
        conn.execute(
            "INSERT INTO history_ink_roots(action_uuid,page_uuid,root_id) VALUES(?1,?2,?3)",
            params![action, page, root.as_slice()],
        )?;
    }
    Ok(())
}

pub(crate) struct Restore {
    pages: Vec<(RetainedPage, Option<Uuid>)>,
    operations: Vec<OpKind>,
}

impl Restore {
    pub(crate) fn operations(&self) -> &[OpKind] {
        &self.operations
    }

    pub(crate) fn restore_native(&self, conn: &Connection) -> CommandResult<()> {
        for (page, publication) in &self.pages {
            conn.execute(
                "INSERT OR IGNORE INTO ink_documents(page_uuid) VALUES(?1)",
                [page.page],
            )?;
            conn.execute("DELETE FROM ink_history WHERE page_uuid=?1", [page.page])?;
            for (seq, root) in &page.history {
                conn.execute(
                    "INSERT INTO ink_history(page_uuid,seq,root_id) VALUES(?1,?2,?3)",
                    params![page.page, seq, root.as_slice()],
                )?;
            }
            let revision = page.working_root.map(|_| Uuid::now_v7().to_string());
            conn.execute("UPDATE ink_documents SET root_id=?2,revision=?3,cursor=?4,dirty=0,editing=0,publication_requested=0,base_version=?5 WHERE page_uuid=?1", params![page.page, page.working_root.map(|root| root.to_vec()), revision, page.cursor, publication])?;
        }
        Ok(())
    }
}

/// Validate retained metadata and bodies before returning a sealed restore plan.
pub(crate) fn prepare_restore(
    conn: &Connection,
    action: Uuid,
    pages: &[Uuid],
) -> CommandResult<Restore> {
    let mut result = Restore {
        pages: Vec::new(),
        operations: Vec::new(),
    };
    for page in pages {
        let (json, hash) = conn.query_row("SELECT metadata_json,metadata_hash FROM history_ink_snapshots WHERE action_uuid=?1 AND page_uuid=?2", params![action, page], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))).optional()?
            .ok_or_else(|| CommandError::not_found("Drawing recovery data is missing; restore a backup"))?;
        if Sha256::digest(json.as_bytes()).as_slice() != hash {
            return Err(CommandError::invalid(
                "Drawing recovery metadata is corrupt",
            ));
        }
        let retained: RetainedPage = serde_json::from_str(&json).map_err(err)?;
        retained.validate_metadata()?;
        if retained.page != *page {
            return Err(CommandError::invalid(
                "Unsupported drawing recovery metadata",
            ));
        }
        let pins = conn
            .prepare("SELECT root_id FROM history_ink_roots WHERE action_uuid=?1 AND page_uuid=?2")?
            .query_map(params![action, page], |row| row.get::<_, Vec<u8>>(0))?
            .map(|row| root_id(row?))
            .collect::<CommandResult<BTreeSet<_>>>()?;
        if pins != retained.roots()
            || (!retained.history.is_empty()
                && !retained.history.iter().any(|(seq, root)| {
                    *seq == retained.cursor && Some(*root) == retained.working_root
                }))
        {
            return Err(CommandError::invalid(
                "Drawing recovery root pins or history cursor are corrupt",
            ));
        }
        for root in &pins {
            validate_root(conn, *root)?;
        }
        if let Some(root) = retained.working_root {
            validate_root(conn, root)?;
        }
        let mut publication = None;
        for variant in &retained.variants {
            if validate_root(conn, variant.root)? != variant.hash {
                return Err(CommandError::invalid("Drawing recovery root hash mismatch"));
            }
            conn.execute("INSERT INTO ink_staged_roots(root_hash,root_id) VALUES(?1,?2) ON CONFLICT(root_hash) DO UPDATE SET root_id=excluded.root_id", params![variant.hash.as_bytes().as_slice(), variant.root.as_slice()])?;
            // The recovered draft is now a publication. Protect its chunks
            // from local compaction just like ordinary publish/download.
            conn.execute("INSERT OR IGNORE INTO ink_sealed_chunks SELECT object_id FROM ink_root_refs WHERE root_id=?1 AND kind=1", [variant.root.as_slice()])?;
            let version_uuid = Uuid::now_v7();
            publication.get_or_insert(version_uuid);
            result.operations.push(OpKind::InkPublish(Publish {
                version_uuid,
                page_uuid: *page,
                parents: retained.previous_heads.clone(),
                root_hash: variant.hash,
                device_name: variant.device_name.clone(),
            }));
        }
        result.pages.push((retained, publication));
    }
    Ok(result)
}

/// Called only after complete history transitions, never between stack moves.
pub(crate) fn cleanup(conn: &Connection) -> CommandResult<()> {
    let removed = conn.execute("DELETE FROM history_ink_snapshots WHERE action_uuid NOT IN (SELECT action_uuid FROM history_undo UNION SELECT action_uuid FROM history_redo)", [])?;
    if removed > 0 {
        storage::history::collect(conn)?;
    }
    Ok(())
}

#[cfg(test)]
mod retained_validation_tests {
    use super::*;

    struct Fixture {
        _directory: tempfile::TempDir,
        connection: crate::Connection,
        ink: Store,
    }

    async fn fixture() -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.db");
        let connection = crate::db::open(&path).await.unwrap();
        crate::configure_sync(&connection, "https://retained-validation.invalid")
            .await
            .unwrap();
        let page = crate::db::create_handwritten_note_with_ops(&connection, None)
            .await
            .unwrap()
            .value;
        Fixture {
            _directory: directory,
            connection,
            ink: Store::new(path, page.uuid),
        }
    }

    fn draw(ink: &Store, index: u128) {
        ink.patch(
            InkDraftPatch {
                order: vec![Uuid::from_u128(index)],
                upserts: vec![InkStroke {
                    id: Uuid::from_u128(index),
                    width: 2.0,
                    points: vec![InkPoint {
                        x: 10.0 + index as f64,
                        y: 20.0,
                        pressure: 0.5,
                        tilt_x: 0.0,
                        tilt_y: 0.0,
                        time: index as f64,
                    }],
                }],
                background: InkBackground::Grid,
            },
            ink.read().unwrap().revision,
        )
        .unwrap();
    }

    async fn validate(connection: &crate::Connection) -> anyhow::Result<()> {
        connection
            .call_domain(|database| -> CommandResult<()> {
                database.pragma_update(None, "query_only", "ON")?;
                let before = database.total_changes();
                let result = {
                    let transaction = database.transaction()?;
                    let result = super::super::validate_retained_roots(&transaction);
                    transaction.commit()?;
                    result
                };
                assert_eq!(
                    database.total_changes(),
                    before,
                    "validation cannot change owners or content"
                );
                database.pragma_update(None, "query_only", "OFF")?;
                result
            })
            .await
    }

    async fn current_root(connection: &crate::Connection, page: Uuid) -> Vec<u8> {
        connection
            .call(move |database| {
                database.query_row(
                    "SELECT root_id FROM ink_documents WHERE page_uuid=?1",
                    [page],
                    |row| row.get(0),
                )
            })
            .await
            .unwrap()
    }

    async fn corrupt_chunk(connection: &crate::Connection, root: Vec<u8>) {
        connection.call(move |database| {
            let (id,mut bytes) = database.query_row("SELECT chunks.id,chunks.data FROM ink_chunks chunks
                JOIN ink_root_refs refs ON refs.kind=1 AND refs.object_id=chunks.id WHERE refs.root_id=?1 LIMIT 1",
                [root], |row| Ok((row.get::<_,Vec<u8>>(0)?,row.get::<_,Vec<u8>>(1)?)))?;
            *bytes.last_mut().unwrap() ^= 1;
            database.execute("UPDATE ink_chunks SET data=?2 WHERE id=?1", params![id,bytes])?;
            Ok(())
        }).await.unwrap();
    }

    #[tokio::test]
    async fn read_only_validation_checks_noncurrent_native_records_and_chunk_hashes() {
        for kind in [0, 1] {
            let fixture = fixture().await;
            draw(&fixture.ink, 1);
            let old = current_root(&fixture.connection, fixture.ink.document()).await;
            draw(&fixture.ink, 2);
            let current = current_root(&fixture.connection, fixture.ink.document()).await;
            validate(&fixture.connection).await.unwrap();
            fixture.connection.call(move |database| {
                let id: Vec<u8> = database.query_row("SELECT old.object_id FROM ink_root_refs old
                    WHERE old.root_id=?1 AND old.kind=?3 AND NOT EXISTS(
                    SELECT 1 FROM ink_root_refs current WHERE current.root_id=?2 AND current.kind=old.kind AND current.object_id=old.object_id) LIMIT 1",
                    params![old,current,kind], |row| row.get(0))?;
                let table = if kind == 0 { "ink_records" } else { "ink_chunks" };
                let mut bytes: Vec<u8> = database.query_row(&format!("SELECT data FROM {table} WHERE id=?1"), [&id], |row| row.get(0))?;
                *bytes.last_mut().unwrap() ^= 1;
                database.execute(&format!("UPDATE {table} SET data=?2 WHERE id=?1"), params![id,bytes])?;
                Ok(())
            }).await.unwrap();
            assert_eq!(
                fixture.ink.read().unwrap().draft.strokes[0].id,
                Uuid::from_u128(2)
            );
            assert!(
                validate(&fixture.connection).await.is_err(),
                "noncurrent native owner must still be validated"
            );
            fixture.connection.close().await.unwrap();
        }
    }

    async fn deleted_fixture(outbox_only: bool) -> (Fixture, Vec<u8>) {
        let fixture = fixture().await;
        draw(&fixture.ink, 3);
        let root = current_root(&fixture.connection, fixture.ink.document()).await;
        let published = fixture
            .ink
            .publish("Retained validation".into())
            .unwrap()
            .unwrap();
        let publication = crate::pending_outbox(&fixture.connection,100).await.unwrap().into_iter()
            .find(|op| matches!(&op.kind,OpKind::InkPublish(p) if p.version_uuid==published.version_uuid)).unwrap();
        crate::db::delete_page(&fixture.connection, fixture.ink.document())
            .await
            .unwrap();
        if outbox_only {
            let archive = crate::db::export_archive(&fixture.connection)
                .await
                .unwrap();
            crate::db::import_archive(&fixture.connection, archive)
                .await
                .unwrap();
        } else {
            crate::acknowledge_server_op(&fixture.connection, publication.op_id, 1)
                .await
                .unwrap();
        }
        fixture
            .connection
            .call(move |database| {
                let (documents, native_history, versions, history, outbox) = database.query_row(
                    "SELECT
                (SELECT COUNT(*) FROM ink_documents),(SELECT COUNT(*) FROM ink_history),
                (SELECT COUNT(*) FROM ink_versions),(SELECT COUNT(*) FROM history_ink_roots),
                (SELECT COUNT(*) FROM ink_outbox_roots)",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, i64>(4)?,
                        ))
                    },
                )?;
                assert_eq!((documents, native_history, versions), (0, 0, 0));
                if outbox_only {
                    assert_eq!(history, 0);
                    assert_eq!(outbox, 1);
                } else {
                    assert!(history > 0);
                    assert_eq!(outbox, 0);
                }
                Ok(())
            })
            .await
            .unwrap();
        (fixture, root)
    }

    #[tokio::test]
    async fn deleted_history_only_and_outbox_only_roots_are_validated() {
        for outbox_only in [false, true] {
            let (fixture, root) = deleted_fixture(outbox_only).await;
            validate(&fixture.connection).await.unwrap();
            corrupt_chunk(&fixture.connection, root).await;
            assert!(validate(&fixture.connection).await.is_err());
            fixture.connection.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn staged_only_root_is_validated_and_its_expected_hash_is_checked() {
        let source = fixture().await;
        draw(&source.ink, 4);
        let publication = source
            .ink
            .publish("Staging validation".into())
            .unwrap()
            .unwrap();
        let graph = transfer::export_graph(&source.connection, publication.root_hash)
            .await
            .unwrap();
        let target = crate::db::open(source._directory.path().join("target.db"))
            .await
            .unwrap();
        transfer::stage_graph(&target, publication.root_hash, graph)
            .await
            .unwrap();
        validate(&target).await.unwrap();
        target
            .call(|database| {
                assert_eq!(
                    database.query_row("SELECT COUNT(*) FROM ink_versions", [], |row| row
                        .get::<_, i64>(0))?,
                    0
                );
                database.execute("UPDATE ink_staged_roots SET root_hash=?1", [vec![7u8; 32]])?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(
            format!("{:#}", validate(&target).await.unwrap_err()).contains("owner hash mismatch")
        );
        target.close().await.unwrap();
        source.connection.close().await.unwrap();
    }

    #[tokio::test]
    async fn every_expected_hash_is_checked_even_when_two_owners_share_one_root() {
        for table in ["ink_versions", "ink_outbox_roots"] {
            let fixture = fixture().await;
            draw(&fixture.ink, 5);
            fixture
                .ink
                .publish("Shared validation".into())
                .unwrap()
                .unwrap();
            validate(&fixture.connection).await.unwrap();
            fixture
                .connection
                .call(move |database| {
                    database
                        .execute(&format!("UPDATE {table} SET root_hash=?1"), [vec![8u8; 32]])?;
                    Ok(())
                })
                .await
                .unwrap();
            assert!(
                format!("{:#}", validate(&fixture.connection).await.unwrap_err())
                    .contains("owner hash mismatch")
            );
            fixture.connection.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn unavailable_version_roots_remain_explicitly_unavailable() {
        let source = fixture().await;
        draw(&source.ink, 6);
        source
            .ink
            .publish("Unavailable validation".into())
            .unwrap()
            .unwrap();
        let snapshot = crate::export_sync_snapshot(&source.connection, 0)
            .await
            .unwrap();
        let target = crate::db::open(source._directory.path().join("unavailable.db"))
            .await
            .unwrap();
        crate::import_sync_snapshot(&target, snapshot)
            .await
            .unwrap();
        validate(&target).await.unwrap();
        target
            .call(|database| {
                assert_eq!(
                    database.query_row(
                        "SELECT COUNT(*) FROM ink_versions WHERE root_id IS NULL",
                        [],
                        |row| row.get::<_, i64>(0)
                    )?,
                    1
                );
                assert_eq!(
                    database.query_row("SELECT COUNT(*) FROM ink_records", [], |row| row
                        .get::<_, i64>(0))?,
                    0
                );
                Ok(())
            })
            .await
            .unwrap();
        target.close().await.unwrap();
        source.connection.close().await.unwrap();
    }
}
