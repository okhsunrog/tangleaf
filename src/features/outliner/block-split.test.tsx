// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query";
import { EditorView } from "@codemirror/view";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { splitBlock, setBlockContent, type Block } from "@/lib/api";
import { PageSessionProvider } from "@/features/pages/page-session";
import { queryKeys } from "@/lib/query";
import { BlockNode } from "./block-node";
import { OutlinerProvider, useOutliner } from "./outliner-store";
import { flattenBlocks } from "./outliner";
vi.mock("@/lib/api", async (original) => ({
  ...(await original<typeof import("@/lib/api")>()),
  splitBlock: vi.fn(),
  setBlockContent: vi.fn(),
}));
vi.mock("@/features/markdown", () => ({
  MarkdownRenderer: ({ markdown }: { markdown: string }) => <span>{markdown}</span>,
}));
vi.mock("@/lib/notify", () => ({ notifyError: vi.fn(), notifyInfo: vi.fn() }));
const block: Block = {
  uuid: "one",
  pageUuid: "page",
  parentUuid: null,
  markdown: "First paragraph",
  markdownRevision: "r1",
  orderKey: "a",
  createdAt: 0,
  updatedAt: 0,
  style: { kind: "paragraph" },
};
const next = { ...block, uuid: "two", orderKey: "b", markdown: "", markdownRevision: "r2" };
let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let client: QueryClient;
function view() {
  return EditorView.findFromDOM(container.querySelector<HTMLElement>(".cm-editor")!)!;
}
function TestRows({ blocks }: { blocks: Block[] }) {
  const store = useOutliner();
  return (
    <ul>
      {flattenBlocks(blocks, store.isCollapsed).map(({ block, depth, ordinal }) => (
        <BlockNode key={block.uuid} block={block} depth={depth} ordinal={ordinal} />
      ))}
    </ul>
  );
}
function TestOutline() {
  const { data } = useQuery<{ document: { blocks: Block[] } }>({
    queryKey: queryKeys.pageRender("page"),
    queryFn: async () =>
      client.getQueryData<{ document: { blocks: Block[] } }>(queryKeys.pageRender("page"))!,
    enabled: false,
  });
  const blocks = data?.document.blocks ?? [];
  return (
    <OutlinerProvider
      initialEditingUuid="one"
      initialEditingRequest={1}
      blocks={blocks}
      layout="outline"
      readOnly={false}
      onOpenMarkdownLink={() => {}}
    >
      <TestRows blocks={blocks} />
    </OutlinerProvider>
  );
}
beforeEach(async () => {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.clearAllMocks();
  localStorage.clear();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  client = new QueryClient();
  client.setQueryData(queryKeys.pageRender("page"), { document: { blocks: [block] }, images: [] });
  vi.mocked(setBlockContent).mockImplementation(async (uuid, content) => ({
    ...next,
    uuid,
    markdown: content.markdown,
    markdownRevision: "r3",
  }));
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <PageSessionProvider>
          <TestOutline />
        </PageSessionProvider>
      </QueryClientProvider>,
    ),
  );
});
afterEach(() => {
  act(() => root.unmount());
  client.clear();
  container.remove();
});
function enter() {
  view().contentDOM.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }),
  );
}

