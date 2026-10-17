import { useLayoutEffect, useRef } from "react";
import { PaneContentKind, type PaneContent } from "./workspace-model";

/** History retains content objects, so two visits to one page can retain different positions. */
export function usePaneScroll(content: PaneContent) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const positions = useRef(new WeakMap<PaneContent, number>());

  useLayoutEffect(() => {
    const scroll = scrollRef.current;
    if (!scroll) return;
    const target = positions.current.get(content) ?? 0;
    let lastPosition = target;
    // Block navigation owns its reveal (including expanding collapsed ancestors).
    let restoring = !(content.kind === PaneContentKind.Page && content.blockUuid);
    const restore = () => {
      if (!restoring) return;
      scroll.scrollTop = target;
      if (Math.abs(scroll.scrollTop - target) < 1) restoring = false;
    };
    const remember = () => {
      if (!restoring) lastPosition = scroll.scrollTop;
    };
    const interrupt = () => {
      restoring = false;
      lastPosition = scroll.scrollTop;
    };
    restore();
    // A restored page may still be fetching or progressively rendering its blocks.
    const observer = new ResizeObserver(restore);
    const children = scroll.firstElementChild;
    if (children) observer.observe(children);
    const mutations = new MutationObserver(() => {
      if (scroll.firstElementChild) observer.observe(scroll.firstElementChild);
      restore();
    });
    mutations.observe(scroll, { childList: true, subtree: true });
    scroll.addEventListener("scroll", remember, { passive: true });
    scroll.addEventListener("wheel", interrupt, { passive: true });
    scroll.addEventListener("pointerdown", interrupt);
    scroll.addEventListener("keydown", interrupt);
    return () => {
      positions.current.set(content, lastPosition);
      observer.disconnect();
      mutations.disconnect();
      scroll.removeEventListener("scroll", remember);
      scroll.removeEventListener("wheel", interrupt);
      scroll.removeEventListener("pointerdown", interrupt);
      scroll.removeEventListener("keydown", interrupt);
    };
  }, [content]);
  return scrollRef;
}
