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

fn validate_root(conn: &Connection, id: Id) -> CommandResult<BlobHash> {
    let root = storage::get_record(conn, id, None)?;
    let bytes = root.encode().map_err(err)?;
    let snapshot = storage::load_root(conn, root)?;
    snapshot.validate(&[1, 2, 3, 4]).map_err(err)?;
    storage::validate_adapter(&snapshot)?;
    Ok(BlobHash::digest(&bytes))
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
