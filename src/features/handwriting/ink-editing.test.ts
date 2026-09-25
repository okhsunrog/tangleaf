import { expect, it } from "vitest";
import type { InkPoint, InkStroke } from "@/lib/bindings";
import {
  cutLasso,
  eraseGesture,
  gestureLoop,
  inside,
  insideNonZero,
  lassoPolygon,
  moveSelection,
  scaleSelection,
  selectionFrame,
  selectionMenuPosition,
  selectLasso,
  SELECTION_DASH,
  SELECTION_FRAME_PADDING,
  SELECTION_FRAME_WIDTH,
  SELECTION_TRACE_WIDTH,
  type XY,
} from "./ink-editing";
const point = (x: number, y: number, time = x): InkPoint => ({
  x,
  y,
  time,
  pressure: 0.5,
  tiltX: 12,
  tiltY: -4,
});
const stroke: InkStroke = { id: "a", width: 2, points: [point(0, 100), point(200, 100)] };
it("cuts the middle of a sparse stroke and interpolates metadata without changing the original", () => {
  const result = eraseGesture([stroke], [point(100, 50), point(100, 150)], "pixel", 9);
  expect(result).toHaveLength(2);
  // The cut is the radius from the centreline, 9 units either side of the eraser.
  const end = result[0]!.points[result[0]!.points.length - 1]!;
  expect(end).toMatchObject({ y: 100, pressure: 0.5, tiltX: 12, tiltY: -4 });
  expect(end.x).toBeCloseTo(91);
  expect(end.time).toBeCloseTo(91);
  expect(result[1]!.points[0]).toMatchObject({ y: 100, pressure: 0.5, tiltX: 12, tiltY: -4 });
  expect(result[1]!.points[0]!.x).toBeCloseTo(109);
  expect(result[1]!.points[0]!.time).toBeCloseTo(109);
  expect(result[1]!.id).not.toBe(stroke.id);
  expect(stroke.points).toHaveLength(2);
});
it("whole-stroke erasing covers the space between eraser samples and keeps unrelated ink", () => {
  const other = { ...stroke, id: "b", points: [point(800, 900)] };
  expect(eraseGesture([stroke, other], [point(100, 0), point(100, 200)], "stroke", 9)).toEqual([
    other,
  ]);
  expect(eraseGesture([other], [point(100, 0)], "pixel", 9)[0]).toBe(other);
});
it("handles dots, fully erased strokes and repeated erasure without zero-length fragments", () => {
  expect(
    eraseGesture([{ ...stroke, points: [point(100, 100)] }], [point(100, 100)], "pixel", 9),
  ).toEqual([]);
  expect(eraseGesture([stroke], [point(0, 100), point(200, 100)], "pixel", 9)).toEqual([]);
  const split = eraseGesture([stroke], [point(100, 100)], "pixel", 9);
  const again = eraseGesture(split, [point(100, 100)], "pixel", 9);
  expect(again).toEqual(split);
});
it("lasso selects crossing segments with both endpoints outside, supports rectangles and rejects taps", () => {
  const polygon = lassoPolygon([point(80, 80), point(120, 120)], "rectangle");
  expect(selectLasso([stroke], polygon)).toEqual(["a"]);
  // The lasso eraser tests vertices only, as stock does: the same loop leaves the line alone.
  const strokes = [stroke];
  expect(eraseGesture(strokes, polygon, "lasso", 9)).toBe(strokes);
  expect(selectLasso([stroke], [point(100, 100)])).toEqual([]);
  expect(
    selectLasso([stroke], [point(80, 80), point(120, 80), point(120, 90), point(80, 90)]),
  ).toEqual([]);
});
it("draws a lasso eraser's loop only where no firmware traces it, and the selection lasso as before", () => {
  const points = [point(0, 0), point(100, 0), point(100, 100), point(0, 100)];
  const loop = (action: "pen" | "erase" | "select" | "move", firmwareTrace: boolean) =>
    gestureLoop({ action, points, firmwareTrace }, "lasso", "rectangle");
  // The portable path and the BOOX lasso tool's eraser end: the page's own free loop.
  expect(loop("erase", false)).toEqual(lassoPolygon(points, "free"));
  // The eraser tool and the eraser end with the pen on BOOX: SurfaceFlinger's dash, nothing else.
  expect(loop("erase", true)).toEqual([]);
  // The selection lasso keeps its own mode either way, and other gestures draw no loop.
  expect(loop("select", true)).toEqual(lassoPolygon(points, "rectangle"));
  expect(loop("select", false)).toEqual(lassoPolygon(points, "rectangle"));
  expect(loop("pen", false)).toEqual([]);
  expect(loop("move", false)).toEqual([]);
  expect(gestureLoop({ action: "erase", points, firmwareTrace: false }, "pixel", "free")).toEqual(
    [],
  );
});
it("moves the whole selection together at sheet edges and preserves input metadata", () => {
  const moved = moveSelection([stroke], ["a"], 900, -200);
  expect(moved[0]!.points).toEqual([
    { ...point(0, 100), x: 800, y: 0 },
    { ...point(200, 100), x: 1000, y: 0 },
  ]);
  expect(scaleSelection(moved, ["a"], 2)[0]!.points).toEqual(moved[0]!.points);
});

