import { useCallback, useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { DocumentPage, pageDocumentToPersisted } from "@/features/document/document-page";
import {
  ContinuousDocumentEditor,
  type DocumentEditorSession,
} from "@/features/document/continuous-document-editor";
import { usePageFind } from "@/features/pages/page-find";
import { DocumentCodec } from "@/features/document/document-codec";
import { documentUnitsForSave } from "@/features/document/document-save-model";
import {
  useDocumentDraftOverlay,
  usePageSessionRegistry,
  usePageWriterLease,
} from "@/features/pages/page-session";
import { useWorkspaceController } from "@/features/workspace/workspace-controller";
import { useWorkspaceStore } from "@/features/workspace/workspace-store";
import type { PaneId } from "@/features/workspace/workspace-model";
import {
  CommandFailure,
  ensureJournal,
  getJournal,
  getPageDocument,
  replacePageDocument,
  type JournalDate,
  type Page,
  type PageDocumentSnapshot,
} from "@/lib/api";
import { queryKeys } from "@/lib/query";
import { DebouncedAction } from "@/lib/debounced-action";
import { notifyError } from "@/lib/notify";

/** One editor for both the journal stream and a date opened from the calendar. */
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
  const pendingDraft = useDocumentDraftOverlay(`journal-draft:${date}`);
  const [committed, setCommitted] = useState(false);
  const onCreated = useCallback(() => setCommitted(true), []);
  // Once typing starts, a background journal refetch must not replace the draft editor.
  const [drafting, setDrafting] = useState(false);
  const editorSession = useRef<DocumentEditorSession>({
    state: null,
    focused: false,
    initialCursor: "end",
  }).current;
  const page = journal.data;
  if (journal.isPending)
    return <p className="py-8 text-sm text-muted-foreground">Loading journal…</p>;
  if (journal.error && !page && !pendingDraft)
    return (
      <p role="alert">
        Could not load journal. <button onClick={() => void journal.refetch()}>Retry</button>
      </p>
    );
  return page && ((!drafting && !pendingDraft) || committed) ? (
    <ExistingJournal
      page={page}
      paneId={paneId}
      autoFocus={autoFocus}
      editorSession={editorSession}
    />
  ) : (
    <FirstEntry
      date={date}
      paneId={paneId}
      autoFocus={autoFocus}
      editorSession={editorSession}
      onTyping={() => {
        setDrafting(true);
        setCommitted(false);
      }}
      onCreated={onCreated}
    />
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
  const find = usePageFind({ ...page, layout: "document" }, paneId, root, 0, {
    label: "Find in journal entry",
  });
  return (
    <div ref={root}>
      {find.bar}
      {!ownsWriter && (
        <p className="mb-4 text-xs text-muted-foreground">
          This journal is being edited in another pane.
        </p>
      )}
      <div className="editor-body">
        <DocumentPage
          findRange={
            find.navigation ? { ...find.navigation.match, request: find.navigation.request } : null
          }
          pageUuid={page.uuid}
          editing={ownsWriter}
          canEdit={ownsWriter}
          authoringMode="live_preview"
          focusRequest={autoFocus && active ? 1 : 0}
          onOpenMarkdownLink={controller.openMarkdownLink}
          editorSession={editorSession}
        />
      </div>
    </div>
  );
}

const EMPTY_DOCUMENT = {
  buffer: "",
  sourceMap: DocumentCodec.encode([]).sourceMap,
  revision: "",
} as const;

function FirstEntry({
  date,
  paneId,
  autoFocus,
  editorSession,
  onTyping,
  onCreated,
}: {
  date: JournalDate;
  paneId: PaneId;
  autoFocus: boolean;
  editorSession: DocumentEditorSession;
  onTyping: () => void;
  onCreated: (page: Page) => void;
}) {
  const sessions = usePageSessionRegistry();
  const draftKey = `journal-draft:${date}`;
  const overlay = useDocumentDraftOverlay(draftKey);
  const canEdit = usePageWriterLease(draftKey, paneId, true);
  const active = useWorkspaceStore((state) => state.activePaneId === paneId);
  const client = useQueryClient();
  const autosave = useRef(new DebouncedAction()).current;
  const flight = useRef<Promise<boolean> | null>(null);
  const composing = useRef(false);
  const lastValue = useRef(overlay?.draft ?? "");
  const root = useRef<HTMLDivElement>(null);
  const find = usePageFind({ uuid: draftKey, layout: "document" }, paneId, root, 0, {
    draft: overlay?.draft ?? lastValue.current,
    label: "Find in journal entry",
  });
  const durable = useRef<{ page: Page; snapshot: PageDocumentSnapshot; prefix: string } | null>(
    null,
  );
  const [status, setStatus] = useState("");
  const mounted = useRef(true);
  const flush = useCallback(async (): Promise<boolean> => {
    autosave.cancel();
    if (flight.current) return flight.current;
    const pending = (async () => {
      let token: ReturnType<typeof sessions.createWriterLeaseToken> | null = null;
      try {
        while (true) {
          const draft = sessions.getSnapshot(draftKey).document;
          if (!draft || (!durable.current && !draft.draft.trim())) return true;
          if (mounted.current) setStatus("Saving…");
          if (!durable.current) {
            const page = await ensureJournal(date);
            const snapshot = await getPageDocument(page.uuid);
            if (!snapshot) throw new Error("Journal is no longer available");
            const existing = pageDocumentToPersisted(snapshot).buffer;
            durable.current = { page, snapshot, prefix: existing ? `${existing}\n\n` : "" };
          }
          const current = durable.current;
          token ??= sessions.createWriterLeaseToken();
          if (!sessions.acquireWriter(current.page.uuid, paneId, token))
            throw new Error(
              "This journal is being edited in another pane. Close that editor and retry.",
            );
          const attempt = sessions.beginDocumentSave(draftKey);
          if (!attempt) return false;
          try {
            const persisted = pageDocumentToPersisted(current.snapshot);
            const next = await replacePageDocument(
              current.page.uuid,
              current.snapshot.revision,
              documentUnitsForSave(current.prefix + attempt.draft, persisted.sourceMap),
            );
            current.snapshot = next;
            sessions.acknowledgeDocumentSave(draftKey, attempt, pageDocumentToPersisted(next));
            client.setQueryData(queryKeys.pageDocument(current.page.uuid), next);
          } catch (error) {
            sessions.failDocumentSave(draftKey, attempt);
            // Use the normal document conflict UI instead of retrying a stale revision forever.
            if (
              error instanceof CommandFailure &&
              error.code === "conflict" &&
              !composing.current
            ) {
              const latest = await getPageDocument(current.page.uuid);
              if (latest) {
                const local = sessions.getSnapshot(draftKey).document?.draft ?? attempt.draft;
                sessions.editDocument(
                  current.page.uuid,
                  current.prefix + local,
                  pageDocumentToPersisted(current.snapshot),
                );
                sessions.acceptDocumentSnapshot(current.page.uuid, pageDocumentToPersisted(latest));
                client.setQueryData(queryKeys.pageDocument(current.page.uuid), latest);
                client.setQueryData(queryKeys.journal(date), current.page);
                sessions.discardPage(draftKey);
                if (mounted.current) onCreated(current.page);
                return false;
              }
            }
            throw error;
          }
          if (sessions.getSnapshot(draftKey).document) continue;
          client.setQueryData(queryKeys.journal(date), current.page);
          client.setQueryData(queryKeys.page(current.page.uuid), current.page);
          void client.invalidateQueries({ queryKey: queryKeys.journals });
          void client.invalidateQueries({ queryKey: queryKeys.history });
          if (mounted.current && !composing.current) onCreated(current.page);
          return true;
        }
      } catch (error) {
        if (mounted.current) setStatus("Could not save. Your draft is kept here.");
        notifyError("journal save", error);
        return false;
      } finally {
        if (token && durable.current)
          sessions.releaseWriter(durable.current.page.uuid, paneId, token);
      }
    })();
    flight.current = pending;
    try {
      return await pending;
    } finally {
      if (flight.current === pending) flight.current = null;
    }
  }, [autosave, client, date, draftKey, onCreated, paneId, sessions]);
  const flushRef = useRef(flush);
  flushRef.current = flush;
  useEffect(() => sessions.registerFlush(() => flushRef.current()), [sessions]);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      autosave.cancel();
      void flushRef.current();
    };
  }, [autosave]);
  const update = (value: string, isComposing: boolean) => {
    if (!canEdit) return;
    composing.current = isComposing;
    lastValue.current = value;
    onTyping();
    sessions.editDocument(draftKey, value, EMPTY_DOCUMENT);
    if (isComposing) autosave.cancel();
    else autosave.schedule(() => void flush(), 400);
  };
  return (
    <div ref={root} className="relative">
      {find.bar}
      <div role="status" className="absolute -top-6 right-0 text-xs text-muted-foreground">
        {status}
        {status.startsWith("Could") && (
          <button className="ml-2 underline" onClick={() => void flush()}>
            Retry
          </button>
        )}
      </div>
      {!overlay?.draft && (
        <p className="pointer-events-none absolute top-1 text-base text-muted-foreground">
          Write a thought…
        </p>
      )}
      <div className="editor-body">
        <ContinuousDocumentEditor
          findRange={
            find.navigation ? { ...find.navigation.match, request: find.navigation.request } : null
          }
          value={overlay?.draft ?? lastValue.current}
          readOnly={!canEdit}
          focusRequest={autoFocus && active ? 1 : 0}
          editorSession={editorSession}
          onChange={update}
          onCompositionEnd={(value) => update(value, false)}
          onBlur={() => void flush()}
        />
      </div>
    </div>
  );
}
