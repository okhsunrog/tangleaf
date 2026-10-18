import { CalendarDays } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { JournalDate } from "@/lib/api";
import { formatJournalDate, todayJournalDate } from "./journal-date";
import { JournalCalendar } from "./journal-calendar";
import {
  dispositionFromShiftKey,
  type OpenDisposition,
} from "@/features/workspace/workspace-model";

type Props = {
  activeDate: JournalDate | null;
  busy: boolean;
  onOpenDate: (date: JournalDate, disposition?: OpenDisposition) => void | Promise<void>;
};

export function JournalNavigation({ activeDate, busy, onOpenDate }: Props) {
  const today = todayJournalDate();

  return (
    <section aria-labelledby="journal-navigation-title" aria-busy={busy} className="space-y-2">
      <div className="flex items-center justify-between px-1">
        <span
          id="journal-navigation-title"
          className="text-[11px] font-semibold tracking-wide text-muted-foreground"
        >
          Journal
        </span>
      </div>

      <div className="grid grid-cols-[1fr_auto] gap-1.5">
        <Button
          type="button"
          variant={activeDate === today ? "secondary" : "ghost"}
          disabled={busy}
          onClick={(event) => void onOpenDate(today, dispositionFromShiftKey(event.shiftKey))}
          className="h-9 justify-start rounded-xl px-2 shadow-none"
        >
          <span className="flex size-6 items-center justify-center rounded-lg bg-primary/10 text-primary">
            <CalendarDays className="size-3.5" />
          </span>
          <span className="text-xs font-medium">Today</span>
          <span className="ml-auto text-[10px] eink:text-xs text-muted-foreground">
            {formatJournalDate(today, { month: "short", day: "numeric" })}
          </span>
        </Button>
        <JournalCalendar activeDate={activeDate} busy={busy} onOpenDate={onOpenDate} />
      </div>
    </section>
  );
}