it("broad-phase erasing reaches the radius from the centreline and keeps distant strokes by identity", () => {
  // A thick dot at full pressure: its ink reaches 17.5 units from the centre. Before, a radius-9
  // eraser at x = 126 deleted it (9 + 17.5); now only a centre within 9 units counts.
  const thick = { ...stroke, width: 20, points: [{ ...point(100, 100), pressure: 1 }] };
  const distant = { ...stroke, id: "distant", points: [point(800, 800)] };
  const strokes = [thick, distant];
  expect(eraseGesture(strokes, [point(109, 100)], "stroke", 9)).toEqual([distant]);
  expect(eraseGesture(strokes, [point(110, 100)], "stroke", 9)).toBe(strokes);
  expect(eraseGesture(strokes, [point(126, 100)], "stroke", 9)).toBe(strokes);
  expect(eraseGesture([distant], [point(0, 0), point(200, 200)], "pixel", 12)[0]).toBe(distant);
});

/** Every `step` from `from` to `to`, both included. */
const range = (from: number, to: number, step: number) =>
  Array.from({ length: Math.round((to - from) / step) + 1 }, (_, i) => from + i * step);
/** A round lasso of `n` points, as a pen draws it: open, the loop closes at the lift. */
const circle = (cx: number, cy: number, radius: number, n = 32): XY[] =>
  range(0, n - 1, 1).map((k) => {
    const angle = (2 * Math.PI * (k + 0.5)) / n;
    return point(cx + radius * Math.cos(angle), cy + radius * Math.sin(angle));
  });
const withPressure = (p: InkPoint, pressure: number): InkPoint => ({ ...p, pressure });

// Step 5 of docs/planning/eraser-stock-parity.md, with the examples of
// docs/onyx-reversing/stock-notes/13-eraser-geometry.md §9. "Before" is the rule up to step 4: the
// lasso eraser deleted what the selection lasso takes, and the stroke and pixel erasers reached the
// radius plus the ink's pressure-scaled half-width.

it("lasso eraser cuts the enclosed vertices out and keeps both ends (SN/13 L1)", () => {
  // A line (0,100)…(300,100) with a vertex every 10 units, a round lasso around (150,100). At
  // radius 25 the vertices at x = 120 and 180 lie clearly outside; on a loop of radius 30 they sit
  // on it, where stock's pixel grid decides (L4).
  const line: InkStroke = {
    id: "line",
    width: 2,
    points: range(0, 300, 10).map((x) => point(x, 100)),
  };
  const lasso = circle(150, 100, 25);
  // Before: the whole line was deleted.
  expect(selectLasso([line], lassoPolygon(lasso, "free"))).toEqual(["line"]);
  // Now: the five vertices from x = 130 to 170 go and two strokes remain.
  const result = eraseGesture([line], lasso, "lasso", 1.7);
  expect(result.map((s) => s.points.map((p) => p.x))).toEqual([
    range(0, 120, 10),
    range(180, 300, 10),
  ]);
  // The first run keeps the id and its place, the second is new; the points are the stroke's own.
  expect(result[0]!.id).toBe("line");
  expect(result[1]!.id).not.toBe("line");
  expect(result[1]!).toMatchObject({ width: 2 });
  expect(result[1]!.points[0]).toBe(line.points[18]);
  expect(line.points).toHaveLength(31);
});

