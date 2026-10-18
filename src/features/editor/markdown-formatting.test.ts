// @vitest-environment jsdom
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { history, undo } from "@codemirror/commands";
import { afterEach, expect, it } from "vite-plus/test";
import { formatMarkdown, markdownFormatting } from "./markdown-formatting";
const views: EditorView[] = [];
afterEach(() => views.splice(0).forEach((view) => view.destroy()));
function editor(text = "hello", readOnly = false) {
  const view = new EditorView({
    state: EditorState.create({
      doc: text,
      selection: { anchor: 0, head: text.length },
      extensions: [history(), markdownFormatting(), EditorState.readOnly.of(readOnly)],
    }),
  });
  views.push(view);
  return view;
}
it("toggles bold around the selection and supports undo", () => {
  const view = editor();
  formatMarkdown(view, "bold");
  expect(view.state.doc.toString()).toBe("**hello**");
  expect(view.state.sliceDoc(view.state.selection.main.from, view.state.selection.main.to)).toBe(
    "hello",
  );
  formatMarkdown(view, "bold");
  expect(view.state.doc.toString()).toBe("hello");
  undo(view);
  expect(view.state.doc.toString()).toBe("**hello**");
});
it("selects the destination when inserting a link", () => {
  const view = editor();
  formatMarkdown(view, "link");
  expect(view.state.doc.toString()).toBe("[hello](https://)");
  expect(view.state.sliceDoc(view.state.selection.main.from, view.state.selection.main.to)).toBe(
    "https://",
  );
});
it("does not change read-only content", () => {
  const view = editor("hello", true);
  expect(formatMarkdown(view, "italic")).toBe(false);
  expect(view.state.doc.toString()).toBe("hello");
});
