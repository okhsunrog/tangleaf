export type BlockAnchor = Readonly<{ uuid: string; offset: number }>;

type AnchorNode = {
  type: string;
  tagName?: string;
  properties?: Record<string, unknown>;
  position?: { start: { offset?: number }; end: { offset?: number } };
  children?: AnchorNode[];
};

const BLOCK_TAGS = new Set([
  "p",
  "li",
  "blockquote",
  "pre",
  "table",
  "hr",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
]);

/** Attach identities to rendered blocks without inserting markers into the user's Markdown. */
export function rehypeBlockAnchors(anchors: readonly BlockAnchor[]) {
  return (tree: AnchorNode) => {
    const candidates = new Map<string, AnchorNode>();
    const visit = (node: AnchorNode) => {
      const from = node.position?.start.offset;
      const to = node.position?.end.offset;
      if (
        node.type === "element" &&
        BLOCK_TAGS.has(node.tagName ?? "") &&
        from !== undefined &&
        to !== undefined
      ) {
        for (const anchor of anchors) {
          if (from <= anchor.offset && anchor.offset <= to) candidates.set(anchor.uuid, node);
        }
      }
      for (const child of node.children ?? []) visit(child);
    };
    visit(tree);
    for (const [uuid, node] of candidates) {
      node.properties ??= {};
      node.properties["data-block-uuid"] = uuid;
    }
  };
}
