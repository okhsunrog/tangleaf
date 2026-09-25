import { useCallback, useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, Lasso, PenLine, Redo2, RefreshCw, Undo2 } from "lucide-react";
import { useConfirmation } from "@/app/confirmation";
import { refreshPanelNow } from "@/app/eink-refresh";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent } from "@/components/ui/popover";
import { usePageNavigationStore } from "@/features/pages/page-navigation-store";
import { RenamePageDialog } from "@/features/pages/rename-page-dialog";
import { usePageTitleEditor } from "@/features/pages/use-page-title-editor";
import type { PaneId } from "@/features/workspace/workspace-model";
import {
  CommandFailure,
  completeHandwritingNote,
  handwritingHistory,
  handwritingNoteStatus,
  loadHandwritingNote,
  setHandwritingBackground,
  unknownErrorMessage,
  type Page,
} from "@/lib/api";
import type { InkDraft, InkHistorySnapshot } from "@/lib/bindings";
import { notifyError, notifyRetryableError } from "@/lib/notify";
import { queryKeys } from "@/lib/query";
import { cn } from "@/lib/utils";
import type { DraftSaveState } from "./draft-writer";
import { canDrawHandwriting, editorAccess, type EditorOwnership } from "./handwriting-access";
import { hasConflict, unsentChanges } from "./handwriting-conflict-model";
import { HandwritingConflictDialog } from "./handwriting-conflicts";
import { HandwritingStatus, type NoteStatus } from "./handwriting-status";
import { NoteMenu, ToolbarButton, ToolbarSeparator, useToolbarLayout } from "./handwriting-toolbar";
import { EraserTypeIcon } from "./eraser-icons";
import {
  ERASER_MENU_CLASS,
  EraserMenu,
  eraserMenuSideOffset,
  eraserMenuStyle,
  eraserMenuUnits,
} from "./eraser-menu";
import { useEraserSettings } from "./eraser-settings";
import {
  acquireEditor,
  awaitCompletion,
  beginSession,
  endSession,
  getWriter,
  releaseEditor,
  requestCompletion,
} from "./handwriting-session";
import { registerPaneLeaveGuard } from "@/features/workspace/pane-leave-guard";
import { useResolvedInkColor } from "@/app/appearance";
import { InkCanvas, type InkTool } from "./ink-canvas";
import type { LassoMode } from "./ink-editing";
import { applyHistoryUpdate } from "./ink-patch";
import { useHandwritingAvailability, useHandwritingPreference } from "./input-capabilities";
import type { OnyxInkStatus } from "./onyx-ink";
import { SHEET_WIDTH_CSS } from "./sheet-fit";

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLInputElement
  );
}

/** The note itself is gone, so nothing is left to store, pack or retry. */
function noteIsGone(error: Error | null): boolean {
  return error instanceof CommandFailure && error.code === "not_found";
}

/**
 * Completion outlives the editor: it is requested detached, retried from a
 * toast, and never treated as server confirmation of the publication.
 */
function completeInBackground(pageUuid: string) {
  requestCompletion(pageUuid, () => completeHandwritingNote(pageUuid)).catch((error: unknown) => {
    // The note was deleted while it was being packed; there is nothing to send.
    if (error instanceof CommandFailure && error.code === "not_found") return;
    notifyRetryableError("Could not prepare sync", error, () => completeInBackground(pageUuid));
  });
}

const TOOLS = [
  { id: "pen", label: "Pen" },
  { id: "eraser", label: "Eraser" },
  { id: "lasso", label: "Lasso" },
] as const satisfies readonly { id: InkTool; label: string }[];

/** What the eraser hands back to when it deselects itself: never the eraser. */
type DrawingTool = Exclude<InkTool, "eraser">;

type Props = {
  paneId: PaneId;
  page: Page;
  onSaved: (updated: Page) => void;
  onDelete: (page: Page) => void | Promise<void>;
  /**
   * Leaves the note. Given in sheet mode, where the pane bar with its Back is gone and the
   * toolbar is the only chrome on screen.
   */
  onBack?: () => void;
};

/**
 * A handwritten note in a workspace pane. It owns the note's ink session for as
 * long as it is mounted; the text editor is never mounted for this page kind.
 */
