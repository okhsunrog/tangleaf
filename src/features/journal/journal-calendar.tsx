import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { DayButton, DayPicker, type DayButtonProps } from "@daypicker/react";
import { CalendarDays } from "lucide-react";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { listJournals, type JournalDate } from "@/lib/api";
import { queryKeys } from "@/lib/query";
import { cn } from "@/lib/utils";
import {
  dispositionFromShiftKey,
  type OpenDisposition,
} from "@/features/workspace/workspace-model";
import {
  formatJournalDate,
  journalDateFromLocalDate,
  journalDateToLocalDate,
  todayJournalDate,
} from "./journal-date";
import "./journal-calendar.css";

type Props = {
  activeDate: JournalDate | null;
  busy?: boolean;
  onOpenDate: (date: JournalDate, disposition?: OpenDisposition) => void | Promise<void>;
  className?: string;
};

export function JournalCalendar({ activeDate, busy = false, onOpenDate, className }: Props) {
  const [open, setOpen] = useState(false);
  const today = todayJournalDate();
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        disabled={busy}
        aria-label="Choose journal date"
        className={cn(
          "flex size-9 shrink-0 items-center justify-center rounded-xl text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-2 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-50 data-[popup-open]:bg-primary/10 data-[popup-open]:text-primary",
          className,
        )}
      >
        <CalendarDays className="size-4" />
      </PopoverTrigger>
      <PopoverContent
        align="end"
        collisionAvoidance={{ side: "shift", align: "shift", fallbackAxisSide: "none" }}
        aria-label="Journal calendar"
        className="max-h-[var(--available-height)] w-[min(21rem,calc(100vw-1.5rem))] overflow-y-auto rounded-2xl p-3"
        initialFocus={false}
      >
        {open && (
          <JournalCalendarMonth
            activeDate={activeDate && activeDate <= today ? activeDate : today}
            today={today}
            busy={busy}
            onChoose={(date, disposition) => {
              setOpen(false);
              void onOpenDate(date, disposition);
            }}
          />
        )}
      </PopoverContent>
    </Popover>
  );
}

function JournalCalendarMonth({
  activeDate,
  today,
  busy,
  onChoose,
}: {
  activeDate: JournalDate;
  today: JournalDate;
  busy: boolean;
  onChoose: (date: JournalDate, disposition: OpenDisposition) => void;
}) {
  const [month, setMonth] = useState(() => journalDateToLocalDate(`${activeDate.slice(0, 7)}-01`));
  const monthStart = journalDateFromLocalDate(month);
  const nextMonth = new Date(month);
  nextMonth.setMonth(nextMonth.getMonth() + 1, 1);
  const beforeDate = journalDateFromLocalDate(nextMonth);
  const entries = useQuery({
    queryKey: [...queryKeys.journals, "calendar", monthStart],
    // At most 31 unique journal dates can fall in a month. Filtering the bounded
    // result avoids fetching a whole multi-year journal index just for its calendar.
    queryFn: () => listJournals({ beforeDate, limit: 31 }),
  });
  const entryDates = new Set(
    (entries.data ?? []).flatMap((page) =>
      page.kind.kind === "journal" && page.kind.date >= monthStart && page.kind.date < beforeDate
        ? [page.kind.date]
        : [],
    ),
  );
  return (
    <>
      <DayPicker
        className="journal-calendar"
        aria-label="Journal dates"
        mode="single"
        required
        autoFocus
        month={month}
        onMonthChange={setMonth}
        endMonth={journalDateToLocalDate(today)}
        selected={journalDateToLocalDate(activeDate)}
        today={journalDateToLocalDate(today)}
        weekStartsOn={1}
        navLayout="around"
        fixedWeeks
        disabled={(date) => {
          const key = journalDateFromLocalDate(date);
          return busy || key > today || (key !== today && !entryDates.has(key));
        }}
        modifiers={{ entry: (date) => entryDates.has(journalDateFromLocalDate(date)) }}
        components={{ DayButton: JournalDayButton }}
        formatters={{
          formatCaption: (date) =>
            date.toLocaleDateString(undefined, { month: "long", year: "numeric" }),
          formatWeekdayName: (date) => date.toLocaleDateString(undefined, { weekday: "short" }),
        }}
        labels={{
          labelPrevious: () => "Previous month",
          labelNext: () => "Next month",
          labelDayButton: (date, modifiers) =>
            `${formatJournalDate(journalDateFromLocalDate(date))}${modifiers.today ? ", today" : ""}${modifiers.entry ? ", journal entry" : ", no entry"}`,
        }}
        onSelect={(date, _trigger, modifiers, event) => {
          if (!modifiers.disabled) {
            onChoose(journalDateFromLocalDate(date), dispositionFromShiftKey(event.shiftKey));
          }
        }}
      />
      <div className="mt-3 flex items-center gap-4 border-t border-border/60 px-1 pt-3 text-xs text-muted-foreground">
        <span className="flex items-center gap-1.5">
          <span className="size-2 rounded-full bg-primary" /> Entries
        </span>
        <button
          type="button"
          disabled={busy}
          className="ml-auto flex min-h-8 items-center gap-1.5 rounded-lg px-2 hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring disabled:opacity-50"
          onClick={(event) => onChoose(today, dispositionFromShiftKey(event.shiftKey))}
        >
          <span className="size-2 rounded-full border border-primary" /> Today
        </button>
      </div>
      {entries.isPending && (
        <p role="status" className="px-1 pt-2 text-xs text-muted-foreground">
          Loading entries…
        </p>
      )}
      {entries.isError && (
        <p role="alert" className="px-1 pt-2 text-xs text-muted-foreground">
          Could not load entries.{" "}
          <button type="button" className="underline" onClick={() => void entries.refetch()}>
            Retry
          </button>
        </p>
      )}
    </>
  );
}

function JournalDayButton(props: DayButtonProps) {
  return (
    <DayButton
      {...props}
      data-entry={props.modifiers.entry || undefined}
      data-today={props.modifiers.today || undefined}
      data-selected={props.modifiers.selected || undefined}
    />
  );
}
