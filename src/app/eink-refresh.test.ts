import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import {
  refreshPanelAfterClose,
  refreshPanelNow,
  requestFullRefreshSoon,
  resetEinkRefresh,
  setEinkRefreshEnabled,
} from "./eink-refresh";

const api = vi.hoisted(() => ({ requestFullRefresh: vi.fn() }));
const ink = vi.hoisted(() => ({ suppressOnyxInk: vi.fn() }));
const core = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@tauri-apps/api/core")>();
  return { ...actual, ...core };
});

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, ...api };
});

vi.mock("@/app/ink-suppression", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/app/ink-suppression")>();
  return { ...actual, ...ink };
});

beforeEach(() => {
  vi.useFakeTimers();
  api.requestFullRefresh.mockResolvedValue(null);
  core.invoke.mockResolvedValue(null);
});

afterEach(() => {
  resetEinkRefresh();
  vi.useRealTimers();
  api.requestFullRefresh.mockReset();
  ink.suppressOnyxInk.mockReset();
  core.invoke.mockReset();
});

it("a refresh the user asks for holds the pen down around a full-screen repaint", async () => {
  refreshPanelNow();
  // The pen goes down first, so SurfaceFlinger has left the handwriting schema by the flash.
  expect(ink.suppressOnyxInk.mock.calls).toEqual([["manual-refresh", true]]);
  expect(core.invoke).not.toHaveBeenCalled();

  await vi.advanceTimersByTimeAsync(200);
  // Stock's Refresh button, not the partial overlay refresh.
  expect(core.invoke.mock.calls).toEqual([["plugin:mobile-system|repaint_panel"]]);
  expect(api.requestFullRefresh).not.toHaveBeenCalled();

  await vi.advanceTimersByTimeAsync(299);
  expect(ink.suppressOnyxInk).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(1);
  expect(ink.suppressOnyxInk).toHaveBeenLastCalledWith("manual-refresh", false);
});

it("the pen comes back even when the repaint fails", async () => {
  core.invoke.mockRejectedValue(new Error("panel is busy"));
  refreshPanelNow();
  await vi.advanceTimersByTimeAsync(500);
  expect(ink.suppressOnyxInk).toHaveBeenLastCalledWith("manual-refresh", false);
});

it("never touches the panel on a display that does not need it", () => {
  requestFullRefreshSoon();
  vi.advanceTimersByTime(10_000);
  expect(api.requestFullRefresh).not.toHaveBeenCalled();
});

it("refreshes once the screen has settled", () => {
  setEinkRefreshEnabled(true);
  requestFullRefreshSoon();

  vi.advanceTimersByTime(299);
  expect(api.requestFullRefresh).not.toHaveBeenCalled();
  vi.advanceTimersByTime(1);
  expect(api.requestFullRefresh).toHaveBeenCalledTimes(1);
});

it("collapses a burst of changes into a single flash", () => {
  setEinkRefreshEnabled(true);
  // A dialog closing into a navigation, or several back presses in a row.
  requestFullRefreshSoon();
  vi.advanceTimersByTime(200);
  requestFullRefreshSoon();
  vi.advanceTimersByTime(200);
  requestFullRefreshSoon();

  vi.advanceTimersByTime(299);
  expect(api.requestFullRefresh).not.toHaveBeenCalled();
  vi.advanceTimersByTime(1);
  expect(api.requestFullRefresh).toHaveBeenCalledTimes(1);
});

it("separate settled changes each get their own refresh", () => {
  setEinkRefreshEnabled(true);
  requestFullRefreshSoon();
  vi.advanceTimersByTime(300);
  requestFullRefreshSoon();
  vi.advanceTimersByTime(300);
  expect(api.requestFullRefresh).toHaveBeenCalledTimes(2);
});

it("drops a scheduled refresh when the display stops being e-ink", () => {
  setEinkRefreshEnabled(true);
  requestFullRefreshSoon();
  // The user picked the standard profile before the pending flash was due.
  setEinkRefreshEnabled(false);
  vi.advanceTimersByTime(10_000);
  expect(api.requestFullRefresh).not.toHaveBeenCalled();
});

it("a closing overlay schedules a refresh and still runs its own handler", () => {
  setEinkRefreshEnabled(true);
  const handler = vi.fn();
  const onOpenChange = refreshPanelAfterClose<[{ reason: string }]>(handler);

  onOpenChange(true, { reason: "trigger-press" });
  vi.advanceTimersByTime(300);
  expect(api.requestFullRefresh).not.toHaveBeenCalled();

  onOpenChange(false, { reason: "escape-key" });
  expect(handler.mock.calls).toEqual([
    [true, { reason: "trigger-press" }],
    [false, { reason: "escape-key" }],
  ]);
  vi.advanceTimersByTime(300);
  expect(api.requestFullRefresh).toHaveBeenCalledTimes(1);
});

it("an overlay with no handler of its own is still safe to close", () => {
  setEinkRefreshEnabled(true);
  refreshPanelAfterClose()(false);
  vi.advanceTimersByTime(300);
  expect(api.requestFullRefresh).toHaveBeenCalledTimes(1);
});

it("a failed refresh is not worth bothering the user with", async () => {
  setEinkRefreshEnabled(true);
  api.requestFullRefresh.mockRejectedValue(new Error("panel is busy"));
  requestFullRefreshSoon();
  vi.advanceTimersByTime(300);
  await vi.waitFor(() => expect(api.requestFullRefresh).toHaveBeenCalledTimes(1));
});

it("an overlay closing cleans the panel once things settle", () => {
  setEinkRefreshEnabled(true);
  refreshPanelAfterClose()(false);
  vi.advanceTimersByTime(1000);
  expect(api.requestFullRefresh).toHaveBeenCalledTimes(1);
});