export function HandwritingNoteView({ paneId, page, onSaved, onDelete, onBack }: Props) {
  const monoInk = useResolvedInkColor() === "mono";
  const uuid = page.uuid;
  const { available, capabilities } = useHandwritingAvailability();
  const mouseEnabled = useHandwritingPreference((state) => state.mouseEnabled);
  const queryClient = useQueryClient();
  const favorite = usePageNavigationStore((state) => state.favoritePageUuids.includes(uuid));
  const toggleFavoritePage = usePageNavigationStore((state) => state.toggleFavoritePage);

  const [draft, setDraft] = useState<InkDraft | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveState, setSaveState] = useState<DraftSaveState>("saved");
  const [tool, setTool] = useState<InkTool>("pen");
  // The tool the eraser was picked from, for Auto-deselect. Stock skips the eraser itself too.
  const [previousTool, setPreviousTool] = useState<DrawingTool>("pen");
  const [width, setWidth] = useState(3);
  // The eraser's type and diameter are device settings (eraser-settings.ts); the tool is not, so
  // every note opens with the pen and the remembered eraser behind the Eraser button.
  const eraserType = useEraserSettings((state) => state.type);
  const eraserDiameter = useEraserSettings((state) => state.diameters[state.type]);
  const autoDeselect = useEraserSettings((state) => state.autoDeselect);
  const [lassoMode, setLassoMode] = useState<LassoMode>("free");
  const [selected, setSelected] = useState<string[]>([]);
  const [active, setActive] = useState(false);
  const [leaving, setLeaving] = useState(false);
  const [nativeStatus, setNativeStatus] = useState<OnyxInkStatus | null>(null);
  const [limit, setLimit] = useState(false);
  const [canUndo, setCanUndo] = useState(false);
  const [canRedo, setCanRedo] = useState(false);
  const [historyBusy, setHistoryBusy] = useState(false);
  const [ownership, setOwnership] = useState<EditorOwnership>("pending");
  // Input stays closed between a background transition and the re-read that
  // adopts whatever completion published while the window was hidden.
  const [suspended, setSuspended] = useState(false);
  const [conflictsOpen, setConflictsOpen] = useState(false);
  // Gestures an earlier mount could not store are shown and retried before any
  // snapshot from storage is allowed to replace them.
  const [recovering, setRecovering] = useState(false);
  const [comparing, setComparing] = useState(false);
  const [renaming, setRenaming] = useState(false);
  // Anchored to the tool button that was tapped, so the options sit under their own tool.
  const [optionsOpen, setOptionsOpen] = useState(false);
  const [optionsAnchor, setOptionsAnchor] = useState<HTMLElement | null>(null);
  // What the sheet and the pen erase with. The menu changes the stored settings as it is used;
  // they reach the eraser once it closes, as stock's reach the firmware only at the resume after
  // its menu. A reconfigure per tap would also pause and re-arm the pen for every step.
  const [appliedEraser, setAppliedEraser] = useState(() => ({
    type: eraserType,
    diameter: eraserDiameter,
  }));
  const confirm = useConfirmation();
  // Erase Page's confirmation takes focus and the pointer; the menu stays open under it.
  const confirmingRef = useRef(false);

  const latestDraft = useRef<InkDraft | null>(null);
  const historyBusyRef = useRef(false);
  const leavingRef = useRef(false);
  const ownerRef = useRef({});
  const generationRef = useRef(0);
  const suspendedRef = useRef(false);
  suspendedRef.current = suspended;
  // Whether the last read asked for an editable session, so a later upgrade is
  // distinguishable from an ordinary re-read.
  const openedForEditing = useRef(false);

  const access = editorAccess(ownership, canDrawHandwriting(available, mouseEnabled));
  const editing = access === "editable";
  const editingRef = useRef(editing);
  editingRef.current = editing;

  const statusQuery = useQuery({
    queryKey: queryKeys.handwritingStatus(uuid),
    queryFn: () => handwritingNoteStatus(uuid),
  });
  const status = statusQuery.data;

  const title = usePageTitleEditor(page, true, onSaved);
  const titleFlush = useRef(title.flush);
  titleFlush.current = title.flush;
  const busy = active || historyBusy || leaving;
  const busyRef = useRef(busy);
  busyRef.current = busy;
  const editingBusy = busy || suspended;

  const adopt = useCallback(
    (history: InkHistorySnapshot) => {
      const { snapshot } = history;
      if (editingRef.current) beginSession(uuid, snapshot, setSaveState);
      latestDraft.current = snapshot.draft;
      setDraft(snapshot.draft);
      setCanUndo(history.canUndo);
      setCanRedo(history.canRedo);
      setSaveState("saved");
      setSelected([]);
      setLimit(false);
      setSuspended(false);
      setRecovering(false);
    },
    [uuid],
  );

  /**
   * Read the note only after any completion this UUID still owes has settled,
   * so a session started here never continues from a superseded revision.
   */
  const openNote = useCallback(async () => {
    const generation = ++generationRef.current;
    openedForEditing.current = editingRef.current;
    setLoadError(null);
    try {
      const carried = getWriter(uuid);
      if (carried?.hasPending()) {
        // Adopt the queue of the mount that could not store it, show its newest
        // state, and retry. Loading waits until storage has accepted it.
        carried.setOnState(setSaveState);
        const queued = carried.latestDraft();
        if (queued) {
          latestDraft.current = queued;
          setDraft(queued);
        }
        setSuspended(false);
        setRecovering(true);
        if (!(await carried.flush())) return;
        if (generation !== generationRef.current) return;
      }
      await awaitCompletion(uuid);
      const history = await loadHandwritingNote(uuid, editingRef.current);
      if (generation !== generationRef.current) return;
      // A gesture completed while the note was being read is newer than it.
      if (getWriter(uuid)?.hasPending()) {
        setRecovering(true);
        return;
      }
      adopt(history);
    } catch (error) {
      if (generation !== generationRef.current) return;
      setLoadError(unknownErrorMessage(error));
    }
  }, [adopt, uuid]);

  // Once the carried gestures are stored, continue with the read they blocked.
  useEffect(() => {
    if (recovering && saveState === "saved") void openNote();
  }, [openNote, recovering, saveState]);

  // Two editors of one note are not a shared session; the second view reads.
  useEffect(() => {
    const owner = ownerRef.current;
    setOwnership(acquireEditor(uuid, owner) ? "owned" : "taken");
    return () => {
      releaseEditor(uuid, owner);
      setOwnership("pending");
    };
  }, [uuid]);

  useEffect(() => {
    if (ownership === "pending") return;
    void openNote();
    return () => {
      generationRef.current += 1;
    };
  }, [openNote, ownership]);

  // A read-only open holds no session and stores nothing, so a pen observed
  // after the note was opened — or mouse drawing switched on in Settings — has
  // to re-read the note with `editing` set before the canvas accepts input.
  useEffect(() => {
    if (ownership === "pending" || !editing || openedForEditing.current) return;
    void openNote();
  }, [editing, openNote, ownership]);

  const suspend = useCallback(() => {
    if (!editingRef.current) return;
    setSuspended(true);
    void setHandwritingBackground(true);
    const writer = getWriter(uuid);
    if (!writer) return;
    void (async () => {
      if (await writer.flush()) completeInBackground(uuid);
    })();
  }, [uuid]);

  const resume = useCallback(() => {
    if (!editingRef.current) {
      setSuspended(false);
      return;
    }
    void setHandwritingBackground(false);
    void openNote();
  }, [openNote]);

  useEffect(() => {
    const visibility = () => {
      if (document.visibilityState === "hidden") suspend();
      else resume();
    };
    // Draining every session is registered once at process level, because it
    // has to run whether or not this view happens to be mounted.
    const leaveApp = () => {
      setSuspended(true);
      void setHandwritingBackground(true);
    };
    document.addEventListener("visibilitychange", visibility);
    window.addEventListener("pagehide", leaveApp);
    return () => {
      document.removeEventListener("visibilitychange", visibility);
      window.removeEventListener("pagehide", leaveApp);
    };
  }, [resume, suspend]);

  useEffect(() => {
    return () => {
      const writer = getWriter(uuid);
      if (!writer) return;
      void (async () => {
        // An unacknowledged gesture keeps the session alive so a later mount can
        // retry it; only a clean queue may be handed to completion and dropped.
        // A deleted note is the exception: its gestures can never be stored, so
        // holding the writer would strand it for the life of the process.
        const stored = await writer.flush();
        if (!stored && !noteIsGone(writer.lastError())) return;
        if (stored && !leavingRef.current) completeInBackground(uuid);
        endSession(uuid, writer);
      })();
    };
  }, [uuid]);

  /** Settle local work before comparing: a resolve must see the real heads. */
  const compareVersions = useCallback(async () => {
    if (busy || comparing) return;
    setComparing(true);
    try {
      const writer = getWriter(uuid);
      if (writer) {
        if (!(await writer.flush())) return;
        await requestCompletion(uuid, () => completeHandwritingNote(uuid));
      }
      await queryClient.invalidateQueries({ queryKey: queryKeys.handwritingStatus(uuid) });
      await statusQuery.refetch();
      setConflictsOpen(true);
    } catch (error) {
      notifyError("handwriting", error);
    } finally {
      setComparing(false);
    }
  }, [busy, comparing, queryClient, statusQuery, uuid]);

  const conflictResolved = useCallback(async () => {
    setConflictsOpen(false);
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.page(uuid) }),
      queryClient.invalidateQueries({ queryKey: queryKeys.pages }),
      queryClient.invalidateQueries({ queryKey: queryKeys.handwritingStatus(uuid) }),
    ]);
    await openNote();
  }, [openNote, queryClient, uuid]);

  /**
   * The pane frame's Back asks before it navigates. Leaving is allowed once the last gestures
   * are written; a failing save keeps the note on screen with its retryable error. Completion
   * runs detached after the answer, so navigation never waits for packing or the network.
   */
  const prepareLeave = useCallback(async (): Promise<boolean> => {
    if (leavingRef.current) return false;
    if (busyRef.current) return false;
    leavingRef.current = true;
    setLeaving(true);
    const writer = getWriter(uuid);
    if (writer && !(await writer.flush())) {
      leavingRef.current = false;
      setLeaving(false);
      return false;
    }
    await titleFlush.current();
    if (writer) completeInBackground(uuid);
    return true;
  }, [uuid]);

  useEffect(() => registerPaneLeaveGuard(paneId, prepareLeave), [paneId, prepareLeave]);

  const deleteNote = useCallback(async () => {
    if (busy) return;
    // Store what is queued before the note goes away: the confirmation may be
    // declined, and a cancelled delete must not have cost a stroke. The session
    // is not closed here for the same reason — onDelete cannot report a
    // declined confirmation — so the unmount path stays in charge, and a
    // completion for a note that is gone is ignored rather than retried.
    const writer = getWriter(uuid);
    if (writer && !(await writer.flush()) && !noteIsGone(writer.lastError())) {
      // Storage refused the queue for a reason that is still on screen. Deleting
      // now would make a declined confirmation cost those gestures.
      return;
    }
    await onDelete(page);
  }, [busy, onDelete, page, uuid]);

  const change = (next: InkDraft) => {
    const previous = latestDraft.current;
    const writer = getWriter(uuid);
    if (
      !previous ||
      !writer ||
      next === previous ||
      historyBusyRef.current ||
      leavingRef.current ||
      suspendedRef.current
    ) {
      return;
    }
    latestDraft.current = next;
    setCanUndo(true);
    setCanRedo(false);
    setDraft(next);
    void writer.write(next);
  };

  const navigateHistory = async (redo: boolean) => {
    if (editingBusy || historyBusyRef.current || !(redo ? canRedo : canUndo)) return;
    historyBusyRef.current = true;
    setHistoryBusy(true);
    try {
      const writer = getWriter(uuid);
      if (!writer || !(await writer.flush())) return;
      const update = await handwritingHistory(uuid, redo, writer.getRevision());
      adopt(applyHistoryUpdate(latestDraft.current, writer.getRevision(), update));
    } catch (error) {
      setSaveState(error instanceof Error ? error : new Error(String(error)));
    } finally {
      historyBusyRef.current = false;
      setHistoryBusy(false);
    }
  };
  const [toolbarRef, layout] = useToolbarLayout();
  // Nothing may hold the pen down once the sheet is gone or the tool moved on.
  useEffect(() => {
    if (!editing) setOptionsOpen(false);
  }, [editing]);
  useEffect(() => setOptionsOpen(false), [tool]);
  useEffect(() => {
    if (optionsOpen) return;
    setAppliedEraser((applied) =>
      applied.type === eraserType && applied.diameter === eraserDiameter
        ? applied
        : { type: eraserType, diameter: eraserDiameter },
    );
  }, [optionsOpen, eraserType, eraserDiameter]);

  const undoStroke = () => void navigateHistory(false);
  const redoStroke = () => void navigateHistory(true);

  const selectTool = (id: InkTool, button: HTMLElement) => {
    // The second tap on the active tool is what opens its options, and a tap while they are
    // open closes them; the first one has to stay a plain tool switch, or every switch costs a
    // dismissal.
    if (tool === id) {
      if (optionsOpen) {
        setOptionsOpen(false);
        return;
      }
      setOptionsAnchor(button);
      setOptionsOpen(true);
      return;
    }
    if (id === "eraser" && tool !== "eraser") setPreviousTool(tool);
    setTool(id);
    if (id !== "lasso") setSelected([]);
  };

  /**
   * Stock's "Erase Current Layer" for a note of one page and one layer: confirm, clear the page
   * as one undoable write, go back to the pen and close the menu. Cancel changes nothing and
   * leaves the menu open. The draft is read after the answer, since the dialog may have been up
   * while a save or an undo moved it on.
   */
  const erasePage = async () => {
    if (confirmingRef.current) return;
    confirmingRef.current = true;
    let erase = false;
    try {
      erase = await confirm({
        title: "Erase page?",
        description: "Are you sure you want to erase strokes on this page?",
        confirmLabel: "Erase",
        destructive: true,
      });
    } finally {
      confirmingRef.current = false;
    }
    if (!erase) return;
    const latest = latestDraft.current;
    // An empty page is left alone: a fresh empty draft would still be a write and an undo step.
    if (latest?.strokes.length) change({ ...latest, strokes: [] });
    setSelected([]);
    setLimit(false);
    setTool("pen");
    setOptionsOpen(false);
  };
  const menuUnits = eraserMenuUnits(capabilities.nativeDeviceEvents);
  const toolDisabled = !draft || editingBusy;
  // A stroke in flight holds the tools (nothing may switch under a gesture) but does not dim
  // them: every stroke would otherwise repaint the whole bar twice on e-ink. A button that is
  // disabled for another reason as well keeps its dimmed look, so nothing flickers either way.
  const idleBusy = historyBusy || leaving || suspended;
  const heldByStroke = (disabledOtherwise: boolean) =>
    active && !disabledOtherwise ? "disabled:opacity-100" : undefined;
  const toolHold = heldByStroke(!draft || idleBusy);

  const statuses: NoteStatus[] = [];
  if (saveState instanceof Error) {
    statuses.push({
      kind: "save-error",
      message: saveState.message,
      onRetry: () => void getWriter(uuid)?.flush(),
    });
  }
  if (title.conflict) {
    statuses.push({
      kind: "title-conflict",
      onUseRemote: title.useRemote,
      onKeepLocal: title.keepLocal,
    });
  }
  if (status && hasConflict(status)) {
    statuses.push({
      kind: "versions",
      count: status.heads.length,
      compareDisabled: busy || comparing,
      onCompare: () => void compareVersions(),
    });
  }
  if (limit) statuses.push({ kind: "sheet-full" });
  if (nativeStatus?.error) statuses.push({ kind: "pen-error", message: nativeStatus.error });

  const readOnlyNotice =
    access === "other_pane"
      ? "This note is open for writing in another pane. Close it there to edit here."
      : "Connect a pen or enable mouse drawing in Settings to edit.";

  return (
    <div
      data-ink-editor
      className="flex h-full min-h-0 flex-col pb-[var(--safe-area-inset-bottom)]"
      onKeyDown={(event) => {
        // Nothing on this screen is editable any more, but the rename dialog is
        // rendered from here and its input must keep its own undo stack.
        if (isEditableTarget(event.target)) return;
        if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "z") {
          event.preventDefault();
          event.stopPropagation();
          if (event.shiftKey) redoStroke();
          else undoStroke();
        }
      }}
    >
      {/* One fixed 44 px row over the sheet, like the stock app's top bar: every extra strip of
          chrome is sheet the pen cannot use, and a row that changed height would move the sheet.
          The tools sit at the right end and never move; whatever comes and goes (the status
          chip, "Unsent", a longer title) only changes the width of the title. */}
      <div
        ref={toolbarRef}
        role="toolbar"
        aria-label="Handwriting tools"
        className={cn(
          "flex h-[44px] shrink-0 items-center border-b bg-background px-[4px] eink:border-black eink:bg-white",
          // On a phone-width bar the 4 px gaps are the title's last room: at 360 CSS px they
          // would push an icon-only chip over the Pen button.
          layout.statusIconOnly ? "gap-0" : "gap-[4px]",
        )}
      >
        {onBack && (
          <ToolbarButton aria-label="Go back" onClick={onBack}>
            <ArrowLeft />
          </ToolbarButton>
        )}
        <div className="flex min-w-0 flex-1 items-center gap-[4px]">
          <button
            type="button"
            onClick={() => setRenaming(true)}
            aria-label="Rename note"
            className="min-w-0 flex-1 truncate rounded-lg px-[4px] py-1 text-left text-sm leading-tight font-semibold text-foreground"
          >
            {title.title || <span className="text-muted-foreground/40">Untitled note</span>}
          </button>
          <HandwritingStatus
            statuses={statuses}
            unsent={unsentChanges(status)}
            iconOnly={layout.statusIconOnly}
          />
        </div>
        {editing ? (
          <>
            {TOOLS.map(({ id, label }) => (
              <ToolbarButton
                key={id}
                variant={tool === id ? "secondary" : "ghost"}
                aria-label={tool === id ? `${label} options` : label}
                aria-pressed={tool === id}
                disabled={toolDisabled}
                className={toolHold}
                onClick={(event) => selectTool(id, event.currentTarget)}
              >
                {id === "pen" ? (
                  <PenLine />
                ) : id === "lasso" ? (
                  <Lasso />
                ) : (
                  // The eraser shows its current type, as stock's toolbar button does.
                  <EraserTypeIcon type={eraserType} />
                )}
              </ToolbarButton>
            ))}
            <ToolbarSeparator />
            <ToolbarButton
              aria-label="Undo stroke"
              disabled={editingBusy || !canUndo}
              className={heldByStroke(idleBusy || !canUndo)}
              onClick={undoStroke}
            >
              <Undo2 />
            </ToolbarButton>
            <ToolbarButton
              aria-label="Redo stroke"
              disabled={editingBusy || !canRedo}
              className={heldByStroke(idleBusy || !canRedo)}
              onClick={redoStroke}
            >
              <Redo2 />
            </ToolbarButton>
            <ToolbarSeparator />
          </>
        ) : (
          <p
            role="status"
            title={readOnlyNotice}
            className="line-clamp-2 max-w-[260px] min-w-0 shrink text-right text-xs leading-tight text-muted-foreground"
          >
            {readOnlyNotice}
          </p>
        )}
        {capabilities.nativeDeviceEvents && (
          <ToolbarButton aria-label="Refresh screen" onClick={refreshPanelNow}>
            <RefreshCw />
          </ToolbarButton>
        )}
        <NoteMenu
          grid={draft?.background === "grid"}
          gridDisabled={!editing || toolDisabled}
          favorite={favorite}
          busy={busy}
          onGridChange={(grid) => {
            if (draft) change({ ...draft, background: grid ? "grid" : "plain" });
          }}
          onToggleFavorite={() => toggleFavoritePage(uuid)}
          onRename={() => setRenaming(true)}
          onDelete={() => void deleteNote()}
        />
      </div>
      {editing && (
        <Popover
          open={optionsOpen}
          // Stock repaints its closed eraser menu with one partial frame, not a full refresh.
          fullRefreshOnClose={tool !== "eraser"}
          onOpenChange={(open, details) => {
            if (!open) {
              // The tool's own button is not a trigger, so its tap reaches the popover as a
              // press outside before its click handler runs; that handler toggles instead.
              if (
                details.reason === "outside-press" &&
                details.event.target instanceof Node &&
                optionsAnchor?.contains(details.event.target)
              ) {
                details.cancel();
                return;
              }
              if (confirmingRef.current) {
                details.cancel();
                return;
              }
            }
            setOptionsOpen(open);
          }}
        >
          {/* The tool options hang under the tool that opened them. The pen and lasso options
              are sized in px so the system font scale cannot narrow them; the eraser menu is
              laid out in stock's dp (eraser-menu.tsx). The pen is paused while it is open. */}
          <PopoverContent
            anchor={optionsAnchor}
            align="center"
            sideOffset={tool === "eraser" ? eraserMenuSideOffset(menuUnits) : undefined}
            data-ink-tool-options
            style={tool === "eraser" ? eraserMenuStyle(menuUnits) : undefined}
            className={
              tool === "eraser"
                ? ERASER_MENU_CLASS
                : "w-auto max-w-[min(322px,calc(100vw-24px))] p-2"
            }
          >
            {tool === "pen" && (
              <div className="flex items-center gap-1" role="group" aria-label="Pen width">
                <span className="px-1 text-xs text-muted-foreground">Width</span>
                {[2, 3, 5].map((value) => (
                  <Button
                    key={value}
                    variant={width === value ? "secondary" : "ghost"}
                    aria-label={`Pen width ${value}`}
                    aria-pressed={width === value}
                    disabled={editingBusy}
                    onClick={() => {
                      setWidth(value);
                      setOptionsOpen(false);
                    }}
                    className="w-10 px-0"
                  >
                    <span
                      className="block rounded-full bg-current"
                      style={{ width: value * 2, height: value * 2 }}
                    />
                  </Button>
                ))}
              </div>
            )}
            {tool === "eraser" && (
              <EraserMenu
                units={menuUnits}
                eraseDisabled={!draft || editingBusy}
                onTypeChosen={() => setOptionsOpen(false)}
                onErasePage={() => void erasePage()}
              />
            )}
            {tool === "lasso" && (
              <div className="flex flex-wrap items-center gap-1">
                {(
                  [
                    ["free", "Freehand"],
                    ["rectangle", "Rectangle"],
                  ] as const
                ).map(([value, label]) => (
                  <Button
                    key={value}
                    variant={lassoMode === value ? "secondary" : "ghost"}
                    aria-pressed={lassoMode === value}
                    disabled={editingBusy}
                    onClick={() => {
                      setLassoMode(value);
                      setSelected([]);
                      setOptionsOpen(false);
                    }}
                  >
                    {label}
                  </Button>
                ))}
                <span className="px-2 text-xs text-muted-foreground">
                  {selected.length > 0
                    ? `${selected.length} selected \u00b7 drag to move, or use the menu on the sheet`
                    : "Draw around handwriting to select it"}
                </span>
              </div>
            )}
          </PopoverContent>
        </Popover>
      )}
      {/* Nothing but the toolbar sits above or below the viewport, so the sheet only moves when
          the viewport itself resizes, which is what the pen session observes. The viewport is a
          size container; the wrapper's width fits the whole 5:7 sheet into it (sheet-fit.ts).
          The letterbox is white on e-ink, where a light grey dithers into a surface that is not
          there. `--sheet-zoom` is the hook a later zoom sets. */}
      <div
        data-ink-viewport
        className="min-h-0 flex-1 overflow-auto overscroll-contain bg-neutral-100 p-[6px] [container-type:size] [--sheet-zoom:1] eink:bg-white min-[1000px]:p-[16px]"
      >
        {loadError ? (
          <p role="alert" className="mx-auto max-w-xl p-6 text-red-800">
            Could not open this note. It has not been changed. {loadError}
          </p>
        ) : draft ? (
          <div
            className="mx-auto border border-neutral-300 bg-white eink:border-black"
            style={{ width: SHEET_WIDTH_CSS }}
          >
            <InkCanvas
              draft={draft}
              mono={monoInk}
              disabled={!editing || suspended || historyBusy || leaving}
              tool={tool}
              eraserMode={appliedEraser.type}
              eraserDiameter={appliedEraser.diameter}
              onEraseFinished={() => {
                if (autoDeselect) setTool(previousTool);
              }}
              lassoMode={lassoMode}
              selected={selected}
              onSelectionChange={setSelected}
              width={width}
              mouseEnabled={mouseEnabled}
              nativeInk={capabilities.nativeDeviceEvents}
              onNativeStatus={setNativeStatus}
              onChange={change}
              onActiveChange={setActive}
              onLimit={() => setLimit(true)}
            />
          </div>
        ) : (
          <p role="status" className="p-6 text-center text-neutral-700">
            Opening your sheet…
          </p>
        )}
      </div>
      {renaming && (
        <RenamePageDialog
          page={page}
          open
          onOpenChange={(open) => setRenaming(open)}
          onSaved={onSaved}
        />
      )}
      {conflictsOpen && status && (
        <HandwritingConflictDialog
          pageUuid={uuid}
          heads={status.heads}
          onClose={() => setConflictsOpen(false)}
          onResolved={conflictResolved}
        />
      )}
    </div>
  );
}
