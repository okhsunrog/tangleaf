import { createContext, useContext, useEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Menu } from "@base-ui/react/menu";
import { MoreHorizontal, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useCompactLayout } from "@/app/use-compact-layout";
import { refreshPanelAfterClose } from "@/app/eink-refresh";
import { useOverlayInkSuppression } from "@/app/ink-suppression";
import { useWorkspaceStore } from "@/features/workspace/workspace-store";
import type { PaneId } from "@/features/workspace/workspace-model";

type FindSlots = { search: HTMLElement | null; menu: HTMLElement | null };
export const AppFindChromeContext = createContext<
  FindSlots & { visible: boolean; onSearchOpenChange?: (open: boolean) => void }
>({
  search: null,
  menu: null,
  visible: true,
});
export const PaneFindChromeContext = createContext<FindSlots>({ search: null, menu: null });

/** Keep note tools outside the scrolling document, with one destination per visible pane. */
export function PageFindChrome({
  paneId,
  open,
  show,
  label,
  children,
}: {
  paneId: PaneId;
  open: boolean;
  show: (returnFocus?: HTMLElement | null) => void;
  label: string;
  children: ReactNode;
}) {
  const compact = useCompactLayout();
  const app = useContext(AppFindChromeContext);
  const pane = useContext(PaneFindChromeContext);
  const visiblePane = useWorkspaceStore((state) => state.compactVisiblePaneId);
  const visible = !compact || (app.visible && visiblePane === paneId);
  const slots = compact ? app : pane;
  useEffect(() => {
    if (!compact || !visible || !open) return;
    app.onSearchOpenChange?.(true);
    return () => app.onSearchOpenChange?.(false);
  }, [compact, visible, open, app.onSearchOpenChange]);
  useEffect(() => {
    const frame = pane.search?.closest<HTMLElement>("[data-pane-id]");
    if (!open || !visible || !frame) return;
    frame.dataset.noteFindOpen = "true";
    return () => {
      delete frame.dataset.noteFindOpen;
    };
  }, [open, visible, pane.search]);
  if (!visible) return null;
  return (
    <>
      {slots.menu && createPortal(<FindMenu label={label} show={show} />, slots.menu)}
      {open && slots.search && createPortal(children, slots.search)}
    </>
  );
}

function FindMenu({
  label,
  show,
}: {
  label: string;
  show: (returnFocus?: HTMLElement | null) => void;
}) {
  const overlay = useOverlayInkSuppression();
  const trigger = useRef<HTMLButtonElement>(null);
  const finding = useRef(false);
  return (
    <Menu.Root onOpenChange={refreshPanelAfterClose(overlay)}>
      <Menu.Trigger
        ref={trigger}
        render={
          <Button
            variant="ghost"
            size="icon"
            className="note-menu-trigger rounded-xl"
            aria-label="Note options"
            title="Note options"
          />
        }
      >
        <MoreHorizontal className="size-5" />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Positioner sideOffset={6} align="end" collisionPadding={12} className="z-50">
          <Menu.Popup
            finalFocus={() => {
              if (!finding.current) return true;
              finding.current = false;
              show(trigger.current);
              return false;
            }}
            className="min-w-56 rounded-xl border bg-popover p-1.5 text-popover-foreground shadow-popover outline-none"
          >
            <Menu.Item
              className="flex min-h-11 cursor-pointer items-center gap-3 rounded-lg px-3 text-sm outline-none data-[highlighted]:bg-accent"
              onClick={() => {
                finding.current = true;
              }}
            >
              <Search className="size-4" />
              {label}
            </Menu.Item>
          </Menu.Popup>
        </Menu.Positioner>
      </Menu.Portal>
    </Menu.Root>
  );
}
