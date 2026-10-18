// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vite-plus/test";
import { findBacklinks, getPage, type Block, type Page } from "@/lib/api";
import type { BacklinksSnapshot } from "@/lib/bindings";
import { queryKeys } from "@/lib/query";
import {
  WorkspaceControllerProvider,
  type WorkspaceController,
} from "@/features/workspace/workspace-controller";
import { adjacentDisposition, currentDisposition } from "@/features/workspace/workspace-model";
import { PageBacklinks } from "./page-backlinks";

vi.mock("@/lib/api", async (original) => ({
  ...(await original<typeof import("@/lib/api")>()),
  findBacklinks: vi.fn(),
  getPage: vi.fn(),
}));

const page: Page = {
  uuid: "source-page",
  kind: { kind: "note" },
  title: "Source page",
  layout: "outline",
  titleRevision: "title-1",
  createdAt: 1,
  updatedAt: 1,
};
const block: Block = {
  uuid: "source-block",
  pageUuid: page.uuid,
  parentUuid: null,
  orderKey: "8000000000000000",
  style: { kind: "paragraph" },
  markdown: "A link to [[Target]]",
  markdownRevision: "markdown-1",
  createdAt: 1,
  updatedAt: 1,
};
const snapshot: BacklinksSnapshot = {
  blocks: [block, { ...block, uuid: "second-block", markdown: "Another reference" }],
  pages: [page],
};
const openContent = vi.fn();
const controller: WorkspaceController = {
  createNewNote: () => undefined,
  createHandwrittenNote: () => undefined,
  openContent,
  openJournal: () => undefined,
  openAllNotes: () => undefined,
  captureJournal: async () => true,
  onSaved: () => undefined,
  onDelete: () => undefined,
  openMarkdownLink: () => undefined,
};

let container: HTMLDivElement;
let root: Root;
let client: QueryClient;

beforeAll(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
});

beforeEach(() => {
  vi.mocked(findBacklinks).mockReset();
  vi.mocked(getPage).mockReset();
  openContent.mockReset();
  client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  client.clear();
  container.remove();
});

async function settle() {
  await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
}

async function mount() {
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <WorkspaceControllerProvider controller={controller}>
          <PageBacklinks pageUuid="target-page" />
        </WorkspaceControllerProvider>
      </QueryClientProvider>,
    );
  });
  await settle();
}

it("loads blocks and distinct source labels together without per-page reads or cache overwrites", async () => {
  let resolve!: (value: BacklinksSnapshot) => void;
  vi.mocked(findBacklinks).mockReturnValue(new Promise((finish) => (resolve = finish)));
  const newer = { ...page, title: "Newer cached title", titleRevision: "title-2" };
  client.setQueryData(queryKeys.page(page.uuid), newer);
  await mount();
  expect(container.textContent).toContain("Loading backlinks…");
  await act(async () => resolve(snapshot));
  await settle();
  expect(container.textContent).toContain("Backlinks (2)");
  const rows = [...container.querySelectorAll("li")];
  expect(rows.map((row) => row.textContent)).toEqual([
    "Source pageA link to Target",
    "Source pageAnother reference",
  ]);
  expect(findBacklinks).toHaveBeenCalledExactlyOnceWith("target-page");
  expect(getPage).not.toHaveBeenCalled();
  expect(client.getQueryData(queryKeys.page(page.uuid))).toEqual(newer);
});

it("keeps native button activation and Shift opening the selected block in an adjacent pane", async () => {
  vi.mocked(findBacklinks).mockResolvedValue(snapshot);
  await mount();
  const buttons = container.querySelectorAll("li button");
  expect([...buttons].map((button) => button.getAttribute("type"))).toEqual(["button", "button"]);
  // Keyboard activation of a native button produces a click with detail zero.
  await act(async () => {
    buttons[0].dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 0 }));
    buttons[1].dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
  });
  expect(openContent).toHaveBeenNthCalledWith(
    1,
    { kind: "block", record: snapshot.blocks[0] },
    currentDisposition,
  );
  expect(openContent).toHaveBeenNthCalledWith(
    2,
    { kind: "block", record: snapshot.blocks[1] },
    adjacentDisposition,
  );
});

it("shows an unavailable owner explicitly while retaining the source block", async () => {
  vi.mocked(findBacklinks).mockResolvedValue({ blocks: [block], pages: [] });
  await mount();
  expect(container.textContent).toContain("Source page unavailable");
  expect(container.textContent).toContain("A link to Target");
  expect(container.textContent).not.toContain("Loading source page");
  expect(getPage).not.toHaveBeenCalled();
});

it("offers retry for a failed snapshot and displays the empty state after recovery", async () => {
  vi.mocked(findBacklinks)
    .mockRejectedValueOnce(new Error("reader unavailable"))
    .mockResolvedValueOnce({ blocks: [], pages: [] });
  await mount();
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "Could not load backlinks.",
  );
  await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
  await settle();
  expect(container.textContent).toContain("No links to this page yet.");
  expect(container.querySelector('[role="alert"]')).toBeNull();
  expect(findBacklinks).toHaveBeenCalledTimes(2);
});
