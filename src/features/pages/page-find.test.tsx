// @vitest-environment jsdom
import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { createInitialWorkspaceState, type PaneId } from "@/features/workspace/workspace-model";
import { useWorkspaceStore } from "@/features/workspace/workspace-store";
import { dismissBackOverlay } from "@/lib/back-overlays";
import { PageSessionProvider } from "./page-session";
import { AppFindChromeContext, PaneFindChromeContext } from "./page-find-chrome";
import { usePageFind } from "./page-find";

const cleanup: (() => void)[] = [];
afterEach(() => {
  cleanup.splice(0).forEach((fn) => fn());
  vi.unstubAllGlobals();
});
function Note({ paneId, page = "draft" }: { paneId: PaneId; page?: string }) {
  const ref = useRef<HTMLElement>(null);
  const find = usePageFind({ uuid: page, layout: "document" }, paneId, ref, 0, {
    draft: "Thought thought thought",
  });
  return (
    <article ref={ref}>
      {find.bar}
      <div className="editor-body" contentEditable suppressContentEditableWarning>
        Thought thought thought
      </div>
      <output data-current>{find.current?.from}</output>
    </article>
  );
}
async function setup(width = 1200) {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("CSS", { highlights: new Map() });
  Object.defineProperty(window, "innerWidth", { value: width, writable: true });
  const state = createInitialWorkspaceState();
  useWorkspaceStore.setState(state);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const shell = document.createElement("div");
  shell.innerHTML =
    '<header class="app-titlebar"><div class="app-search"></div><div class="app-menu"></div></header><section data-pane-id><div class="pane-search"></div><div class="pane-menu"></div><div class="document"></div></section>';
  document.body.append(shell);
  const app = {
    onSearchOpenChange: vi.fn(),
    visible: true,
    search: shell.querySelector<HTMLElement>(".app-search"),
    menu: shell.querySelector<HTMLElement>(".app-menu"),
  };
  const pane = {
    search: shell.querySelector<HTMLElement>(".pane-search"),
    menu: shell.querySelector<HTMLElement>(".pane-menu"),
  };
  const root = createRoot(shell.querySelector(".document")!);
  const render = (page = "draft") =>
    root.render(
      <QueryClientProvider client={client}>
        <PageSessionProvider>
          <AppFindChromeContext.Provider value={app}>
            <PaneFindChromeContext.Provider value={pane}>
              <Note paneId={state.primaryPaneId} page={page} />
            </PaneFindChromeContext.Provider>
          </AppFindChromeContext.Provider>
        </PageSessionProvider>
      </QueryClientProvider>,
    );
  await act(async () => render());
  cleanup.push(() => {
    act(() => root.unmount());
    client.clear();
    shell.remove();
  });
  const key = async (key: string, options: KeyboardEventInit = {}) =>
    act(async () => {
      (document.activeElement ?? window).dispatchEvent(
        new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...options }),
      );
      await new Promise((resolve) => requestAnimationFrame(resolve));
    });
  const type = async (text: string) =>
    act(async () => {
      const input = shell.querySelector("input")!;
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, text);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  return { shell, app, pane, key, type, render };
}

it("searches the current draft, cycles matches and restores focus without putting controls in the document", async () => {
  const app = await setup();
  const editor = app.shell.querySelector<HTMLElement>("[contenteditable]")!;
  editor.focus();
  await app.key("f", { ctrlKey: true });
  expect(app.pane.search?.querySelector('[role="search"]')).not.toBeNull();
  expect(app.shell.querySelector("article [role=search]")).toBeNull();
  await app.type("thought");
  expect(app.shell.querySelector('[role="status"]')?.textContent).toBe("1 of 3");
  await app.key("Enter");
  expect(app.shell.querySelector('[role="status"]')?.textContent).toBe("2 of 3");
  await app.key("Enter", { shiftKey: true });
  expect(app.shell.querySelector('[role="status"]')?.textContent).toBe("1 of 3");
  act(() => app.shell.querySelector<HTMLButtonElement>('[aria-label="Match case"]')!.click());
  expect(app.shell.querySelector('[role="status"]')?.textContent).toBe("1 of 2");
  await app.type("missing");
  expect(app.shell.querySelector<HTMLButtonElement>('[aria-label="Next match"]')!.disabled).toBe(
    true,
  );
  await app.key("Escape");
  expect(app.shell.querySelector('[role="search"]')).toBeNull();
  expect(document.activeElement).toBe(editor);
});

it("moves search to the compact header on resize without losing the query or stealing a blurred field's focus", async () => {
  const app = await setup();
  await app.key("f", { ctrlKey: true });
  await app.type("thought");
  await act(async () => {
    window.innerWidth = 320;
    window.dispatchEvent(new Event("resize"));
  });
  expect(app.pane.search?.children).toHaveLength(0);
  expect(app.app.search?.querySelector("input")?.value).toBe("thought");
  expect(document.activeElement).toBe(app.app.search?.querySelector("input"));
  act(() => app.app.search?.querySelector("input")?.blur());
  await act(async () => {
    window.innerWidth = 1200;
    window.dispatchEvent(new Event("resize"));
  });
  expect(app.pane.search?.querySelector("input")?.value).toBe("thought");
  expect(document.activeElement).not.toBe(app.pane.search?.querySelector("input"));
});

it("dismisses compact search via Back without reopening the editor keyboard, and cleans up on navigation", async () => {
  const app = await setup(390);
  await app.key("f", { ctrlKey: true });
  await app.type("thought");
  await act(async () => {
    expect(dismissBackOverlay()).toBe(true);
    await new Promise((resolve) => requestAnimationFrame(resolve));
  });
  expect(app.app.search?.children).toHaveLength(0);
  expect(app.app.onSearchOpenChange).toHaveBeenLastCalledWith(false);
  expect(document.activeElement?.getAttribute("aria-label")).toBe("Note options");
  expect(dismissBackOverlay()).toBe(false);
  await app.key("f", { ctrlKey: true });
  await act(async () => app.render("another-page"));
  expect(app.shell.querySelector('[role="search"]')).toBeNull();
});

it("provides the same visible menu entry on a narrow screen", async () => {
  const app = await setup(360);
  const trigger = app.app.menu!.querySelector<HTMLButtonElement>("button")!;
  trigger.focus();
  await app.key("ArrowDown");
  const item = document.querySelector<HTMLElement>('[role="menuitem"]');
  expect(item?.textContent).toContain("Find in note");
  await act(async () => {
    item?.click();
    await new Promise((resolve) => setTimeout(resolve, 50));
  });
  expect(app.app.search?.querySelector('[role="search"]')).not.toBeNull();
  expect(document.activeElement).toBe(app.app.search?.querySelector("input"));
});
