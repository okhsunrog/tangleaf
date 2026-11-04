import { useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { DocumentPage } from "@/features/document/document-page";
import type { DocumentEditorSession } from "@/features/document/continuous-document-editor";
import { Outliner } from "@/features/outliner/outliner";
import { usePageFind } from "@/features/pages/page-find";
import { usePageWriterLease } from "@/features/pages/page-session";
import { useWorkspaceController } from "@/features/workspace/workspace-controller";
import { useWorkspaceStore } from "@/features/workspace/workspace-store";
import type { PaneId } from "@/features/workspace/workspace-model";
import {
  createBlock,
  ensureJournal,
  getJournal,
  getPageDocument,
  type JournalDate,
  type Page,
} from "@/lib/api";
import { queryKeys } from "@/lib/query";
import { notifyError } from "@/lib/notify";

/** Journal dates use the same persisted layout and editor as ordinary pages. */
export function JournalEntry({
  date,
  paneId,
  autoFocus = false,
}: {
  date: JournalDate;
  paneId: PaneId;
  autoFocus?: boolean;
}) {
  const journal = useQuery({ queryKey: queryKeys.journal(date), queryFn: () => getJournal(date) });
  const client = useQueryClient();
  const [starting, setStarting] = useState(false);
  const startingRef = useRef(false);
  const [error, setError] = useState(false);
  const canStart = usePageWriterLease(`journal-draft:${date}`, paneId, !journal.data || starting);
  const active = useWorkspaceStore((state) => state.activePaneId === paneId);
  const editorSession = useRef<DocumentEditorSession>({
    state: null,
    focused: false,
    initialCursor: "end",
  }).current;
  const start = async () => {
    if (!canStart || startingRef.current) return;
    startingRef.current = true;
    setStarting(true);
    setError(false);
    try {
      const page = await ensureJournal(date);
      const snapshot = await getPageDocument(page.uuid);
      if (!snapshot) throw new Error("Journal is no longer available");
      if (snapshot.blocks.length === 0) {
        await createBlock({ pageUuid: page.uuid, parentUuid: null, afterUuid: null });
      }
      client.setQueryData(queryKeys.page(page.uuid), page);
      client.setQueryData(queryKeys.journal(date), page);
      await client.invalidateQueries({ queryKey: queryKeys.pageRender(page.uuid) });
      void client.invalidateQueries({ queryKey: queryKeys.journals });
    } catch (error) {
      setError(true);
      notifyError("start journal", error);
    } finally {
      startingRef.current = false;
      setStarting(false);
    }
  };
  if (journal.isPending)
    return <p className="py-8 text-sm text-muted-foreground">Loading journal…</p>;
  if (journal.error && !journal.data)
    return (
      <p role="alert">
        Could not load journal. <button onClick={() => void journal.refetch()}>Retry</button>
      </p>
    );
  if (journal.data && !starting)
    return (
      <ExistingJournal
        page={journal.data}
        paneId={paneId}
        autoFocus={autoFocus}
        editorSession={editorSession}
      />
    );
  // Opening an empty date does not write anything. Explicitly starting it creates
  // a normal first block before accepting text, so no draft/editor handoff can
  // normalize an unfinished line or discard its selection and local history.
  return (
    <div className="py-1">
      {error && (
        <p role="alert" className="mb-3 text-sm text-destructive">
          Could not start the journal. Try again.
        </p>
      )}
      <button
        type="button"
        disabled={!canStart || starting}
        autoFocus={autoFocus && active}
        onClick={() => void start()}
        className="rounded-md px-2 py-1 text-sm text-muted-foreground hover:bg-accent hover:text-foreground"
      >
        {starting ? "Opening journal…" : "Start writing"}
      </button>
      {!canStart && (
        <p className="mt-2 text-xs text-muted-foreground">
          This journal is being edited in another pane.
        </p>
      )}
    </div>
  );
}

function ExistingJournal({
  page,
  paneId,
  autoFocus,
  editorSession,
}: {
  page: Page;
  paneId: PaneId;
  autoFocus: boolean;
  editorSession: DocumentEditorSession;
}) {
  const ownsWriter = usePageWriterLease(page.uuid, paneId, true);
  const active = useWorkspaceStore((state) => state.activePaneId === paneId);
  const controller = useWorkspaceController();
  const root = useRef<HTMLDivElement>(null);
  const find = usePageFind(page, paneId, root, 0, { label: "Find in journal entry" });
  const focusRequest = autoFocus && active ? 1 : 0;
  return (
    <div ref={root}>
      {find.bar}
      {!ownsWriter && (
        <p className="mb-4 text-xs text-muted-foreground">
          This journal is being edited in another pane.
        </p>
      )}
      <div className="editor-body">
        {page.layout === "outline" ? (
          <Outliner
            page={page}
            readOnly={!ownsWriter}
            focusRequest={focusRequest}
            focusEdge="last"
            targetBlockUuid={find.navigation?.match.blockUuid}
            navigationRequest={find.navigation?.request}
            onOpenMarkdownLink={controller.openMarkdownLink}
          />
        ) : (
          <DocumentPage
            findRange={
              find.navigation
                ? { ...find.navigation.match, request: find.navigation.request }
                : null
            }
            pageUuid={page.uuid}
            editing={ownsWriter}
            canEdit={ownsWriter}
            authoringMode="live_preview"
            focusRequest={focusRequest}
            onOpenMarkdownLink={controller.openMarkdownLink}
            editorSession={editorSession}
          />
        )}
      </div>
    </div>
  );
}
