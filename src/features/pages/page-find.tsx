import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type RefObject,
} from "react";
import { useQuery } from "@tanstack/react-query";
import { EditorView } from "@codemirror/view";
import { ArrowDown, ArrowUp, CaseSensitive, Search, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useCompactLayout } from "@/app/use-compact-layout";
import { registerBackOverlay } from "@/lib/back-overlays";
import { PageFindChrome } from "./page-find-chrome";
import { getPageRenderSnapshot, type Page } from "@/lib/api";
import { queryKeys } from "@/lib/query";
import { usePageSessionRegistry } from "./page-session";
import { useWorkspaceStore } from "@/features/workspace/workspace-store";
import type { PaneId } from "@/features/workspace/workspace-model";
import { DocumentCodec } from "@/features/document/document-codec";
import { flattenBlocks } from "@/features/outliner/outliner";
import { findTextMatches, type NoteSearchMatch } from "./page-find-model";

export function usePageFind(
  page: Pick<Page, "uuid" | "layout">,
  paneId: PaneId,
  root: RefObject<HTMLElement | null>,
  navigationRequest: number,
  { draft, label = "Find in note" }: { draft?: string; label?: string } = {},
) {
  const compact = useCompactLayout();
  const active = useWorkspaceStore((state) => state.activePaneId === paneId);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [matchCase, setMatchCase] = useState(false);
  const [index, setIndex] = useState(0);
  const [request, setRequest] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const inputHadFocus = useRef(false);
  const attachInput = useCallback((node: HTMLInputElement | null) => {
    if (!node) inputHadFocus.current = document.activeElement === input.current;
    input.current = node;
    // A breakpoint can move the portal. Preserve focus, but never summon a dismissed keyboard.
    if (node && inputHadFocus.current) node.focus({ preventScroll: true });
  }, []);
  const previousFocus = useRef<HTMLElement | null>(null);
  const registry = usePageSessionRegistry();
  const session = useSyncExternalStore(
    (listener) => registry.subscribe(page.uuid, listener),
    () => registry.getSnapshot(page.uuid),
  );
  const snapshot = useQuery({
    queryKey: queryKeys.pageRender(page.uuid),
    queryFn: () => getPageRenderSnapshot(page.uuid),
    enabled: open && draft === undefined,
  });
  const matches = useMemo<NoteSearchMatch[]>(() => {
    if (!open || !query) return [];
    if (draft !== undefined)
      return findTextMatches(draft, query, matchCase).map((match, occurrence) => ({
        ...match,
        occurrence,
        blockUuid: null,
      }));
    if (!snapshot.data) return [];
    const blocks = flattenBlocks(snapshot.data.document.blocks, () => false).map(
      ({ block }) => block,
    );
    if (page.layout === "document") {
      const encoded = DocumentCodec.encode(
        blocks.map((block) => ({
          uuid: block.uuid,
          parentUuid: block.parentUuid,
          style: block.style,
          markdown: block.markdown,
          revision: block.markdownRevision,
        })),
      );
      const buffer = session.document?.draft ?? encoded.markdown;
      const units = DocumentCodec.reconcile(
        buffer,
        session.document?.baseSourceMap ?? encoded.sourceMap,
      ).units;
      return findTextMatches(buffer, query, matchCase).map((match, occurrence) => ({
        ...match,
        occurrence,
        blockUuid:
          units.find(
            (unit) => match.from >= unit.contentRange.from && match.from <= unit.contentRange.to,
          )?.previousUuid ?? null,
      }));
    }
    return blocks.flatMap((block) =>
      findTextMatches(session.blocks[block.uuid]?.draft ?? block.markdown, query, matchCase).map(
        (match, occurrence) => ({ ...match, occurrence, blockUuid: block.uuid }),
      ),
    );
  }, [open, query, matchCase, snapshot.data, page.layout, session, draft]);
  const selectedIndex = matches.length ? Math.min(index, matches.length - 1) : 0;
  const current = matches[selectedIndex] ?? null;
  const [lastNavigation, setLastNavigation] = useState<{
    pageUuid: string;
    navigationRequest: number;
    match: NoteSearchMatch;
    request: number;
  } | null>(null);
  useEffect(() => {
    if (current)
      setLastNavigation({ pageUuid: page.uuid, navigationRequest, match: current, request });
  }, [current, page.uuid, navigationRequest, request]);
  const close = useCallback(() => {
    const focus = compact
      ? document.querySelector<HTMLElement>(".app-titlebar .note-menu-trigger")
      : previousFocus.current;
    input.current?.blur();
    setOpen(false);
    requestAnimationFrame(() => {
      if (focus?.isConnected) focus.focus({ preventScroll: true });
    });
  }, [compact]);
  const show = useCallback(
    (returnFocus?: HTMLElement | null) => {
      if (!open)
        previousFocus.current =
          returnFocus ??
          (document.activeElement instanceof HTMLElement
            ? document.activeElement.closest('[role="menu"]')
              ? (root.current
                  ?.closest("[data-pane-id]")
                  ?.querySelector<HTMLElement>(".note-menu-trigger") ?? null)
              : document.activeElement
            : null);
      useWorkspaceStore.getState().dispatch({ type: "focus_pane", paneId });
      setOpen(true);
      setRequest((value) => value + 1);
      requestAnimationFrame(() => {
        input.current?.focus();
        input.current?.select();
      });
    },
    [open, paneId, root],
  );
  useEffect(() => {
    if (open && active) return registerBackOverlay(close);
  }, [open, active, close]);
  const move = useCallback(
    (direction: number) => {
      setIndex((selectedIndex + direction + matches.length) % Math.max(1, matches.length));
      setRequest((value) => value + 1);
    },
    [selectedIndex, matches.length],
  );
  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (useWorkspaceStore.getState().activePaneId !== paneId) return;
      if (root.current?.closest("[inert]")) return;
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") {
        if (document.querySelector('[role="dialog"], [role="alertdialog"]')) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        show();
      } else if (open && event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        close();
      } else if (open && event.key === "F3") {
        event.preventDefault();
        move(event.shiftKey ? -1 : 1);
      }
    };
    window.addEventListener("keydown", keydown, true);
    return () => window.removeEventListener("keydown", keydown, true);
  }, [paneId, open, show, close, move, root]);
  useEffect(() => {
    setOpen(false);
    setIndex(0);
  }, [page.uuid]);

  useEffect(() => {
    const element = root.current;
    if (!element || !open || !query) return;
    const name = `note-find-${paneId}`;
    const activeName = `${name}-active`;
    const highlights = (CSS as typeof CSS & { highlights?: Map<string, unknown> }).highlights;
    const HighlightConstructor = (
      window as typeof window & { Highlight?: new (...ranges: Range[]) => unknown }
    ).Highlight;
    const style = document.createElement("style");
    style.textContent = `::highlight(${name}) { background: color-mix(in oklch, var(--primary) 22%, var(--background)); color: var(--foreground); } ::highlight(${activeName}) { background: var(--primary); color: var(--primary-foreground); }`;
    document.head.append(style);
    const paint = () => {
      element
        .querySelectorAll("[data-find-current]")
        .forEach((node) => node.removeAttribute("data-find-current"));
      const body = element.querySelector(".editor-body");
      if (!body) return;
      const walker = document.createTreeWalker(body, NodeFilter.SHOW_TEXT, {
        acceptNode(node) {
          return node.parentElement?.closest("button, textarea, [hidden], [aria-hidden=true]")
            ? NodeFilter.FILTER_REJECT
            : NodeFilter.FILTER_ACCEPT;
        },
      });
      const ranges: Range[] = [];
      const active: Range[] = [];
      let node: Node | null;
      let currentOccurrence = 0;
      const blockOccurrence =
        page.layout === "document"
          ? matches
              .slice(0, selectedIndex)
              .filter((match) => match.blockUuid === current?.blockUuid).length
          : current?.occurrence;
      while ((node = walker.nextNode())) {
        for (const match of findTextMatches(node.textContent ?? "", query, matchCase)) {
          const range = document.createRange();
          range.setStart(node, match.from);
          range.setEnd(node, match.to);
          ranges.push(range);
          const block = node.parentElement?.closest<HTMLElement>("[data-block-uuid]");
          const editor = node.parentElement?.closest<HTMLElement>(".cm-content");
          if (
            editor &&
            current &&
            (page.layout === "document" || block?.dataset.blockUuid === current.blockUuid)
          ) {
            const view = EditorView.findFromDOM(editor);
            if (view?.posAtDOM(node, match.from) === current.from) active.push(range);
            continue;
          }
          if (current?.blockUuid && block?.dataset.blockUuid === current.blockUuid) {
            block.dataset.findCurrent = "true";
            if (currentOccurrence++ === blockOccurrence) active.push(range);
          }
        }
      }
      if (HighlightConstructor && highlights) {
        highlights.set(name, new HighlightConstructor(...ranges));
        highlights.set(activeName, new HighlightConstructor(...active));
      }
    };
    paint();
    const observer = new MutationObserver(paint);
    observer.observe(element.querySelector(".editor-body") ?? element, {
      childList: true,
      subtree: true,
      characterData: true,
    });
    return () => {
      observer.disconnect();
      highlights?.delete(name);
      highlights?.delete(activeName);
      style.remove();
      element
        .querySelectorAll("[data-find-current]")
        .forEach((node) => node.removeAttribute("data-find-current"));
    };
  }, [root, paneId, open, query, matchCase, current, request, page.layout, matches, selectedIndex]);
  return {
    navigation:
      lastNavigation?.pageUuid === page.uuid &&
      lastNavigation.navigationRequest === navigationRequest
        ? lastNavigation
        : null,
    open,
    show,
    current,
    request,
    bar: (
      <PageFindChrome paneId={paneId} open={open} show={show} label={label}>
        <div
          role="search"
          aria-label={label}
          className="note-find"
          onPointerDown={(event) => {
            // Keep the current keyboard state when stepping through matches with a pointer.
            if ((event.target as HTMLElement).closest("button")) event.preventDefault();
          }}
        >
          <div className="note-find-field">
            <Search aria-hidden className="size-4 shrink-0 text-muted-foreground" />
            <input
              ref={attachInput}
              type="text"
              aria-label={label}
              placeholder={`${label}…`}
              autoComplete="off"
              autoCapitalize="off"
              spellCheck={false}
              enterKeyHint="search"
              value={query}
              onChange={(event) => {
                setQuery(event.target.value);
                setIndex(0);
                setRequest((value) => value + 1);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  if (event.nativeEvent.isComposing) return;
                  event.preventDefault();
                  move(event.shiftKey ? -1 : 1);
                }
              }}
              className="min-w-0 w-full bg-transparent text-sm outline-none"
            />
          </div>
          <span role="status" aria-live="polite" aria-atomic="true" className="note-find-count">
            {draft === undefined && snapshot.isPending
              ? "Loading…"
              : draft === undefined && snapshot.error
                ? "Search unavailable"
                : !query
                  ? ""
                  : matches.length
                    ? `${selectedIndex + 1} of ${matches.length}`
                    : "No matches"}
          </span>
          <div className="note-find-actions">
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Match case"
              title="Match case"
              aria-pressed={matchCase}
              className="note-find-button data-[active=true]:bg-accent data-[active=true]:text-primary"
              data-active={matchCase}
              onClick={() => {
                setMatchCase(!matchCase);
                setIndex(0);
              }}
            >
              <CaseSensitive className="size-4" />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Previous match"
              title="Previous match (Shift+Enter)"
              className="note-find-button"
              disabled={!matches.length}
              onClick={() => move(-1)}
            >
              <ArrowUp className="size-4" />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Next match"
              title="Next match (Enter)"
              className="note-find-button"
              disabled={!matches.length}
              onClick={() => move(1)}
            >
              <ArrowDown className="size-4" />
            </Button>
          </div>
          <Button
            variant="ghost"
            size="icon-sm"
            className="note-find-button note-find-close"
            title="Close find (Esc)"
            aria-label="Close find"
            onClick={close}
          >
            <X className="size-4" />
          </Button>
        </div>
      </PageFindChrome>
    ),
  };
}
