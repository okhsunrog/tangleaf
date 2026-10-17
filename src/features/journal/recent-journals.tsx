import { formatJournalDate, pageDisplayTitle } from "./journal-date";
import type { Page } from "@/lib/api";
import { cn } from "@/lib/utils";
import {
  dispositionFromShiftKey,
  type OpenDisposition,
} from "@/features/workspace/workspace-model";

type Props = {
  pages: Page[];
  activeUuid: string | null;
  busy: boolean;
  onOpen: (page: Page, disposition?: OpenDisposition) => void | Promise<void>;
};

export function RecentJournals({ pages, activeUuid, busy, onOpen }: Props) {
  if (pages.length === 0) return null;

  return (
    <section aria-label="Recent entries">
      <h3 className="mb-2 text-xs font-medium text-muted-foreground">Recent entries</h3>
      <div className="flex flex-wrap gap-1">
        {pages.map((page) => (
          <button
            key={page.uuid}
            type="button"
            disabled={busy}
            onClick={(event) => void onOpen(page, dispositionFromShiftKey(event.shiftKey))}
            className={cn(
              "flex min-w-0 items-center rounded-md border border-transparent px-2 py-1.5 text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-primary disabled:pointer-events-none disabled:opacity-50",
              activeUuid === page.uuid && "border-primary/20 bg-primary/8 text-primary",
            )}
            title={pageDisplayTitle(page)}
            aria-label={`Open journal ${pageDisplayTitle(page)}`}
          >
            {page.kind.kind === "journal" ? (
              <span className="text-xs leading-none font-medium tabular-nums">
                {formatJournalDate(page.kind.date, { day: "numeric", month: "short" })}
              </span>
            ) : (
              <span className="max-w-full truncate text-[10px] eink:text-xs">
                {pageDisplayTitle(page)}
              </span>
            )}
          </button>
        ))}
      </div>
    </section>
  );
}
