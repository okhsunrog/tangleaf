import { expect, it } from "vite-plus/test";
import type { Block, PageDocumentSnapshot } from "@/lib/api";
import { mergedBlockDocument, mergeSourceMatches } from "./block-merge-model";
const block = (uuid: string, parentUuid: string | null = null): Block => ({
  uuid,
  parentUuid,
  pageUuid: "page",
  orderKey: uuid,
  style: { kind: "paragraph" },
  markdown: uuid,
  markdownRevision: "r1",
  createdAt: 0,
  updatedAt: 0,
});
const document = (blocks: Block[]): PageDocumentSnapshot => ({
  pageUuid: "page",
  revision: "d1",
  blocks,
});

it("joins siblings and appends the source subtree after the target's existing children", () => {
  const units = mergedBlockDocument(
    document([
      block("a"),
      block("b", "a"),
      block("c"),
      block("d", "c"),
      block("e", "d"),
      block("f"),
    ]),
    "c",
    "a",
    "ac",
  );
  expect(units.map((unit) => [unit.previousUuid, unit.parentIndex, unit.markdown])).toEqual([
    ["a", null, "ac"],
    ["b", 0, "b"],
    ["d", 0, "d"],
    ["e", 2, "e"],
    ["f", null, "f"],
  ]);
});
it("merges the first child into its parent without changing surrounding siblings", () => {
  const units = mergedBlockDocument(
    document([block("a"), block("b", "a"), block("c", "b"), block("d", "a")]),
    "b",
    "a",
    "ab",
  );
  expect(units.map((unit) => [unit.previousUuid, unit.parentIndex])).toEqual([
    ["a", null],
    ["c", 0],
    ["d", 0],
  ]);
});
it("keeps the destination's style and places a cross-level merge in visible order", () => {
  const target = { ...block("b", "a"), style: { kind: "quote" as const } };
  const units = mergedBlockDocument(
    document([block("a"), target, block("c"), block("d", "c")]),
    "c",
    "b",
    "bc",
  );
  expect(units[1]).toMatchObject({
    previousUuid: "b",
    parentIndex: 0,
    style: { kind: "quote" },
    markdown: "bc",
  });
  expect(units[2]).toMatchObject({ previousUuid: "d", parentIndex: 1 });
});
it("rejects a missing or reversed merge target", () => {
  expect(() => mergedBlockDocument(document([block("a"), block("b")]), "a", "b", "ba")).toThrow();
  expect(() =>
    mergedBlockDocument(document([block("a"), block("b")]), "b", "missing", "ab"),
  ).toThrow();
});
it("accepts an acknowledged save of the same text but rejects a concurrent different edit", () => {
  expect(
    mergeSourceMatches(
      { ...block("a"), markdownRevision: "r2", markdown: "Draft" },
      block("a"),
      "Draft",
    ),
  ).toBe(true);
  expect(
    mergeSourceMatches(
      { ...block("a"), markdownRevision: "r2", markdown: "Remote" },
      block("a"),
      "Draft",
    ),
  ).toBe(false);
});
