// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vite-plus/test";
import { ConfirmationProvider } from "@/app/confirmation";
import { useWorkspaceStore } from "@/features/workspace/workspace-store";
import {
  adjacentDisposition,
  currentDisposition,
  DockVisibility,
  pageTarget,
  PaneContentKind,
  WorkspaceNodeKind,
} from "@/features/workspace/workspace-model";
import { PagePresentation } from "./page-presentation";
import type { Page } from "@/lib/api";
import type { ChangeSet, HistoryMoveResult } from "@/lib/bindings";
import { notifyInfo } from "@/lib/notify";
import { applyDomainEvent, createAppQueryClient, queryKeys } from "@/lib/query";
import { PageSessionProvider, PageSessionRegistry } from "./page-session";
import { useNotesWorkspace } from "./use-notes-workspace";

const api = vi.hoisted(() => ({
  getPage: vi.fn(),
  getHistoryStatus: vi.fn(),
  undo: vi.fn(),
  redo: vi.fn(),
}));
vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, ...api };
});
vi.mock("@/lib/notify", () => ({
  notifyError: vi.fn(),
  notifyInfo: vi.fn(),
  notifySuccess: vi.fn(),
}));

const page: Page = {
  uuid: "019f0000-0000-7000-8000-00000000000a",
  kind: { kind: "note" },
  title: "Storage lifecycle check",
  layout: "outline",
  titleRevision: "revision-1",
  createdAt: 1,
  updatedAt: 1,
};
const otherPage: Page = { ...page, uuid: "other-page", title: "Another note" };
const historyPage: Page = { ...page, uuid: "history-page", title: "Navigation history" };
const initialHistory = { undoCount: 2, redoCount: 1 };

let workspace: ReturnType<typeof useNotesWorkspace>;
function Harness() {
  workspace = useNotesWorkspace(true, () => undefined);
  return <output>{workspace.activePage?.title ?? workspace.activePane.content.kind}</output>;
}

let client: QueryClient;
let container: HTMLDivElement;
let root: Root;
let pageSessions: PageSessionRegistry;

beforeAll(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
});

beforeEach(async () => {
  api.getPage.mockReset().mockResolvedValue(page);
  api.getHistoryStatus.mockReset().mockResolvedValue(initialHistory);
  api.undo.mockReset().mockResolvedValue("empty");
  api.redo.mockReset().mockResolvedValue("empty");
  vi.mocked(notifyInfo).mockClear();
  pageSessions = new PageSessionRegistry();
  client = createAppQueryClient();
  client.setQueryData(queryKeys.page(page.uuid), page);
  client.setQueryData(queryKeys.page(otherPage.uuid), otherPage);
  client.setQueryData(queryKeys.page(historyPage.uuid), historyPage);
  client.setQueryData(queryKeys.history, initialHistory);
  useWorkspaceStore.getState().dispatch({ type: "reset" });
  useWorkspaceStore.getState().dispatch({
    type: "open_target",
    target: pageTarget(page.uuid),
    disposition: currentDisposition,
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <ConfirmationProvider>
          <PageSessionProvider registry={pageSessions}>
            <Harness />
          </PageSessionProvider>
        </ConfirmationProvider>
      </QueryClientProvider>,
    );
  });
});

it.each([
  ["undo", "before response"],
  ["undo", "after response"],
  ["redo", "before response"],
  ["redo", "after response"],
] as const)(
  "preserves panes, navigation and unrelated drafts for applied %s with commit %s",
  async (direction, notificationOrder) => {
    await act(async () => arrangeSplitWorkspace());
    const before = useWorkspaceStore.getState();
    expect(before.tree.kind).toBe(WorkspaceNodeKind.Split);
    expect(before.panes[before.primaryPaneId]?.forward).toHaveLength(1);

    pageSessions.editTitle(otherPage.uuid, "Unsaved title", {
      text: otherPage.title ?? "",
      revision: otherPage.titleRevision,
    });
    pageSessions.editBlock(otherPage.uuid, "draft-block", "Unsaved paragraph", {
      text: "Persisted paragraph",
      revision: "block-revision",
    });
    pageSessions.editDocument(historyPage.uuid, "Unsaved document", {
      buffer: "Persisted document",
      revision: "document-revision",
      sourceMap: {
        version: 1,
        offsetEncoding: "utf-16",
        sourceMarkdown: "Persisted document",
        entries: [],
      },
    });
    const titleAndBlockDrafts = pageSessions.getSnapshot(otherPage.uuid);
    const documentDraft = pageSessions.getSnapshot(historyPage.uuid);
    client.setQueryData(queryKeys.pageDocument(page.uuid), { buffer: "Before split" });
    client.setQueryData(queryKeys.children(page.uuid), []);
    client.setQueryData(queryKeys.block("source-block"), { markdown: "Before split" });
    const nextHistory =
      direction === "undo" ? { undoCount: 1, redoCount: 2 } : { undoCount: 3, redoCount: 0 };
    api.getHistoryStatus.mockResolvedValue(nextHistory);
    api[direction].mockImplementation(async () => {
      if (notificationOrder === "before response") await structuralCommit();
      return "applied";
    });

    await act(async () => {
      await workspace.moveHistory(direction);
      if (notificationOrder === "after response") await structuralCommit();
      await nextNotification();
    });

    expect(api[direction]).toHaveBeenCalledTimes(1);
    expect(useWorkspaceStore.getState()).toBe(before);
    expect(workspace.activePageUuid).toBe(page.uuid);
    expect(workspace.history).toEqual(nextHistory);
    expect(pageSessions.getSnapshot(otherPage.uuid)).toBe(titleAndBlockDrafts);
    expect(pageSessions.getSnapshot(historyPage.uuid)).toBe(documentDraft);
    expect(client.getQueryState(queryKeys.page(otherPage.uuid))?.isInvalidated).toBe(false);
    expect(client.getQueryState(queryKeys.page(historyPage.uuid))?.isInvalidated).toBe(false);
    expect(client.getQueryState(queryKeys.pageDocument(page.uuid))?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.children(page.uuid))?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.block("source-block"))?.isInvalidated).toBe(true);
  },
);

