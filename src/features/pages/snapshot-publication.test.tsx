// @vitest-environment jsdom

import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vite-plus/test";
import {
  CommandFailure,
  getBlock,
  getPageDocument,
  getPageRenderSnapshot,
  replacePageDocument,
  setBlockContent,
  type Block,
  type PageDocumentSnapshot,
  type PageRenderSnapshot,
} from "@/lib/api";
import { applyDomainEvent, queryKeys } from "@/lib/query";
import { DocumentPage, pageDocumentToPersisted } from "@/features/document/document-page";
import { BlockNode } from "@/features/outliner/block-node";
import { OutlinerProvider } from "@/features/outliner/outliner-store";
import { PageSessionProvider, PageSessionRegistry } from "./page-session";

vi.mock("@/lib/api", async (original) => ({
  ...(await original<typeof import("@/lib/api")>()),
  getBlock: vi.fn(),
  getPageDocument: vi.fn(),
  getPageRenderSnapshot: vi.fn(),
  replacePageDocument: vi.fn(),
  setBlockContent: vi.fn(),
}));
vi.mock("@/features/markdown", () => ({
  MarkdownRenderer: ({ markdown }: { markdown: string }) => <span>{markdown}</span>,
  useAttachmentImageResolver: () => () => null,
}));
vi.mock("@/lib/notify", () => ({ notifyError: vi.fn(), notifyInfo: vi.fn() }));

type Mode = "block" | "document";
const cleanup: Array<() => void> = [];
const base = snapshot("Base", "1");

beforeAll(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
});
beforeEach(() => vi.resetAllMocks());
afterEach(() => {
  for (const dispose of cleanup.splice(0)) dispose();
  vi.restoreAllMocks();
});

function BlockPage() {
  const query = useQuery({
    queryKey: queryKeys.pageRender("page"),
    queryFn: () => getPageRenderSnapshot("page"),
  });
  const blocks = query.data?.document.blocks ?? [];
  return (
    <OutlinerProvider
      initialEditingUuid="block"
      initialEditingRequest={1}
      blocks={blocks}
      layout="outline"
      readOnly={false}
      onOpenMarkdownLink={() => undefined}
    >
      {blocks.map((block) => (
        <BlockNode key={block.uuid} block={block} depth={0} ordinal={1} />
      ))}
    </OutlinerProvider>
  );
}

async function editor(mode: Mode) {
  const client = new QueryClient({
    defaultOptions: { queries: { staleTime: Infinity, retry: false } },
  });
  const registry = new PageSessionRegistry();
  const blockSave = deferred<Block>();
  const documentSave = deferred<PageDocumentSnapshot>();
  const renderRead = deferred<PageRenderSnapshot | null>();
  const documentRead = deferred<PageDocumentSnapshot | null>();
  const key = mode === "block" ? queryKeys.pageRender("page") : queryKeys.pageDocument("page");
  client.setQueryData(key, mode === "block" ? { document: base, images: [] } : base);
  vi.mocked(setBlockContent).mockReturnValue(blockSave.promise);
  vi.mocked(replacePageDocument).mockReturnValue(documentSave.promise);
  vi.mocked(getPageRenderSnapshot).mockReturnValue(renderRead.promise);
  vi.mocked(getPageDocument).mockReturnValue(documentRead.promise);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  cleanup.push(() => {
    act(() => root.unmount());
    client.clear();
    container.remove();
  });
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <PageSessionProvider registry={registry}>
          {mode === "block" ? (
            <BlockPage />
          ) : (
            <DocumentPage
              pageUuid="page"
              editing
              canEdit
              authoringMode="live_preview"
              focusRequest={0}
              onOpenMarkdownLink={() => undefined}
            />
          )}
        </PageSessionProvider>
      </QueryClientProvider>,
    ),
  );
  const overlay = () => {
    const session = registry.getSnapshot("page");
    return mode === "block" ? session.blocks.block : session.document;
  };
  return {
    client,
    registry,
    container,
    overlay,
    cached: () =>
      mode === "block"
        ? client.getQueryData<PageRenderSnapshot>(key)?.document
        : client.getQueryData<PageDocumentSnapshot>(key),
    async save() {
      let saving!: Promise<void>;
      await act(async () => {
        if (mode === "block") {
          registry.editBlock("page", "block", "Saved draft", { text: "Base", revision: "b1" });
        } else {
          registry.editDocument("page", "Saved draft", pageDocumentToPersisted(base));
        }
        saving = registry.flushAll();
        await Promise.resolve();
      });
      return { done: saving };
    },
    acknowledge: () => {
      const saved = snapshot("Saved draft", "2");
      if (mode === "block") blockSave.resolve(saved.blocks[0]);
      else documentSave.resolve(saved);
    },
    rejectSave: () => {
      const error = new CommandFailure({ code: "conflict", message: "Newer content exists" });
      if (mode === "block") blockSave.reject(error);
      else documentSave.reject(error);
    },
    resolveRead: (markdown: string, revision: string) => {
      const document = snapshot(markdown, revision);
      if (mode === "block") renderRead.resolve({ document, images: [] });
      else documentRead.resolve(document);
    },
  };
}

