// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { EditorView } from "@codemirror/view";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import {
  getPageDocument,
  getPageRenderSnapshot,
  replacePageDocument,
  setBlockContent,
  type Block,
  type Page,
  type PageRenderSnapshot,
} from "@/lib/api";
import { notifyError } from "@/lib/notify";
import { queryKeys } from "@/lib/query";
import { PageSessionProvider, PageSessionRegistry } from "@/features/pages/page-session";
import { Outliner } from "./outliner";

vi.mock("@/lib/api", async (original) => ({
  ...(await original<typeof import("@/lib/api")>()),
  getPageDocument: vi.fn(),
  getPageRenderSnapshot: vi.fn(),
  replacePageDocument: vi.fn(),
  setBlockContent: vi.fn(),
}));
vi.mock("@/lib/notify", () => ({ notifyError: vi.fn(), notifyInfo: vi.fn() }));
vi.mock("@/features/markdown", () => ({
  useResolvedAttachmentImages: () => undefined,
  MarkdownRenderer: ({ markdown }: { markdown: string }) => <span>{markdown}</span>,
}));

const page: Page = {
  uuid: "page",
  kind: { kind: "note" },
  title: "Selection",
  layout: "outline",
  titleRevision: "t1",
  createdAt: 0,
  updatedAt: 0,
};
const block = (index: number, parentUuid: string | null = null): Block => ({
  uuid: `b${index}`,
  pageUuid: page.uuid,
  parentUuid,
  orderKey: String(index).padStart(4, "0"),
  markdown: `Text ${index}`,
  markdownRevision: "r1",
  style: { kind: "paragraph" },
  createdAt: 0,
  updatedAt: 0,
});
let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let client: QueryClient;
let sessions: PageSessionRegistry;
let snapshot: PageRenderSnapshot;
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const row = (index: number) =>
  container.querySelector<HTMLElement>(`[data-block-uuid="b${index}"]`)!;