it.each([
  ["undo", "before response"],
  ["undo", "after response"],
  ["redo", "before response"],
  ["redo", "after response"],
] as const)(
  "cleans up a deleted current page for applied %s with commit %s, preserving unrelated panes",
  async (direction, notificationOrder) => {
    await act(async () => arrangeSplitWorkspace());
    const before = useWorkspaceStore.getState();
    const otherPaneId = Object.keys(before.panes).find((id) => id !== before.primaryPaneId)!;
    const otherPane = before.panes[otherPaneId];
    pageSessions.editTitle(page.uuid, "Draft on the deleted note", {
      text: page.title ?? "",
      revision: page.titleRevision,
    });
    pageSessions.editTitle(otherPage.uuid, "Unrelated draft", {
      text: otherPage.title ?? "",
      revision: otherPage.titleRevision,
    });
    const unrelatedDraft = pageSessions.getSnapshot(otherPage.uuid);
    api[direction].mockImplementation(async () => {
      if (notificationOrder === "before response") await committed({ deleted: true });
      return "applied";
    });

    await act(async () => {
      await workspace.moveHistory(direction);
      if (notificationOrder === "after response") await committed({ deleted: true });
      await nextNotification();
    });

    const after = useWorkspaceStore.getState();
    expect(workspace.activePane.content.kind).toBe(PaneContentKind.Home);
    expect(after.tree).toBe(before.tree);
    expect(after.panes[otherPaneId]?.content).toEqual(otherPane?.content);
    expect(pageSessions.getSnapshot(page.uuid).title).toBeNull();
    expect(pageSessions.getSnapshot(otherPage.uuid)).toBe(unrelatedDraft);
    expect(
      Object.values(after.panes).flatMap((pane) => [pane.content, ...pane.back, ...pane.forward]),
    ).not.toContainEqual(expect.objectContaining({ pageUuid: page.uuid }));
  },
);

it.each([
  ["undo", "skipped"],
  ["undo", "empty"],
  ["redo", "skipped"],
  ["redo", "empty"],
] as const)("preserves the workspace for %s returning %s", async (direction, result) => {
  await act(async () => arrangeSplitWorkspace());
  const before = useWorkspaceStore.getState();
  api[direction].mockResolvedValue(result);
  await act(async () => workspace.moveHistory(direction));
  expect(useWorkspaceStore.getState()).toBe(before);
  expect(client.getQueryState(queryKeys.page(otherPage.uuid))?.isInvalidated).toBe(false);
  expect(notifyInfo).toHaveBeenCalledTimes(result === "skipped" ? 1 : 0);
});

it.each(["undo", "redo"] as const)(
  "preserves newer navigation while %s is in flight",
  async (direction) => {
    let finishHistory: (result: HistoryMoveResult) => void = () => undefined;
    api[direction].mockReturnValue(
      new Promise<HistoryMoveResult>((resolve) => {
        finishHistory = resolve;
      }),
    );
    let movement: Promise<void> | undefined;
    await act(async () => {
      movement = workspace.moveHistory(direction);
      await workspace.controller.openContent({ kind: "page", record: otherPage });
    });
    const afterNavigation = useWorkspaceStore.getState();
    await act(async () => {
      await structuralCommit();
      finishHistory("applied");
      await movement;
      await nextNotification();
    });
    expect(workspace.activePageUuid).toBe(otherPage.uuid);
    expect(useWorkspaceStore.getState()).toBe(afterNavigation);
    expect(notifyInfo).not.toHaveBeenCalled();
  },
);

