// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { EditorView } from "@codemirror/view";
import { undo } from "@codemirror/commands";
import { expect, it, vi } from "vite-plus/test";
import { PageSessionProvider, PageSessionRegistry } from "@/features/pages/page-session";
import { createInitialWorkspaceState } from "@/features/workspace/workspace-model";
import { queryKeys } from "@/lib/query";
import {
  ensureJournal,
  getPageDocument,
  getJournal,
  replacePageDocument,
  type Page,
  type PageDocumentSnapshot,
} from "@/lib/api";
import { JournalEntry } from "./journal-entry";
vi.mock("@/features/workspace/workspace-controller", () => ({
  useWorkspaceController: () => ({ openMarkdownLink: () => {} }),
}));
vi.mock("@/lib/notify", () => ({ notifyError: vi.fn() }));
vi.mock("@/lib/api", async (original) => ({
  ...(await original<typeof import("@/lib/api")>()),
  ensureJournal: vi.fn(),
  getJournal: vi.fn(),
  getPageDocument: vi.fn(),
  replacePageDocument: vi.fn(),
}));
const page: Page = {
  uuid: "journal",
  kind: { kind: "journal", date: "2026-10-18" },
  title: null,
  layout: "document",
  titleRevision: "t1",
  createdAt: 0,
  updatedAt: 0,
};
const empty: PageDocumentSnapshot = { pageUuid: page.uuid, revision: "r1", blocks: [] };
function snapshot(text: string): PageDocumentSnapshot {
  return {
    pageUuid: page.uuid,
    revision: "r2",
    blocks: [
      {
        uuid: "block",
        pageUuid: page.uuid,
        parentUuid: null,
        orderKey: "a",
        style: { kind: "paragraph" },
        markdown: text,
        markdownRevision: "b1",
        createdAt: 0,
        updatedAt: 0,
      },
    ],
  };
}
async function setup() {
  vi.clearAllMocks();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
  vi.mocked(ensureJournal).mockResolvedValue(page);
  vi.mocked(getJournal).mockResolvedValue(page);
  vi.mocked(getPageDocument).mockResolvedValue(empty);
  vi.mocked(replacePageDocument).mockImplementation(async (_page, _revision, units) =>
    snapshot(units.map((unit) => unit.markdown).join("\n\n")),
  );
  const client = new QueryClient({
    defaultOptions: { queries: { staleTime: Infinity, retry: false } },
  });
  client.setQueryData(queryKeys.journal("2026-10-18"), null);
  const sessions = new PageSessionRegistry();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
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
  return {
    sessions,
    client,
    container,
    view: () => EditorView.findFromDOM(container.querySelector<HTMLElement>(".cm-editor")!)!,
    cleanup: async () => {
      await act(async () => root.unmount());
      client.clear();
      container.remove();
    },
  };
}
it("does not create an empty day and retains text, selection and Undo through its first save", async () => {
  const app = await setup();
  try {
    expect(ensureJournal).not.toHaveBeenCalled();
    act(() =>
      app
        .view()
        .dispatch({ changes: { from: 0, insert: "First thought" }, selection: { anchor: 5 } }),
    );
    await act(async () => app.sessions.flushAll());
    expect(ensureJournal).toHaveBeenCalledTimes(1);
    expect(app.view().state.doc.toString()).toBe("First thought");
    expect(app.view().state.selection.main.anchor).toBe(5);
    act(() => {
      undo(app.view());
    });
    expect(app.view().state.doc.toString()).toBe("");
  } finally {
    await app.cleanup();
  }
});
it("drains typing that arrives while journal creation is pending", async () => {
  const app = await setup();
  let release!: (value: Page) => void;
  vi.mocked(ensureJournal).mockImplementation(
    () =>
      new Promise((resolve) => {
        release = resolve;
      }),
  );
  try {
    act(() => app.view().dispatch({ changes: { from: 0, insert: "First" } }));
    let flushing!: Promise<boolean>;
    act(() => {
      flushing = app.sessions.flushAll();
    });
    act(() => app.view().dispatch({ changes: { from: 5, insert: " and second" } }));
    await act(async () => {
      release(page);
      await flushing;
    });
    expect(app.view().state.doc.toString()).toBe("First and second");
    expect(vi.mocked(replacePageDocument).mock.calls.slice(-1)[0]?.[2][0].markdown).toBe(
      "First and second",
    );
  } finally {
    await app.cleanup();
  }
});
it("keeps a failed first save available for retry", async () => {
  const app = await setup();
  vi.mocked(replacePageDocument).mockRejectedValueOnce(new Error("offline"));
  try {
    act(() => app.view().dispatch({ changes: { from: 0, insert: "Keep this" } }));
    await act(async () => app.sessions.flushAll());
    expect(app.view().state.doc.toString()).toBe("Keep this");
    expect(app.container.textContent).toContain("Retry");
    await act(async () => app.sessions.flushAll());
    expect(app.view().state.doc.toString()).toBe("Keep this");
    expect(app.sessions.getSnapshot("journal-draft:2026-10-18").document).toBeNull();
  } finally {
    await app.cleanup();
  }
});

it("offers conflict resolution without overwriting another writer's first entry", async () => {
  const app = await setup();
  const { CommandFailure } = await import("@/lib/api");
  vi.mocked(getPageDocument)
    .mockResolvedValueOnce(empty)
    .mockResolvedValueOnce(snapshot("Remote thought"));
  vi.mocked(replacePageDocument).mockRejectedValueOnce(
    new CommandFailure({ code: "conflict", message: "Revision changed" }),
  );
  try {
    act(() => app.view().dispatch({ changes: { from: 0, insert: "Local thought" } }));
    await act(async () => app.sessions.flushAll());
    expect(app.container.textContent).toContain("Use remote");
    expect(app.container.textContent).toContain("Keep mine");
    expect(app.view().state.doc.toString()).toBe("Local thought");
    expect(replacePageDocument).toHaveBeenCalledTimes(1);
  } finally {
    await app.cleanup();
  }
});

it("starts at the end of an existing journal and returns to an empty editor after deletion", async () => {
  const app = await setup();
  try {
    await act(async () => {
      app.client.setQueryData(queryKeys.pageDocument(page.uuid), snapshot("Existing thought"));
      app.client.setQueryData(queryKeys.journal("2026-10-18"), page);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(app.view().state.selection.main.anchor).toBe("Existing thought".length);
    await act(async () => {
      app.client.setQueryData(queryKeys.journal("2026-10-18"), null);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(app.view().state.doc.toString()).toBe("");
    expect(ensureJournal).not.toHaveBeenCalled();
  } finally {
    await app.cleanup();
  }
});
