import { measureElement, type Virtualizer } from "@tanstack/react-virtual";

/** Register new desktop rows without a forced layout in every React ref callback.
 * The virtualizer still observes each row; ResizeObserver supplies accurate sizes
 * in a batch before paint, including later image/font/width changes.
 */
export function measureOutlineRow<TScroll extends Element, TRow extends Element>(
  element: TRow,
  entry: ResizeObserverEntry | undefined,
  instance: Virtualizer<TScroll, TRow>,
): number {
  if (!entry) {
    const index = instance.indexFromElement(element);
    const key = instance.options.getItemKey(index);
    return instance.itemSizeCache.get(key) ?? instance.options.estimateSize(index);
  }
  return measureElement(element, entry, instance);
}
