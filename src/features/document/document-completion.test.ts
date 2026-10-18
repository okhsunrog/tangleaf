import { CompletionContext } from "@codemirror/autocomplete";
import { EditorState } from "@codemirror/state";
import { expect, it, vi } from "vite-plus/test";
import { searchPagesByTitle } from "@/lib/api";
import { completeNoteLink } from "./document-completion";
vi.mock("@/lib/api", () => ({ searchPagesByTitle: vi.fn(), searchBlocksFts: vi.fn() }));
it("offers page links and consumes an existing closing delimiter", async () => {
  vi.mocked(searchPagesByTitle).mockResolvedValue([
    {
      uuid: "page",
      title: "Rust notes",
      kind: { kind: "note" },
      layout: "outline",
      titleRevision: "r",
      createdAt: 0,
      updatedAt: 0,
    },
  ]);
  const context = new CompletionContext(EditorState.create({ doc: "See [[Ru]]" }), 8, false);
  const result = await completeNoteLink(context);
  expect(searchPagesByTitle).toHaveBeenCalledWith("Ru", 8);
  expect(result).toMatchObject({
    from: 4,
    to: 10,
    options: [{ label: "Rust notes", apply: "[[Rust notes]]" }],
  });
});
it("ignores ordinary text", async () => {
  const result = await completeNoteLink(
    new CompletionContext(EditorState.create({ doc: "Rust" }), 4, false),
  );
  expect(result).toBeNull();
});
