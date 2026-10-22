import * as React from "react";
import { Popover as PopoverPrimitive } from "@base-ui/react/popover";

import { cn } from "@/lib/utils";
import { refreshPanelAfterClose } from "@/app/eink-refresh";
import { useOverlayInkSuppression } from "@/app/ink-suppression";

function Popover({
  onOpenChange,
  actionsRef,
  fullRefreshOnClose = true,
  ...props
}: React.ComponentProps<typeof PopoverPrimitive.Root> & {
  /**
   * Whether a close reported by the popover schedules a full panel refresh. A menu that only
   * covers part of the page can leave its repaint to the ordinary partial frame, as stock Notes
   * does with its eraser menu.
   */
  fullRefreshOnClose?: boolean;
}) {
  const internalActions = React.useRef<PopoverPrimitive.Root.Actions | null>(null);
  const actions = actionsRef ?? internalActions;
  const [reportedOpen, setReportedOpen] = React.useState(props.defaultOpen ?? false);
  const open = props.open ?? reportedOpen;
  const overlay = useOverlayInkSuppression(props.open ?? props.defaultOpen);
  const closed = fullRefreshOnClose ? refreshPanelAfterClose(onOpenChange) : onOpenChange;

  React.useEffect(() => {
    if (!open) return;
    // Tauri consumes titlebar mousedown and starts a native drag, so the
    // completed click used by Base UI's outside dismissal may never arrive.
    const dismissOnTitlebar = (event: MouseEvent) => {
      if (event.button !== 0 || !(event.target instanceof HTMLElement)) return;
      const region = event.target.getAttribute("data-tauri-drag-region");
      if (region !== null && region !== "false") actions.current?.close();
    };
    document.addEventListener("mousedown", dismissOnTitlebar, true);
    return () => document.removeEventListener("mousedown", dismissOnTitlebar, true);
  }, [open, actions]);

  return (
    <PopoverPrimitive.Root
      data-slot="popover"
      actionsRef={actions}
      onOpenChange={(open, ...rest) => {
        overlay(open);
        closed?.(open, ...rest);
        if (!rest[0].isCanceled) setReportedOpen(open);
      }}
      {...props}
    />
  );
}

function PopoverTrigger(props: React.ComponentProps<typeof PopoverPrimitive.Trigger>) {
  return <PopoverPrimitive.Trigger data-slot="popover-trigger" {...props} />;
}

function PopoverContent({
  className,
  side,
  align = "center",
  sideOffset = 8,
  anchor,
  collisionAvoidance,
  ...props
}: React.ComponentProps<typeof PopoverPrimitive.Popup> &
  Pick<
    React.ComponentProps<typeof PopoverPrimitive.Positioner>,
    "side" | "align" | "sideOffset" | "anchor" | "collisionAvoidance"
  >) {
  return (
    <PopoverPrimitive.Portal data-slot="popover-portal">
      <PopoverPrimitive.Positioner
        side={side}
        align={align}
        sideOffset={sideOffset}
        anchor={anchor}
        collisionPadding={12}
        collisionAvoidance={collisionAvoidance}
      >
        <PopoverPrimitive.Popup
          data-slot="popover-content"
          className={cn(
            "z-50 w-72 origin-(--transform-origin) rounded-xl border bg-popover p-4 text-popover-foreground shadow-popover outline-hidden data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 data-[open]:animate-in data-[open]:fade-in-0 data-[open]:zoom-in-95",
            className,
          )}
          {...props}
        />
      </PopoverPrimitive.Positioner>
    </PopoverPrimitive.Portal>
  );
}

export { Popover, PopoverContent, PopoverTrigger };
