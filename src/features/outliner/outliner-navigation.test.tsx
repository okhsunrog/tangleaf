// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";
import type { Block, Page } from "@/lib/api";
import { queryKeys } from "@/lib/query";
import { Outliner } from "./outliner";
import { useOutliner } from "./outliner-store";

vi.mock("./block-node", () => ({
  BlockNode: ({ block }: { block: Block }) => {
    const store = useOutliner();
    return (
      <li data-block-uuid={block.uuid} data-editing={store.editingUuid ?? ""}>
        {block.markdown}
      </li>
    );
  },
}));
vi.mock("@/features/markdown", () => ({ useResolvedAttachmentImages: () => undefined }));

const page: Page = {
  uuid: "page",
  kind: { kind: "note" },
  title: "Test",
  layout: "outline",
  titleRevision: "r",
  createdAt: 0,
  updatedAt: 0,
};
const block = (index: number, parentUuid: string | null = null): Block => ({
  uuid: `block-${index}`,
  pageUuid: page.uuid,
  parentUuid,
  orderKey: String(index).padStart(4, "0"),
  style: { kind: "paragraph" },
  markdown: `Block ${index}`,
  markdownRevision: "r",
  createdAt: 0,
  updatedAt: 0,
});
let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let client: QueryClient;
let scroll: ReturnType<typeof vi.fn>;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.useFakeTimers();
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
      unobserve() {}
    },
  );
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  scroll = vi.fn();
  vi.stubGlobal("requestAnimationFrame", (cb: () => void) => setTimeout(cb, 16));
  vi.stubGlobal("cancelAnimationFrame", clearTimeout);
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    value: scroll,
  });
  localStorage.clear();
  container = document.createElement("div");
  container.setAttribute("data-workspace-scroll", "");
  document.body.append(container);
  root = createRoot(container);
  client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity } } });
});
afterEach(() => {
  act(() => root.unmount());
  client.clear();
  container.remove();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

function render(blocks: Block[], target: string, request = 0) {
  client.setQueryData(queryKeys.pageRender(page.uuid), {
    document: { pageUuid: page.uuid, revision: "r", blocks },
    images: [],
  });
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <Outliner
          page={page}
          targetBlockUuid={target}
          navigationRequest={request}
          onOpenMarkdownLink={() => undefined}
          readOnly
        />
      </QueryClientProvider>,
    ),
  );
}

describe("outliner block navigation", () => {
  it("reveals a late block before progressive batches and without editing it", () => {
    const blocks = Array.from({ length: 395 }, (_, index) => block(index));
    render(blocks, "block-349");
    expect(container.querySelector('[data-block-uuid="block-349"]')).not.toBeNull();
    expect(scroll.mock.instances).toContain(
      container.querySelector('[data-block-uuid="block-349"]'),
    );
    expect(container.querySelector('[data-editing]:not([data-editing=""])')).toBeNull();
    const count = scroll.mock.calls.length;
    render(blocks, "block-349", 1);
    expect(scroll.mock.calls.length).toBe(count + 1);
  });
  it("expands collapsed ancestors on initial and subsequent navigation", () => {
    const blocks = [block(0), block(1, "block-0"), block(2, "block-1")];
    localStorage.setItem("outliner.collapsed.block-0", "1");
    localStorage.setItem("outliner.collapsed.block-1", "1");
    render(blocks, "block-0");
    expect(container.querySelector('[data-block-uuid="block-2"]')).toBeNull();
    render(blocks, "block-2", 1);
    expect(container.querySelector('[data-block-uuid="block-2"]')).not.toBeNull();
    expect(localStorage.getItem("outliner.collapsed.block-0")).toBeNull();
    expect(localStorage.getItem("outliner.collapsed.block-1")).toBeNull();
  });
});