it("still resets navigation explicitly when the workspace is replaced", async () => {
  await act(async () => arrangeSplitWorkspace());
  const before = useWorkspaceStore.getState();
  await act(async () => {
    await applyDomainEvent(client, {
      kind: "mutation_committed",
      commit: {
        commitVersion: 5,
        workspaceEpoch: 1,
        workspaceReplaced: true,
        changes: emptyChanges(),
      },
    });
    // The data-tools completion path explicitly invokes this controller action.
    workspace.resetWorkspace();
    await nextNotification();
  });
  const after = useWorkspaceStore.getState();
  expect(workspace.activePane.content.kind).toBe(PaneContentKind.Home);
  expect(after.tree.kind).toBe(WorkspaceNodeKind.Pane);
  expect(Object.keys(after.panes)).toHaveLength(1);
  expect(after.assistantDock).toEqual(before.assistantDock);
  expect(client.getQueryData(queryKeys.page(otherPage.uuid))).toBeUndefined();
});

afterEach(async () => {
  await act(async () => root.unmount());
  client.clear();
  container.remove();
  useWorkspaceStore.getState().dispatch({ type: "reset" });
});

it.each(["before opening", "after opening"])(
  "keeps a restored page open when its committed notification arrives %s",
  async (notificationOrder) => {
    await act(async () => {
      await committed({ deleted: true });
      await nextNotification();
    });
    expect(workspace.activePane.content.kind).toBe(PaneContentKind.Home);
    expect(client.getQueryData(queryKeys.page(page.uuid))).toBeNull();

    let finishRead: (value: Page) => void = () => undefined;
    api.getPage.mockReturnValue(
      new Promise<Page>((resolve) => {
        finishRead = resolve;
      }),
    );
    if (notificationOrder === "before opening") {
      await act(async () => committed({ deleted: false }));
    }

    // Normal navigation seeds the fresh Page returned by the restored list or
    // search result. Raw store dispatch would bypass this application contract.
    await act(async () => {
      await workspace.controller.openContent({ kind: "page", record: page });
      await nextNotification();
    });

    let notification: Promise<void> | undefined;
    if (notificationOrder === "after opening") {
      await act(async () => {
        notification = committed({ deleted: false });
        await nextNotification();
      });
    }
    expect(workspace.activePane.content.kind).toBe(PaneContentKind.Page);
    expect(workspace.activePageUuid).toBe(page.uuid);
    expect(container.textContent).toBe(page.title);
    expect(api.getPage).toHaveBeenCalledTimes(notificationOrder === "after opening" ? 1 : 0);

    await act(async () => {
      finishRead(page);
      await notification;
      await nextNotification();
    });
    expect(workspace.activePane.content.kind).toBe(PaneContentKind.Page);
    expect(workspace.activePageUuid).toBe(page.uuid);
    expect(container.textContent).toBe(page.title);
  },
);

function committed({ deleted }: { deleted: boolean }) {
  return applyDomainEvent(client, {
    kind: "mutation_committed",
    commit: {
      commitVersion: deleted ? 1 : 2,
      changes: {
        changedPages: deleted ? [] : [page.uuid],
        deletedPages: deleted ? [page.uuid] : [],
        affectedPages: [],
        blocks: [],
        graphChangedContent: [page.uuid],
        attachmentOwners: [],
        handwritingPages: [],
        historyChanged: true,
      },
    },
  });
}

function arrangeSplitWorkspace() {
  const dispatch = useWorkspaceStore.getState().dispatch;
  const primaryPaneId = useWorkspaceStore.getState().primaryPaneId;
  dispatch({
    type: "open_target",
    target: pageTarget(historyPage.uuid),
    disposition: currentDisposition,
  });
  dispatch({ type: "go_back", paneId: primaryPaneId });
  dispatch({
    type: "open_target",
    target: pageTarget(otherPage.uuid, { presentation: PagePresentation.Reading }),
    disposition: adjacentDisposition,
  });
  const secondaryPaneId = useWorkspaceStore.getState().activePaneId;
  dispatch({
    type: "open_target",
    target: pageTarget(historyPage.uuid),
    disposition: currentDisposition,
  });
  dispatch({ type: "go_back", paneId: secondaryPaneId });
  const tree = useWorkspaceStore.getState().tree;
  if (tree.kind !== WorkspaceNodeKind.Split) throw new Error("Expected a split workspace");
  dispatch({ type: "resize_split", splitId: tree.splitId, ratio: 0.63 });
  dispatch({ type: "set_dock_visibility", visibility: DockVisibility.Open });
  dispatch({ type: "set_dock_width", width: 420 });
  dispatch({ type: "focus_pane", paneId: primaryPaneId });
}

function emptyChanges(): ChangeSet {
  return {
    changedPages: [],
    deletedPages: [],
    affectedPages: [],
    blocks: [],
    graphChangedContent: [],
    attachmentOwners: [],
    handwritingPages: [],
    historyChanged: false,
  };
}

function structuralCommit() {
  const placement = { pageUuid: page.uuid, parentUuid: null, orderKey: "a0" };
  return applyDomainEvent(client, {
    kind: "mutation_committed",
    commit: {
      commitVersion: 3,
      changes: {
        ...emptyChanges(),
        affectedPages: [page.uuid],
        blocks: [{ uuid: "source-block", before: placement, after: placement }],
        historyChanged: true,
      },
    },
  });
}

function nextNotification() {
  return new Promise<void>((resolve) => setTimeout(resolve, 0));
}