describe.each(["block", "document"] as const)("%s editor snapshot publication", (mode) => {
  it.each([false, true])(
    "rejects a late save result after another commit (refresh completed: %s)",
    async (completeRefreshFirst) => {
      const mounted = await editor(mode);
      const saving = await mounted.save();
      let refreshing!: Promise<void>;
      await act(async () => {
        refreshing = changed(mounted.client);
      });
      if (completeRefreshFirst) {
        await act(async () => {
          mounted.resolveRead("Newer remote", "3");
          await refreshing;
        });
      }
      await act(async () => {
        mounted.acknowledge();
        await saving.done;
      });
      if (!completeRefreshFirst) {
        await act(async () => {
          mounted.resolveRead("Newer remote", "3");
          await refreshing;
        });
      }

      expect(mounted.cached()?.blocks[0].markdown).toBe("Newer remote");
      expect(mode === "block" ? getPageRenderSnapshot : getPageDocument).toHaveBeenCalledTimes(
        completeRefreshFirst ? 2 : 1,
      );
    },
  );

  it("acknowledges a saved draft when its own event retires the publication token", async () => {
    const mounted = await editor(mode);
    const saving = await mounted.save();
    let refreshing!: Promise<void>;
    await act(async () => {
      refreshing = changed(mounted.client);
    });
    await act(async () => {
      mounted.acknowledge();
      await saving.done;
      mounted.resolveRead("Saved draft", "2");
      await refreshing;
      // TanStack delivers React observer notifications on the next timer turn.
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    expect(mounted.overlay()).toBeFalsy();
    expect(mounted.cached()?.blocks[0].markdown).toBe("Saved draft");
    expect(mounted.container.querySelector(".cm-content")?.textContent).toBe("Saved draft");
    expect(mode === "block" ? setBlockContent : replacePageDocument).toHaveBeenCalledTimes(1);
  });

  it("discards an obsolete conflict read without replacing the newer session conflict", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    const mounted = await editor(mode);
    const blockConflict = deferred<Block | null>();
    const documentConflict = deferred<PageDocumentSnapshot | null>();
    if (mode === "block") vi.mocked(getBlock).mockReturnValue(blockConflict.promise);
    else vi.mocked(getPageDocument).mockReturnValueOnce(documentConflict.promise);
    const saving = await mounted.save();
    await act(async () => {
      mounted.rejectSave();
      await Promise.resolve();
    });
    expect(mode === "block" ? getBlock : getPageDocument).toHaveBeenCalledTimes(1);
    await act(async () => {
      const refreshing = changed(mounted.client);
      mounted.resolveRead("Newer remote", "3");
      await refreshing;
    });
    await act(async () => {
      const old = snapshot("Older remote", "2");
      if (mode === "block") blockConflict.resolve(old.blocks[0]);
      else documentConflict.resolve(old);
      await saving.done;
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    expect(mounted.cached()?.blocks[0].markdown).toBe("Newer remote");
    expect(mounted.overlay()?.conflict).toEqual(
      mode === "block"
        ? { remoteText: "Newer remote", remoteRevision: "b3" }
        : { remote: pageDocumentToPersisted(snapshot("Newer remote", "3")) },
    );
  });
});

function changed(client: QueryClient) {
  return applyDomainEvent(client, {
    kind: "mutation_committed",
    commit: {
      commitVersion: 1,
      changes: {
        changedPages: [],
        deletedPages: [],
        affectedPages: ["page"],
        blocks: [
          {
            uuid: "block",
            before: { pageUuid: "page", parentUuid: null, orderKey: "a0" },
            after: { pageUuid: "page", parentUuid: null, orderKey: "a0" },
          },
        ],
        graphChangedContent: [],
        attachmentOwners: [],
        handwritingPages: [],
        historyChanged: false,
      },
    },
  });
}

function snapshot(markdown: string, revision: string): PageDocumentSnapshot {
  return {
    pageUuid: "page",
    revision: `d${revision}`,
    blocks: [
      {
        uuid: "block",
        pageUuid: "page",
        parentUuid: null,
        orderKey: "a0",
        style: { kind: "paragraph" },
        markdown,
        markdownRevision: `b${revision}`,
        createdAt: 0,
        updatedAt: 0,
      },
    ],
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}
