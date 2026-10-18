// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { undo } from "@codemirror/commands";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { afterEach, beforeAll, expect, it, vi } from "vite-plus/test";
import { BlockEdit } from "./block-edit";

beforeAll(() => {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
  (
    globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
});
const cleanup: (() => void)[] = [];
afterEach(() => {
  cleanup.splice(0).forEach((fn) => fn());
  vi.restoreAllMocks();
});

function mount() {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  cleanup.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const view = () => EditorView.findFromDOM(container.querySelector<HTMLElement>(".cm-editor")!)!;
  return { root, view };
}

it("retains undo after leaving a block and binds restored state to the current callbacks", async () => {
  const { root, view } = mount();
  const cache = new Map<string, EditorState>();
  const oldChange = vi.fn();
  const newChange = vi.fn();
  await act(async () =>
    root.render(
      <BlockEdit
        blockUuid="a"
        stateCache={cache}
        initial="First"
        onChange={oldChange}
        onBlur={() => {}}
      />,
    ),
  );
  act(() => view().dispatch({ changes: { from: 5, insert: " addition" } }));
  await act(async () => root.render(null));
  oldChange.mockClear();
  await act(async () =>
    root.render(
      <BlockEdit
        blockUuid="a"
        stateCache={cache}
        initial="First addition"
        onChange={newChange}
        onBlur={() => {}}
      />,
    ),
  );
  act(() => {
    undo(view());
  });
  expect(view().state.doc.toString()).toBe("First");
  expect(newChange).toHaveBeenCalledWith("First", expect.any(Number));
  expect(oldChange).not.toHaveBeenCalled();
});

it("does not restore stale history over a remotely changed block", async () => {
  const { root, view } = mount();
  const cache = new Map<string, EditorState>();
  const render = (initial: string) => (
    <BlockEdit
      blockUuid="a"
      stateCache={cache}
      initial={initial}
      onChange={() => {}}
      onBlur={() => {}}
    />
  );
  await act(async () => root.render(render("First")));
  act(() => view().dispatch({ changes: { from: 5, insert: " addition" } }));
  await act(async () => root.render(null));
  await act(async () => root.render(render("Remote text")));
  act(() => {
    undo(view());
  });
  expect(view().state.doc.toString()).toBe("Remote text");
});

it("uses the activation click for caret placement instead of jumping to the end", async () => {
  const { root, view } = mount();
  const position = vi.spyOn(EditorView.prototype, "posAtCoords").mockReturnValue(2);
  await act(async () =>
    root.render(
      <BlockEdit
        initial="First paragraph"
        initialPointer={{ x: 42, y: 80 }}
        autoFocus
        onChange={() => {}}
        onBlur={() => {}}
      />,
    ),
  );
  expect(position).toHaveBeenCalledWith({ x: 42, y: 80 });
  expect(view().state.selection.main.anchor).toBe(2);
});
