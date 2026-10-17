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
import { ArrowDown, ArrowUp, CaseSensitive, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { getPageRenderSnapshot, type Page } from "@/lib/api";
import { queryKeys } from "@/lib/query";
import { usePageSessionRegistry } from "./page-session";
import { useWorkspaceStore } from "@/features/workspace/workspace-store";
import type { PaneId } from "@/features/workspace/workspace-model";
import { DocumentCodec } from "@/features/document/document-codec";
import { flattenBlocks } from "@/features/outliner/outliner";
import { findTextMatches, type NoteSearchMatch } from "./page-find-model";

export function usePageFind(
  page: Page,
  paneId: PaneId,
  root: RefObject<HTMLElement | null>,
  navigationRequest: number,
) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [matchCase, setMatchCase] = useState(false);
  const [index, setIndex] = useState(0);
  const [request, setRequest] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const previousFocus = useRef<HTMLElement | null>(null);
  const registry = usePageSessionRegistry();
  const session = useSyncExternalStore(
    (listener) => registry.subscribe(page.uuid, listener),
    () => registry.getSnapshot(page.uuid),
  );
  const snapshot = useQuery({
    queryKey: queryKeys.pageRender(page.uuid),
    queryFn: () => getPageRenderSnapshot(page.uuid),
    enabled: open,
  });
  const matches = useMemo<NoteSearchMatch[]>(() => {
    if (!open || !query || !snapshot.data) return [];
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
  }, [open, query, matchCase, snapshot.data, page.layout, session]);
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
    setOpen(false);
    previousFocus.current?.focus({ preventScroll: true });
  }, []);
  const show = useCallback(() => {
    previousFocus.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setOpen(true);
    setRequest((value) => value + 1);
    requestAnimationFrame(() => {
      input.current?.focus();
      input.current?.select();
    });
  }, []);
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
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") {
        if (document.querySelector('[role="dialog"], [role="alertdialog"]')) return;
        event.preventDefault();
        event.stopPropagation();
        show();
      } else if (open && event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        close();
      } else if (open && event.key === "F3") {
        event.preventDefault();
        move(event.shiftKey ? -1 : 1);
      }
    };
    window.addEventListener("keydown", keydown, true);
    return () => window.removeEventListener("keydown", keydown, true);
  }, [paneId, open, show, close, move]);
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
    style.textContent = `::highlight(${name}) { background: #fde68a; color: #292524; } ::highlight(${activeName}) { background: #fb923c; color: #1c1917; }`;
    document.head.append(style);
    const paint = () => {
      element
        .querySelectorAll("[data-find-current]")
        .forEach((node) => node.removeAttribute("data-find-current"));
      const body = element.querySelector(".editor-body");
      if (!body) return;
      const walker = document.createTreeWalker(body, NodeFilter.SHOW_TEXT, {
        acceptNode(node) {
          return node.parentElement?.closest("button, textarea, [aria-hidden=true]")
            ? NodeFilter.FILTER_REJECT
            : NodeFilter.FILTER_ACCEPT;
        },
      });
      const ranges: Range[] = [];
      const active: Range[] = [];
      let node: Node | null;
      let currentOccurrence = 0;
      while ((node = walker.nextNode())) {
        for (const match of findTextMatches(node.textContent ?? "", query, matchCase)) {
          const range = document.createRange();
          range.setStart(node, match.from);
          range.setEnd(node, match.to);
          ranges.push(range);
          const block = node.parentElement?.closest<HTMLElement>("[data-block-uuid]");
          if (current?.blockUuid && block?.dataset.blockUuid === current.blockUuid) {
            block.dataset.findCurrent = "true";
            if (currentOccurrence++ === current.occurrence) active.push(range);
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
  }, [root, paneId, open, query, matchCase, current, request]);
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
    bar: open ? (
      <div
        role="search"
        aria-label="Find in note"
        className="sticky top-0 z-10 mb-4 flex flex-wrap items-center gap-1 rounded-lg border bg-background p-2 shadow-sm"
      >
        <input
          ref={input}
          aria-label="Find in note"
          placeholder="Find in note…"
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setIndex(0);
            setRequest((value) => value + 1);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              move(event.shiftKey ? -1 : 1);
            }
          }}
          className="min-w-24 flex-1 bg-transparent px-2 py-1 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />
        <span role="status" aria-live="polite" className="px-2 text-xs text-muted-foreground">
          {snapshot.isPending
            ? "Loading…"
            : snapshot.error
              ? "Search unavailable"
              : !query
                ? ""
                : matches.length
                  ? `${selectedIndex + 1} / ${matches.length}`
                  : "No matches"}
        </span>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Match case"
          aria-pressed={matchCase}
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
          disabled={!matches.length}
          onClick={() => move(-1)}
        >
          <ArrowUp className="size-4" />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Next match"
          disabled={!matches.length}
          onClick={() => move(1)}
        >
          <ArrowDown className="size-4" />
        </Button>
        <Button variant="ghost" size="icon-sm" aria-label="Close find" onClick={close}>
          <X className="size-4" />
        </Button>
      </div>
    ) : null,
  };
}
