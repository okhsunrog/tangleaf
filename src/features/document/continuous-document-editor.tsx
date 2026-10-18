import { minimalExternalDocumentChange } from "@/features/editor/minimal-document-change";
export { minimalExternalDocumentChange } from "@/features/editor/minimal-document-change";
import { defaultKeymap, history, historyKeymap, redo, undo } from "@codemirror/commands";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { defaultHighlightStyle, syntaxHighlighting } from "@codemirror/language";
import {
  Annotation,
  Compartment,
  EditorState,
  Prec,
  StateEffect,
  Transaction,
} from "@codemirror/state";
import { drawSelection, EditorView, keymap } from "@codemirror/view";
import { useEffect, useRef } from "react";
import { useResolvedDisplay } from "@/app/appearance";
import type { MarkdownOpenHandler } from "@/features/markdown";
import { documentAuthoringExtensions, type DocumentAuthoringMode } from "./document-live-preview";
import { resolveDocumentHistoryKey } from "./document-editor-model";
import type { DocumentHistoryAction } from "./document-editor-model";
import { resolveDocumentLinkClickDisposition } from "./document-link-navigation";
import { notesLinkMarkdownExtension } from "./notes-link-markdown-extension";
import { markdownFormatting } from "@/features/editor/markdown-formatting";
import { documentLinkCompletion } from "./document-completion";

export type { DocumentAuthoringMode } from "./document-live-preview";

export type DocumentEditorSession = {
  state: EditorState | null;
  focused: boolean;
  initialCursor?: "end";
};

type Props = {
  editorSession?: DocumentEditorSession;
  value: string;
  readOnly: boolean;
  focusRequest: number;
  mode?: DocumentAuthoringMode;
  pageUuid?: string;
  onChange: (value: string, composing: boolean) => void;
  onCompositionEnd: (value: string) => void;
  onBlur: () => void;
  onOpenMarkdownLink?: MarkdownOpenHandler;
  navigation?: { uuid: string; request: number; from: number; to: number } | null;
  findRange?: { from: number; to: number; request: number } | null;
};

const externalDocumentUpdate = Annotation.define<boolean>();
const MODE_RETRY_DELAYS_MS = [0, 16, 32, 64, 128, 256] as const;

const documentEditorTheme = EditorView.theme({
  "&": {
    width: "100%",
    minHeight: "24rem",
    color: "var(--foreground)",
    backgroundColor: "transparent",
    fontSize: "1rem",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": {
    overflow: "visible",
    fontFamily: "inherit",
    lineHeight: "1.75",
  },
  ".cm-content": {
    minHeight: "24rem",
    padding: "0.25rem 0 4rem",
    caretColor: "var(--primary)",
    fontFamily: "inherit",
  },
  ".cm-line": { padding: "0" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--primary)" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {
    backgroundColor: "color-mix(in oklab, var(--primary) 20%, transparent)",
  },
  ".cm-gutters": { display: "none" },
  ".cm-tooltip.cm-tooltip-autocomplete": {
    backgroundColor: "var(--popover)",
    color: "var(--popover-foreground)",
    border: "1px solid var(--border)",
    borderRadius: "8px",
    overflow: "hidden",
    boxShadow: "0 4px 16px #0002",
  },
  ".cm-tooltip-autocomplete > ul": {
    fontFamily: "inherit",
    fontSize: "0.875rem",
    padding: "4px",
    maxHeight: "16rem",
  },
  ".cm-tooltip-autocomplete > ul > li": { padding: "5px 10px", borderRadius: "4px" },
  '.cm-tooltip-autocomplete > ul > li[aria-selected="true"]': {
    backgroundColor: "var(--accent)",
    color: "var(--accent-foreground)",
  },
});

function editableExtensions(readOnly: boolean) {
  return [EditorView.editable.of(!readOnly), EditorState.readOnly.of(readOnly)];
}

/**
 * A blinking caret repaints the panel twice a second on e-ink, so the caret is drawn by
 * CodeMirror there and never blinks. Elsewhere the native caret stays untouched.
 */
function cursorExtensions(display: "standard" | "eink") {
  return display === "eink" ? [drawSelection({ cursorBlinkRate: 0 })] : [];
}

/** Always consumes local history keys; read-only panes never mutate their CM state. */
export function applyDocumentHistoryAction(
  view: EditorView,
  action: DocumentHistoryAction,
): boolean {
  if (!view.state.readOnly) {
    if (action === "undo") undo(view);
    else redo(view);
  }
  return true;
}

