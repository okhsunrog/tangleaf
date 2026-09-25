import { useLayoutEffect } from "react";
import { create } from "zustand";
import { setImmersiveMode } from "@/lib/api";
import { requestFullRefreshSoon } from "./eink-refresh";
import { suppressOnyxInk } from "./ink-suppression";
import {
  onNativeSafeAreaChange,
  readSafeAreaVariables,
  restoreSafeAreaVariables,
} from "./safe-area";

/**
 * Sheet mode: while a handwritten note is the page on screen of a compact Android layout, the
 * note gets the whole screen, the way the stock notes app gives its page the screen. The system
 * bars, the app header, the pane bar and the pane tabs step aside, and the note's own toolbar
 * (with its Back) is the only chrome left.
 *
 * Desktop never enters it: its header carries the window controls and the drag region.
 */
export type SheetModeInput = {
  compact: boolean;
  android: boolean;
  /** The active pane shows a handwritten note. In compact layout that is the visible pane. */
  handwriting: boolean;
  /** The workspace is what is on screen, not settings or the startup screen. */
  workspace: boolean;
};

/**
 * Derived state, not an effect of the editor mounting: going from one handwritten note to another
 * keeps it on, so the bars do not flicker between notes.
 */
export function sheetModeActive({ compact, android, handwriting, workspace }: SheetModeInput) {
  return compact && android && handwriting && workspace;
}

export function isAndroid(userAgent: string = navigator.userAgent): boolean {
  return userAgent.toLocaleLowerCase().includes("android");
}

/** For the parts that live outside the App tree, such as the toaster. */
export const useSheetMode = create<{ sheet: boolean }>(() => ({ sheet: false }));

/** Long enough for the insets of a hidden or shown bar to reach the page. */
export const SYSTEM_BARS_SETTLE_MS = 500;

/**
 * Holds the firmware pen down under `reason` until the sheet has settled after the native safe
 * area next changes, or `SYSTEM_BARS_SETTLE_MS` passes, then asks for one full refresh. The sheet
 * moves while the bars come and go, and the stock app shows a dead ink rectangle when it draws
 * across that transition. Returns a function that drops the hold early.
 *
 * The change event only writes the CSS variables; the sheet moves at the next layout, and the pen
 * session hears of it from its resize observers in that same frame. So the hold ends one frame
 * later, once that `configure` has gone out, and the pen comes back with the new geometry rather
 * than being re-armed on the old one and reconfigured a moment after. Another change in the
 * meantime restarts the wait; the timer caps it.
 */
function holdPenUntilInsetsSettle(reason: string): () => void {
  let holding = true;
  let frame: number | undefined;
  const release = (refresh: boolean) => {
    if (!holding) return;
    holding = false;
    clearTimeout(timer);
    if (frame !== undefined) cancelAnimationFrame(frame);
    unsubscribe();
    suppressOnyxInk(reason, false);
    if (refresh) requestFullRefreshSoon();
  };
  suppressOnyxInk(reason, true);
  const unsubscribe = onNativeSafeAreaChange(() => {
    if (frame !== undefined) cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(() => release(true));
    });
  });
  const timer = setTimeout(() => release(true), SYSTEM_BARS_SETTLE_MS);
  return () => release(false);
}

const ENTER_REASON = "system-bars";
const LEAVE_REASON = "system-bars-restore";

/**
 * Hides the system bars for as long as `sheet` is on, and gives them back after. A no-op off
 * Android.
 *
 * Leaving writes the windowed insets back before the bars return, so the app header never sits
 * under a bar that has just reappeared; the next native change corrects any difference.
 *
 * A layout effect on purpose: the commit that turns sheet mode on or off also adds or removes the
 * header and the pane bar, which resizes the sheet, and the pen session reconfigures from its
 * resize observers before the browser paints. The hold (and, on leaving, the restored insets) has
 * to be in place before that, not in a passive effect that may run after it.
 */
export function useSheetSystemBars(sheet: boolean): void {
  useLayoutEffect(() => {
    if (!sheet || !isAndroid()) return;
    const windowed = readSafeAreaVariables();
    const dropEnterHold = holdPenUntilInsetsSettle(ENTER_REASON);
    void setImmersiveMode(true).catch(() => dropEnterHold());
    return () => {
      dropEnterHold();
      restoreSafeAreaVariables(windowed);
      const dropLeaveHold = holdPenUntilInsetsSettle(LEAVE_REASON);
      void setImmersiveMode(false).catch(() => dropLeaveHold());
    };
  }, [sheet]);
}
