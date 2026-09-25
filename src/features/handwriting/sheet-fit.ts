/**
 * Fitting the sheet into the editor's viewport.
 *
 * The ink format is a fixed 5:7 sheet, so the sheet is scaled to the screen and never reshaped:
 * contained in the viewport, centred horizontally, aligned to the top. The fit is plain CSS on the
 * sheet's wrapper — the viewport is a size container and the wrapper's width is a formula of its
 * content box — so nothing measures the sheet in JS, and the canvas keeps taking its size from the
 * wrapper's width. Every change of fit is a resize of the viewport and of the canvas, which is
 * exactly what the pen session observes.
 *
 * `fitSheet` is the same arithmetic, for tests and for reasoning about device sizes.
 */

/** The wrapper's border, on each side. */
export const SHEET_BORDER_PX = 1;
/**
 * Room left under the sheet so rounding can never make the viewport scrollable by a pixel. A one
 * pixel scroll still reconfigures the pen, which pauses and re-arms raw drawing for nothing.
 */
export const SHEET_FIT_SLACK_PX = 1;
/** Below this the sheet stops shrinking and the viewport scrolls instead. */
export const SHEET_MIN_WIDTH_PX = 280;
/** Bounds the canvas buffer on large desktop windows. */
export const SHEET_MAX_WIDTH_PX = 900;

const BORDERS = 2 * SHEET_BORDER_PX;

/**
 * The wrapper width, in container units of the viewport. `--sheet-zoom` (1 unless a later zoom
 * sets it) scales the fitted width; the viewport already scrolls whatever does not fit.
 */
export const SHEET_WIDTH_CSS =
  `calc(max(${SHEET_MIN_WIDTH_PX}px, min(100cqw, ` +
  `(100cqh - ${BORDERS + SHEET_FIT_SLACK_PX}px) * 5 / 7 + ${BORDERS}px, ` +
  `${SHEET_MAX_WIDTH_PX}px)) * var(--sheet-zoom, 1))`;

export type Size = { width: number; height: number };

/** The canvas size `SHEET_WIDTH_CSS` produces in a viewport whose content box is `content`. */
export function fitSheet(content: Size, zoom = 1): Size {
  const byHeight = ((content.height - BORDERS - SHEET_FIT_SLACK_PX) * 5) / 7 + BORDERS;
  const wrapper =
    Math.max(SHEET_MIN_WIDTH_PX, Math.min(content.width, byHeight, SHEET_MAX_WIDTH_PX)) * zoom;
  const width = wrapper - BORDERS;
  return { width, height: (width * 7) / 5 };
}
