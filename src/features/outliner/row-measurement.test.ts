// @vitest-environment jsdom
import { Virtualizer } from "@tanstack/react-virtual";
import { describe, expect, it } from "vite-plus/test";
import { measureOutlineRow } from "./row-measurement";

function fixture() {
  const keys = ["first", "second"];
  const instance = new Virtualizer<HTMLElement, HTMLLIElement>({
    count: keys.length,
    getItemKey: (index) => keys[index],
    getScrollElement: () => null,
    estimateSize: () => 42,
    observeElementRect: () => undefined,
    observeElementOffset: () => undefined,
    scrollToFn: () => undefined,
  });
  const element = document.createElement("li");
  element.dataset.index = "0";
  Object.defineProperty(element, "offsetHeight", {
    get() {
      throw new Error("Unexpected forced layout during row registration");
    },
  });
  return { instance, element, keys };
}

const entry = (height: number) =>
  ({ borderBoxSize: [{ blockSize: height, inlineSize: 600 }] }) as unknown as ResizeObserverEntry;

describe("desktop outline measurement", () => {
  it("registers an unmeasured row using its estimate without reading layout", () => {
    const { instance, element } = fixture();
    expect(measureOutlineRow(element, undefined, instance)).toBe(42);
  });

  it("uses measured identity after a reorder instead of an old index's height", () => {
    const { instance, element, keys } = fixture();
    instance.itemSizeCache.set("first", 70);
    instance.itemSizeCache.set("second", 120);
    expect(measureOutlineRow(element, undefined, instance)).toBe(70);
    keys.reverse();
    expect(measureOutlineRow(element, undefined, instance)).toBe(120);
  });

  it("accepts fresh observer sizes over estimates and stale cached sizes", () => {
    const { instance, element } = fixture();
    instance.itemSizeCache.set("first", 42);
    expect(measureOutlineRow(element, entry(120.4), instance)).toBe(120);
    // Later image/font/width changes still reach the normal measurement path.
    expect(measureOutlineRow(element, entry(300.7), instance)).toBe(301);
  });
});
