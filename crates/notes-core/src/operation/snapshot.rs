use super::*;
use crate::store::bulk::BulkMutation;

#[derive(Clone, Copy)]
pub(crate) enum SnapshotImportGuard {
    Replace,
    RequireEmpty,
}

/// A validated replacement payload. Only the bulk store boundary may apply it
/// to a live replica; projection uses its own isolated in-memory transaction.
pub(crate) struct PreparedSyncSnapshot {
    snapshot: SyncSnapshot,
    guard: SnapshotImportGuard,
}

impl crate::store::NotesStore {
    /// Replace a replica from an authoritative snapshot, preserving the
    /// existing workspace/active-handwriting safety checks.
    pub async fn import_sync_snapshot(
        &self,
        snapshot: SyncSnapshot,
    ) -> Result<crate::store::Committed<()>> {
        let prepared = PreparedSyncSnapshot::prepare(snapshot, SnapshotImportGuard::Replace)?;
        self.execute_bulk(prepared).await
    }

    /// Adopt the server baseline only while this replica is still empty.
    /// The emptiness check and replacement share one writer transaction.
    pub async fn bootstrap_sync_snapshot(
        &self,
        snapshot: SyncSnapshot,
    ) -> Result<crate::store::Committed<()>> {
        let prepared = PreparedSyncSnapshot::prepare(snapshot, SnapshotImportGuard::RequireEmpty)?;
        self.execute_bulk(prepared).await
    }
}

impl PreparedSyncSnapshot {
    pub(crate) fn prepare(snapshot: SyncSnapshot, guard: SnapshotImportGuard) -> CoreResult<Self> {
        if !matches!(snapshot.format_version, 6 | FORMAT_VERSION)
            || (snapshot.format_version == 6
                && (!snapshot.ink_versions.is_empty()
                    || snapshot
                        .page_identities
                        .iter()
                        .any(|p| p.kind == PageKind::Handwriting)))
        {
            return Err(CoreError::invalid(format!(
                "unsupported snapshot format version {}",
                snapshot.format_version
            )));
        }
        validate_server_sequence(snapshot.seq)?;
        validate_snapshot(&snapshot)?;
        for page in &snapshot.pages {
            validate_title(page.title.as_deref())?;
        }
        for structure in &snapshot.structures {
            if structure.parent_uuid == Some(structure.block_uuid) {
                return Err(CoreError::invalid("a block cannot be its own parent"));
            }
        }
        for attachment in &snapshot.attachments {
            if attachment.attachment_uuid
                != attachment_uuid(attachment.owner, &attachment.blob_hash)
            {
                return Err(CoreError::invalid(
                    "attachment UUID does not match its owner and hash",
                ));
            }
            if attachment.present {
                let filename = attachment.filename.as_deref().ok_or_else(|| {
                    CoreError::invalid("present attachment is missing its filename")
                })?;
                validate_attachment_filename(filename)?;
                if attachment
                    .mime
                    .as_deref()
                    .is_none_or(|mime| mime.trim().is_empty())
                {
                    return Err(CoreError::invalid(
                        "present attachment is missing its MIME type",
                    ));
                }
                if attachment.size.is_none_or(|size| size > i64::MAX as u64) {
                    return Err(CoreError::invalid("present attachment has an invalid size"));
                }
            }
        }
        Ok(Self { snapshot, guard })
    }

