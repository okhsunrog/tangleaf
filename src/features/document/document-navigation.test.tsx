// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { DocumentReadingSurface } from "./document-page";

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let client: QueryClient;
let scroll: ReturnType<typeof vi.fn>;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  scroll = vi.fn();
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    value: scroll,
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  client = new QueryClient();
});
afterEach(() => {
  act(() => root.unmount());
  client.clear();
  container.remove();
  vi.unstubAllGlobals();
});
describe("document block navigation", () => {
  it("reveals the correct repeated paragraph in reading mode, including a repeated request", () => {
    const render = (request: number) =>
      act(() =>
        root.render(
          <QueryClientProvider client={client}>
            <DocumentReadingSurface
              pageUuid="page"
              markdown={"same\n\nsame"}
              blockAnchors={[
                { uuid: "first", offset: 0 },
                { uuid: "second", offset: 6 },
              ]}
              navigation={{ uuid: "second", request, from: 6, to: 10 }}
            />
          </QueryClientProvider>,
        ),
      );
    render(0);
    expect(scroll.mock.instances).toEqual([container.querySelector('[data-block-uuid="second"]')]);
    render(0);
    expect(scroll.mock.calls).toHaveLength(1);
    render(1);
    expect(scroll.mock.calls).toHaveLength(2);
    expect(container.textContent).toBe("same\nsame");
  });
});
