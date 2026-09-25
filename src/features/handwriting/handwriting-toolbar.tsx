import { useLayoutEffect, useRef, useState, type ComponentProps } from "react";
import { Menu } from "@base-ui/react/menu";
import { Check, Grid2X2, MoreHorizontal, Pencil, PenLine, Star, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { refreshPanelAfterClose } from "@/app/eink-refresh";
import { useOverlayInkSuppression } from "@/app/ink-suppression";
import { cn } from "@/lib/utils";

/**
 * Pieces of the handwriting toolbar: one 44 px row over the sheet with 40 px targets.
 *
 * Sizes are px, not rem: rem follows the system font scale, and a toolbar that grows with it
 * costs sheet height. Text inside the bar stays in rem.
 */

/** One tap picks the pen and its width, like the stock app's pen presets. */
export const PEN_PRESETS = [
  { width: 2, label: "fine" },
  { width: 3, label: "medium" },
  { width: 5, label: "bold" },
] as const;

/** Below this bar width the presets fold into one Pen whose second tap opens the widths. */
const PRESETS_MIN_WIDTH = 600;
/** Below this bar width the status chip shows its icon only. */
const STATUS_LABEL_MIN_WIDTH = 480;

export type ToolbarLayout = { penPresets: boolean; statusIconOnly: boolean };

/** A bar not measured yet (width 0) gets the wide layout. */
export function toolbarLayout(width: number): ToolbarLayout {
  if (width <= 0) return { penPresets: true, statusIconOnly: false };
  return {
    penPresets: width >= PRESETS_MIN_WIDTH,
    statusIconOnly: width < STATUS_LABEL_MIN_WIDTH,
  };
}

/**
 * Measures the bar, so the DOM holds one set of buttons rather than one per container query.
 * It fires on resize only; nothing here measures the sheet.
 */
export function useToolbarLayout() {
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const measure = () => setWidth(Math.round(element.getBoundingClientRect().width));
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  return [ref, toolbarLayout(width)] as const;
}

/** A 40 px square target with a 24 px icon. */
export function ToolbarButton({ className, ...props }: ComponentProps<typeof Button>) {
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-lg"
      className={cn(
        "size-[40px] shrink-0 rounded-lg [&_svg:not([class*='size-'])]:size-[24px]",
        className,
      )}
      {...props}
    />
  );
}

/** A hard rule between groups; black on e-ink. */
export function ToolbarSeparator() {
  return (
    <span
      aria-hidden
      data-ink-toolbar-separator
      className="mx-[2px] h-[24px] w-px shrink-0 bg-border"
    />
  );
}

/** A nib over a sample of the stroke the preset draws. */
export function PenPresetIcon({ width }: { width: number }) {
  return (
    <span className="flex flex-col items-center gap-[3px]">
      <PenLine className="size-[20px]" />
      <span
        className="block w-[16px] rounded-full bg-current"
        style={{ height: Math.max(1, Math.round(width * 0.6)) }}
      />
    </span>
  );
}

/**
 * Grid paper, favourite, rename and delete, off the toolbar.
 *
 * The grid is a property of the sheet and changes rarely; as a toggle in the bar it was inverted
 * like a second selected tool. A menu is an overlay, so opening it holds the firmware pen down:
 * the items take their taps instead of the sheet taking a stroke through them.
 */
export function NoteMenu({
  grid,
  gridDisabled,
  favorite,
  busy,
  onGridChange,
  onToggleFavorite,
  onRename,
  onDelete,
}: {
  grid: boolean;
  gridDisabled: boolean;
  favorite: boolean;
  busy: boolean;
  onGridChange: (grid: boolean) => void;
  onToggleFavorite: () => void;
  onRename: () => void;
  onDelete: () => void;
}) {
  const overlay = useOverlayInkSuppression();
  const item =
    "flex cursor-pointer items-center gap-2 rounded-lg px-3 py-2.5 text-sm outline-none data-[disabled]:opacity-50 data-[highlighted]:bg-accent";
  return (
    <Menu.Root onOpenChange={refreshPanelAfterClose(overlay)}>
      <Menu.Trigger
        render={
          // Black on e-ink like every other tool in the bar: a grey icon dithers on the panel.
          <ToolbarButton
            aria-label="Note options"
            className="text-muted-foreground eink:text-foreground"
          />
        }
      >
        <MoreHorizontal />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Positioner sideOffset={6} align="end">
          <Menu.Popup className="z-50 min-w-44 rounded-xl border bg-popover p-1 text-popover-foreground shadow-panel outline-none">
            <Menu.CheckboxItem
              checked={grid}
              disabled={gridDisabled}
              closeOnClick
              onCheckedChange={(checked) => onGridChange(checked)}
              className={item}
            >
              <Grid2X2 className="size-4" />
              Grid paper
              <Menu.CheckboxItemIndicator className="ml-auto">
                <Check className="size-4" />
              </Menu.CheckboxItemIndicator>
            </Menu.CheckboxItem>
            <Menu.Item className={item} onClick={onToggleFavorite}>
              <Star className={cn("size-4", favorite && "fill-current text-primary")} />
              {favorite ? "Remove from favorites" : "Add to favorites"}
            </Menu.Item>
            <Menu.Item className={item} onClick={onRename}>
              <Pencil className="size-4" />
              Rename
            </Menu.Item>
            <Menu.Item disabled={busy} className={cn(item, "text-destructive")} onClick={onDelete}>
              <Trash2 className="size-4" />
              Delete note
            </Menu.Item>
          </Menu.Popup>
        </Menu.Positioner>
      </Menu.Portal>
    </Menu.Root>
  );
}
