use super::*;
use crate::PageListFilter;
use crate::operation::{
    BlockCreate, BlockDelete, PageCreate, PageDelete, PageSetLayout, PageSetTitle,
};
use rusqlite::OptionalExtension;

pub async fn get_page(conn: &Connection, uuid: uuid::Uuid) -> Result<Option<Page>> {
    conn.call(move |database| {
        let sql = format!("SELECT {PAGE_COLUMNS} FROM pages WHERE uuid = ?1");
        database.query_row(&sql, [uuid], row_to_page).optional()
    })
    .await
}

pub async fn get_page_by_title(conn: &Connection, title: String) -> Result<Option<Page>> {
    let title = title.trim().to_owned();
    if title.is_empty() {
        return Ok(None);
    }
    let normalized_title = crate::model::normalize_title(&title);
    conn.call(move |database| {
        let transaction = database.transaction()?;
        let page = page_by_normalized_title(&transaction, &normalized_title)?;
        transaction.commit()?;
        Ok(page)
    })
    .await
}

fn page_by_normalized_title(
    database: &rusqlite::Connection,
    normalized_title: &str,
) -> rusqlite::Result<Option<Page>> {
    let Some(page_uuid) = operation::resolve_page_alias(database, normalized_title)? else {
        return Ok(None);
    };
    let sql = format!("SELECT {PAGE_COLUMNS} FROM pages WHERE uuid = ?1");
    database
        .query_row(&sql, [page_uuid], row_to_page)
        .optional()
}

pub async fn list_pages(conn: &Connection, limit: u32) -> Result<Vec<Page>> {
    list_pages_filtered(conn, PageListFilter::Notes, limit).await
}

pub async fn list_pages_filtered(
    conn: &Connection,
    filter: PageListFilter,
    limit: u32,
) -> Result<Vec<Page>> {
    conn.call(move |database| {
        let transaction = database.transaction()?;
        let pages = list_pages_in_transaction(&transaction, filter, limit)?;
        transaction.commit()?;
        Ok(pages)
    })
    .await
}

pub(super) fn list_pages_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    filter: PageListFilter,
    limit: u32,
) -> rusqlite::Result<Vec<Page>> {
    let predicate = match filter {
        // Handwritten notes live alongside text notes in every list; only journals are apart.
        PageListFilter::Notes => "WHERE page_identity.page_kind IN ('note', 'handwriting')",
        PageListFilter::Journals => "WHERE page_identity.page_kind = 'journal'",
        PageListFilter::All => "",
    };
    let sql = format!(
        "SELECT {PAGE_COLUMNS} FROM pages
           JOIN page_identities AS page_identity ON page_identity.page_uuid = pages.uuid
           {predicate}
          ORDER BY pages.updated_at DESC, pages.uuid LIMIT ?1"
    );
    transaction
        .prepare_cached(&sql)?
        .query_map([limit], row_to_page)?
        .collect()
}

pub async fn create_page(conn: &Connection, title: String) -> Result<Page> {
    Ok(create_page_with_ops(conn, title).await?.value)
}

#[doc(hidden)]
pub async fn create_page_with_ops(
    conn: &Connection,
    title: String,
) -> Result<AppliedMutation<Page>> {
    let committed = crate::store::NotesStore::new(conn.clone())
        .create_page(title)
        .await?;
    Ok(AppliedMutation {
        value: committed.value,
        operations: committed.operations,
    })
}

pub async fn get_or_create_page_by_title(conn: &Connection, title: String) -> Result<Page> {
    Ok(get_or_create_page_by_title_with_ops(conn, title)
        .await?
        .value)
}

#[doc(hidden)]
pub async fn get_or_create_page_by_title_with_ops(
    conn: &Connection,
    title: String,
) -> Result<AppliedMutation<Page>> {
    create_page_with_ops(conn, title).await
}

pub async fn rename_page(
    conn: &Connection,
    uuid: uuid::Uuid,
    title: Option<String>,
) -> Result<Page> {
    Ok(crate::store::NotesStore::new(conn.clone())
        .rename_page_with_expected_revision(uuid, title, None)
        .await?
        .value)
}

