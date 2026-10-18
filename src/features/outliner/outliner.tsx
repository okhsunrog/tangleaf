import {
  type ComponentProps,
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { defaultRangeExtractor, type Range, useVirtualizer } from "@tanstack/react-virtual";
import { Plus } from "lucide-react";
import { OutlineSkeleton } from "./outline-skeleton";
import { useProgressiveRows, ROW_BATCH_SIZE } from "./use-progressive-rows";
import { type MarkdownOpenHandler, useResolvedAttachmentImages } from "@/features/markdown";
import { PagePresentation } from "@/features/pages/page-presentation";
import { isAndroid } from "@/app/sheet-mode";
import { createBlock, getPageRenderSnapshot, type Block, type Page } from "@/lib/api";
import { queryKeys } from "@/lib/query";
import { BlockNode } from "./block-node";
import { OutlinerProvider, useOutliner } from "./outliner-store";

type Props = {
  page: Page;
  targetBlockUuid?: string | null;
  focusRequest?: number;
  navigationRequest?: number;
  onOpenMarkdownLink: MarkdownOpenHandler;
  presentation?: PagePresentation;
  readOnly?: boolean;
};

type FlatBlock = { block: Block; depth: number; ordinal: number };

// Verified on-device: 300 fully rendered markdown rows still fling natively-smooth,
// while the virtualizer's per-scroll re-render is what causes stutter. So only
// genuinely huge pages pay the virtualization trade-off.
const ANDROID_NON_VIRTUAL_MAX = 400;
const DESKTOP_NON_VIRTUAL_MAX = 80;

export function Outliner({
  page,
  targetBlockUuid = null,
  focusRequest = 0,
  navigationRequest = 0,
  onOpenMarkdownLink,
  presentation = PagePresentation.Editing,
  readOnly = presentation === PagePresentation.Reading,
}: Props) {
  const snapshotQuery = useQuery({
    queryKey: queryKeys.pageRender(page.uuid),
    queryFn: () => getPageRenderSnapshot(page.uuid),
  });
  const snapshot = snapshotQuery.data;
  const resolveImage = useResolvedAttachmentImages(snapshot?.images ?? []);

  if (snapshotQuery.isPending) {
    return <OutlineSkeleton />;
  }
  if (snapshotQuery.error) {
    return (
      <div className="text-sm text-destructive">load error: {snapshotQuery.error.message}</div>
    );
  }
  if (!snapshot) {
    return <div className="text-sm text-muted-foreground">This page is no longer available.</div>;
  }

  return (
    <OutlinerProvider
      key={page.uuid}
      blocks={snapshot.document.blocks}
      initialEditingRequest={focusRequest}
      navigationTargetUuid={targetBlockUuid}
      navigationRequest={navigationRequest}
      layout={page.layout}
      onOpenMarkdownLink={onOpenMarkdownLink}
      readOnly={readOnly}
      resolveImage={resolveImage}
    >
      <VirtualOutline
        blocks={snapshot.document.blocks}
        page={page}
        targetBlockUuid={targetBlockUuid}
        navigationRequest={navigationRequest}
        focusFirstBlockRequest={focusRequest}
      />
    </OutlinerProvider>
  );
}

function VirtualOutline({
  blocks,
  page,
  focusFirstBlockRequest,
  targetBlockUuid,
  navigationRequest,
}: {
  blocks: readonly Block[];
  page: Page;
  focusFirstBlockRequest: number;
  targetBlockUuid: string | null;
  navigationRequest: number;
}) {
  const store = useOutliner();
  const queryClient = useQueryClient();
  const rootRef = useRef<HTMLUListElement>(null);
  const handledFocusRequest = useRef(0);
  const [scrollElement, setScrollElement] = useState<HTMLElement | null>(null);
  const [scrollMargin, setScrollMargin] = useState(0);
  const rows = useMemo(
    () => flattenBlocks(blocks, store.layout === "outline" ? store.isCollapsed : () => false),
    [blocks, store.isCollapsed, store.layout],
  );

  useLayoutEffect(() => {
    const root = rootRef.current;
    const scroll = root?.closest<HTMLElement>("[data-workspace-scroll]") ?? null;
    setScrollElement(scroll);
    if (!root || !scroll) return;
    const update = () => {
      const rootRect = root.getBoundingClientRect();
      const scrollRect = scroll.getBoundingClientRect();
      setScrollMargin(rootRect.top - scrollRect.top + scroll.scrollTop);
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(scroll);
    observer.observe(root);
    window.addEventListener("resize", update);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", update);
    };
  }, [page.uuid]);

  useLayoutEffect(() => {
    const grouped = groupChildren(blocks, page.uuid);
    for (const [containerUuid, children] of grouped) {
      queryClient.setQueryData(queryKeys.children(containerUuid), children);
    }
  }, [blocks, page.uuid, queryClient]);

  // Preserve Android's device-tested native fling behavior. Desktop can window
  // moderately large lists so Split does not mount every preceding Markdown row
  // just to focus its final part. Viewport width does not identify a platform.
  const virtualize =
    rows.length > (isAndroid() ? ANDROID_NON_VIRTUAL_MAX : DESKTOP_NON_VIRTUAL_MAX);
  const progressiveCount = useProgressiveRows(
    rows.length,
    !virtualize && rows.length > ROW_BATCH_SIZE,
    scrollElement,
  );
  // Navigation to a specific block must never wait behind the background batches.
  const editingIndex = rows.findIndex((row) => row.block.uuid === store.editingUuid);
  const targetIndex = rows.findIndex((row) => row.block.uuid === targetBlockUuid);
  const visibleCount = Math.max(progressiveCount, editingIndex + 1, targetIndex + 1);
  const rangeExtractor = useCallback(
    (range: Range) => {
      const indices = new Set(defaultRangeExtractor(range));
      // An editor owns its pending autosave/flush registration. Scrolling it
      // outside overscan must not unmount it and abandon a draft. Navigation
      // targets also mount immediately without rendering the whole prefix.
      if (editingIndex >= 0) indices.add(editingIndex);
      if (targetIndex >= 0) indices.add(targetIndex);
      return [...indices].sort((left, right) => left - right);
    },
    [editingIndex, targetIndex],
  );

  const virtualizer = useVirtualizer({
    count: rows.length,
    enabled: virtualize,
    estimateSize: (index) => estimateBlockHeight(rows[index]?.block),
    getItemKey: (index) => rows[index]?.block.uuid ?? index,
    getScrollElement: () => scrollElement,
    // The scroll element already exists when a live update crosses the
    // threshold. Seed its size so that transition never renders an empty range
    // before ResizeObserver runs and unmounts the active editor for one frame.
    initialRect: {
      width: scrollElement?.clientWidth ?? 0,
      height: scrollElement?.clientHeight ?? 0,
    },
    // Keep enough neighbours for smooth keyboard/trackpad scrolling without
    // eagerly decoding a whole run of image blocks just below the viewport.
    overscan: 4,
    rangeExtractor,
    scrollMargin,
  });
  const virtualItems = virtualizer.getVirtualItems();
  // A hidden pane (or the frame before its first measurement) has no visible
  // range. Keep its pinned editor alive even then; its position is measured
  // when the pane becomes visible again.
  const renderedVirtualItems = virtualItems.length
    ? virtualItems
    : [...new Set([editingIndex, targetIndex])]
        .filter((index) => index >= 0)
        .map((index) => ({ index, start: scrollMargin }));
  const revealed = useRef<{ uuid: string; request: number } | null>(null);
  useLayoutEffect(() => {
    if (!targetBlockUuid || targetIndex < 0 || !scrollElement) return;
    if (
      revealed.current?.uuid === targetBlockUuid &&
      revealed.current.request === navigationRequest
    )
      return;
    if (virtualize) virtualizer.scrollToIndex(targetIndex, { align: "center" });
    const element = rootRef.current?.querySelector<HTMLElement>(
      `[data-block-uuid="${CSS.escape(targetBlockUuid)}"]`,
    );
    if (!element) return;
    if (!virtualize) element.scrollIntoView({ block: "center", behavior: "instant" });
    revealed.current = { uuid: targetBlockUuid, request: navigationRequest };
  }, [
    targetBlockUuid,
    targetIndex,
    navigationRequest,
    scrollElement,
    virtualize,
    virtualizer,
    virtualItems,
  ]);

  useEffect(() => {
    if (
      focusFirstBlockRequest <= handledFocusRequest.current ||
      store.readOnly ||
      rows.length === 0
    ) {
      return;
    }
    handledFocusRequest.current = focusFirstBlockRequest;
    store.setEditing(rows[0].block.uuid);
  }, [focusFirstBlockRequest, rows, store]);

  useLayoutEffect(() => {
    if (!virtualize || !store.editingUuid) return;
    const index = rows.findIndex(({ block }) => block.uuid === store.editingUuid);
    if (index >= 0) virtualizer.scrollToIndex(index, { align: "auto" });
  }, [rows, store.editingUuid, virtualize, virtualizer]);

  const addRootBlock = async () => {
    const roots = blocks.filter((block) => block.parentUuid === null);
    const last = roots.length > 0 ? roots[roots.length - 1] : null;
    await createBlock({
      pageUuid: page.uuid,
      parentUuid: null,
      afterUuid: last?.uuid ?? null,
    });
    await queryClient.invalidateQueries({ queryKey: queryKeys.pageRender(page.uuid) });
  };

  if (blocks.length === 0) {
    return (
      <div className="rounded-md border border-dashed surface-card-soft p-6 text-center text-sm text-muted-foreground">
        <p>
          {page.kind.kind === "journal"
            ? "Nothing captured for this day yet."
            : "No blocks yet on this page."}
        </p>
        {!store.readOnly && (
          <button
            type="button"
            onClick={() => void addRootBlock()}
            className="mt-3 inline-flex items-center gap-1 rounded-md border bg-background px-2 py-1 text-xs text-foreground hover:bg-accent"
          >
            <Plus className="size-3" />
            {page.kind.kind === "journal" ? "Start writing" : "Add first block"}
          </button>
        )}
      </div>
    );
  }

  return (
    <>
      <ul
        ref={rootRef}
        className="relative"
        style={
          virtualize ? { height: virtualizer.getTotalSize(), contain: "layout style" } : undefined
        }
      >
        {virtualize
          ? renderedVirtualItems.map((item) => {
              const row = rows[item.index];
              if (!row) return null;
              return (
                <OutlineRow
                  key={row.block.uuid}
                  block={row.block}
                  depth={row.depth}
                  ordinal={row.ordinal}
                  measureRef={virtualizer.measureElement}
                  virtualIndex={item.index}
                  virtualStart={item.start - virtualizer.options.scrollMargin}
                />
              );
            })
          : rows
              .slice(0, visibleCount)
              .map((row) => (
                <OutlineRow
                  key={row.block.uuid}
                  block={row.block}
                  depth={row.depth}
                  ordinal={row.ordinal}
                />
              ))}
      </ul>
      {!virtualize && visibleCount < rows.length && <OutlineSkeleton />}
      {!store.readOnly && (
        <button
          type="button"
          onClick={() => void addRootBlock()}
          className="group mt-3 flex items-center gap-2 rounded-lg px-2 py-1.5 text-xs text-muted-foreground/55 transition hover:bg-primary/5 hover:text-primary"
        >
          <Plus className="size-3.5 transition group-hover:rotate-90" />
          Add another block
          <span className="reveal-on-hover ml-auto transition">or press Enter</span>
        </button>
      )}
    </>
  );
}

// Both rendering modes keep the same keyed component type so crossing the
// threshold while editing does not replace the editor and its pending draft.
const OutlineRow = memo(function OutlineRow({
  block,
  depth,
  ordinal,
  measureRef,
  virtualIndex,
  virtualStart,
}: FlatBlock &
  Pick<ComponentProps<typeof BlockNode>, "measureRef" | "virtualIndex"> & {
    virtualStart?: number;
  }) {
  return (
    <BlockNode
      block={block}
      depth={depth}
      ordinal={ordinal}
      measureRef={measureRef}
      virtualIndex={virtualIndex}
      style={
        virtualStart === undefined
          ? { paddingInlineStart: `${depth * 1.75}rem` }
          : {
              position: "absolute",
              top: 0,
              left: 0,
              width: "100%",
              paddingInlineStart: `${depth * 1.75}rem`,
              transform: `translateY(${virtualStart}px)`,
            }
      }
    />
  );
});

export function flattenBlocks(
  blocks: readonly Block[],
  isCollapsed: (uuid: string) => boolean,
): FlatBlock[] {
  const children = new Map<string, Block[]>();
  for (const block of blocks) {
    const key = block.parentUuid ?? block.pageUuid;
    const siblings = children.get(key) ?? [];
    siblings.push(block);
    children.set(key, siblings);
  }
  for (const siblings of children.values()) {
    siblings.sort((left, right) => left.orderKey.localeCompare(right.orderKey));
  }
  const rows: FlatBlock[] = [];
  const visited = new Set<string>();
  const append = (siblings: readonly Block[], depth: number) => {
    siblings.forEach((block, index) => {
      if (visited.has(block.uuid)) return;
      visited.add(block.uuid);
      rows.push({ block, depth, ordinal: index + 1 });
      if (!isCollapsed(block.uuid)) append(children.get(block.uuid) ?? [], depth + 1);
    });
  };
  const pageUuid = blocks[0]?.pageUuid;
  if (pageUuid) append(children.get(pageUuid) ?? [], 0);
  return rows;
}

function groupChildren(blocks: readonly Block[], pageUuid: string): Map<string, Block[]> {
  const grouped = new Map<string, Block[]>();
  grouped.set(pageUuid, []);
  for (const block of blocks) {
    grouped.set(block.uuid, grouped.get(block.uuid) ?? []);
    const key = block.parentUuid ?? pageUuid;
    const siblings = grouped.get(key) ?? [];
    siblings.push(block);
    grouped.set(key, siblings);
  }
  for (const siblings of grouped.values()) {
    siblings.sort((left, right) => left.orderKey.localeCompare(right.orderKey));
  }
  return grouped;
}

function estimateBlockHeight(block?: Block): number {
  if (!block) return 42;
  if (block.style.kind === "divider") return 34;
  if (block.style.kind.startsWith("heading_")) return 58;
  const imageCount = block.markdown.match(/notes-attachment:/g)?.length ?? 0;
  if (imageCount > 0) {
    // Inline previews are capped at 70vh. A realistic initial estimate keeps
    // the virtualizer from mounting and decoding many image rows before their
    // measured heights arrive.
    return Math.min(2_400, 42 + imageCount * 560);
  }
  const lines = Math.max(1, Math.ceil(block.markdown.length / 80));
  return Math.min(240, 38 + (lines - 1) * 24);
}
