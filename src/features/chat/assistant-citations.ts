import type { Root, RootContent } from "mdast";
import { unified } from "unified";
import remarkParse from "remark-parse";
import remarkGfm from "remark-gfm";
import { decodeString } from "micromark-util-decode-string";
import { blockTargetHref, pageIdTargetHref } from "@/features/markdown";
import type { ChatSource } from "@/lib/api";
import { scanNotesLinks } from "@/features/markdown/notes-link-scanner";

const parser = unified().use(remarkParse).use(remarkGfm);
const UUID = "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";
const OPAQUE = new Set([
  "code",
  "inlineCode",
  "html",
  "link",
  "linkReference",
  "image",
  "imageReference",
]);

export type CitationMarker = { uuid: string; from: number; to: number };
export type NumberedSource = ChatSource & { number: number; href: string };

/** Only literal markers in prose count; code, links, escapes and image labels stay untouched. */
export function assistantCitationMarkers(markdown: string): CitationMarker[] {
  const markers: CitationMarker[] = [];
  const notesLinks = scanNotesLinks(markdown).occurrences;
  const visit = (node: Root | RootContent): void => {
    if (OPAQUE.has(node.type)) return;
    if (node.type === "text") {
      const start = node.position?.start.offset;
      const end = node.position?.end.offset;
      if (start === undefined || end === undefined) return;
      const raw = markdown.slice(start, end);
      for (const match of raw.matchAll(new RegExp(`\\[@?(${UUID})\\]`, "gi"))) {
        const from = start + match.index;
        if (notesLinks.some((link) => from >= link.sourceRange.from && from < link.sourceRange.to))
          continue;
        let backslashes = 0;
        for (let index = match.index - 1; index >= 0 && raw[index] === "\\"; index--) backslashes++;
        if (backslashes % 2) continue;
        // Never reinterpret an entity-decoded or escaped lookalike as a citation.
        const prefix = decodeString(raw.slice(0, match.index));
        if (!node.value.startsWith(prefix + match[0])) continue;
        markers.push({
          uuid: match[1].toLowerCase(),
          from: start + match.index,
          to: start + match.index + match[0].length,
        });
      }
    } else if ("children" in node) {
      for (const child of node.children) visit(child as RootContent);
    }
  };
  visit(parser.parse(markdown));
  return markers;
}

export function sourceHref(source: ChatSource): string {
  return source.target.kind === "page"
    ? pageIdTargetHref(source.target.uuid)
    : blockTargetHref(source.target.uuid);
}

export function prepareAssistantCitations(
  markdown: string,
  sources: readonly ChatSource[],
  markers = assistantCitationMarkers(markdown),
  pending = false,
): { markdown: string; citations: NumberedSource[] } {
  const available = new Map(sources.map((source) => [source.target.uuid.toLowerCase(), source]));
  const numbered = new Map<string, NumberedSource>();
  const replacements: { from: number; to: number; text: string }[] = [];
  for (const marker of markers) {
    const source = available.get(marker.uuid);
    if (!source) {
      replacements.push({
        ...marker,
        text: pending ? "\\[source…\\]" : "\\[source unavailable\\]",
      });
      continue;
    }
    let citation = numbered.get(marker.uuid);
    if (!citation) {
      citation = { ...source, number: numbered.size + 1, href: sourceHref(source) };
      numbered.set(marker.uuid, citation);
    }
    const title = `${citation.title}${citation.snippet ? `: ${citation.snippet}` : ""}`
      .replace(/\\/g, "\\\\")
      .replace(/"/g, '\\"')
      .replace(/\s+/g, " ");
    replacements.push({
      ...marker,
      text: `[\\[${citation.number}\\]](${citation.href} "${title}")`,
    });
  }
  let prepared = markdown;
  for (const { from, to, text } of replacements.reverse()) {
    prepared = prepared.slice(0, from) + text + prepared.slice(to);
  }
  return { markdown: prepared, citations: [...numbered.values()] };
}