/// Rename a page only when the caller still represents its current title.
/// The read, revision comparison, and local operation emission are atomic.
pub async fn rename_page_if_revision(
    conn: &Connection,
    uuid: uuid::Uuid,
    title: Option<String>,
    expected_revision: ContentRevision,
) -> Result<(Page, bool)> {
    let applied = rename_page_if_revision_with_ops(conn, uuid, title, expected_revision).await?;
    let changed = !applied.operations.is_empty();
    Ok((applied.value, changed))
}

#[doc(hidden)]
pub async fn rename_page_if_revision_with_ops(
    conn: &Connection,
    uuid: uuid::Uuid,
    title: Option<String>,
    expected_revision: ContentRevision,
) -> Result<AppliedMutation<Page>> {
    let committed = crate::store::NotesStore::new(conn.clone())
        .rename_page_if_revision(uuid, title, expected_revision)
        .await?;
    Ok(AppliedMutation {
        value: committed.value,
        operations: committed.operations,
    })
}

pub async fn set_page_layout(
    conn: &Connection,
    uuid: uuid::Uuid,
    layout: PageLayout,
) -> Result<Page> {
    Ok(set_page_layout_with_ops(conn, uuid, layout).await?.value)
}

#[doc(hidden)]
pub async fn set_page_layout_with_ops(
    conn: &Connection,
    uuid: uuid::Uuid,
    layout: PageLayout,
) -> Result<AppliedMutation<Page>> {
    let committed = crate::store::NotesStore::new(conn.clone())
        .set_page_layout(uuid, layout)
        .await?;
    Ok(AppliedMutation {
        value: committed.value,
        operations: committed.operations,
    })
}

impl crate::store::NotesStore {
    pub async fn create_page(&self, title: String) -> Result<crate::store::Committed<Page>> {
        let title = title.trim().to_owned();
        if title.is_empty() {
            return Err(crate::CoreError::invalid("title is required").into());
        }
        self.write(move |write| -> crate::CoreResult<Page> {
            if let Some(page) =
                page_by_normalized_title(write.sql(), &crate::model::normalize_title(&title))?
            {
                return Ok(page);
            }
            let uuid = uuid::Uuid::now_v7();
            write.apply_action(
                "create page",
                vec![OpKind::PageCreate(PageCreate {
                    uuid,
                    kind: PageKind::Note,
                    title: Some(title),
                    layout: PageLayout::Outline,
                    created_at: chrono::Utc::now().timestamp(),
                })],
                operation::ProjectionPolicy::Immediate,
            )?;
            Ok(write.sql().query_row(
                &format!("SELECT {PAGE_COLUMNS} FROM pages WHERE uuid = ?1"),
                [uuid],
                row_to_page,
            )?)
        })
        .await
    }

    pub async fn get_or_create_page_by_title(
        &self,
        title: String,
    ) -> Result<crate::store::Committed<Page>> {
        self.create_page(title).await
    }

    /// Ensure a named page and append a nonblank Markdown paragraph atomically.
    /// Existing titles and aliases resolve to their current owner.
    pub async fn create_page_with_markdown(
        &self,
        title: String,
        markdown: String,
    ) -> Result<crate::store::Committed<Page>> {
        let title = title.trim().to_owned();
        if title.is_empty() {
            return Err(crate::CoreError::invalid("title is required").into());
        }
        self.write(move |write| -> crate::CoreResult<Page> {
            let existing =
                page_by_normalized_title(write.sql(), &crate::model::normalize_title(&title))?;
            if markdown.trim().is_empty()
                && let Some(page) = existing
            {
                return Ok(page);
            }
            let page_uuid = existing
                .as_ref()
                .map_or_else(uuid::Uuid::now_v7, |page| page.uuid);
            let now = chrono::Utc::now().timestamp();
            let mut kinds = Vec::new();
            if existing.is_none() {
                kinds.push(OpKind::PageCreate(PageCreate {
                    uuid: page_uuid,
                    kind: PageKind::Note,
                    title: Some(title),
                    layout: PageLayout::Outline,
                    created_at: now,
                }));
            }
            if !markdown.trim().is_empty() {
                let (order_key, moves) =
                    super::blocks::append_root_placement(write.sql(), page_uuid)?;
                kinds.push(OpKind::BlockCreate(BlockCreate {
                    uuid: uuid::Uuid::now_v7(),
                    page_uuid,
                    parent_uuid: None,
                    order_key,
                    style: BlockStyle::Paragraph,
                    markdown,
                    created_at: now,
                }));
                kinds.extend(moves);
            }
            write.apply_action(
                "capture note",
                kinds,
                operation::ProjectionPolicy::Immediate,
            )?;
            Ok(write.sql().query_row(
                &format!("SELECT {PAGE_COLUMNS} FROM pages WHERE uuid = ?1"),
                [page_uuid],
                row_to_page,
            )?)
        })
        .await
    }

