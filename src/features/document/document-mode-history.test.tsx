// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { EditorView } from "@codemirror/view";
import { undo } from "@codemirror/commands";
import { expect, it, vi } from "vite-plus/test";
import { PageSessionProvider } from "@/features/pages/page-session";
import { queryKeys } from "@/lib/query";
import { DocumentPage } from "./document-page";
vi.mock("@/lib/api", async (original) => ({
  ...(await original<typeof import("@/lib/api")>()),
  replacePageDocument: vi.fn().mockResolvedValue({ pageUuid: "page", revision: "r2", blocks: [] }),
}));

it("retains the editor and selection when switching through Read", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
  const client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity } } });
  client.setQueryData(queryKeys.pageDocument("page"), {
    pageUuid: "page",
    revision: "r1",
    blocks: [],
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const render = (editing: boolean) => (
    <QueryClientProvider client={client}>
      <PageSessionProvider>
        <DocumentPage
          pageUuid="page"
          editing={editing}
          canEdit={editing}
          authoringMode="live_preview"
          focusRequest={0}
          onOpenMarkdownLink={() => {}}
        />
      </PageSessionProvider>
    </QueryClientProvider>
  );
  try {
    await act(async () => root.render(render(true)));
    const dom = container.querySelector<HTMLElement>(".cm-editor")!;
    const view = EditorView.findFromDOM(dom)!;
    act(() => view.dispatch({ changes: { from: 0, insert: "Draft" }, selection: { anchor: 2 } }));
    await act(async () => root.render(render(false)));
    expect(container.querySelector("section")?.hidden).toBe(true);
    await act(async () => root.render(render(true)));
    expect(container.querySelector(".cm-editor")).toBe(dom);
    expect(view.state.selection.main.anchor).toBe(2);
    act(() => {
      undo(view);
    });
    expect(view.state.doc.toString()).toBe("");
  } finally {
    act(() => root.unmount());
    client.clear();
    container.remove();
  }
});