const outline = () => container.querySelector<HTMLUListElement>('ul[aria-label="Note blocks"]')!;
function pointer(target: Element, type: string, y = 100, pointerType = "mouse") {
  const event = new MouseEvent(type, {
    button: 0,
    buttons: type === "pointerup" ? 0 : 1,
    clientX: 300,
    clientY: y,
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperties(event, { pointerId: { value: 1 }, pointerType: { value: pointerType } });
  target.dispatchEvent(event);
  return event;
}
async function drag(from: number, to: number, release = true) {
  await act(async () => pointer(row(from), "pointerdown"));
  await act(async () => pointer(row(to), "pointermove"));
  if (release) await act(async () => pointer(row(to), "pointerup"));
}
function key(value: string, isComposing = false) {
  const event = new KeyboardEvent("keydown", {
    key: value,
    isComposing,
    bubbles: true,
    cancelable: true,
  });
  outline().dispatchEvent(event);
  return event;
}
function copy(type = "copy") {
  const event = new Event(type, { bubbles: true, cancelable: true });
  const setData = vi.fn();
  Object.defineProperty(event, "clipboardData", { value: { setData } });
  outline().dispatchEvent(event);
  return { event, setData };
}
async function render(blocks = [block(0), block(1), block(2), block(3)], readOnly = false) {
  snapshot = { document: { pageUuid: page.uuid, revision: "d1", blocks }, images: [] };
  client.setQueryData(queryKeys.pageRender(page.uuid), snapshot);
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <PageSessionProvider registry={sessions}>
          <Outliner page={page} onOpenMarkdownLink={() => {}} readOnly={readOnly} />
        </PageSessionProvider>
      </QueryClientProvider>,
    ),
  );
}
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.clearAllMocks();
  localStorage.clear();
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
  Object.defineProperty(document, "elementFromPoint", {
    configurable: true,
    value: vi.fn(() => null),
  });
  container = document.createElement("div");
  container.setAttribute("data-workspace-scroll", "");
  Object.defineProperties(container, { clientHeight: { value: 240 }, clientWidth: { value: 800 } });
  container.getBoundingClientRect = () => new DOMRect(0, 0, 800, 240);
  container.scrollTo = vi.fn(({ top }: ScrollToOptions) => {
    container.scrollTop = top ?? 0;
  }) as typeof container.scrollTo;
  document.body.append(container);
  root = createRoot(container);
  client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity, retry: false } } });
  sessions = new PageSessionRegistry();
  vi.mocked(getPageRenderSnapshot).mockImplementation(async () => snapshot);
  vi.mocked(getPageDocument).mockImplementation(async () => snapshot.document);
  vi.mocked(replacePageDocument).mockImplementation(async (_uuid, _revision, units) => {
    const blocks = units.map((unit, index) => ({
      ...block(index),
      uuid: unit.previousUuid ?? "empty",
      markdown: unit.markdown,
      style: unit.style,
      parentUuid: unit.parentIndex === null ? null : units[unit.parentIndex].previousUuid,
      markdownRevision: "r2",
    }));
    snapshot = { ...snapshot, document: { ...snapshot.document, revision: "d2", blocks } };
    return snapshot.document;
  });
  vi.mocked(setBlockContent).mockImplementation(async (uuid, content) => {
    const updated = {
      ...snapshot.document.blocks.find((block) => block.uuid === uuid)!,
      markdown: content.markdown,
      markdownRevision: "r2",
    };
    snapshot = {
      ...snapshot,
      document: {
        ...snapshot.document,
        revision: "saved",
        blocks: snapshot.document.blocks.map((block) => (block.uuid === uuid ? updated : block)),
      },
    };
    return updated;
  });
});
afterEach(() => {
  act(() => root.unmount());
  client.clear();
  container.remove();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

it("promotes cross-row drags to whole blocks and suppresses the trailing edit click", async () => {
  await render();
  await drag(2, 0);
  expect(container.querySelectorAll('[data-block-selected="true"]')).toHaveLength(3);
  expect(document.activeElement).toBe(outline());
  await act(async () =>
    row(0)
      .querySelector(".cursor-text")!
      .dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true })),
  );
  expect(container.querySelector(".cm-editor")).toBeNull();
  const { event, setData } = copy();
  expect(event.defaultPrevented).toBe(true);
  expect(setData).toHaveBeenCalledWith("text/plain", "- Text 0\n\n- Text 1\n\n- Text 2");
  expect(setData).toHaveBeenCalledWith("text/markdown", "- Text 0\n\n- Text 1\n\n- Text 2");
});
it("leaves drags within one block and touch gestures native", async () => {
  await render();
  await drag(0, 0);
  expect(container.querySelector('[data-block-selected="true"]')).toBeNull();
  const { event, setData } = copy();
  expect(event.defaultPrevented).toBe(false);
  expect(setData).not.toHaveBeenCalled();
  await act(async () => {
    pointer(row(0), "pointerdown", 100, "touch");
    pointer(row(2), "pointermove", 100, "touch");
    pointer(row(2), "pointerup", 100, "touch");
  });
  expect(container.querySelector('[data-block-selected="true"]')).toBeNull();
});
it("ignores drags starting on block controls", async () => {
  await render([block(0), block(1, "b0"), block(2)]);
  await act(async () => pointer(row(0).querySelector("button")!, "pointerdown"));
  await act(async () => pointer(row(2), "pointermove"));
  expect(container.querySelector('[data-block-selected="true"]')).toBeNull();
});
it("retains the active editor and copies its unsaved text", async () => {
  await render();
  await act(async () => row(0).querySelector<HTMLElement>(".cursor-text")!.click());
  const editor = container.querySelector<HTMLElement>(".cm-editor")!;
  const view = EditorView.findFromDOM(editor)!;
  act(() =>
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: "Unsaved text" } }),
  );
  await act(async () => pointer(view.contentDOM, "pointerdown"));
  await act(async () => pointer(row(2), "pointermove"));
  await act(async () => pointer(row(2), "pointerup"));
  expect(container.querySelector(".cm-editor")).toBe(editor);
  expect(copy().setData).toHaveBeenCalledWith(
    "text/plain",
    "- Unsaved text\n\n- Text 1\n\n- Text 2",
  );
});
it.each(["Backspace", "Delete"])(
  "deletes the group with one guarded write on %s and focuses its next neighbor",
  async (shortcut) => {
    await render();
    await drag(0, 1);
    await act(async () => {
      key(shortcut);
      await tick();
      await tick();
    });
    expect(replacePageDocument).toHaveBeenCalledTimes(1);
    expect(replacePageDocument).toHaveBeenCalledWith(
      page.uuid,
      "d1",
      expect.arrayContaining([
        expect.objectContaining({ previousUuid: "b2", parentIndex: null }),
        expect.objectContaining({ previousUuid: "b3", parentIndex: null }),
      ]),
    );
    expect(row(0)).toBeNull();
    expect(row(1)).toBeNull();
    expect(
      container
        .querySelector(".cm-editor")
        ?.closest("[data-block-uuid]")
        ?.getAttribute("data-block-uuid"),
    ).toBe("b2");
  },
);
it("includes folded descendants without moving unrelated children", async () => {
  localStorage.setItem("outliner.collapsed.b0", "1");
  await render([block(0), block(1, "b0"), block(2, "b1"), block(3), block(4), block(5, "b4")]);
  await drag(0, 3);
  expect(copy().setData).toHaveBeenCalledWith(
    "text/plain",
    "- Text 0\n\n  - Text 1\n\n    - Text 2\n\n- Text 3",
  );
  await act(async () => {
    key("Delete");
    await tick();
    await tick();
  });
  expect(replacePageDocument).toHaveBeenCalledWith(page.uuid, "d1", [
    expect.objectContaining({ previousUuid: "b4", parentIndex: null }),
    expect.objectContaining({ previousUuid: "b5", parentIndex: 0 }),
  ]);
});
it("keeps an editable empty block when deleting the entire outline", async () => {
  await render([block(0), block(1)]);
  await drag(0, 1);
  await act(async () => {
    key("Delete");
    await tick();
    await tick();
  });
  expect(snapshot.document.blocks).toHaveLength(1);
  expect(
    container
      .querySelector(".cm-editor")
      ?.closest("[data-block-uuid]")
      ?.getAttribute("data-block-uuid"),
  ).toBe("empty");
});
it("keeps the selection and all blocks on a conflict", async () => {
  await render();
  vi.mocked(replacePageDocument).mockRejectedValue(new Error("Document changed"));
  await drag(0, 1);
  await act(async () => {
    key("Delete");
    await tick();
    await tick();
  });
  expect(container.querySelectorAll('[data-block-selected="true"]')).toHaveLength(2);
  expect(snapshot.document.blocks).toHaveLength(4);
  expect(notifyError).toHaveBeenCalled();
});
it("does not delete when drafts could not be saved", async () => {
  await render();
  sessions.registerFlush(async () => false);
  await drag(0, 1);
  await act(async () => {
    key("Delete");
    await tick();
    await tick();
  });
  expect(getPageDocument).not.toHaveBeenCalled();
  expect(replacePageDocument).not.toHaveBeenCalled();
  expect(container.querySelectorAll('[data-block-selected="true"]')).toHaveLength(2);
});
it("allows copying but protects reading pages and composition from deletion", async () => {
  await render(undefined, true);
  await drag(0, 1);
  expect(copy().setData).toHaveBeenCalled();
  await act(async () => key("Delete"));
  expect(replacePageDocument).not.toHaveBeenCalled();
  await render();
  await drag(0, 1);
  await act(async () => {
    expect(key("Backspace", true).defaultPrevented).toBe(false);
  });
  expect(replacePageDocument).not.toHaveBeenCalled();
});
it("clears block selection on Escape and on an outside click", async () => {
  await render();
  await drag(0, 1);
  await act(async () => key("Escape"));
  expect(container.querySelector('[data-block-selected="true"]')).toBeNull();
  await drag(0, 1);
  await act(async () => pointer(document.body, "pointerdown"));
  expect(container.querySelector('[data-block-selected="true"]')).toBeNull();
});