    pub async fn rename_page_if_revision(
        &self,
        uuid: uuid::Uuid,
        title: Option<String>,
        expected_revision: ContentRevision,
    ) -> Result<crate::store::Committed<Page>> {
        self.rename_page_with_expected_revision(uuid, title, Some(expected_revision))
            .await
    }

    // The legacy unguarded API retains its existing title-collision semantics.
    // Interactive edits use the revision-safe entry point above.
    async fn rename_page_with_expected_revision(
        &self,
        uuid: uuid::Uuid,
        title: Option<String>,
        expected_revision: Option<ContentRevision>,
    ) -> Result<crate::store::Committed<Page>> {
        let title = title
            .map(|title| title.trim().to_owned())
            .filter(|title| !title.is_empty());
        self.write(move |write| -> crate::CoreResult<Page> {
            let sql = format!("SELECT {PAGE_COLUMNS} FROM pages WHERE uuid = ?1");
            let page = write
                .sql()
                .query_row(&sql, [uuid], row_to_page)
                .optional()?
                .ok_or_else(|| crate::CoreError::not_found("page not found"))?;
            if page.kind.is_journal() {
                return Err(crate::CoreError::invalid(
                    "journal page titles are derived from their date",
                ));
            }
            // Retrying the same title is idempotent even with an older revision.
            if page.title == title {
                return Ok(page);
            }
            if let Some(expected_revision) = expected_revision {
                if page.title_revision != expected_revision {
                    return Err(crate::CoreError::conflict(
                        "page title changed since editing began",
                    ));
                }
                if let Some(title) = title.as_deref() {
                    let normalized_title = crate::model::normalize_title(title);
                    if operation::resolve_page_alias(write.sql(), &normalized_title)?
                        .is_some_and(|owner_uuid| owner_uuid != uuid)
                    {
                        return Err(crate::CoreError::conflict(
                            "another page already owns this title",
                        ));
                    }
                }
            }
            write.apply_local(
                vec![OpKind::PageSetTitle(PageSetTitle { uuid, title })],
                operation::ProjectionPolicy::Immediate,
            )?;
            Ok(write.sql().query_row(&sql, [uuid], row_to_page)?)
        })
        .await
    }

