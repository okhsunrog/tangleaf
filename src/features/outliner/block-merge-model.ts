import type { Block, DocumentUnitDraft, PageDocumentSnapshot } from "@/lib/api";
import { flattenBlocks } from "./outliner-tree";

/** Merge into the previous visible row; children retain their order and identity. */
export function mergedBlockDocument(
  document: PageDocumentSnapshot,
  sourceUuid: string,
  targetUuid: string,
  markdown: string,
): DocumentUnitDraft[] {
  const rows = flattenBlocks(document.blocks, () => false);
  const source = rows.findIndex(({ block }) => block.uuid === sourceUuid);
  const target = rows.findIndex(({ block }) => block.uuid === targetUuid);
  if (target < 0 || source <= target)
    throw new Error("The merge target is no longer before this block");
  const indices = new Map<string, number>();
  const units: DocumentUnitDraft[] = [];
  for (const { block } of rows) {
    if (block.uuid === sourceUuid) continue;
    const parentUuid = block.parentUuid === sourceUuid ? targetUuid : block.parentUuid;
    const parentIndex = parentUuid ? indices.get(parentUuid) : null;
    if (parentIndex === undefined) throw new Error("The merge would leave an orphaned block");
    indices.set(block.uuid, units.length);
    units.push({
      previousUuid: block.uuid,
      parentIndex,
      style: block.style,
      markdown: block.uuid === targetUuid ? markdown : block.markdown,
    });
  }
  return units;
}

/** A concurrent save of these exact words is safe; a different edit is not. */
export function mergeSourceMatches(current: Block, expected: Block, draft: string): boolean {
  return current.markdownRevision === expected.markdownRevision || current.markdown === draft;
}
