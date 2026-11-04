import type { Block, DocumentUnitDraft, PageDocumentSnapshot } from "@/lib/api";
import { DocumentCodec } from "@/features/document/document-codec";
import { flattenBlocks } from "./outliner-tree";

export function blockRange(
  visibleUuids: readonly string[],
  anchor: string,
  head: string,
): string[] {
  const from = visibleUuids.indexOf(anchor);
  const to = visibleUuids.indexOf(head);
  return from < 0 || to < 0 ? [] : visibleUuids.slice(Math.min(from, to), Math.max(from, to) + 1);
}

/** A selected parent owns its entire subtree, including folded descendants. */
export function expandBlockSelection(
  blocks: readonly Block[],
  selected: Iterable<string>,
): Set<string> {
  const children = new Map<string, string[]>();
  const existing = new Set(blocks.map((block) => block.uuid));
  for (const block of blocks) {
    if (!block.parentUuid) continue;
    const rows = children.get(block.parentUuid) ?? [];
    rows.push(block.uuid);
    children.set(block.parentUuid, rows);
  }
  const result = new Set<string>();
  const pending = [...selected];
  while (pending.length) {
    const uuid = pending.pop()!;
    if (!existing.has(uuid) || result.has(uuid)) continue;
    result.add(uuid);
    pending.push(...(children.get(uuid) ?? []));
  }
  return result;
}

/** Reuse the notes Markdown dialect, adding outline markers to non-list blocks. */
export function selectedBlockMarkdown(
  blocks: readonly Block[],
  selected: Iterable<string>,
  drafts: Readonly<Record<string, { draft: string }>> = {},
): string {
  const included = expandBlockSelection(blocks, selected);
  const projected = flattenBlocks(blocks, () => false)
    .filter(({ block }) => included.has(block.uuid))
    .map(({ block }) => {
      const markdown = drafts[block.uuid]?.draft ?? block.markdown;
      const parentUuid =
        block.parentUuid && included.has(block.parentUuid) ? block.parentUuid : null;
      if (["bullet", "numbered", "task"].includes(block.style.kind)) {
        return { ...block, parentUuid, markdown };
      }
      const content = DocumentCodec.encode([{ ...block, parentUuid: null, markdown }]).markdown;
      return { ...block, parentUuid, markdown: content, style: { kind: "bullet" as const } };
    });
  return DocumentCodec.encode(projected).markdown;
}

/** Preserve every surviving UUID, style and parent for the guarded atomic writer. */
export function documentWithoutBlocks(
  document: PageDocumentSnapshot,
  selected: Iterable<string>,
): DocumentUnitDraft[] {
  const removed = expandBlockSelection(document.blocks, selected);
  const indices = new Map<string, number>();
  const units: DocumentUnitDraft[] = [];
  for (const { block } of flattenBlocks(document.blocks, () => false)) {
    if (removed.has(block.uuid)) continue;
    const parentIndex = block.parentUuid ? indices.get(block.parentUuid) : null;
    if (parentIndex === undefined) throw new Error("A surviving block has no parent");
    indices.set(block.uuid, units.length);
    units.push({
      previousUuid: block.uuid,
      parentIndex,
      style: block.style,
      markdown: block.markdown,
    });
  }
  // Keep the outline usable by keyboard after deleting its final block.
  return units.length
    ? units
    : [{ previousUuid: null, parentIndex: null, style: { kind: "paragraph" }, markdown: "" }];
}

export function nextBlockAfterDeletion(
  visibleUuids: readonly string[],
  removed: ReadonlySet<string>,
): string | null {
  const selectedIndices = visibleUuids.flatMap((uuid, index) => (removed.has(uuid) ? [index] : []));
  if (!selectedIndices.length) return null;
  return (
    visibleUuids
      .slice(selectedIndices[selectedIndices.length - 1] + 1)
      .find((uuid) => !removed.has(uuid)) ??
    visibleUuids
      .slice(0, selectedIndices[0])
      .reverse()
      .find((uuid) => !removed.has(uuid)) ??
    null
  );
}