it("lasso eraser keeps a stroke none of whose vertices is inside, even through the loop (SN/13 L2)", () => {
  const line: InkStroke = { id: "line", width: 2, points: [point(0, 100), point(300, 100)] };
  const strokes = [line];
  const lasso = circle(150, 100, 25);
  // Before: the segment crosses the loop, so the line was deleted.
  expect(selectLasso(strokes, lassoPolygon(lasso, "free"))).toEqual(["line"]);
  // Now: no vertex inside, unchanged.
  expect(eraseGesture(strokes, lasso, "lasso", 1.7)).toBe(strokes);
});

it("lasso eraser fills its loop non-zero, so a second turn round the ink still erases it (SN/13 L3)", () => {
  // The lasso drawn twice round the same place, a short stroke and a dot inside both turns.
  const twice = [...circle(150, 100, 30), ...circle(150, 100, 27)];
  const short: InkStroke = { id: "short", width: 2, points: [point(145, 100), point(155, 100)] };
  const dot: InkStroke = { id: "dot", width: 2, points: [point(150, 104)] };
  const centre = { x: 150, y: 100 };
  // Before: even-odd counts two turns as outside, and nothing crosses the loop, so both were kept.
  expect(inside(centre, lassoPolygon(twice, "free"))).toBe(false);
  expect(selectLasso([short, dot], lassoPolygon(twice, "free"))).toEqual([]);
  // Now: winding number 2 is inside, and both go.
  expect(insideNonZero(centre, lassoPolygon(twice, "free"))).toBe(true);
  expect(eraseGesture([short, dot], twice, "lasso", 1.7)).toEqual([]);
});

it("lasso eraser removes a dot it encloses and ignores ink outside or a tiny loop", () => {
  const dot: InkStroke = { id: "dot", width: 2, points: [point(150, 100)] };
  const outside: InkStroke = {
    id: "outside",
    width: 2,
    points: [point(600, 600), point(610, 600)],
  };
  // A single vertex inside: nothing survives, so the stroke goes (unchanged from before).
  expect(eraseGesture([dot, outside], circle(150, 100, 25), "lasso", 1.7)).toEqual([outside]);
  // A 1.5-unit scribble over the dot erases nothing: the guard against tiny loops stays, although
  // stock has none (SN/13 L5).
  const strokes = [dot];
  expect(
    eraseGesture(strokes, [point(149.5, 99.5), point(151, 99.5), point(151, 101)], "lasso", 1.7),
  ).toBe(strokes);
});

it("lasso eraser drops runs of a single vertex, and the stroke with them", () => {
  // Only the middle vertex of A–B–C inside: the runs [A] and [C] are too short for a line. Stock
  // leaves such a stroke unchanged (SN/13 P2); here it goes, since the loop enclosed its middle.
  const abc: InkStroke = {
    id: "abc",
    width: 2,
    points: [point(100, 100), point(150, 100), point(200, 100)],
  };
  expect(cutLasso([abc], circle(150, 100, 25))).toEqual([]);
  // A–B–C–D with B inside keeps C–D alone, under the stroke's id.
  const abcd = { ...abc, points: [...abc.points, point(250, 100)] };
  expect(cutLasso([abcd], circle(150, 100, 25))).toEqual([
    { ...abcd, points: [point(200, 100), point(250, 100)] },
  ]);
});

it("stroke eraser reaches its radius from the centreline, not the ink's pressure width (SN/13 S1)", () => {
  // Stroke (0,0)…(100,0) with a vertex every unit, w = 4 and p = 1: 3.5 units of half-width.
  const thick: InkStroke = {
    id: "thick",
    width: 4,
    points: range(0, 100, 1).map((x) => withPressure(point(x, 0), 1)),
  };
  const strokes = [thick];
  const along = (y: number) => [point(20, y), point(80, y)];
  // Before: 14 ≤ 12 + 3.5, deleted. Now: 14 > 12, kept, as in stock.
  expect(eraseGesture(strokes, along(14), "stroke", 12)).toBe(strokes);
  // The reach is the radius itself.
  expect(eraseGesture(strokes, along(12.1), "stroke", 12)).toBe(strokes);
  expect(eraseGesture(strokes, along(11.9), "stroke", 12)).toEqual([]);
});

