// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import type { ChatSource } from "@/lib/api";
import MarkdownResponse from "./markdown-response";
import { ToolActivity } from "./tool-activity";

const PAGE = "019c8d1a-4ab1-7f31-8f00-f594337c3ca5";
const BLOCK = "019c8d1a-4ab1-7f31-8f00-f594337c3ca6";
const api = vi.hoisted(() => ({ getPage: vi.fn(), getBlock: vi.fn(), getContainingPage: vi.fn() }));
vi.mock("@/lib/api", async (original) => ({
  ...(await original<typeof import("@/lib/api")>()),
  ...api,
}));
const sources: ChatSource[] = [
  {
    target: { kind: "block", uuid: BLOCK, pageUuid: PAGE },
    title: "Charging board",
    snippet: "probably need for charging esk8",
  },
  { target: { kind: "page", uuid: PAGE }, title: "Charging board", snippet: "" },
];
let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let client: QueryClient;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  api.getPage.mockReset().mockResolvedValue(null);
  api.getBlock.mockReset().mockResolvedValue({
    uuid: BLOCK,
    pageUuid: PAGE,
    markdown: "probably need for charging esk8",
  });
  api.getContainingPage
    .mockReset()
    .mockResolvedValue({ uuid: PAGE, kind: { kind: "note" }, title: "Charging board" });
});
afterEach(() => {
  act(() => root.unmount());
  client.clear();
  container.remove();
});

it("opens numbered citations and source rows with the correct typed target and disposition", async () => {
  const open = vi.fn();
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <MarkdownResponse
          sources={sources}
          onOpenLink={open}
        >{`BQ25756 [@${BLOCK}] and note [@${PAGE}]. Again [${BLOCK}].`}</MarkdownResponse>
      </QueryClientProvider>,
    ),
  );
  expect(container.textContent).not.toContain(BLOCK);
  expect(container.textContent).toContain("BQ25756 [1] and note [2]. Again [1].");
  const links = container.querySelectorAll<HTMLAnchorElement>(".assistant-markdown a");
  expect(links[0]?.title).toContain("Charging board: probably need for charging esk8");
  act(() => links[0]?.click());
  expect(open).toHaveBeenLastCalledWith({
    context: { kind: "assistant" },
    disposition: "current",
    target: { kind: "block", uuid: BLOCK },
  });
  act(() => {
    links[1]?.dispatchEvent(
      new MouseEvent("click", { bubbles: true, cancelable: true, shiftKey: true }),
    );
  });
  expect(open).toHaveBeenLastCalledWith({
    context: { kind: "assistant" },
    disposition: "adjacent",
    target: { kind: "page_id", uuid: PAGE },
  });
  const disclosure = container.querySelector("details");
  expect(disclosure?.open).toBe(false);
  expect(disclosure?.querySelector("summary")?.textContent).toBe("Sources · 2");
  act(() => container.querySelector<HTMLAnchorElement>("ol a")?.click());
  expect(open).toHaveBeenLastCalledWith({
    context: { kind: "assistant" },
    disposition: "current",
    target: { kind: "block", uuid: BLOCK },
  });
  expect(api.getPage).not.toHaveBeenCalled();
});

it("resolves historical UUID markers locally and exposes the source snapshot for persistence", async () => {
  const resolved = vi.fn();
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <MarkdownResponse
          onOpenLink={() => undefined}
          onSourcesResolved={resolved}
        >{`BQ25756 [${BLOCK}]`}</MarkdownResponse>
      </QueryClientProvider>,
    ),
  );
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  expect(api.getBlock).toHaveBeenCalledWith(BLOCK);
  expect(resolved).toHaveBeenCalledWith([sources[0]]);
  expect(container.querySelector(".assistant-markdown a")?.textContent).toBe("[1]");
});

it("does not resolve unverified citations from modern responses against unrelated local content", async () => {
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <MarkdownResponse
          sources={[]}
          onOpenLink={() => undefined}
        >{`BQ25756 [@${BLOCK}]\n\n[source unavailable]: https://example.com/unverified`}</MarkdownResponse>
      </QueryClientProvider>,
    ),
  );
  expect(container.textContent).toContain("[source unavailable]");
  expect(container.querySelector("a")).toBeNull();
  expect(api.getPage).not.toHaveBeenCalled();
});

it("keeps tool calls collapsed behind a readable activity summary", () => {
  act(() =>
    root.render(
      <ToolActivity
        busy={false}
        tools={[
          { id: "1", name: "search_and_expand", args: { query: "charging board" }, result: "[]" },
          { id: "2", name: "search_and_expand", args: { query: "TI BQ" }, result: "[]" },
        ]}
      />,
    ),
  );
  expect(container.querySelector("details")?.open).toBe(false);
  expect(container.querySelector("summary")?.textContent).toBe("Searched notes · 2 queries");
});
