//! Citation metadata comes only from typed, successful tool outputs, never model text.
use std::collections::{HashMap, HashSet};

use notes_core::{
    Connection, PageKind,
    db::{self, Content, Page, SearchHit},
};
use notes_protocol::{ChatSource, ChatSourceTarget};
use rig::message::ToolResult;

#[derive(Default)]
pub(crate) struct SourceRegistry {
    seen: HashSet<uuid::Uuid>,
    sources: Vec<ChatSource>,
}

impl SourceRegistry {
    pub(crate) async fn register(
        &mut self,
        notes: &Connection,
        result: &ToolResult,
    ) -> anyhow::Result<Option<Vec<ChatSource>>> {
        let candidates = tool_contents(result)
            .into_iter()
            .filter(|(content, _)| !self.seen.contains(&content.uuid()))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return Ok(None);
        }
        let page_ids = candidates
            .iter()
            .map(|(content, _)| content.page_uuid())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let pages = db::get_contents(notes, page_ids)
            .await?
            .into_iter()
            .filter_map(|content| match content {
                Content::Page(page) => Some((page.uuid, page)),
                Content::Block(_) => None,
            })
            .collect::<HashMap<_, _>>();
        let before = self.sources.len();
        for (content, snippet) in candidates {
            let Some(page) = pages.get(&content.page_uuid()) else {
                continue;
            };
            if !self.seen.insert(content.uuid()) {
                continue;
            }
            let target = match &content {
                Content::Page(page) => ChatSourceTarget::Page { uuid: page.uuid },
                Content::Block(block) => ChatSourceTarget::Block {
                    uuid: block.uuid,
                    page_uuid: block.page_uuid,
                },
            };
            let title = match &page.kind {
                PageKind::Journal { date } => date.to_string(),
                PageKind::Note | PageKind::Handwriting => {
                    page.title.clone().unwrap_or_else(|| "Untitled".into())
                }
            };
            self.sources.push(ChatSource {
                target,
                title,
                snippet: snippet
                    .as_deref()
                    .unwrap_or_else(|| content.text())
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(280)
                    .collect(),
            });
        }
        Ok((self.sources.len() != before).then(|| self.sources.clone()))
    }
}

fn tool_contents(result: &ToolResult) -> Vec<(Content, Option<String>)> {
    if result.is_error {
        return Vec::new();
    }
    let mut contents = Vec::new();
    for part in &result.content {
        // Explicit decoding also accepts historical Rig text-wrapped JSON results.
        match result.name.as_str() {
            "search_and_expand" | "search_agentic" => {
                if let Ok(hits) = part.deserialize_json::<Vec<SearchHit>>() {
                    contents.extend(hits.into_iter().map(|hit| (hit.content, hit.snippet)));
                }
            }
            "list_pages" => {
                if let Ok(pages) = part.deserialize_json::<Vec<Page>>() {
                    contents.extend(pages.into_iter().map(|page| (Content::Page(page), None)));
                }
            }
            "neighbors" | "find_backlinks" | "read_ancestors" | "read_subtree" => {
                if let Ok(records) = part.deserialize_json::<Vec<Content>>() {
                    contents.extend(records.into_iter().map(|content| (content, None)));
                }
            }
            "get_content" => {
                if let Ok(Some(content)) = part.deserialize_json::<Option<Content>>() {
                    contents.push((content, None));
                }
            }
            "create_page" => {
                if let Ok(page) = part.deserialize_json::<Page>() {
                    contents.push((Content::Page(page), None));
                }
            }
            _ => {}
        }
    }
    contents
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn result(name: &str, value: serde_json::Value) -> ToolResult {
        serde_json::from_value(json!({
            "call": {"provider": "call-1"}, "name": name,
            "content": [{"type": "json", "value": value}]
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn sources_keep_typed_targets_and_retrieved_snippets_without_duplicates() {
        let directory = tempfile::tempdir().unwrap();
        let notes = db::open(directory.path().join("notes.db")).await.unwrap();
        let page = db::create_page(&notes, "Charging board".into())
            .await
            .unwrap();
        let block = db::create_block(
            &notes,
            page.uuid,
            None,
            None,
            notes_core::BlockStyle::Paragraph,
            "Long original block".into(),
        )
        .await
        .unwrap();
        let hit = SearchHit {
            content: Content::Block(block.clone()),
            score: 1.0,
            snippet: Some("probably need for\ncharging esk8".into()),
        };
        let mut registry = SourceRegistry::default();
        let search = result("search_and_expand", json!([hit.clone(), hit]));
        let sources = registry.register(&notes, &search).await.unwrap().unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].title, "Charging board");
        assert_eq!(sources[0].snippet, "probably need for charging esk8");
        assert!(
            matches!(sources[0].target, ChatSourceTarget::Block { uuid, page_uuid }
            if uuid == block.uuid && page_uuid == page.uuid)
        );
        assert!(registry.register(&notes, &search).await.unwrap().is_none());

        let inventory = result("list_pages", json!([page]));
        let sources = registry
            .register(&notes, &inventory)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sources.len(), 2);
        assert!(matches!(sources[1].target, ChatSourceTarget::Page { uuid } if uuid == page.uuid));

        // A nested UUID in user text is never treated as another retrieved source.
        let mut failed = search.clone();
        failed.is_error = true;
        assert!(tool_contents(&failed).is_empty());
        assert!(tool_contents(&result("unrecognized_tool", json!([page]))).is_empty());
        assert!(tool_contents(&result("get_content", json!({"uuid": block.uuid}))).is_empty());
    }
}