it("pixel eraser leaves a thick stroke whose centreline it misses (SN/13 P3)", () => {
  // A stroke along y = 0, w = 20, p = 1: its ink reaches 17.5 units from the centreline.
  const wide: InkStroke = {
    id: "wide",
    width: 20,
    points: [withPressure(point(0, 0), 1), withPressure(point(100, 0), 1)],
  };
  const strokes = [wide];
  // Before: 25 ≤ 10 + 17.5, cut. Now: 25 > 10, kept, as in stock.
  expect(eraseGesture(strokes, [point(40, 25), point(60, 25)], "pixel", 10)).toBe(strokes);
  // Along y = 9 the cut ends where the eraser's circle meets the centreline: 40 − √19, 60 + √19.
  const [left, right] = eraseGesture(strokes, [point(40, 9), point(60, 9)], "pixel", 10);
  expect(left!.points.map((p) => p.x)).toEqual([0, expect.closeTo(40 - Math.sqrt(19))]);
  expect(right!.points.map((p) => p.x)).toEqual([expect.closeTo(60 + Math.sqrt(19)), 100]);
});

it("pressure no longer changes what the eraser reaches (SN/13 C5)", () => {
  // Two strokes 11 units from the eraser's path, w = 4, one at p = 0 and one at p = 1.
  const light: InkStroke = {
    id: "light",
    width: 4,
    points: [withPressure(point(0, 11), 0), withPressure(point(100, 11), 0)],
  };
  const heavy: InkStroke = {
    ...light,
    id: "heavy",
    points: light.points.map((p) => withPressure(p, 1)),
  };
  const strokes = [light, heavy];
  // Before: light kept (11 > 10 + 0.5), heavy deleted (11 ≤ 10 + 3.5). Now both are kept.
  expect(eraseGesture(strokes, [point(0, 0), point(100, 0)], "stroke", 10)).toBe(strokes);
  // Two units closer, both go.
  expect(eraseGesture(strokes, [point(0, 2), point(100, 2)], "stroke", 10)).toEqual([]);
});

it("anchors the selection menu above the selection, or below it when the sheet has no room", () => {
  const sheet = { width: 500, height: 700 };
  const menu = { width: 180, height: 36 };
  // Room above: the menu is centred on the selection and sits over its top edge.
  expect(
    selectionMenuPosition({ left: 400, top: 400, right: 600, bottom: 500 }, sheet, menu),
  ).toEqual({ left: 160, top: 156 });
  // Against the top of the sheet it flips below, and a selection at the left edge is pulled in.
  expect(selectionMenuPosition({ left: 0, top: 0, right: 100, bottom: 40 }, sheet, menu)).toEqual({
    left: 8,
    top: 28,
  });
  // A selection filling the sheet leaves no room either way: the menu stays on the sheet.
  expect(
    selectionMenuPosition({ left: 900, top: 0, right: 1000, bottom: 1400 }, sheet, menu),
  ).toEqual({ left: 312, top: 656 });
});

it("frames a selection the way stock Notes does, scaled to our sheet units", () => {
  // Stock: 12 px padding, 3 px frame, 4 px trace, 10/10 dashes on a ~1404 px wide page.
  const sheetUnits = (devicePixels: number) => Math.round((devicePixels * 1000) / 1404);
  expect(SELECTION_FRAME_PADDING).toBe(sheetUnits(12));
  expect(SELECTION_FRAME_WIDTH).toBe(sheetUnits(3));
  expect(SELECTION_TRACE_WIDTH).toBe(sheetUnits(4));
  expect(SELECTION_DASH.map(sheetUnits)).toEqual([5, 5]); // Dashes read better a touch longer.
  expect(SELECTION_DASH).toEqual([7, 7]);
  expect(selectionFrame({ left: 100, top: 200, right: 300, bottom: 500 })).toEqual({
    left: 91,
    top: 191,
    right: 309,
    bottom: 509,
  });
  // The repaint region has to clear the frame's own stroke as well as its padding.
  expect(
    selectionFrame(
      { left: 100, top: 200, right: 300, bottom: 500 },
      SELECTION_FRAME_PADDING + SELECTION_FRAME_WIDTH,
    ),
  ).toEqual({ left: 89, top: 189, right: 311, bottom: 511 });
});
