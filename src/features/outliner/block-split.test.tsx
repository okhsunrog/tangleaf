// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { EditorView } from "@codemirror/view";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { splitBlock, setBlockContent, type Block } from "@/lib/api";
import { PageSessionProvider } from "@/features/pages/page-session";
import { queryKeys } from "@/lib/query";
import { BlockNode } from "./block-node";
import { OutlinerProvider } from "./outliner-store";
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
beforeEach(async () => {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.clearAllMocks();
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
          <OutlinerProvider
            initialEditingUuid="one"
            initialEditingRequest={1}
            blocks={[block]}
            layout="outline"
            readOnly={false}
            onOpenMarkdownLink={() => {}}
          >
            <ul>
              <BlockNode block={block} depth={0} ordinal={0} />
            </ul>
          </OutlinerProvider>
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
