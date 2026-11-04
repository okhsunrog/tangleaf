import type { Block } from "@/lib/api";

export type FlatBlock = { block: Block; depth: number; ordinal: number };

export function flattenBlocks(
  blocks: readonly Block[],
  isCollapsed: (uuid: string) => boolean,
): FlatBlock[] {
  const children = new Map<string, Block[]>();
  for (const block of blocks) {
    const key = block.parentUuid ?? block.pageUuid;
    const siblings = children.get(key) ?? [];
    siblings.push(block);
    children.set(key, siblings);
  }
  for (const siblings of children.values()) {
    siblings.sort((left, right) => left.orderKey.localeCompare(right.orderKey));
  }
  const rows: FlatBlock[] = [];
  const visited = new Set<string>();
  const append = (siblings: readonly Block[], depth: number) => {
    siblings.forEach((block, index) => {
      if (visited.has(block.uuid)) return;
      visited.add(block.uuid);
      rows.push({ block, depth, ordinal: index + 1 });
      if (!isCollapsed(block.uuid)) append(children.get(block.uuid) ?? [], depth + 1);
    });
  };
  const pageUuid = blocks[0]?.pageUuid;
  if (pageUuid) append(children.get(pageUuid) ?? [], 0);
  return rows;
}

export function groupChildren(blocks: readonly Block[], pageUuid: string): Map<string, Block[]> {
  const grouped = new Map<string, Block[]>();
  grouped.set(pageUuid, []);
  for (const block of blocks) {
    grouped.set(block.uuid, grouped.get(block.uuid) ?? []);
    const key = block.parentUuid ?? pageUuid;
    const siblings = grouped.get(key) ?? [];
    siblings.push(block);
    grouped.set(key, siblings);
  }
  for (const siblings of grouped.values()) {
    siblings.sort((left, right) => left.orderKey.localeCompare(right.orderKey));
  }
  return grouped;
}