    pub(crate) fn apply_in_transaction(
        self,
        transaction: &rusqlite::Transaction<'_>,
    ) -> CoreResult<BulkMutation<()>> {
        let Self { snapshot, guard } = self;
        if matches!(guard, SnapshotImportGuard::RequireEmpty) {
            require_empty_replica(transaction)?;
        }
        let current_workspace_uuid = crate::db::transaction_workspace_uuid(transaction)?;
        if current_workspace_uuid != snapshot.workspace_uuid {
            let has_source_state: bool = transaction.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM pages
                    UNION ALL SELECT 1 FROM tombstones
                    UNION ALL SELECT 1 FROM page_identities
                 )",
                [],
                |row| row.get(0),
            )?;
            if has_source_state {
                return Err(CoreError::conflict(
                    "snapshot belongs to a different non-empty workspace",
                ));
            }
        }
        let incoming_pages: HashSet<_> = snapshot.pages.iter().map(|p| p.uuid).collect();
        let active_ink = transaction.prepare("SELECT d.page_uuid FROM ink_documents d JOIN pages p ON p.uuid=d.page_uuid WHERE d.dirty=1 OR d.editing=1 OR d.publication_requested=1")?
            .query_map([], |r|r.get::<_,uuid::Uuid>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if active_ink.iter().any(|id| !incoming_pages.contains(id)) {
            return Err(CoreError::conflict(
                "Snapshot would remove an open or unpublished handwritten note",
            ));
        }
        // Notes the snapshot does not contain are gone; their drawings must go
        // with them rather than surviving as unreachable bodies that every
        // later export would carry.
        let retained: HashSet<_> = snapshot
            .page_identities
            .iter()
            .map(|identity| identity.uuid)
            .chain(incoming_pages.iter().copied())
            .collect();
        let stored_ink = transaction
            .prepare(
                "SELECT page_uuid FROM ink_documents UNION SELECT page_uuid FROM ink_versions",
            )?
            .query_map([], |r| r.get::<_, uuid::Uuid>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for page in stored_ink
            .into_iter()
            .filter(|page| !retained.contains(page))
        {
            crate::ink::purge_page(transaction, page)?;
        }
        transaction.execute_batch(
            "DELETE FROM page_links;
             DELETE FROM block_refs;
             DELETE FROM attachments;
             DELETE FROM blocks;
             DELETE FROM pages;
             DELETE FROM page_identities WHERE page_uuid NOT IN (SELECT page_uuid FROM ink_documents UNION SELECT page_uuid FROM ink_versions);
             DELETE FROM page_alias_lww;
             DELETE FROM block_structure_lww;
             DELETE FROM attachment_lww;
             DELETE FROM tombstones;
             DELETE FROM applied_ops;
             DELETE FROM sync_outbox;
             DELETE FROM history_undo;
             DELETE FROM history_redo;",
        )?;
        if current_workspace_uuid != snapshot.workspace_uuid {
            transaction.execute("DELETE FROM external_import_receipts", [])?;
        }
        transaction.execute(
            "UPDATE workspace SET uuid = ?1 WHERE singleton = 1",
            [snapshot.workspace_uuid],
        )?;
        for identity in snapshot.page_identities {
            ensure_page_identity(transaction, identity.uuid, &identity.kind)?;
        }
        for version in &snapshot.ink_versions {
            crate::ink::versions::apply(transaction, version)?;
        }
        for page in snapshot.pages {
            let normalized_title = page.title.as_deref().map(crate::model::normalize_title);
            let title_stemmed = page
                .title
                .as_deref()
                .map(crate::stem::stem)
                .unwrap_or_default();
            transaction.execute(
                "INSERT INTO pages(uuid, title, normalized_title, title_stemmed, layout,
                                   title_hlc, layout_hlc, existence_hlc, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    page.uuid,
                    page.title,
                    normalized_title,
                    title_stemmed,
                    page.layout,
                    page.title_hlc.map(|value| value.to_string()),
                    page.layout_hlc.map(|value| value.to_string()),
                    page.existence_hlc.to_string(),
                    page.created_at,
                    page.updated_at,
                ],
            )?;
            materialize_page_kind(transaction, page.uuid, &page.kind)?;
        }
        for alias in snapshot.page_aliases {
            transaction.execute(
                "INSERT INTO page_alias_lww(page_uuid, alias, hlc, present)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    alias.page_uuid,
                    alias.alias,
                    alias.hlc.to_string(),
                    alias.present,
                ],
            )?;
        }
        for block in &snapshot.blocks {
            transaction.execute(
                "INSERT INTO blocks(uuid, page_uuid, parent_uuid, order_key, style, markdown,
                                    body_stemmed, markdown_hlc, style_hlc, structure_hlc,
                                    existence_hlc, created_at, updated_at)
                 VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    block.uuid,
                    block.page_uuid,
                    block.order_key,
                    block.style,
                    block.markdown,
                    crate::stem::stem(&block.markdown),
                    block.markdown_hlc.as_ref().map(ToString::to_string),
                    block.style_hlc.as_ref().map(ToString::to_string),
                    block.structure_hlc.as_ref().map(ToString::to_string),
                    block.existence_hlc.to_string(),
                    block.created_at,
                    block.updated_at,
                ],
            )?;
        }
        for structure in snapshot.structures {
            transaction.execute(
                "INSERT INTO block_structure_lww
                   (block_uuid, page_uuid, parent_uuid, order_key, hlc)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    structure.block_uuid,
                    structure.page_uuid,
                    structure.parent_uuid,
                    structure.order_key,
                    structure.hlc.to_string(),
                ],
            )?;
        }
        for tombstone in snapshot.tombstones {
            transaction.execute(
                "INSERT INTO tombstones(uuid, object_kind, deleted_hlc, root_page_uuid)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    tombstone.uuid,
                    tombstone.object_kind,
                    tombstone.deleted_hlc.to_string(),
                    tombstone.root_page_uuid,
                ],
            )?;
        }
        for attachment in snapshot.attachments {
            write_attachment_intent(transaction, &attachment)?;
        }
        reconcile_structure(transaction)?;
        let blocks = {
            let mut statement = transaction.prepare(
                "SELECT uuid, markdown, COALESCE(markdown_hlc, existence_hlc) FROM blocks",
            )?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, uuid::Uuid>(0)?,
                        row.get::<_, String>(1)?,
                        sql_hlc(row.get::<_, String>(2)?, 2)?.timestamp_seconds(),
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (uuid, markdown, timestamp) in blocks {
            replace_block_refs(transaction, uuid, &markdown, timestamp)?;
        }
        reconcile_all_attachments(transaction)?;
        observe_max_persisted_hlc(transaction)?;
        transaction.execute(
            "INSERT INTO sync_meta(key, value) VALUES ('last_server_seq', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [snapshot.seq.to_string()],
        )?;
        crate::ink::history_retention::cleanup(transaction)?;
        Ok(BulkMutation {
            value: (),
            wrote: true,
        })
    }
}