it("retains a logical range after its anchor leaves the virtual window", async () => {
  await render(
    Array.from({ length: 180 }, (_, index) => block(index)),
    true,
  );
  await drag(0, 3, false);
  await act(async () => {
    container.scrollTop = 4_200;
    container.dispatchEvent(new Event("scroll"));
    await tick();
  });
  expect(row(0)).toBeNull();
  const mounted = [...container.querySelectorAll<HTMLElement>("[data-block-uuid]")];
  expect(mounted.length).toBeLessThan(30);
  const last = mounted[mounted.length - 1];
  const end = Number(last.dataset.blockUuid!.slice(1));
  expect(end).toBeGreaterThan(80);
  await act(async () => pointer(last, "pointermove"));
  await act(async () => pointer(last, "pointerup"));
  const text = copy().setData.mock.calls.find(([type]) => type === "text/plain")?.[1];
  expect(text).toContain("- Text 0\n");
  expect(text).toContain("- Text 50\n");
  expect(text).toContain(`- Text ${end}`);
});

it("scrolls at the viewport edge and stops on pointer cancellation", async () => {
  await render(Array.from({ length: 12 }, (_, index) => block(index)));
  await drag(0, 3, false);
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => row(5) });
  await act(async () => {
    pointer(row(3), "pointermove", 239);
    await new Promise((resolve) => setTimeout(resolve, 40));
  });
  expect(container.scrollTop).toBeGreaterThan(0);
  await act(async () => pointer(row(5), "pointercancel"));
  const stoppedAt = container.scrollTop;
  await new Promise((resolve) => setTimeout(resolve, 40));
  expect(container.scrollTop).toBe(stoppedAt);
});

