# 11 — Hardware observations, 2026-09-25

What was seen on the Note Air 4C (firmware 4.2, see [00 §0](00-overview.md)) with the panel driver log
and the call tracer running together. Every line quoted here was recorded; nothing in this document
is inferred unless it says so.

## Tools

- **Driver log**: `adb logcat -b all -v threadtime`, grepping `onyx_tcon_send_cmd`,
  `HWEpdcManager: ### EUS`, `SurfaceFlinger: Pid N State change`, `Post enabled`, `EpdcManager: CU`,
  `HandlePenTrigger`.
- **Call tracer**: a libxposed API 102 module built into the Tangleaf debug APK
  (`src-tauri/gen/android/app/src/debug/java/dev/okhsunrog/tangleaf/pentrace/`), scoped to
  `system`, `com.onyx.android.note` and `dev.okhsunrog.tangleaf` through Vector 2.2. It logs, under
  the tag `PenTrace`, every call into `ViewUpdateHelper` (decoding each SurfaceFlinger transaction
  and its payload), `EInkHelper`, the Onyx `View` methods, the app's Pen SDK (`TouchHelper`,
  `EpdController`, `RawInputReader` including its native entry points, the raw-input callbacks),
  and in `system_server` the EinkWise refresh and display implementations and
  `ValidateAppConfigAction.supportEAC`. Vector loads a changed module only after a reboot; a changed
  app loads on relaunch.
- **Condenser**: `~/tmp_zfs/pen-observation/penlog.py` (top-level calls, SF transactions, driver lines,
  runs of identical updates collapsed). Raw recordings are in `~/tmp_zfs/pen-observation/`.
- Actions were marked in the log with `adb shell log -t PenMark <action>`.

## EinkWise

- `ValidateAppConfigAction.supportEAC` returns `false` for `com.onyx.android.note` and `true` for
  `dev.okhsunrog.tangleaf`. The firmware decides this; the stock app never writes it.
- On every resume `system_server` applies the stored profile to the foreground app. Stock gets
  `DEBOUNCER [0, 0, 0, 0, 0]`; Tangleaf gets `DEBOUNCER [1, 5, 80, 400, 20]` and a
  `DEBOUNCE_INC_REFRESH` for every lift (the firmware's tap counter).

## Stock Notes

**Open a page (A1).** `TouchHelper.create(EditorView, 3, …)`, limit = the view, `openRawDrawing`
(pen state 1, `ENABLE_POST(0)`), then straight away `forceSetRawDrawingEnabled(false)` →
`ENABLE_POST(1)`, pen state 3. About 1.8 s later the resume sends region mode, colour, style 1,
width, `setStrokeParameters(1, [0.3, 0.6])`, `(5, [5.0])`, `(8, [80.3, 0.5, 0.1])`,
`setEraserRawDrawingEnabled(true, 8)`, brush on, pen state 2. The exclude table gets
`[[0,0][0,0]]` (after a transient `[[0,0][0,0], [0,0][1755,2340]]` while loading).

**Write (A1, A2).** No call at all while writing or after it: SF switches itself to scheme 3 at
pen-down (`HandlePenTrigger`, `EUS s 3`) and stays there. `onPenUpRefresh` arrives 500 ms after each
lift and the app does nothing with it. Ten strokes and 30 s of waiting produced no SF transaction and
no panel update.

**Erase with the eraser end (A4, A5).** The session stays running: no pause while the eraser is on
the glass, SF draws the eraser trace. Only `ENABLE_COLOR_CU [1]` at down and `[0]` at lift. After the
lift: `resetViewUpdateMode`; `ENABLE_POST(1)` (`Post enabled!`, `EUS s 2`) while the session still
runs; ~170 ms later render off (pen state 3) and one partial update `waveform[2] update_mode[0]` of
the erased area; ~340 ms later the full re-send of regions, stroke settings, eraser style 8 and brush,
then pen state 2. No A2, no full-screen update. The paused window lasts about 270–620 ms after the
lift; flipping the stylus took 0.86–1.4 s, so a tip stroke never landed in it.

**Erase with the eraser tool (A7).** The eraser tool is an ordinary pen stroke
(`onBeginRawDrawing`) with render on: selecting it makes style 8 the main stroke style with
parameters `(8, [80.3, 0.5, 0.1])` (soft eraser) or style 5 with `(8, [5.9, 0.5, 0.1])` (area
eraser). After each lift the same hand-back as the eraser end. No full-screen update.

**Menus.** Each close of a popup menu is `ENABLE_REGAL [1]`, `setViewDefaultUpdateMode(REGAL_PLUS)`,
`SET_APPLICATION_DISPLAY_SCHEME [3]` and one `waveform[16] update_mode[0]` repaint — the flash the
user sees after closing the eraser menu. It is partial, not a full-screen GC16. Eight menu closes
gave eight such updates. Selecting a tool from the toolbar pauses the session with
`0x400000`-flagged partial updates and no full-screen update.

## Tangleaf, and the cause of the flash after erasing

**Write (T-A1).** The WebView receives the pen as ordinary MotionEvents as well. The framework's
WebView patch (`WebView.onTouchEvent` → `applyFastMode` → `View.beforeScroll` →
`EInkHelper.beforeScroll`, confirmed with a Java stack trace) calls `applyTransientUpdate(2308)` on
any vertical travel of 5 px or more, and `clearTransientUpdate(true)` one second
(`scroll_refresh_delay_v2` = 1000) after the lift. While the session runs, SF stays in scheme 3 and
nothing shows.

**Erase with the eraser end (T-A4).** Our code turns render off at eraser-down (pen state 3). The
same WebView path then fires: `APPLY_TRANSIENT_UPDATE [2308]` → `EUS s 4` → `waveform[4] flag[0x2000]`
frames; one second after the lift `CLEAR_TRANSIENT_UPDATE [1]` → `EUS s 2` →
`waveform[2] update_mode[1] flags[0x1000]`, the full-screen flash. The WebView also scrolled the page
slightly under the eraser. So the flash comes from the framework's scroll-A2 path in our own process,
not from SurfaceFlinger reacting to pen input in a paused session; pausing only removes the scheme-3
guard that otherwise keeps the A2 request from reaching the panel. Stock has no WebView and is
exempt from the other scroll entries (`com.onyx` package), so none of this happens there.

**After keeping the pen on the sheet away from the WebView (F6-A4).** `OnyxInk` now consumes stylus
and eraser-end gestures that start inside the drawing region and not on a floating control
(`InkStylusRouting`). Three erases with the eraser end: no `beforeScroll`, no `EUS s 4`, no
`update_mode[1]`; the page did not move; pen strokes, the eraser tool, lasso, dragging a selection,
its menu and the toolbar kept working.

**What is left (R).** Pen taps that still reach the WebView — toolbar buttons, the selection menu —
can move 5 px and queue the same A2 request. SF holds it while in scheme 3; the next exit from
handwriting (a tool switch, the pause after deleting a selection) applies it (`Post enabled!`,
`EUS s 4`) and one second later the full-screen update follows. Stock's behaviour for deleting a
selection (A10) has not been observed yet.

**Finger scrolling and pinch on the handwriting page do not work**, and never did: the canvas is
`touch-none` and covers the sheet, and its finger handler only drags an existing selection.
