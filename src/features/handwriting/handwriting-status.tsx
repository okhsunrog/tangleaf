import { useState, type ComponentType } from "react";
import { CloudAlert, FileWarning, GitBranch, PenOff, TriangleAlert } from "lucide-react";
import { requestFullRefreshSoon } from "@/app/eink-refresh";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";

/**
 * What the note has to tell the writer, as one chip in the toolbar.
 *
 * These used to be banners above the sheet and a footer below it. Under a sheet fitted to the
 * screen each of them rescaled the whole sheet when it appeared, and one appearing while another
 * went away moved the sheet without resizing anything the pen session observes. In the toolbar
 * they change only the width of the title.
 */
export type NoteStatus =
  | { kind: "save-error"; message: string; onRetry: () => void }
  | { kind: "title-conflict"; onUseRemote: () => void; onKeepLocal: () => void }
  | { kind: "versions"; count: number; compareDisabled: boolean; onCompare: () => void }
  | { kind: "sheet-full" }
  | { kind: "pen-error"; message: string };

export type NoteStatusKind = NoteStatus["kind"];

const PRIORITY: readonly NoteStatusKind[] = [
  "save-error",
  "title-conflict",
  "versions",
  "sheet-full",
  "pen-error",
];

/** Most urgent first: a failing save outranks everything, a pen fallback is the least urgent. */
export function byPriority(statuses: readonly NoteStatus[]): NoteStatus[] {
  return [...statuses].sort((a, b) => PRIORITY.indexOf(a.kind) - PRIORITY.indexOf(b.kind));
}

export function statusLabel(status: NoteStatus): string {
  switch (status.kind) {
    case "save-error":
      return "Not saved";
    case "title-conflict":
      return "Title conflict";
    case "versions":
      return `${status.count} versions`;
    case "sheet-full":
      return "Sheet full";
    case "pen-error":
      return "Pen input";
  }
}

/** An alert inverts on e-ink; everything else is outlined. */
function isAlert(status: NoteStatus): boolean {
  return status.kind === "save-error" || status.kind === "title-conflict";
}

const ICONS: Record<NoteStatusKind, ComponentType<{ className?: string }>> = {
  "save-error": TriangleAlert,
  "title-conflict": FileWarning,
  versions: GitBranch,
  "sheet-full": CloudAlert,
  "pen-error": PenOff,
};

type Props = {
  statuses: readonly NoteStatus[];
  /** Changes stored here but not yet published. Plain text: there is nothing to do about it. */
  unsent: boolean;
  /** A narrow toolbar shows the chip's icon only; `aria-label` carries the label either way. */
  iconOnly?: boolean;
};

/**
 * The chip shows the most urgent status. Its popover lists every current status with its full
 * text and actions, and pauses the pen while it is open, like any popover.
 */
export function HandwritingStatus({ statuses, unsent, iconOnly = false }: Props) {
  const [open, setOpen] = useState(false);
  const sorted = byPriority(statuses);
  const top = sorted[0];
  // The chip goes away with its last status; a status arriving later must not find it open.
  if (!top && open) setOpen(false);

  if (!top) {
    return unsent ? (
      <span className="shrink-0 text-xs whitespace-nowrap text-muted-foreground">Unsent</span>
    ) : null;
  }

  const Icon = ICONS[top.kind];
  const label = statusLabel(top);
  const more = sorted.length - 1;
  // Closing through an action sets `open` directly, which the popover's own close hook never
  // hears; the details sit over the top of the sheet, so the panel gets its refresh here.
  const act = (action: () => void) => () => {
    setOpen(false);
    requestFullRefreshSoon();
    action();
  };

  return (
    <span className="flex shrink-0" role={top.kind === "save-error" ? "alert" : undefined}>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger
          data-ink-status
          data-alert={isAlert(top) || undefined}
          // Named in words either way: the visible "+1" would otherwise run into the label.
          aria-label={`${label}${more > 0 ? `, ${more} more` : ""}`}
          className={cn(
            "flex h-[28px] max-w-[160px] items-center gap-1 rounded-md border px-2 text-xs font-medium",
            top.kind === "save-error"
              ? "border-destructive/40 bg-destructive/10 text-destructive"
              : isAlert(top) || top.kind === "versions"
                ? "border-amber-500/40 bg-amber-500/10 text-foreground"
                : "text-foreground",
          )}
        >
          <Icon className="size-4 shrink-0" />
          {!iconOnly && <span className="truncate">{label}</span>}
          {!iconOnly && more > 0 && <span className="shrink-0">+{more}</span>}
        </PopoverTrigger>
        <PopoverContent
          align="end"
          data-ink-status-details
          className="w-[min(322px,calc(100vw-24px))] space-y-3 p-3 text-sm"
        >
          {sorted.map((status) => (
            <StatusDetails key={status.kind} status={status} act={act} />
          ))}
          {unsent && (
            <p className="text-xs text-muted-foreground">Some changes have not been sent yet.</p>
          )}
        </PopoverContent>
      </Popover>
    </span>
  );
}

function StatusDetails({
  status,
  act,
}: {
  status: NoteStatus;
  act: (action: () => void) => () => void;
}) {
  switch (status.kind) {
    case "save-error":
      return (
        <section className="space-y-2">
          <p className="text-destructive">Could not save your changes. {status.message}</p>
          <Button type="button" size="sm" onClick={act(status.onRetry)}>
            Retry saving
          </Button>
        </section>
      );
    case "title-conflict":
      return (
        <section className="space-y-2">
          <p>This title changed on another replica. Choose which version to keep.</p>
          <div className="flex gap-2">
            <Button type="button" variant="outline" size="sm" onClick={act(status.onUseRemote)}>
              Use remote
            </Button>
            <Button type="button" size="sm" onClick={act(status.onKeepLocal)}>
              Keep mine
            </Button>
          </div>
        </section>
      );
    case "versions":
      return (
        <section className="space-y-2">
          <p>This note has {status.count} versions from other devices</p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={status.compareDisabled}
            onClick={act(status.onCompare)}
          >
            Compare
          </Button>
        </section>
      );
    case "sheet-full":
      return (
        <p>
          This sheet is full. Your completed strokes are kept; erase handwriting or undo to make
          room.
        </p>
      );
    case "pen-error":
      return <p>Fast pen input is unavailable: {status.message}</p>;
  }
}
