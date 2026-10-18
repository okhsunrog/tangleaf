// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vite-plus/test";
import { ConfirmationProvider } from "@/app/confirmation";
import { useWorkspaceStore } from "@/features/workspace/workspace-store";
import {
  currentDisposition,
  pageTarget,
  PaneContentKind,
} from "@/features/workspace/workspace-model";
import type { Page } from "@/lib/api";
import { applyDomainEvent, createAppQueryClient, queryKeys } from "@/lib/query";
import { PageSessionProvider } from "./page-session";
import { useNotesWorkspace } from "./use-notes-workspace";

const api = vi.hoisted(() => ({ getPage: vi.fn() }));
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

let workspace: ReturnType<typeof useNotesWorkspace>;
function Harness() {
  workspace = useNotesWorkspace(false, () => undefined);
  return <output>{workspace.activePage?.title ?? workspace.activePane.content.kind}</output>;
}

let client: QueryClient;
let container: HTMLDivElement;
let root: Root;

beforeAll(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
});

beforeEach(async () => {
  api.getPage.mockReset().mockResolvedValue(page);
  client = createAppQueryClient();
  client.setQueryData(queryKeys.page(page.uuid), page);
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
          <PageSessionProvider>
            <Harness />
          </PageSessionProvider>
        </ConfirmationProvider>
      </QueryClientProvider>,
    );
  });
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
        historyChanged: true,
      },
    },
  });
}

function nextNotification() {
  return new Promise<void>((resolve) => setTimeout(resolve, 0));
}
