// @vitest-environment jsdom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";
import type { Block, Page } from "@/lib/api";
import { queryKeys } from "@/lib/query";
import { Outliner } from "./outliner";
import { useOutliner } from "./outliner-store";

let editingStore: ReturnType<typeof useOutliner>;
let nextInstance = 0;
vi.mock("./block-node", () => ({
  BlockNode: ({ block }: { block: Block }) => {
    const store = useOutliner();
    editingStore = store;
    const [instance] = useState(() => ++nextInstance);
    return (
      <li
        data-block-uuid={block.uuid}
        data-editing={store.editingUuid ?? ""}
        data-instance={instance}
      >
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
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Desktop WebView");
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
  Object.defineProperties(container, {
    clientHeight: { configurable: true, value: 400 },
    clientWidth: { configurable: true, value: 1000 },
  });
  container.scrollTo = vi.fn(({ top }: ScrollToOptions) => {
    container.scrollTop = top ?? 0;
  }) as typeof container.scrollTo;
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
  vi.restoreAllMocks();
});

function render(blocks: Block[], target: string | null, request = 0, readOnly = true) {
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
          readOnly={readOnly}
        />
      </QueryClientProvider>,
    ),
  );
}

describe("outliner block navigation", () => {
  it("reveals a late block before progressive batches and without editing it", () => {
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Android WebView");
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
  it("windows a live desktop Split and mounts the final target without its prefix", () => {
    render([block(0)], null, 0, false);
    act(() => editingStore.setEditing("block-0"));
    const first = container.querySelector('[data-block-uuid="block-0"]');
    const instance = first?.getAttribute("data-instance");
    const blocks = Array.from({ length: 243 }, (_, index) => block(index));
    render(blocks, null, 0, false);
    expect(
      container.querySelector('[data-block-uuid="block-0"]')?.getAttribute("data-instance"),
    ).toBe(instance);
    act(() => editingStore.setEditing("block-242"));
    expect(
      container.querySelector('[data-block-uuid="block-242"]')?.getAttribute("data-editing"),
    ).toBe("block-242");
    expect(container.querySelectorAll("[data-block-uuid]").length).toBeLessThan(30);
  });
  it("keeps the same active editor mounted outside the viewport", () => {
    const blocks = Array.from({ length: 243 }, (_, index) => block(index));
    render(blocks, null, 0, false);
    act(() => editingStore.setEditing("block-0"));
    const editor = container.querySelector('[data-block-uuid="block-0"]');
    act(() => {
      container.scrollTop = 7000;
      container.dispatchEvent(new Event("scroll"));
    });
    expect(container.querySelector('[data-block-uuid="block-0"]')).toBe(editor);
    expect(container.querySelectorAll("[data-block-uuid]").length).toBeLessThan(30);
    expect(container.querySelector('[data-block-uuid="block-180"]')).not.toBeNull();
  });
  it("preserves the editor when a hidden pane crosses the windowing threshold", () => {
    Object.defineProperty(container, "clientHeight", { configurable: true, value: 0 });
    render([block(0)], null, 0, false);
    act(() => editingStore.setEditing("block-0"));
    const editor = container.querySelector('[data-block-uuid="block-0"]');
    render(
      Array.from({ length: 243 }, (_, index) => block(index)),
      null,
      0,
      false,
    );
    expect(container.querySelector('[data-block-uuid="block-0"]')).toBe(editor);
    expect(container.querySelectorAll("[data-block-uuid]")).toHaveLength(1);
  });
  it("does not reveal the active editor again when a persisted snapshot refreshes", () => {
    const blocks = Array.from({ length: 243 }, (_, index) => block(index));
    render(blocks, null, 0, false);
    act(() => editingStore.setEditing("block-242"));
    act(() => {
      container.scrollTop = 4000;
      container.dispatchEvent(new Event("scroll"));
    });
    render(
      blocks.map((row, index) => (index ? row : { ...row, markdown: "Updated" })),
      null,
      0,
      false,
    );
    expect(container.scrollTop).toBe(4000);
    expect(container.querySelector('[data-block-uuid="block-242"]')).not.toBeNull();
  });
  it("virtualizes a narrow desktop and preserves Android's static target path", () => {
    Object.defineProperty(container, "clientWidth", { configurable: true, value: 390 });
    const blocks = Array.from({ length: 243 }, (_, index) => block(index));
    render(blocks, "block-242");
    expect(container.querySelector('[data-block-uuid="block-242"]')).not.toBeNull();
    expect(container.querySelectorAll("[data-block-uuid]").length).toBeLessThan(30);
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Android WebView");
    render(blocks, "block-242", 1);
    expect(container.querySelectorAll("[data-block-uuid]")).toHaveLength(243);
    expect(scroll.mock.instances).toContain(
      container.querySelector('[data-block-uuid="block-242"]'),
    );
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
