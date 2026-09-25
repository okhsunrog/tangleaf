// @vitest-environment jsdom
import { act, useLayoutEffect } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { nativeSafeAreaChanged } from "./safe-area";
import {
  SYSTEM_BARS_SETTLE_MS,
  isAndroid,
  sheetModeActive,
  useSheetSystemBars,
} from "./sheet-mode";

const mocks = vi.hoisted(() => ({
  setImmersiveMode: vi.fn(),
  suppressOnyxInk: vi.fn(),
  requestFullRefreshSoon: vi.fn(),
}));
vi.mock("@/lib/api", () => ({ setImmersiveMode: mocks.setImmersiveMode }));
vi.mock("./ink-suppression", () => ({ suppressOnyxInk: mocks.suppressOnyxInk }));
vi.mock("./eink-refresh", () => ({ requestFullRefreshSoon: mocks.requestFullRefreshSoon }));

const ANDROID = "Mozilla/5.0 (Linux; Android 14; NoteAir4C) AppleWebKit/537.36 Chrome/151.0";
const DESKTOP = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15";

describe("sheetModeActive", () => {
  const on = { compact: true, android: true, handwriting: true, workspace: true };

  it("is on only for a handwritten note in the compact Android workspace", () => {
    expect(sheetModeActive(on)).toBe(true);
    // A narrow desktop window is compact too, but its header holds the window controls.
    expect(sheetModeActive({ ...on, android: false })).toBe(false);
    // Android tablets at least 1000 CSS px wide keep the desktop layout.
    expect(sheetModeActive({ ...on, compact: false })).toBe(false);
    expect(sheetModeActive({ ...on, handwriting: false })).toBe(false);
    // Settings and the startup screen replace the workspace; the bars come back for them.
    expect(sheetModeActive({ ...on, workspace: false })).toBe(false);
  });

  it("recognises the Android WebView by its user agent", () => {
    expect(isAndroid(ANDROID)).toBe(true);
    expect(isAndroid(DESKTOP)).toBe(false);
  });
});

describe("useSheetSystemBars", () => {
  let container: HTMLDivElement;
  let root: Root;
  const style = document.documentElement.style;

  function Bars({ sheet }: { sheet: boolean }) {
    useSheetSystemBars(sheet);
    return null;
  }
  const render = (sheet: boolean) => act(async () => root.render(<Bars sheet={sheet} />));
  const suppressed = () => mocks.suppressOnyxInk.mock.calls.map(([reason, on]) => [reason, on]);
  const lastSuppression = () => suppressed()[suppressed().length - 1];
  /** The frame that lays out the new insets, and the one after it. */
  const settleLayout = () =>
    act(async () => {
      vi.advanceTimersToNextFrame();
      vi.advanceTimersToNextFrame();
    });

  beforeEach(() => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    vi.useFakeTimers();
    for (const mock of Object.values(mocks)) mock.mockReset();
    mocks.setImmersiveMode.mockResolvedValue(null);
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue(ANDROID);
    // What the startup query wrote while the status bar was showing.
    style.setProperty("--safe-area-inset-top", "22px");
    style.setProperty("--safe-area-inset-bottom", "0px");
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    for (const edge of ["top", "right", "bottom", "left"]) {
      style.removeProperty(`--safe-area-inset-${edge}`);
    }
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it("holds the pen until the insets of the hidden bars arrive", async () => {
    await render(true);

    expect(mocks.setImmersiveMode).toHaveBeenCalledWith(true);
    expect(suppressed()).toEqual([["system-bars", true]]);
    expect(mocks.requestFullRefreshSoon).not.toHaveBeenCalled();

    await act(async () => nativeSafeAreaChanged({ top: 0, right: 0, bottom: 0, left: 0 }));

    expect(style.getPropertyValue("--safe-area-inset-top")).toBe("0px");
    // The variables are written, but the sheet only moves at the next layout, and the pen session
    // reconfigures from that layout. The pen stays down until it has.
    expect(suppressed()).toEqual([["system-bars", true]]);

    await settleLayout();
    expect(suppressed()).toEqual([
      ["system-bars", true],
      ["system-bars", false],
    ]);
    expect(mocks.requestFullRefreshSoon).toHaveBeenCalledTimes(1);

    // Later inset changes (a rotation, the keyboard) are none of its business.
    await act(async () => nativeSafeAreaChanged({ top: 0, right: 0, bottom: 0, left: 0 }));
    await settleLayout();
    expect(suppressed()).toHaveLength(2);
  });

  it("waits again when the insets change twice", async () => {
    await render(true);

    await act(async () => nativeSafeAreaChanged({ top: 0, right: 0, bottom: 24, left: 0 }));
    await act(async () => vi.advanceTimersToNextFrame());
    await act(async () => nativeSafeAreaChanged({ top: 0, right: 0, bottom: 0, left: 0 }));
    await act(async () => vi.advanceTimersToNextFrame());
    expect(suppressed()).toEqual([["system-bars", true]]);

    await act(async () => vi.advanceTimersToNextFrame());
    expect(lastSuppression()).toEqual(["system-bars", false]);
  });

  it("holds the pen within the commit, before the sheet's resize observers run", async () => {
    // A layout effect later in the same commit stands in for the frame's resize observers: both
    // run after the header and pane bar leave the DOM and before the browser paints.
    let heldAtLayout: unknown[][] = [];
    function Sheet() {
      useLayoutEffect(() => {
        heldAtLayout = suppressed();
      }, []);
      return null;
    }
    await act(async () =>
      root.render(
        <>
          <Bars sheet />
          <Sheet />
        </>,
      ),
    );

    expect(heldAtLayout).toEqual([["system-bars", true]]);
  });

  it("gives the pen back after a while even if no insets arrive", async () => {
    await render(true);

    await act(async () => vi.advanceTimersByTime(SYSTEM_BARS_SETTLE_MS - 1));
    expect(suppressed()).toEqual([["system-bars", true]]);
    await act(async () => vi.advanceTimersByTime(1));
    expect(suppressed()).toEqual([
      ["system-bars", true],
      ["system-bars", false],
    ]);
  });

  it("puts the windowed insets back before the bars return", async () => {
    await render(true);
    await act(async () => nativeSafeAreaChanged({ top: 0, right: 0, bottom: 0, left: 0 }));
    await settleLayout();

    let topWhenShown = "";
    mocks.setImmersiveMode.mockImplementation(async () => {
      topWhenShown = style.getPropertyValue("--safe-area-inset-top");
      return null;
    });
    await render(false);

    expect(mocks.setImmersiveMode).toHaveBeenLastCalledWith(false);
    // The header must never lay out under a status bar that has just reappeared.
    expect(topWhenShown).toBe("22px");
    expect(style.getPropertyValue("--safe-area-inset-top")).toBe("22px");
    // The sheet moves again while the bars come back, so the pen is held for that too.
    expect(lastSuppression()).toEqual(["system-bars-restore", true]);
    await act(async () => nativeSafeAreaChanged({ top: 22, right: 0, bottom: 0, left: 0 }));
    await settleLayout();
    expect(lastSuppression()).toEqual(["system-bars-restore", false]);
  });

  it("releases the entry hold when the note closes before the insets arrive", async () => {
    await render(true);
    await render(false);

    const releases = suppressed().filter(([, on]) => !on);
    expect(releases).toContainEqual(["system-bars", false]);
  });

  it("does nothing off Android", async () => {
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue(DESKTOP);
    await render(true);
    await render(false);

    expect(mocks.setImmersiveMode).not.toHaveBeenCalled();
    expect(mocks.suppressOnyxInk).not.toHaveBeenCalled();
  });
});
