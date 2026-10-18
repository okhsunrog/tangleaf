import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { JournalDate } from "@/lib/api";
import { formatJournalDate, shiftJournalDate } from "./journal-date";
import { JournalEntry } from "./journal-entry";
import {
  dispositionFromShiftKey,
  type OpenDisposition,
  type PaneId,
} from "@/features/workspace/workspace-model";

type Props = {
  date: JournalDate;
  paneId: PaneId;
  busy: boolean;
  onOpenDate: (date: JournalDate, disposition?: OpenDisposition) => void | Promise<void>;
};
export function EmptyJournalView({ date, paneId, busy, onOpenDate }: Props) {
  return (
    <article className="journal-stream mx-auto min-h-full max-w-[48rem] px-6 py-10 sm:px-12">
      <header className="mb-8 flex items-start justify-between gap-4">
        <h1 className="text-2xl font-semibold tracking-tight">{formatJournalDate(date)}</h1>
        <div className="flex">
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label="Open previous journal day"
            disabled={busy}
            onClick={(event) =>
              void onOpenDate(shiftJournalDate(date, -1), dispositionFromShiftKey(event.shiftKey))
            }
          >
            <ChevronLeft className="size-4" />
          </Button>
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label="Open next journal day"
            disabled={busy}
            onClick={(event) =>
              void onOpenDate(shiftJournalDate(date, 1), dispositionFromShiftKey(event.shiftKey))
            }
          >
            <ChevronRight className="size-4" />
          </Button>
        </div>
      </header>
      <JournalEntry key={date} date={date} paneId={paneId} autoFocus />
    </article>
  );
}
