import { useEffect, useRef, type RefObject } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  getPageDocument,
  replacePageDocument,
  type Block,
  type PageRenderSnapshot,
} from "@/lib/api";
import { usePageSessionRegistry } from "@/features/pages/page-session";
import {
  captureSnapshotPublication,
  publishQuerySnapshot,
  queryKeys,
  refreshQueries,
} from "@/lib/query";
import { notifyError } from "@/lib/notify";
import {
  blockRange,
  documentWithoutBlocks,
  expandBlockSelection,
  nextBlockAfterDeletion,
  selectedBlockMarkdown,
} from "./block-selection-model";
import { useOutliner } from "./outliner-store";
import type { FlatBlock } from "./outliner-tree";

type Drag = {
  anchor: string;
  head: string;
  pointerId: number;
  x: number;
  y: number;
  promoted: boolean;
};
const interactive =
  'button, a, input, textarea, select, [role="button"], [role="checkbox"], [role="combobox"], [role="dialog"]';

/** Owns block selection above the single active CodeMirror's text selection. */
export function useBlockSelection(
  rootRef: RefObject<HTMLUListElement | null>,
  pageUuid: string,
  blocks: readonly Block[],
  rows: readonly FlatBlock[],
) {
  const store = useOutliner();
  const sessions = usePageSessionRegistry();
  const queryClient = useQueryClient();
  const current = useRef({ store, blocks, rows });
  current.current = { store, blocks, rows };

  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const doc = root.ownerDocument;
    const win = doc.defaultView!;
    let drag: Drag | null = null;
    let frame = 0;
    let swallowClick = false;
    let deleting = false;

    const pending = () => root.querySelector('[data-block-pending="true"]') !== null;
    const clearNativeSelection = () => win.getSelection()?.removeAllRanges();
    const blockElement = (target: EventTarget | null) => {
      const row =
        target instanceof Element ? target.closest<HTMLElement>("[data-block-uuid]") : null;
      return row && root.contains(row) ? row : null;
    };
    const blockAt = (x: number, y: number, target?: EventTarget | null) => {
      const rect = root.getBoundingClientRect();
      const hit = blockElement(
        doc.elementFromPoint?.(Math.max(rect.left + 1, Math.min(rect.right - 1, x)), y),
      );
      const direct = hit ?? blockElement(target ?? null);
      if (direct) return direct.dataset.blockUuid ?? null;
      // Between rows or just outside the scrollport, use the nearest rendered
      // row. Logical range membership never depends on mounted DOM rows.
      let nearest: HTMLElement | null = null;
      let distance = Infinity;
      for (const row of root.querySelectorAll<HTMLElement>("[data-block-uuid]")) {
        const bounds = row.getBoundingClientRect();
        const delta = Math.max(bounds.top - y, y - bounds.bottom, 0);
        if (delta < distance) {
          nearest = row;
          distance = delta;
        }
      }
      return nearest?.dataset.blockUuid ?? null;
    };
    const selectTo = (uuid: string) => {
      if (!drag || pending()) return;
      const visible = current.current.rows.map(({ block }) => block.uuid);
      const range = blockRange(visible, drag.anchor, uuid);
      if (!range.length) {
        stopDrag();
        return;
      }
      if (!drag.promoted && uuid === drag.anchor) return;
      if (drag.promoted && drag.head === uuid) return;
      drag.head = uuid;
      drag.promoted = true;
      swallowClick = true;
      current.current.store.selectBlocks(range);
      clearNativeSelection();
      root.focus({ preventScroll: true });
    };
    const autoScroll = () => {
      frame = 0;
      if (!drag?.promoted) return;
      const scroll = root.closest<HTMLElement>("[data-workspace-scroll]");
      if (!scroll) return;
      const rect = scroll.getBoundingClientRect();
      const velocity =
        drag.y < rect.top + 32
          ? -Math.min(24, (rect.top + 32 - drag.y) / 2)
          : drag.y > rect.bottom - 32
            ? Math.min(24, (drag.y - rect.bottom + 32) / 2)
            : 0;
      if (!velocity) return;
      const before = scroll.scrollTop;
      scroll.scrollTop += velocity;
      const uuid = blockAt(drag.x, drag.y);
      if (uuid) selectTo(uuid);
      if (scroll.scrollTop !== before) frame = win.requestAnimationFrame(autoScroll);
    };
    function stopDrag() {
      drag = null;
      win.cancelAnimationFrame(frame);
      frame = 0;
    }
    const down = (event: PointerEvent) => {
      stopDrag();
      swallowClick = false;
      if (event.button !== 0 || event.pointerType === "touch") return;
      const inside = event.target instanceof Node && root.contains(event.target);
      if (current.current.store.selectedUuids.size) current.current.store.selectBlocks([]);
      if (
        !inside ||
        pending() ||
        (event.target instanceof Element && event.target.closest(interactive))
      )
        return;
      const anchor = blockAt(event.clientX, event.clientY, event.target);
      if (anchor)
        drag = {
          anchor,
          head: anchor,
          pointerId: event.pointerId,
          x: event.clientX,
          y: event.clientY,
          promoted: false,
        };
    };
    const move = (event: PointerEvent) => {
      if (!drag || drag.pointerId !== event.pointerId || !(event.buttons & 1)) return;
      drag.x = event.clientX;
      drag.y = event.clientY;
      const uuid = blockAt(drag.x, drag.y, event.target);
      if (uuid) selectTo(uuid);
      if (drag?.promoted) {
        event.preventDefault();
        event.stopPropagation();
        if (!frame) frame = win.requestAnimationFrame(autoScroll);
      }
    };
    const mouseMove = (event: MouseEvent) => {
      if (!drag?.promoted) return;
      // CodeMirror's mouse drag listener must stop extending its text range
      // after this gesture becomes a block selection.
      event.preventDefault();
      event.stopPropagation();
    };
    const up = (event: PointerEvent) => {
      if (!drag || drag.pointerId !== event.pointerId) return;
      if (drag.promoted) {
        event.preventDefault();
        clearNativeSelection();
      }
      stopDrag();
    };
    const click = (event: MouseEvent) => {
      if (!swallowClick) return;
      swallowClick = false;
      event.preventDefault();
      event.stopPropagation();
    };
    const remove = async () => {
      const { store: owner } = current.current;
      if (deleting || owner.readOnly || !owner.selectedUuids.size) return;
      deleting = true;
      const selected = new Set(owner.selectedUuids);
      try {
        if (!(await sessions.flushPage(pageUuid)))
          throw new Error("Resolve unsaved changes before deleting blocks");
        const document = await getPageDocument(pageUuid);
        if (!document) throw new Error("This page no longer exists");
        const removed = expandBlockSelection(document.blocks, selected);
        if (!removed.size) {
          owner.selectBlocks([]);
          return;
        }
        const next = nextBlockAfterDeletion(
          current.current.rows.map(({ block }) => block.uuid),
          removed,
        );
        const publication = captureSnapshotPublication(queryClient, queryKeys.pageRender(pageUuid));
        const updated = await replacePageDocument(
          pageUuid,
          document.revision,
          documentWithoutBlocks(document, removed),
        );
        for (const uuid of removed) {
          sessions.discardBlock(pageUuid, uuid);
          owner.editorStates.delete(uuid);
        }
        publishQuerySnapshot<PageRenderSnapshot>(queryClient, publication, (snapshot) =>
          snapshot ? { ...snapshot, document: updated } : snapshot,
        );
        owner.selectBlocks([]);
        owner.setEditing(
          next && updated.blocks.some((block) => block.uuid === next)
            ? next
            : (updated.blocks[0]?.uuid ?? null),
        );
        void refreshQueries(queryClient, { queryKey: queryKeys.pageRender(pageUuid) });
      } catch (error) {
        notifyError("delete blocks", error);
      } finally {
        deleting = false;
      }
    };
    const copy = (event: ClipboardEvent) => {
      const { store: owner, blocks: source } = current.current;
      if (!owner.selectedUuids.size || !event.clipboardData) return;
      if (pending()) {
        event.preventDefault();
        return;
      }
      const markdown = selectedBlockMarkdown(
        source,
        owner.selectedUuids,
        sessions.getSnapshot(pageUuid).blocks,
      );
      event.clipboardData.setData("text/plain", markdown);
      event.clipboardData.setData("text/markdown", markdown);
      event.preventDefault();
      event.stopPropagation();
      if (event.type === "cut" && !owner.readOnly) void remove();
    };
    const key = (event: KeyboardEvent) => {
      if (!current.current.store.selectedUuids.size || event.isComposing || event.keyCode === 229)
        return;
      if (event.key === "Escape") {
        current.current.store.selectBlocks([]);
        root.querySelector<HTMLElement>(".cm-content")?.focus();
      } else if (event.key === "Backspace" || event.key === "Delete") {
        void remove();
      } else return;
      event.preventDefault();
      event.stopPropagation();
    };
    doc.addEventListener("pointerdown", down, true);
    doc.addEventListener("pointermove", move, true);
    doc.addEventListener("mousemove", mouseMove, true);
    doc.addEventListener("pointerup", up, true);
    doc.addEventListener("pointercancel", stopDrag, true);
    win.addEventListener("blur", stopDrag);
    root.addEventListener("click", click, true);
    root.addEventListener("copy", copy, true);
    root.addEventListener("cut", copy, true);
    root.addEventListener("keydown", key, true);
    return () => {
      stopDrag();
      doc.removeEventListener("pointerdown", down, true);
      doc.removeEventListener("pointermove", move, true);
      doc.removeEventListener("mousemove", mouseMove, true);
      doc.removeEventListener("pointerup", up, true);
      doc.removeEventListener("pointercancel", stopDrag, true);
      win.removeEventListener("blur", stopDrag);
      root.removeEventListener("click", click, true);
      root.removeEventListener("copy", copy, true);
      root.removeEventListener("cut", copy, true);
      root.removeEventListener("keydown", key, true);
    };
  }, [rootRef, pageUuid, sessions, queryClient, blocks.length === 0]);
}