it("keeps typing after Enter separate while the split command is pending", async () => {
  let finish!: (blocks: Block[]) => void;
  vi.mocked(splitBlock).mockReturnValue(
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  await act(async () => {
    enter();
  });
  expect(view().state.doc.toString()).toBe("");
  act(() => view().dispatch({ changes: { from: 0, insert: "Second paragraph" } }));
  expect(splitBlock).toHaveBeenCalledWith(
    "one",
    [{ markdown: "First paragraph" }, { markdown: "" }],
    "r1",
    "siblings",
  );
  await act(async () => finish([block, next]));
  expect(setBlockContent).toHaveBeenCalledWith("two", { markdown: "Second paragraph" }, "r2");
  const snapshot = client.getQueryData<{ document: { blocks: Block[] } }>(
    queryKeys.pageRender("page"),
  );
  expect(snapshot?.document.blocks.map((row) => row.markdown)).toEqual([
    "First paragraph",
    "Second paragraph",
  ]);
});

it("restores both paragraphs as an editable draft when splitting fails", async () => {
  let fail!: (reason: Error) => void;
  vi.mocked(splitBlock).mockReturnValue(
    new Promise((_resolve, reject) => {
      fail = reject;
    }),
  );
  const error = vi.spyOn(console, "error").mockImplementation(() => {});
  await act(async () => {
    enter();
  });
  act(() => view().dispatch({ changes: { from: 0, insert: "Still here" } }));
  await act(async () => fail(new Error("offline")));
  expect(view().state.doc.toString()).toBe("First paragraph\nStill here");
  error.mockRestore();
});

const child = { ...block, uuid: "child", parentUuid: block.uuid, markdown: "Existing child" };
async function addChild() {
  await act(async () => {
    client.setQueryData(queryKeys.pageRender("page"), {
      document: { blocks: [block, child] },
      images: [],
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

it("inserts and focuses the first child immediately below an expanded parent", async () => {
  await addChild();
  const inserted = { ...next, parentUuid: block.uuid, orderKey: "0" };
  vi.mocked(setBlockContent).mockImplementation(async (uuid, content) => ({
    ...inserted,
    uuid,
    markdown: content.markdown,
    markdownRevision: "r3",
  }));
  let finish!: (blocks: Block[]) => void;
  vi.mocked(splitBlock).mockReturnValue(
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  await act(async () => enter());
  act(() => view().dispatch({ changes: { from: 0, insert: "New child" } }));
  expect(splitBlock).toHaveBeenCalledWith(
    block.uuid,
    [{ markdown: block.markdown }, { markdown: "" }],
    "r1",
    "first_child",
  );
  await act(async () => finish([block, inserted]));
  expect(setBlockContent).toHaveBeenCalledWith(inserted.uuid, { markdown: "New child" }, "r2");
  expect(
    [...container.querySelectorAll("[data-block-uuid]")].map((row) =>
      row.getAttribute("data-block-uuid"),
    ),
  ).toEqual([block.uuid, inserted.uuid, child.uuid]);
  const editor = container.querySelector(".cm-editor")!;
  expect(editor.closest("[data-block-uuid]")?.getAttribute("data-block-uuid")).toBe(inserted.uuid);
  expect(view().hasFocus).toBe(true);
  expect(view().state.doc.toString()).toBe("New child");
  expect(container.querySelector('[data-block-uuid="child"]')?.textContent).toContain(
    "Existing child",
  );
});

it("creates a sibling when the parent's children are collapsed", async () => {
  await addChild();
  act(() => container.querySelector<HTMLButtonElement>('button[title="collapse"]')!.click());
  vi.mocked(splitBlock).mockResolvedValue([block, next]);
  await act(async () => enter());
  expect(splitBlock).toHaveBeenCalledWith(
    block.uuid,
    [{ markdown: block.markdown }, { markdown: "" }],
    "r1",
    "siblings",
  );
  expect(container.querySelector('[data-block-uuid="child"]')).toBeNull();
  expect(
    container
      .querySelector(".cm-editor")
      ?.closest("[data-block-uuid]")
      ?.getAttribute("data-block-uuid"),
  ).toBe(next.uuid);
});

it("keeps a mid-text split at the parent's level", async () => {
  await addChild();
  act(() => view().dispatch({ selection: { anchor: 5 } }));
  vi.mocked(splitBlock).mockResolvedValue([
    { ...block, markdown: "First" },
    { ...next, markdown: " paragraph" },
  ]);
  await act(async () => enter());
  expect(splitBlock).toHaveBeenCalledWith(
    block.uuid,
    [{ markdown: "First" }, { markdown: " paragraph" }],
    "r1",
    "siblings",
  );
});
