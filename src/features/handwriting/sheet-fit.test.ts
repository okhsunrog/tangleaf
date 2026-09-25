import { describe, expect, it } from "vite-plus/test";
import { SHEET_WIDTH_CSS, fitSheet } from "./sheet-fit";

/** The editor's viewport content box: the window minus the toolbar and the gutters. */
function viewport(window: { width: number; height: number }, chromeAbove: number, gutter = 6) {
  return { width: window.width - 2 * gutter, height: window.height - chromeAbove - 2 * gutter };
}

// The Note Air 4C WebView at the per-app DPI: 1860x2480 px at DPR 2.8125.
const NOTE_AIR = { width: 661.33, height: 881.78 };
const DPR = 2.8125;
const TOOLBAR = 44;
const STATUS_BAR = 22.04;

describe("fitSheet", () => {
  it("shows the whole sheet on the Note Air in sheet mode", () => {
    const canvas = fitSheet(viewport(NOTE_AIR, TOOLBAR));

    // Height-limited: the sheet touches the top and bottom gutters.
    expect(canvas.height).toBeCloseTo(822.8, 1);
    expect(canvas.width).toBeCloseTo(587.7, 1);
    expect(canvas.width / canvas.height).toBeCloseTo(5 / 7, 6);
    // In panel pixels, and below the toolbar, the gutter and the border: stock's page starts at 140.
    expect(Math.round(canvas.height * DPR)).toBe(2314);
    expect(Math.round((TOOLBAR + 6 + 1) * DPR)).toBe(143);
    // Everything fits, including the border and the one pixel of slack.
    expect(canvas.height + 2 + 1).toBeCloseTo(viewport(NOTE_AIR, TOOLBAR).height, 6);
  });

  it("still shows the whole sheet with the status bar kept", () => {
    const canvas = fitSheet(viewport(NOTE_AIR, STATUS_BAR + TOOLBAR));
    expect(canvas.width).toBeCloseTo(572, 0);
    expect(canvas.height).toBeCloseTo(800.7, 1);
  });

  it("fits the width on a phone, top-aligned with room below", () => {
    const phone = viewport({ width: 390, height: 844 }, TOOLBAR);
    const canvas = fitSheet(phone);
    expect(canvas.width).toBe(376);
    expect(canvas.height).toBeCloseTo(526.4, 1);
    expect(phone.height - canvas.height).toBeGreaterThan(250);
  });

  it("fits the height in landscape", () => {
    const canvas = fitSheet(viewport({ width: 881.78, height: 661.33 }, TOOLBAR));
    expect(canvas.height).toBeCloseTo(602.3, 1);
    expect(canvas.width).toBeCloseTo(430.2, 1);
  });

  it("stops shrinking at the floor, so a tiny viewport scrolls instead", () => {
    const canvas = fitSheet(viewport({ width: 844, height: 390 }, TOOLBAR));
    expect(canvas.width).toBe(278);
    expect(canvas.height).toBeGreaterThan(viewport({ width: 844, height: 390 }, TOOLBAR).height);
  });

  it("caps the sheet on large desktop panes", () => {
    expect(fitSheet({ width: 2000, height: 3000 }).width).toBe(898);
  });

  it("scales the fitted width by the zoom", () => {
    const fitted = fitSheet(viewport(NOTE_AIR, TOOLBAR));
    const zoomed = fitSheet(viewport(NOTE_AIR, TOOLBAR), 2);
    expect(zoomed.width + 2).toBeCloseTo((fitted.width + 2) * 2, 6);
  });
});

describe("SHEET_WIDTH_CSS", () => {
  it("is the same formula, in container units of the viewport", () => {
    expect(SHEET_WIDTH_CSS).toBe(
      "calc(max(280px, min(100cqw, (100cqh - 3px) * 5 / 7 + 2px, 900px)) * var(--sheet-zoom, 1))",
    );
  });
});
