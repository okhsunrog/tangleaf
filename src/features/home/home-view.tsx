import { useEffect, useRef, useState } from "react";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { FilePlus2, Library } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useCompactLayout } from "@/app/use-compact-layout";
import { JournalEntry } from "@/features/journal/journal-entry";
import { JournalCalendar } from "@/features/journal/journal-calendar";
import { formatJournalDate, todayJournalDate } from "@/features/journal/journal-date";
import { DocumentReadingSurface, pageDocumentToPersisted } from "@/features/document/document-page";
import { useWorkspaceStore } from "@/features/workspace/workspace-store";
import { useWorkspaceController } from "@/features/workspace/workspace-controller";
import { dispositionFromShiftKey, type PaneId } from "@/features/workspace/workspace-model";
import { getPageDocument, listJournals, type JournalDate, type Page } from "@/lib/api";
import { queryKeys } from "@/lib/query";

export function HomeView({ paneId }: { paneId: PaneId }) {
  const root = useRef<HTMLElement>(null);
  const navigationRequest = useWorkspaceStore(
    (state) => state.panes[paneId]?.navigationRequest ?? 0,
  );
  useEffect(() => {
    const scroll = root.current?.closest<HTMLElement>("[data-workspace-scroll]");
    if (scroll) scroll.scrollTop = 0;
    root.current
      ?.querySelector<HTMLElement>("[contenteditable=true]")
      ?.focus({ preventScroll: true });
  }, [navigationRequest]);
  const [today, setToday] = useState(todayJournalDate);
  const controller = useWorkspaceController();
  const compact = useCompactLayout();
  useEffect(() => {
    const update = () => setToday(todayJournalDate());
    const timer = window.setInterval(update, 30_000);
    window.addEventListener("focus", update);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", update);
    };
  }, []);
  const journals = useInfiniteQuery({
    queryKey: [...queryKeys.journals, "stream", today],
    initialPageParam: today,
    queryFn: ({ pageParam }) => listJournals({ beforeDate: pageParam, limit: 7 }),
    getNextPageParam: (last) =>
      last.length === 7 && last[last.length - 1]?.kind.kind === "journal"
        ? (last[last.length - 1]!.kind as { kind: "journal"; date: JournalDate }).date
        : undefined,
  });
  return (
    <article
      ref={root}
      className="journal-stream mx-auto max-w-[48rem] px-6 pt-7 pb-20 sm:px-12 sm:pt-10"
    >
      {compact && (
        <nav aria-label="Notes" className="mb-7 flex gap-2">
          <Button variant="ghost" size="sm" onClick={() => controller.openAllNotes()}>
            <Library className="size-4" />
            All notes
          </Button>
          <Button variant="ghost" size="sm" onClick={() => void controller.createNewNote()}>
            <FilePlus2 className="size-4" />
            New note
          </Button>
        </nav>
      )}
      <section aria-label="Today's journal" className="journal-today">
        <header className="mb-8 flex items-center justify-between gap-3">
          <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
            <h1 className="text-2xl font-semibold tracking-tight">Today</h1>
            <time dateTime={today} className="text-sm text-muted-foreground">
              {formatJournalDate(today, { weekday: "long", day: "numeric", month: "long" })}
            </time>
          </div>
          {compact && (
            <JournalCalendar
              activeDate={today}
              onOpenDate={(date, disposition) => controller.openJournal(date, disposition)}
              className="size-11"
            />
          )}
        </header>
        <JournalEntry key={today} date={today} paneId={paneId} autoFocus />
      </section>
      {journals.data?.pages.flat().map((page) => (
        <EarlierJournal key={page.uuid} page={page} />
      ))}
      {journals.error && (
        <p role="alert" className="py-6 text-sm">
          Could not load earlier entries.{" "}
          <button className="underline" onClick={() => void journals.refetch()}>
            Retry
          </button>
        </p>
      )}
      {journals.hasNextPage && (
        <Button
          variant="ghost"
          className="mt-8"
          disabled={journals.isFetchingNextPage}
          onClick={() => void journals.fetchNextPage()}
        >
          {journals.isFetchingNextPage ? "Loading…" : "Earlier entries"}
        </Button>
      )}
    </article>
  );
}

function EarlierJournal({ page }: { page: Page }) {
  const controller = useWorkspaceController();
  const document = useQuery({
    queryKey: queryKeys.pageDocument(page.uuid),
    queryFn: () => getPageDocument(page.uuid),
  });
  if (page.kind.kind !== "journal") return null;
  const date = page.kind.date;
  return (
    <section
      className="journal-earlier mt-8 border-t border-border/50 pt-8"
      aria-label={formatJournalDate(date)}
    >
      <h2 className="mb-5 text-lg font-medium tracking-tight">
        <button
          className="rounded-md text-left hover:text-primary focus-visible:outline-ring"
          onClick={(event) =>
            void controller.openJournal(date, dispositionFromShiftKey(event.shiftKey))
          }
          title="Open journal for editing"
        >
          {formatJournalDate(date, {
            weekday: "long",
            day: "numeric",
            month: "long",
            year: "numeric",
          })}
        </button>
      </h2>
      {document.isPending ? (
        <p className="text-sm text-muted-foreground">Loading entry…</p>
      ) : document.data ? (
        <DocumentReadingSurface
          pageUuid={page.uuid}
          markdown={pageDocumentToPersisted(document.data).buffer}
          onOpenMarkdownLink={controller.openMarkdownLink}
        />
      ) : (
        <p role="alert">
          Could not load entry.{" "}
          <button className="underline" onClick={() => void document.refetch()}>
            Retry
          </button>
        </p>
      )}
    </section>
  );
}
