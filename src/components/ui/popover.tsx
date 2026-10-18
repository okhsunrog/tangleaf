import * as React from "react";
import { Popover as PopoverPrimitive } from "@base-ui/react/popover";

import { cn } from "@/lib/utils";
import { refreshPanelAfterClose } from "@/app/eink-refresh";
import { useOverlayInkSuppression } from "@/app/ink-suppression";

function Popover({
  onOpenChange,
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
  const overlay = useOverlayInkSuppression(props.open ?? props.defaultOpen);
  const closed = fullRefreshOnClose ? refreshPanelAfterClose(onOpenChange) : onOpenChange;
  return (
    <PopoverPrimitive.Root
      data-slot="popover"
      onOpenChange={(open, ...rest) => {
        overlay(open);
        closed?.(open, ...rest);
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
