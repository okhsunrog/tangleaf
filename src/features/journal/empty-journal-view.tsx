import type { JournalDate } from "@/lib/api";
import { formatJournalDate } from "./journal-date";
import { JournalEntry } from "./journal-entry";
import type { PaneId } from "@/features/workspace/workspace-model";

type Props = {
  date: JournalDate;
  paneId: PaneId;
};
export function EmptyJournalView({ date, paneId }: Props) {
  return (
    <article className="journal-stream mx-auto min-h-full max-w-[48rem] px-6 py-10 sm:px-12">
      <header className="mb-8">
        <h1 className="text-2xl font-semibold tracking-tight">{formatJournalDate(date)}</h1>
      </header>
      <JournalEntry key={date} date={date} paneId={paneId} autoFocus />
    </article>
  );
}
