// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { EditorView } from "@codemirror/view";
import { undo } from "@codemirror/commands";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { PageSessionProvider, PageSessionRegistry } from "@/features/pages/page-session";
import { createInitialWorkspaceState } from "@/features/workspace/workspace-model";
import { queryKeys } from "@/lib/query";
import {
  createBlock,
  ensureJournal,
  getPageDocument,
  getPageRenderSnapshot,
  getJournal,
  setBlockContent,
  splitBlock,
  replacePageDocument,
  type Block,
  type Page,
  type PageDocumentSnapshot,
} from "@/lib/api";
import { JournalEntry } from "./journal-entry";
vi.mock("@/features/workspace/workspace-controller", () => ({
  useWorkspaceController: () => ({ openMarkdownLink: () => {} }),
}));
vi.mock("@/features/markdown", () => ({
  useResolvedAttachmentImages: () => undefined,
  MarkdownRenderer: ({ markdown }: { markdown: string }) => <span>{markdown}</span>,
}));
vi.mock("@/lib/notify", () => ({ notifyError: vi.fn(), notifyInfo: vi.fn() }));
vi.mock("@/lib/api", async (original) => ({
  ...(await original<typeof import("@/lib/api")>()),
  createBlock: vi.fn(),
  ensureJournal: vi.fn(),
  getJournal: vi.fn(),
  getPageDocument: vi.fn(),
  getPageRenderSnapshot: vi.fn(),
  setBlockContent: vi.fn(),
  splitBlock: vi.fn(),
  replacePageDocument: vi.fn(),
}));
const page: Page = {
  uuid: "journal",
  kind: { kind: "journal", date: "2026-10-18" },
  title: null,
  layout: "outline",
  titleRevision: "t1",
  createdAt: 0,
  updatedAt: 0,
};
const block = (uuid: string, text: string): Block => ({
  uuid,
  pageUuid: page.uuid,
  parentUuid: null,
  orderKey: uuid,
  style: { kind: "paragraph" },
  markdown: text,
  markdownRevision: "b1",
  createdAt: 0,
  updatedAt: 0,
});
let snapshot: PageDocumentSnapshot;
let client: QueryClient;
let sessions: PageSessionRegistry;
let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
async function editorReady() {
  await vi.waitFor(async () => {
    await act(tick);
    expect(container.querySelector(".cm-editor")).not.toBeNull();
  });
}
const view = () => EditorView.findFromDOM(container.querySelector<HTMLElement>(".cm-editor")!)!;
function key(key: string, shiftKey = false) {
  view().contentDOM.dispatchEvent(
    new KeyboardEvent("keydown", { key, shiftKey, bubbles: true, cancelable: true }),
  );
}
async function render(value: Page | null = page) {
  client.setQueryData(queryKeys.journal("2026-10-18"), value);
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <PageSessionProvider registry={sessions}>
          <JournalEntry
            date="2026-10-18"
            paneId={createInitialWorkspaceState().primaryPaneId}
            autoFocus
          />
        </PageSessionProvider>
      </QueryClientProvider>,
    ),
  );
  await act(tick);
}
beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
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
  HTMLElement.prototype.scrollIntoView = vi.fn();
  snapshot = { pageUuid: page.uuid, revision: "r1", blocks: [block("first", "Existing thought")] };
  vi.mocked(ensureJournal).mockResolvedValue(page);
  vi.mocked(getJournal).mockResolvedValue(page);
  vi.mocked(getPageDocument).mockImplementation(async () => snapshot);
  vi.mocked(getPageRenderSnapshot).mockImplementation(async () => ({
    document: snapshot,
    images: [],
  }));
  vi.mocked(createBlock).mockImplementation(async () => {
    const next = block("created", "");
    snapshot = { ...snapshot, blocks: [...snapshot.blocks, next] };
    return next;
  });
  vi.mocked(setBlockContent).mockImplementation(async (uuid, content) => {
    const next = {
      ...snapshot.blocks.find((block) => block.uuid === uuid)!,
      markdown: content.markdown,
      markdownRevision: "b2",
    };
    snapshot = {
      ...snapshot,
      revision: "r2",
      blocks: snapshot.blocks.map((block) => (block.uuid === uuid ? next : block)),
    };
    return next;
  });
  vi.mocked(splitBlock).mockImplementation(async (uuid, parts) => {
    const previous = snapshot.blocks.find((block) => block.uuid === uuid)!;
    const next = parts.map((content, index) => ({
      ...previous,
      uuid: index ? "second" : uuid,
      orderKey: index ? "z" : previous.orderKey,
      markdown: content.markdown,
      markdownRevision: "split",
    }));
    snapshot = {
      ...snapshot,
      revision: "split",
      blocks: snapshot.blocks.flatMap((block) => (block.uuid === uuid ? next : [block])),
    };
    return next;
  });
  client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity, retry: false } } });
  sessions = new PageSessionRegistry();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  client.clear();
  container.remove();
  vi.unstubAllGlobals();
});
it("uses the persisted Outline layout and keeps an Enter-created empty block through autosave", async () => {
  await render();
  expect(container.querySelector('ul[aria-label="Note blocks"]')).not.toBeNull();
  expect(view().state.selection.main.head).toBe("Existing thought".length);
  await act(async () => {
    key("Enter");
    await tick();
  });
  expect(splitBlock).toHaveBeenCalledWith(
    "first",
    [{ markdown: "Existing thought" }, { markdown: "" }],
    "b1",
    "siblings",
  );
  expect(snapshot.blocks).toHaveLength(2);
  await vi.waitFor(async () => {
    await act(tick);
    expect(container.querySelector('[data-block-uuid="second"] .cm-content')).not.toBeNull();
  });
  expect(view().state.doc.toString()).toBe("");
  act(() =>
    view().dispatch({ changes: { from: 0, insert: "Second thought" }, selection: { anchor: 6 } }),
  );
  const activeEditor = view();
  await act(async () => {
    await sessions.flushAll();
    await tick();
  });
  expect(view()).toBe(activeEditor);
  expect(view().state.doc.toString()).toBe("Second thought");
  expect(view().state.selection.main.head).toBe(6);
  expect(snapshot.blocks.map((block) => block.markdown)).toEqual([
    "Existing thought",
    "Second thought",
  ]);
  expect(replacePageDocument).not.toHaveBeenCalled();
  expect(container.textContent).not.toContain("document saved");
});
it("keeps Shift+Enter inside one journal block instead of splitting it", async () => {
  await render();
  act(() => key("Enter", true));
  expect(view().state.doc.toString()).toBe("Existing thought\n");
  await act(async () => sessions.flushAll());
  expect(snapshot.blocks[0].markdown).toBe("Existing thought\n");
  expect(splitBlock).not.toHaveBeenCalled();
});
it("honors an explicitly selected Document layout", async () => {
  await render({ ...page, layout: "document" });
  expect(container.querySelector('ul[aria-label="Note blocks"]')).toBeNull();
  expect(container.querySelector('[aria-label="Document Markdown"]')).not.toBeNull();
  expect(view().state.selection.main.head).toBe("Existing thought".length);
});
it("does not persist an empty day until Start writing, then opens the ordinary block editor", async () => {
  snapshot = { ...snapshot, blocks: [] };
  await render(null);
  expect(ensureJournal).not.toHaveBeenCalled();
  expect(createBlock).not.toHaveBeenCalled();
  expect(container.querySelector(".cm-editor")).toBeNull();
  const start = container.querySelector<HTMLButtonElement>("button")!;
  await act(async () => {
    start.click();
    start.click();
    await tick();
  });
  expect(ensureJournal).toHaveBeenCalledTimes(1);
  expect(createBlock).toHaveBeenCalledTimes(1);
  await editorReady();
  expect(container.querySelector('[data-block-uuid="created"] .cm-content')).not.toBeNull();
  await act(async () => {
    key("Enter");
    await tick();
  });
  expect(snapshot.blocks).toHaveLength(2);
});
it("waits for initialization despite a background journal notification", async () => {
  snapshot = { ...snapshot, blocks: [] };
  let release!: (value: Page) => void;
  vi.mocked(ensureJournal).mockImplementation(
    () =>
      new Promise((resolve) => {
        release = resolve;
      }),
  );
  await render(null);
  act(() => container.querySelector<HTMLButtonElement>("button")!.click());
  await act(async () => {
    client.setQueryData(queryKeys.journal("2026-10-18"), page);
    await tick();
  });
  expect(container.querySelector(".cm-editor")).toBeNull();
  expect(container.textContent).toContain("Opening journal");
  await act(async () => {
    release(page);
    await tick();
  });
  await editorReady();
  expect(container.querySelector('[data-block-uuid="created"] .cm-content')).not.toBeNull();
});
it("retains the first block's selection and local Undo through its first save", async () => {
  snapshot = { ...snapshot, blocks: [] };
  await render(null);
  await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
  await editorReady();
  act(() =>
    view().dispatch({ changes: { from: 0, insert: "First thought" }, selection: { anchor: 5 } }),
  );
  const firstEditor = view();
  await act(async () => {
    await sessions.flushAll();
    await tick();
  });
  expect(view()).toBe(firstEditor);
  expect(view().state.selection.main.head).toBe(5);
  act(() => {
    undo(view());
  });
  expect(view().state.doc.toString()).toBe("");
});
it("reuses a concurrently created journal without appending a blank block", async () => {
  await render(null);
  await act(async () => {
    container.querySelector<HTMLButtonElement>("button")!.click();
    await tick();
  });
  expect(createBlock).not.toHaveBeenCalled();
  await editorReady();
  expect(view().state.doc.toString()).toBe("Existing thought");
});
it("keeps journal initialization failures retryable", async () => {
  snapshot = { ...snapshot, blocks: [] };
  vi.mocked(ensureJournal).mockRejectedValueOnce(new Error("offline"));
  await render(null);
  await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
  expect(container.querySelector('[role="alert"]')?.textContent).toContain("Could not start");
  await act(async () => {
    container.querySelector<HTMLButtonElement>("button")!.click();
    await tick();
  });
  await editorReady();
  expect(container.querySelector('[data-block-uuid="created"] .cm-content')).not.toBeNull();
});