it("leaves another page's unsaved conflict alone when deleting this selection", async () => {
  await render();
  sessions.editBlock("other", "other-block", "Unsaved", { text: "Base", revision: "r" });
  const other = vi.fn(async () => false);
  sessions.registerFlush(other, "other");
  await drag(0, 1);
  await act(async () => {
    key("Delete");
    await tick();
    await tick();
  });
  expect(replacePageDocument).toHaveBeenCalledTimes(1);
  expect(other).not.toHaveBeenCalled();
  expect(sessions.getSnapshot("other").blocks["other-block"].draft).toBe("Unsaved");
});

async function backspaceAtStart(index: number) {
  await act(async () => row(index).querySelector<HTMLElement>(".cursor-text")!.click());
  const view = EditorView.findFromDOM(row(index).querySelector<HTMLElement>(".cm-editor")!)!;
  act(() => view.dispatch({ selection: { anchor: 0 } }));
  await act(async () => {
    view.contentDOM.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Backspace", bubbles: true, cancelable: true }),
    );
    await tick();
    await tick();
  });
  return view;
}

it("joins a nonempty block into the previous visible block at the exact text boundary", async () => {
  await render();
  await backspaceAtStart(2);
  expect(replacePageDocument).toHaveBeenCalledTimes(1);
  expect(snapshot.document.blocks.map((block) => block.markdown)).toEqual([
    "Text 0",
    "Text 1Text 2",
    "Text 3",
  ]);
  const view = EditorView.findFromDOM(row(1).querySelector<HTMLElement>(".cm-editor")!)!;
  expect(view.state.selection.main.head).toBe("Text 1".length);
  expect(view.hasFocus).toBe(true);
});

it("joins empty blocks and preserves children when merging across outline levels", async () => {
  await render([block(0), block(1, "b0"), { ...block(2), markdown: "" }, block(3, "b2")]);
  await backspaceAtStart(2);
  expect(snapshot.document.blocks.map((block) => [block.uuid, block.parentUuid])).toEqual([
    ["b0", null],
    ["b1", "b0"],
    ["b3", "b1"],
  ]);
  expect(snapshot.document.blocks[1].markdown).toBe("Text 1");
});

it("uses a folded previous parent as the target while preserving its hidden descendants", async () => {
  localStorage.setItem("outliner.collapsed.b0", "1");
  await render([block(0), block(1, "b0"), block(2)]);
  await backspaceAtStart(2);
  expect(
    snapshot.document.blocks.map((block) => [block.uuid, block.parentUuid, block.markdown]),
  ).toEqual([
    ["b0", null, "Text 0Text 2"],
    ["b1", "b0", "Text 1"],
  ]);
});

it("does nothing at the first block boundary", async () => {
  await render();
  await backspaceAtStart(0);
  expect(getPageDocument).not.toHaveBeenCalled();
  expect(replacePageDocument).not.toHaveBeenCalled();
});

it("retains typing at the join while the atomic merge is in flight", async () => {
  await render();
  const apply = vi.mocked(replacePageDocument).getMockImplementation()!;
  let finish!: () => void;
  vi.mocked(replacePageDocument).mockImplementation(async (...args) => {
    const document = await apply(...args);
    return new Promise((resolve) => {
      finish = () => resolve(document);
    });
  });
  const view = await backspaceAtStart(2);
  expect(view.state.doc.toString()).toBe("Text 1Text 2");
  act(() =>
    view.dispatch({
      changes: { from: "Text 1".length, insert: "X" },
      selection: { anchor: "Text 1X".length },
    }),
  );
  await act(async () => {
    finish();
    await tick();
    await tick();
  });
  expect(setBlockContent).toHaveBeenCalledWith("b1", { markdown: "Text 1XText 2" }, "r2");
  const targetView = EditorView.findFromDOM(row(1).querySelector<HTMLElement>(".cm-editor")!)!;
  expect(targetView.state.doc.toString()).toBe("Text 1XText 2");
  expect(targetView.state.selection.main.head).toBe("Text 1X".length);
});

it("restores the current block and all typing if the merge fails", async () => {
  await render();
  let fail!: () => void;
  vi.mocked(replacePageDocument).mockImplementation(
    () =>
      new Promise((_resolve, reject) => {
        fail = () => reject(new Error("offline"));
      }),
  );
  const view = await backspaceAtStart(2);
  act(() => view.dispatch({ changes: { from: "Text 1".length, insert: "X" } }));
  await act(async () => {
    fail();
    await tick();
    await tick();
  });
  expect(view.state.doc.toString()).toBe("XText 2");
  expect(snapshot.document.blocks).toHaveLength(4);
  expect(sessions.getSnapshot(page.uuid).blocks.b2.draft).toBe("XText 2");
  expect(notifyError).toHaveBeenCalled();
});
