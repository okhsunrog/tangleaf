// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vite-plus/test";
import { allNotesTarget, pageTarget, type PaneContent } from "./workspace-model";
import { usePaneScroll } from "./use-pane-scroll";

it("opens a new screen at the top and restores each history entry independently", async () => {
  (
    globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  const host = document.createElement("div");
  const root = createRoot(host);
  function Harness({ content }: { content: PaneContent }) {
    const ref = usePaneScroll(content);
    return (
      <div ref={ref}>
        <article>{content.kind}</article>
      </div>
    );
  }
  const first = pageTarget("one");
  const revisit = pageTarget("one");
  const render = async (content: PaneContent) => {
    await act(async () => root.render(<Harness content={content} />));
    return host.firstElementChild as HTMLDivElement;
  };
  try {
    const scroll = await render(first);
    scroll.scrollTop = 740;
    scroll.dispatchEvent(new Event("scroll"));
    expect((await render(allNotesTarget)).scrollTop).toBe(0);
    expect((await render(first)).scrollTop).toBe(740);
    expect((await render(revisit)).scrollTop).toBe(0);
  } finally {
    await act(async () => root.unmount());
    vi.unstubAllGlobals();
  }
});