fn require_empty_replica(transaction: &rusqlite::Transaction<'_>) -> CoreResult<()> {
    // This runs inside the same writer transaction as replacement, after any
    // local edits queued during network/bootstrap work. Do not erase source
    // content, unpublished ink, or pending/history work based on an older
    // in-memory snapshot. Configuration and binding metadata are not content.
    let has_source_state: bool = transaction.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM pages
            UNION ALL SELECT 1 FROM page_identities
            UNION ALL SELECT 1 FROM page_alias_lww
            UNION ALL SELECT 1 FROM blocks
            UNION ALL SELECT 1 FROM block_structure_lww
            UNION ALL SELECT 1 FROM tombstones
            UNION ALL SELECT 1 FROM attachment_lww
            UNION ALL SELECT 1 FROM attachments
            UNION ALL SELECT 1 FROM ink_documents
            UNION ALL SELECT 1 FROM ink_versions
            UNION ALL SELECT 1 FROM applied_ops
            UNION ALL SELECT 1 FROM sync_outbox
            UNION ALL SELECT 1 FROM history_undo
            UNION ALL SELECT 1 FROM history_redo
            UNION ALL SELECT 1 FROM external_import_receipts
        )",
        [],
        |row| row.get(0),
    )?;
    if has_source_state {
        return Err(CoreError::conflict(
            "local workspace changed before the sync bootstrap snapshot could be imported",
        ));
    }
    Ok(())
}