    pub async fn set_page_layout(
        &self,
        uuid: uuid::Uuid,
        layout: PageLayout,
    ) -> Result<crate::store::Committed<Page>> {
        self.write(move |write| -> crate::CoreResult<Page> {
            let sql = format!("SELECT {PAGE_COLUMNS} FROM pages WHERE uuid = ?1");
            let page = write
                .sql()
                .query_row(&sql, [uuid], row_to_page)
                .optional()?
                .ok_or_else(|| crate::CoreError::not_found("page not found"))?;
            if page.layout == layout {
                return Ok(page);
            }
            write.apply_local(
                vec![OpKind::PageSetLayout(PageSetLayout { uuid, layout })],
                operation::ProjectionPolicy::Immediate,
            )?;
            Ok(write.sql().query_row(&sql, [uuid], row_to_page)?)
        })
        .await
    }
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CreatedNote {
    pub page: Page,
    pub initial_block: Block,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum CreateNoteResult {
    Created { note: CreatedNote },
    Existing { page: Page },
}

impl CreateNoteResult {
    pub fn into_created(self) -> Option<CreatedNote> {
        match self {
            Self::Created { note } => Some(note),
            Self::Existing { .. } => None,
        }
    }
}

pub async fn create_note(conn: &Connection, title: Option<String>) -> Result<CreateNoteResult> {
    Ok(create_note_with_ops(conn, title).await?.value)
}

#[doc(hidden)]
pub async fn create_note_with_ops(
    conn: &Connection,
    title: Option<String>,
) -> Result<AppliedMutation<CreateNoteResult>> {
    let committed = crate::store::NotesStore::new(conn.clone())
        .create_note(title)
        .await?;
    Ok(AppliedMutation {
        value: committed.value,
        operations: committed.operations,
    })
}

impl crate::store::NotesStore {
    pub async fn create_note(
        &self,
        title: Option<String>,
    ) -> Result<crate::store::Committed<CreateNoteResult>> {
        let title = title
            .map(|title| title.trim().to_owned())
            .filter(|title| !title.is_empty());
        self.write(move |write| -> crate::CoreResult<CreateNoteResult> {
            if let Some(normalized_title) = title.as_deref().map(crate::model::normalize_title)
                && let Some(page) = page_by_normalized_title(write.sql(), &normalized_title)?
            {
                return Ok(CreateNoteResult::Existing { page });
            }

            let page_uuid = uuid::Uuid::now_v7();
            let block_uuid = uuid::Uuid::now_v7();
            let now = chrono::Utc::now().timestamp();
            let operations = vec![
                OpKind::PageCreate(PageCreate {
                    uuid: page_uuid,
                    kind: PageKind::Note,
                    title,
                    layout: PageLayout::Outline,
                    created_at: now,
                }),
                OpKind::BlockCreate(BlockCreate {
                    uuid: block_uuid,
                    page_uuid,
                    parent_uuid: None,
                    order_key: OrderKey::first(),
                    style: BlockStyle::Paragraph,
                    markdown: String::new(),
                    created_at: now,
                }),
            ];
            write.apply_action(
                "create note",
                operations,
                operation::ProjectionPolicy::Immediate,
            )?;
            let page = write
                .sql()
                .query_row(
                    &format!("SELECT {PAGE_COLUMNS} FROM pages WHERE uuid = ?1"),
                    [page_uuid],
                    row_to_page,
                )
                .optional()?
                .ok_or_else(|| crate::CoreError::not_found("created page disappeared"))?;
            let initial_block = write
                .sql()
                .query_row(
                    &format!("SELECT {BLOCK_COLUMNS} FROM blocks WHERE uuid = ?1"),
                    [block_uuid],
                    row_to_block,
                )
                .optional()?
                .ok_or_else(|| crate::CoreError::not_found("created initial block disappeared"))?;
            Ok(CreateNoteResult::Created {
                note: CreatedNote {
                    page,
                    initial_block,
                },
            })
        })
        .await
    }
}

#[derive(Debug)]
pub struct DeletedPage {
    pub page_uuid: uuid::Uuid,
    pub block_uuids: Vec<uuid::Uuid>,
    pub attachments: Vec<Attachment>,
    pub operations: Vec<OpKind>,
}

pub async fn delete_page(conn: &Connection, uuid: uuid::Uuid) -> Result<Option<DeletedPage>> {
    Ok(crate::store::NotesStore::new(conn.clone())
        .delete_page(uuid)
        .await?
        .value)
}

impl crate::store::NotesStore {
    pub async fn delete_page(
        &self,
        uuid: uuid::Uuid,
    ) -> Result<crate::store::Committed<Option<DeletedPage>>> {
        self.write(move |write| -> crate::CoreResult<Option<DeletedPage>> {
            let transaction = write.sql();
            let page_exists = transaction
                .query_row("SELECT 1 FROM pages WHERE uuid = ?1", [uuid], |_| Ok(()))
                .optional()?
                .is_some();
            if !page_exists {
                return Ok(None);
            }
            let blocks = {
                let sql = format!(
                    "WITH RECURSIVE tree(uuid, depth) AS (
                   SELECT uuid, 0 FROM blocks
                    WHERE page_uuid = ?1 AND parent_uuid IS NULL
                   UNION ALL
                   SELECT child.uuid, tree.depth + 1
                     FROM tree JOIN blocks child ON child.parent_uuid = tree.uuid
                 )
                 SELECT {QUALIFIED_BLOCK_COLUMNS}
                   FROM tree JOIN blocks ON blocks.uuid = tree.uuid
                  ORDER BY tree.depth DESC, blocks.order_key, blocks.uuid"
                );
                let mut statement = transaction.prepare(&sql)?;
                statement
                    .query_map([uuid], row_to_block)?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            let attachments = {
                let sql = format!(
                    "SELECT {} FROM attachments
                  WHERE page_uuid = ?1
                     OR block_uuid IN (SELECT uuid FROM blocks WHERE page_uuid = ?1)
                  ORDER BY created_at, uuid",
                    super::attachments::ATTACHMENT_COLUMNS
                );
                let mut statement = transaction.prepare(&sql)?;
                statement
                    .query_map([uuid], super::attachments::row_to_attachment)?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            let mut kinds = attachments
                .iter()
                .map(|attachment| {
                    OpKind::AttachmentRemove(crate::operation::AttachmentRemove {
                        owner: attachment.owner,
                        blob_hash: attachment.blob_hash,
                    })
                })
                .collect::<Vec<_>>();
            kinds.extend(blocks.iter().map(|block| {
                OpKind::BlockDelete(BlockDelete {
                    uuid: block.uuid,
                    page_uuid: block.page_uuid,
                })
            }));
            kinds.push(OpKind::PageDelete(PageDelete { uuid }));
            write.apply_action(
                "delete page",
                kinds.clone(),
                operation::ProjectionPolicy::Immediate,
            )?;
            let deleted = DeletedPage {
                page_uuid: uuid,
                block_uuids: blocks.into_iter().map(|block| block.uuid).collect(),
                attachments,
                operations: kinds,
            };
            Ok(Some(deleted))
        })
        .await
    }
}

pub async fn get_containing_page(
    conn: &Connection,
    block_uuid: uuid::Uuid,
) -> Result<Option<Page>> {
    conn.call(move |database| {
        let sql = format!(
            "SELECT {PAGE_COLUMNS} FROM pages
              WHERE uuid = (SELECT page_uuid FROM blocks WHERE uuid = ?1)"
        );
        database
            .query_row(&sql, [block_uuid], row_to_page)
            .optional()
    })
    .await
}

/// Create an ordinary note whose body is immutable typed handwriting, without a text block.
pub async fn create_handwritten_note_with_ops(
    conn: &Connection,
    title: Option<String>,
) -> Result<AppliedMutation<Page>> {
    let committed = crate::store::NotesStore::new(conn.clone())
        .create_handwritten_note(title)
        .await?;
    Ok(AppliedMutation {
        value: committed.value,
        operations: committed.operations,
    })
}

impl crate::store::NotesStore {
    pub async fn create_handwritten_note(
        &self,
        title: Option<String>,
    ) -> Result<crate::store::Committed<Page>> {
        self.write(move |write| -> crate::CoreResult<Page> {
            let uuid = uuid::Uuid::now_v7();
            write.apply_local(
                vec![OpKind::PageCreate(PageCreate {
                    uuid,
                    kind: PageKind::Handwriting,
                    title: title.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()),
                    layout: PageLayout::Outline,
                    created_at: chrono::Utc::now().timestamp(),
                })],
                operation::ProjectionPolicy::Immediate,
            )?;
            Ok(write.sql().query_row(
                &format!("SELECT {PAGE_COLUMNS} FROM pages WHERE uuid = ?1"),
                [uuid],
                row_to_page,
            )?)
        })
        .await
    }
}
