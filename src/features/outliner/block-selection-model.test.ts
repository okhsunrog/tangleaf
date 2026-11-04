import { describe, expect, it } from "vite-plus/test";
import type { Block, PageDocumentSnapshot } from "@/lib/api";
import {
  blockRange,
  documentWithoutBlocks,
  expandBlockSelection,
  nextBlockAfterDeletion,
  selectedBlockMarkdown,
} from "./block-selection-model";

const block = (uuid: string, parentUuid: string | null = null, orderKey = uuid): Block => ({
  uuid,
  parentUuid,
  orderKey,
  pageUuid: "page",
  markdown: uuid,
  markdownRevision: "r",
  style: { kind: "paragraph" },
  createdAt: 0,
  updatedAt: 0,
});
const blocks = [block("a"), block("b", "a"), block("c", "b"), block("d"), block("e", "d")];
const document: PageDocumentSnapshot = { pageUuid: "page", revision: "revision", blocks };

describe("outline block selection", () => {
  it("selects the same visible range in either drag direction", () => {
    expect(blockRange(["a", "b", "c", "d"], "c", "a")).toEqual(["a", "b", "c"]);
    expect(blockRange(["a", "b", "c", "d"], "a", "c")).toEqual(["a", "b", "c"]);
    expect(blockRange(["a", "b"], "missing", "b")).toEqual([]);
  });
  it("includes folded subtrees once when parent and child are both selected", () => {
    expect(expandBlockSelection(blocks, ["a", "b", "missing"])).toEqual(new Set(["a", "b", "c"]));
  });
  it("copies complete ordered subtrees and the latest unsaved text", () => {
    expect(
      selectedBlockMarkdown([...blocks].reverse(), ["a", "b", "d"], { b: { draft: "**Edited**" } }),
    ).toBe("- a\n\n  - **Edited**\n\n    - c\n\n- d\n\n  - e");
  });
  it("keeps task, heading and code semantics in the clipboard", () => {
    expect(
      selectedBlockMarkdown(
        [
          { ...block("a"), style: { kind: "heading_1" }, markdown: "Title" },
          { ...block("b", "a"), style: { kind: "task", state: "done" }, markdown: "Done" },
          { ...block("c", "a"), style: { kind: "code" }, markdown: "const x = 1;" },
        ],
        ["a"],
      ),
    ).toBe("- # Title\n\n  - [x] Done\n\n  - ```\n    const x = 1;\n    ```");
  });
  it("rebases a selected child to the clipboard root", () => {
    expect(selectedBlockMarkdown(blocks, ["b"])).toBe("- b\n\n  - c");
  });
  it("removes a subtree while preserving every surviving identity and parent", () => {
    expect(documentWithoutBlocks(document, ["a", "b"])).toEqual([
      { previousUuid: "d", parentIndex: null, style: { kind: "paragraph" }, markdown: "d" },
      { previousUuid: "e", parentIndex: 0, style: { kind: "paragraph" }, markdown: "e" },
    ]);
    expect(documentWithoutBlocks(document, ["b"]).map((unit) => unit.previousUuid)).toEqual([
      "a",
      "d",
      "e",
    ]);
  });
  it("leaves an editable empty block when everything is deleted", () => {
    expect(documentWithoutBlocks(document, ["a", "d"])).toEqual([
      { previousUuid: null, parentIndex: null, style: { kind: "paragraph" }, markdown: "" },
    ]);
  });
  it("focuses the next survivor, or the previous one at the end", () => {
    expect(nextBlockAfterDeletion(["a", "b", "c", "d"], new Set(["a", "b", "c"]))).toBe("d");
    expect(nextBlockAfterDeletion(["a", "b", "c", "d"], new Set(["c", "d"]))).toBe("b");
    expect(nextBlockAfterDeletion(["a"], new Set(["a"]))).toBeNull();
  });
});
