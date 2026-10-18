import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { markdown } from "@codemirror/lang-markdown";
import { defaultHighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { EditorState, Prec, StateEffect, type Extension } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";
import { markdownFormatting } from "@/features/editor/markdown-formatting";
import {
  replaceEditorRange,
  type BlockEditKeyEvent,
  type BlockEditPasteEvent,
} from "./block-edit-model";

export type BlockEditHandle = {
  beginBuffer: (value: string) => void;
  getState: () => EditorState | undefined;
  /** Replace text and selection in one CodeMirror transaction. */
  replaceRange: (start: number, end: number, replacement: string, caret?: number) => void;
  focus: () => void;
  getCaret: () => number;
};

type Props = {
  stateCache?: Map<string, EditorState>;
  blockUuid?: string;
  initialPointer?: { x: number; y: number } | null;
  initial: string;
  onChange: (value: string, caret: number) => void;
  onBlur: () => void;
  /** Return true when the outliner handled the key and CM must not. */
  onKeyDown?: (event: BlockEditKeyEvent) => boolean;
  /** Return true when the outliner handled the paste and CM must not. */
  onPaste?: (event: BlockEditPasteEvent) => boolean;
  autoFocus?: boolean;
};

const editorTheme = EditorView.theme({
  "&": {
    width: "100%",
    minHeight: "1.5rem",
    color: "var(--foreground)",
    backgroundColor: "transparent",
    fontSize: "0.875rem",
  },
  "&.cm-focused": {
    outline: "none",
  },
  ".cm-scroller": {
    overflow: "visible",
    fontFamily: "inherit",
    lineHeight: "1.625",
  },
  ".cm-content": {
    minHeight: "1.5rem",
    padding: "0",
    caretColor: "var(--primary)",
    fontFamily: "inherit",
  },
  ".cm-line": {
    padding: "0",
  },
  ".cm-cursor, .cm-dropCursor": {
    borderLeftColor: "var(--primary)",
  },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {
    backgroundColor: "color-mix(in oklab, var(--primary) 20%, transparent)",
  },
  ".cm-gutters": {
    display: "none",
  },
});

export const BlockEdit = forwardRef<BlockEditHandle, Props>(function BlockEdit(
  {
    initial,
    onChange,
    onBlur,
    onKeyDown,
    onPaste,
    autoFocus,
    stateCache,
    blockUuid,
    initialPointer,
  },
  ref,
) {
  const mountRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const extensionsRef = useRef<Extension[]>([]);
  const initialRef = useRef(initial);
  const autoFocusRef = useRef(autoFocus);
  const callbacksRef = useRef({ onChange, onBlur, onKeyDown, onPaste });
  callbacksRef.current = { onChange, onBlur, onKeyDown, onPaste };

  useImperativeHandle(
    ref,
    () => ({
      beginBuffer(value) {
        viewRef.current?.setState(
          EditorState.create({ doc: value, extensions: extensionsRef.current }),
        );
        viewRef.current?.focus();
      },
      getState() {
        return viewRef.current?.state;
      },
      replaceRange(start, end, replacement, caret) {
        const view = viewRef.current;
        if (!view) return;
        view.dispatch(replaceEditorRange(view.state, start, end, replacement, caret));
        view.focus();
      },
      focus() {
        viewRef.current?.focus();
      },
      getCaret() {
        return viewRef.current?.state.selection.main.from ?? 0;
      },
    }),
    [],
  );

  useEffect(() => {
    const parent = mountRef.current;
    if (!parent) return;

    const extensions = [
      markdown(),
      syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
      history(),
      markdownFormatting(),
      EditorView.lineWrapping,
      EditorView.contentAttributes.of({
        "aria-label": "Block content",
        autocapitalize: "sentences",
        autocomplete: "off",
        autocorrect: "on",
        spellcheck: "true",
      }),
      Prec.highest(
        EditorView.domEventHandlers({
          keydown(event, view) {
            const selection = view.state.selection.main;
            const handled =
              callbacksRef.current.onKeyDown?.({
                key: event.key,
                altKey: event.altKey,
                ctrlKey: event.ctrlKey,
                metaKey: event.metaKey,
                shiftKey: event.shiftKey,
                isComposing: event.isComposing || view.composing,
                selectionStart: selection.from,
                selectionEnd: selection.to,
                value: view.state.doc.toString(),
              }) ?? false;
            if (handled) event.preventDefault();
            return handled;
          },
          paste(event, view) {
            const text = event.clipboardData?.getData("text/plain") ?? "";
            if (!text) return false;
            const selection = view.state.selection.main;
            const handled =
              callbacksRef.current.onPaste?.({
                text,
                isComposing: view.composing,
                selectionStart: selection.from,
                selectionEnd: selection.to,
              }) ?? false;
            if (handled) event.preventDefault();
            return handled;
          },
          blur() {
            callbacksRef.current.onBlur();
            return false;
          },
        }),
      ),
      EditorView.updateListener.of((update) => {
        if (!update.docChanged && !update.selectionSet) return;
        callbacksRef.current.onChange(
          update.state.doc.toString(),
          update.state.selection.main.from,
        );
      }),
      keymap.of([...defaultKeymap, ...historyKeymap]),
      editorTheme,
    ];
    // Reconfigure retained state with this mount's callbacks. Reusing the old
    // extensions would retain handlers belonging to a destroyed React component.
    extensionsRef.current = extensions;
    const retained = blockUuid ? stateCache?.get(blockUuid) : undefined;
    const state =
      retained?.doc.toString() === initialRef.current
        ? retained.update({ effects: StateEffect.reconfigure.of(extensions) }).state
        : EditorState.create({
            doc: initialRef.current,
            selection: autoFocusRef.current ? { anchor: initialRef.current.length } : undefined,
            extensions,
          });

    const view = new EditorView({ state, parent });
    viewRef.current = view;
    if (initialPointer) {
      const anchor = view.posAtCoords(initialPointer);
      if (anchor !== null) view.dispatch({ selection: { anchor } });
    }
    if (autoFocusRef.current) view.focus();

    return () => {
      if (blockUuid) stateCache?.set(blockUuid, view.state);
      viewRef.current = null;
      view.destroy();
    };
  }, []);

  return <div ref={mountRef} className="min-h-6 w-full" />;
});