export function ContinuousDocumentEditor({
  editorSession,
  value,
  readOnly,
  focusRequest,
  mode = "live_preview",
  pageUuid,
  onChange,
  onCompositionEnd,
  onBlur,
  onOpenMarkdownLink,
  navigation,
  findRange,
}: Props) {
  const display = useResolvedDisplay();
  const mountRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const revealed = useRef<{ uuid: string; request: number } | null>(null);
  const initialValueRef = useRef(value);
  const callbacksRef = useRef({ onChange, onCompositionEnd, onBlur, onOpenMarkdownLink, pageUuid });
  const modeCompartmentRef = useRef(new Compartment());
  const editableCompartmentRef = useRef(new Compartment());
  const cursorCompartmentRef = useRef(new Compartment());
  const displayRef = useRef(display);
  displayRef.current = display;
  const pendingModeRef = useRef(mode);
  const appliedModeRef = useRef(mode);
  const applyPendingModeRef = useRef<() => void>(() => undefined);
  callbacksRef.current = { onChange, onCompositionEnd, onBlur, onOpenMarkdownLink, pageUuid };
  pendingModeRef.current = mode;

  useEffect(() => {
    const parent = mountRef.current;
    if (!parent) return;
    const modeCompartment = modeCompartmentRef.current;
    const editableCompartment = editableCompartmentRef.current;
    const cursorCompartment = cursorCompartmentRef.current;
    let view: EditorView;
    let destroyed = false;
    let modeRetryTimer: ReturnType<typeof setTimeout> | null = null;
    let modeRetryAttempt = 0;
    const clearModeRetry = () => {
      if (modeRetryTimer !== null) clearTimeout(modeRetryTimer);
      modeRetryTimer = null;
    };
    const applyPendingMode = () => {
      if (destroyed || viewRef.current !== view) return true;
      const pendingMode = pendingModeRef.current;
      if (appliedModeRef.current === pendingMode) return true;
      if (view.composing || view.compositionStarted) return false;
      clearModeRetry();
      view.dispatch({
        effects: modeCompartment.reconfigure(documentAuthoringExtensions(pendingMode)),
      });
      appliedModeRef.current = pendingMode;
      return true;
    };
    const applyModeAfterComposition = () => {
      modeRetryTimer = null;
      if (applyPendingMode()) return;
      const delay = MODE_RETRY_DELAYS_MS[modeRetryAttempt];
      if (delay === undefined) return;
      modeRetryAttempt += 1;
      modeRetryTimer = setTimeout(applyModeAfterComposition, delay);
    };
    const scheduleModeAfterComposition = () => {
      clearModeRetry();
      modeRetryAttempt = 0;
      modeRetryTimer = setTimeout(applyModeAfterComposition, 0);
    };
    const extensions = [
      markdown({
        base: markdownLanguage,
        extensions: notesLinkMarkdownExtension,
      }),
      syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
      history(),
      markdownFormatting(),
      documentLinkCompletion,
      EditorView.lineWrapping,
      EditorView.contentAttributes.of({
        "aria-label": "Document Markdown",
        autocapitalize: "sentences",
        autocomplete: "off",
        autocorrect: "on",
        spellcheck: "true",
      }),
      modeCompartment.of(documentAuthoringExtensions(mode)),
      editableCompartment.of(editableExtensions(readOnly)),
      cursorCompartment.of(cursorExtensions(displayRef.current)),
      Prec.highest(
        EditorView.domEventHandlers({
          keydown(event, view) {
            const action = resolveDocumentHistoryKey({
              key: event.key,
              ctrlKey: event.ctrlKey,
              metaKey: event.metaKey,
              shiftKey: event.shiftKey,
              altKey: event.altKey,
              isComposing: event.isComposing || view.composing,
            });
            if (!action) return false;
            applyDocumentHistoryAction(view, action);
            event.preventDefault();
            event.stopPropagation();
            return true;
          },
          compositionend(_event, view) {
            queueMicrotask(() => {
              callbacksRef.current.onCompositionEnd(view.state.doc.toString());
            });
            scheduleModeAfterComposition();
            return false;
          },
          blur() {
            callbacksRef.current.onBlur();
            scheduleModeAfterComposition();
            return false;
          },
          // Resolved on mousedown, before CodeMirror's own default handling would place the
          // caret and retroactively reveal the link's raw source — the decoration state must
          // reflect what the user actually saw when they clicked, not what it becomes after.
          mousedown(event, view) {
            const { onOpenMarkdownLink, pageUuid: contextPageUuid } = callbacksRef.current;
            if (!onOpenMarkdownLink || !contextPageUuid) return false;
            const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
            if (pos === null) return false;
            const resolved = resolveDocumentLinkClickDisposition(
              view.state,
              pos,
              {
                button: event.button,
                ctrlKey: event.ctrlKey,
                metaKey: event.metaKey,
                shiftKey: event.shiftKey,
              },
              pendingModeRef.current,
            );
            if (!resolved) return false;
            event.preventDefault();
            void onOpenMarkdownLink({
              context: { kind: "note", presentation: "live_preview", pageUuid: contextPageUuid },
              disposition: resolved.disposition,
              target: resolved.target,
            });
            return true;
          },
        }),
      ),
      EditorView.updateListener.of((update) => {
        if (
          appliedModeRef.current !== pendingModeRef.current &&
          !update.view.composing &&
          !update.view.compositionStarted
        ) {
          queueMicrotask(applyPendingMode);
        }
        if (!update.docChanged) return;
        if (
          update.transactions.some((transaction) => transaction.annotation(externalDocumentUpdate))
        ) {
          return;
        }
        callbacksRef.current.onChange(update.state.doc.toString(), update.view.composing);
      }),
      keymap.of([...defaultKeymap, ...historyKeymap]),
      documentEditorTheme,
    ];
    const freshState = EditorState.create({
      doc: initialValueRef.current,
      selection: {
        anchor: editorSession?.initialCursor === "end" ? initialValueRef.current.length : 0,
      },
      extensions,
    });
    const cached = editorSession?.state;
    const state =
      cached?.doc.toString() === initialValueRef.current
        ? cached.update({ effects: StateEffect.reconfigure.of(extensions) }).state
        : freshState;
    view = new EditorView({ state, parent });
    viewRef.current = view;
    if (editorSession?.focused && !readOnly) view.focus();
    applyPendingModeRef.current = applyPendingMode;
    return () => {
      if (editorSession) {
        editorSession.state = view.state;
        editorSession.focused = view.hasFocus;
      }
      destroyed = true;
      clearModeRetry();
      applyPendingModeRef.current = () => undefined;
      viewRef.current = null;
      view.destroy();
    };
  }, []);

  useEffect(() => {
    const view = viewRef.current;
    if (!view || view.composing || view.state.doc.toString() === value) return;
    const changes = minimalExternalDocumentChange(view.state.doc.toString(), value);
    const selection = view.state.selection.map(changes);
    view.dispatch({
      changes,
      selection,
      annotations: [externalDocumentUpdate.of(true), Transaction.addToHistory.of(false)],
    });
  }, [value]);

  useEffect(() => {
    viewRef.current?.dispatch({
      effects: editableCompartmentRef.current.reconfigure(editableExtensions(readOnly)),
    });
  }, [readOnly]);

  useEffect(() => {
    viewRef.current?.dispatch({
      effects: cursorCompartmentRef.current.reconfigure(cursorExtensions(display)),
    });
  }, [display]);

  useEffect(() => {
    applyPendingModeRef.current();
  }, [mode]);

  useEffect(() => {
    if (focusRequest > 0 && !readOnly) viewRef.current?.focus();
  }, [focusRequest, readOnly]);

  useEffect(() => {
    const view = viewRef.current;
    if (
      !view ||
      !navigation ||
      (revealed.current?.uuid === navigation.uuid &&
        revealed.current.request === navigation.request)
    )
      return;
    const from = Math.min(navigation.from, view.state.doc.length);
    view.dispatch({ effects: EditorView.scrollIntoView(from, { y: "center" }) });
    revealed.current = navigation;
  }, [navigation]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view || !findRange) return;
    const from = Math.min(findRange.from, view.state.doc.length);
    const to = Math.min(findRange.to, view.state.doc.length);
    view.dispatch({
      selection: { anchor: from, head: to },
      effects: EditorView.scrollIntoView(from, { y: "center" }),
    });
  }, [findRange]);

  return <div ref={mountRef} className="continuous-document-editor min-h-96 w-full" />;
}
