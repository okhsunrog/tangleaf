// @vitest-environment jsdom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vite-plus/test";
import { Popover, PopoverContent, PopoverTrigger } from "./popover";

it.each([false, true])(
  "dismisses on a native titlebar press without a click (controlled=%s)",
  async (controlled) => {
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    const changed = vi.fn();
    function Example() {
      const [open, setOpen] = useState(true);
      return (
        <>
          <header data-tauri-drag-region>Titlebar</header>
          <Popover
            defaultOpen
            defaultTriggerId="sync-status"
            open={controlled ? open : undefined}
            onOpenChange={(next, details) => {
              changed(next, details);
              setOpen(next);
            }}
          >
            <PopoverTrigger id="sync-status">Synced</PopoverTrigger>
            <PopoverContent initialFocus={false}>
              <button type="button">Sync settings</button>
            </PopoverContent>
          </Popover>
        </>
      );
    }
    // Tauri installs this document listener before the application's React tree.
    const nativeDrag = vi.fn((event: MouseEvent) => {
      if (!(event.target instanceof HTMLElement)) return;
      if (!event.target.hasAttribute("data-tauri-drag-region")) return;
      event.preventDefault();
      event.stopImmediatePropagation();
    });
    document.addEventListener("mousedown", nativeDrag);
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const popup = () => document.querySelector('[data-slot="popover-content"][data-open]');
    try {
      await act(async () => root.render(<Example />));
      expect(popup()).not.toBeNull();

      await act(async () => {
        popup()!
          .querySelector("button")!
          .dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
      });
      expect(popup()).not.toBeNull();
      changed.mockClear();

      const press = new MouseEvent("mousedown", { bubbles: true, cancelable: true, detail: 1 });
      await act(async () => container.querySelector("header")!.dispatchEvent(press));
      expect(press.defaultPrevented).toBe(true);
      expect(nativeDrag).toHaveBeenCalled();
      expect(changed).toHaveBeenCalledWith(false, expect.anything());
      expect(popup()).toBeNull();
    } finally {
      await act(async () => root.unmount());
      container.remove();
      document.removeEventListener("mousedown", nativeDrag);
    }
  },
);
