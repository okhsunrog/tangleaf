# 00 — Stock Notes pen and panel path on the Note Air 4C, end to end

This is the reconciled description of how the stock BOOX Notes app (`com.onyx.android.note`,
"knote2") drives the pen session and the e-ink panel on a BOOX Note Air 4C. It is built from the
subsystem documents 01–08 in this directory and from a reconciliation pass that re-read the sources
where the documents disagreed (listed in [09](09-contradictions-and-gaps.md)). It is static
analysis only. Nothing here has been observed on the tablet; [10](10-observation-protocol.md) is
the plan for that. No fixes and no design for Tangleaf are proposed.

Tags, as in every document here: **[code]** read in source or disassembly (the cited document gives
the file and line), **[inferred]** reasoned from code, **[unknown]** not settled statically. A
section reference like "01 §4.4" points to the subsystem document that holds the detail and the
line-level citations.

| doc                                    | subject                                                                              |
| -------------------------------------- | ------------------------------------------------------------------------------------ |
| [01](01-pen-session-lifecycle.md)      | pen session lifecycle: open, park, pause, resume, close; the `PenEventHandler` gates |
| [02](02-writing.md)                    | writing: live ink, pen-up, SF handwriting schema                                     |
| [03](03-erasing.md)                    | erasing in every mode, SF eraser reader                                              |
| [04](04-selection-and-lasso.md)        | lasso, selection, transforms                                                         |
| [05](05-panel-refresh-and-einkwise.md) | app refresh policy, fast mode, counted GC, EinkWise                                  |
| [06](06-input-routing.md)              | finger, palm, eraser end, overlays                                                   |
| [07](07-firmware-state-machine.md)     | SurfaceFlinger and framework state machine, update schemes, the flash mechanism      |
| [08](08-tangleaf-current-path.md)      | Tangleaf's current path in the same terms                                            |
| [09](09-contradictions-and-gaps.md)    | contradictions between the documents, resolutions, gaps                              |
| [10](10-observation-protocol.md)       | hardware observation plan with predicted driver-log sequences                        |

## 0. Build identity

The extracted images are firmware 4.2, build 8548 (`2026-04-28_17-50_4.2-rel_04282_555977efe`).
Their vendor partition reports Android 11 (`ro.vendor.build.version.sdk=30`,
`vendor.img:/build.prop`) and their system partition Android 13 (`ro.system.build.version.sdk=33`,
`system.img:/system/build.prop`) [code, read in this pass]. The "Android 11" and "Android 13" in
earlier notes therefore describe the same build. The same `vendor/build.prop` sets
`ro.board.platform=lito`, `vendor.onyx.tablet=true`, `vendor.onyx.htcon=true` [code].

**Verified on the tablet (2026-09-25, action A0 done).** The tablet runs exactly this build:
`ro.build.display.id=2026-04-28_17-50_4.2-rel_04282_555977efe`, `ro.build.version.sdk=33`,
`ro.vendor.build.version.sdk=30`, `ro.board.platform=lito`, `vendor.onyx.tablet=true`,
`vendor.onyx.htcon=true`, `scroll_refresh_delay_v2=1000`, stock Notes `versionCode=45326` (the
decompiled APK). `/system/bin/surfaceflinger`, `/system/framework/framework.jar`,
`libonyx_pen_touch_reader.so` and `libneo_pen.so` on the device are byte-identical (sha256) to the
analysed copies under `~/tmp_zfs/onyx_framework/`, so every SurfaceFlinger address and framework
claim in these documents applies to the device. `services.jar` differs only because the device runs
the AMS-fix overlay from `../06-rooting.md`. The "Android 11 / firmware 4.1" in the original research
brief was an unverified statement and is wrong; "Android 11" is only the vendor partition.

## 1. The layers and who owns what

1. **Digitizer, read twice.** SurfaceFlinger (SF) runs its own pen reader (`TouchReader` /
   `PenManager`) and draws live ink straight to the panel. Independently, the stock app's bundled
   SDK loads `libonyx_pen_touch_reader.so` into the app process and reads the same evdev device to
   produce the app's raw callbacks (02 §4, 06 §3.2, 07 §2.1) [code]. Android also delivers the stylus
   as `MotionEvent`s; stock uses that copy only for bookkeeping, HWR hold detection and the lasso
   path (06 §1) [code].
2. **Bundled SDK.** `NoteManager` holds one `TouchHelper` created with feature mask 3
   (`SFTouchRender` + `AppTouchRender`) and the SDK touch listener off (01 §4.1) [code]. Every SDK
   call ends in `SDMDevice` → reflection on `android.onyx.ViewUpdateHelper` → a Binder transaction to
   SF (01 §3.1) [code]. Device class `SDMDevice` follows from `lito` [code + image fact].
3. **App policy.** `PenEventHandler` owns every resume and its gates; `EpdEventHandler` owns fast
   mode; `EpdDeviceManager` (app copy) owns the counted GC; `InkRenderManager` does every repaint
   (01 §2, 05 §2) [code].
4. **Framework.** `ViewUpdateHelper` parcels; `View.invalidate()` carries the view's default update
   mode to `ViewRootImpl.addUpdateEntry` (05 §4.3) [code]. The scroll-A2 manager in `ViewRootImpl`
   (`OnyxScrollRefreshManager`) skips any package containing `"com.onyx"`, so stock is outside it
   (07 §6) [code].
5. **SF.** Pen owner pid, reader states, handwriting schema 7, update scheme (2, 3, 4), the fixed
   update-entry slots (app scope and transient), one-shot GC entry, global region store, global
   brush/eraser raw-drawing flags (07 §2) [code].

## 2. The session state machine

### 2.1 App-side states

States are the triple (last pen state sent to SF, `TouchHelper` render flag, input flag) (01 §7.1)
[code]:

| state | pen state | render | input | when stock is in it                                                                    |
| ----- | --------- | ------ | ----- | -------------------------------------------------------------------------------------- |
| U     | none / 0  | –      | –     | no `TouchHelper` (before first bind, after close)                                      |
| O     | 1 START   | off    | off   | bound without window focus (split screen), or after `restartRawDrawing`                |
| P     | 3 PAUSE   | off    | off   | any UI state: menus, popups, dialogs, focus loss, a tool that refuses render and input |
| R     | 3 PAUSE   | off    | on    | post-erase window (~300 ms); fill-colour tool; selection shown (TRANSFORM)             |
| D     | 2 DRAWING | on     | on    | writing, erasing (types 0/1/2), lasso waiting (smart/free)                             |
| Dr    | 2 DRAWING | on     | off   | only via a render-only `PenEvent` while input was off (rare)                           |

Transitions and their SF-visible transactions (01 §4, §7.1; 07 §3) [code]:

- **Open** (`bindPenHostView(view, focused=true)`): `SET_STROKE_STYLE(0)`, pen state 1, style 0
  again, `ENABLE_POST(0)`, pen state 1 again (ignored by SF, same state), then the park:
  `ENABLE_POST(1)`, pen state 3, `ENABLE_POST(1)`. State U → P.
- **Pause, both flags** (`PauseRawPenAction(pauseArgs)`, "PR"): `ENABLE_POST(1)`, pen state 3,
  `ENABLE_POST(1)` only if render was on; then always `SET_BRUSH_RAW_DRAWING_ENABLED(1)` and
  `SET_ERASER_RAW_DRAWING_ENABLED(0, 5)` (`resetPenDefaultRawDrawing`, even when already paused)
  (01 §4.5) [code].
- **Pause, render only**: `ENABLE_POST(1)`, pen state 3, `ENABLE_POST(1)`, no flag reset (03 §3.4)
  [code].
- **Resume** ("RS(d)"): only through `PenEvent` → `PenEventHandler.q()` gates → `ResumeRawDrawingRequest`
  on the single `pen` thread; nothing happens unless render or input is actually off. Then: sleep
  `d` (500 ms default on a colour panel), limit ×2, exclude ×2 (`[Rect(0,0,0,0)]` shifted to the view
  origin when no overlay slot is set; see 09 C4), region mode 1, colour, style, width (×2 each, once
  per render), per-pen stroke parameters, `SET_STROKE_PARAMETERS(5,[5.0])`,
  `SET_STROKE_PARAMETERS(8,[eraserWidth,0.5,0.1])`, `SET_ERASER_RAW_DRAWING_ENABLED(1, 8 or 5)`,
  `SET_BRUSH_RAW_DRAWING_ENABLED(canBrushRawRender)`, input on (in-process only), pen state 2 if
  render passed the gate (01 §4.4) [code].
- **Close** (`NoteManager.reset`): style 0, `ENABLE_POST(1)`, pen state 3, pen state 0, style 0,
  `ENABLE_POST(1)`, pen state 0; later one or two `resetEpdPost()` (`ENABLE_POST(1)` + pen state 0).
  Limit and exclude are left in SF (01 §4.6) [code].

The gates (`PenEventHandler.A()` for render, `z()` for input) are evaluated once, on the main
thread, when the `PenEvent` arrives: keyboard, popup counter, toast, status bar, no-focus system
dialog, window focus, system float button, screenshot, scene, host bound, current layer, named
pause registry, document opening, render dialog, float-menu press, and the active tool's
`canPenRawRender()` / `isUsePenInput()` (01 §4.4) [code]. Pauses are never gated. Most note
requests (`BaseNoteRequest` without `setPauseRawDraw(false)`) post an implicit PR that lands
asynchronously after the request body has started (01 §5.2, 04 §5.4) [code + inferred ordering].
There is no implicit resume.

Two named shortcuts used below:

- **INV** = `InvalidateScreenAction` → PR, `InkRenderManager.invalidate()` (a whole-view
  `View.invalidate()` in mode 5, or mode 9 `REGAL_PLUS` once when armed), then RS(500). It is a no-op
  before the document is open, when the active handler is not a `ScribbleHandler`, and while a
  selection is shown (01 §1 item 5) [code].
- **FM on / FM off** = `ApplyFastModeEvent(true/false)` → `EpdEventHandler`: on =
  dither threshold 160, `setEpdTurbo(5)`, `applyTransientUpdate(2308)`; off = reset the counted-GC
  counter, arm a 5 s timer, post an immediate resume; the timer then sets dither 128 and
  `clearTransientUpdate(false)`, plus a PR+RS if a raw pen-up happened in fast mode. The timer does
  nothing if the pen is down when it fires (`TouchStateInfo.isPenRawTouch`), and is not re-armed
  (05 §5.4; 09 C7) [code]. Entry is possible from any handler; only the exit is blocked while the
  active handler `renderInFastMode()` (selection in TRANSFORM, universal shape) (09 C8) [code].

### 2.2 SF-side states

(07 §4–§5, 02 §5.6) [code unless marked]

- **Reader control state** (`TouchReader+0x98`): 0 stop, 1 start, 2 running, 3 toPause, 4 paused,
  5 toResume. Pen state 2 → `ensureStart` + `resume`; 3 → `pause` then `stopHandwriting`; 0 → `stop`
  then `stopHandwriting` (09 C3); 1 → `start`; ≥ 4 → no action (a 4 still logs
  `State change to: paused`). A pen-state transaction equal to the live reader state is ignored; a 3
  sent while the reader is already paused (4) is not equal and runs again. In states 0 and 1 SF
  forwards no samples; while paused it neither inks nor triggers handwriting.
- **Reader selection** (`BtnReader`): tip → reader 2 (draws) if the brush flag is set, else 1
  (no-op); rubber → reader 4 (draws, loads the eraser style into the single painter style slot) if
  the eraser flag is set, else 3 (no-op). The flags are process-global in SF, not per app.
- **Handwriting schema 7.** The first drawn sample with `penTriggered == 0` logs
  `HandlePenTrigger…`, enters schema 7 and calls `setEbcUpdateScheme(3, 0)` (`### EUS, s 3 c 0`).
  An app `MOVE_TO` transaction also enters it. Nothing leaves schema 7 at pen-up. It is left only by
  pen state 3 or 0, `ENABLE_POST(v)` with `v >= 0` and `v != androidDrawing` (in practice
  `ENABLE_POST(1)` while in schema 7), dream or `showImage`. Leaving logs `Post enabled!`, sleeps
  150 ms and calls `clearHandwritingFBSchema`: scheme 4 if the fixed (transient or app-scope) slot
  holds a fast mode, else scheme 2.
- **While in schema 7** app frames reach the panel only if a layer entry carries exactly mode
  `0x80002` (`HAND_WRITING_REPAINT_MODE`) or a queued one-shot entry exists; otherwise they are
  dropped (`### disable refresh.` under SF debug). Whether stock's plain invalidates ever produce such
  an entry is **[unknown]**; the documents infer they do not (02 §5.6).
- **Update scheme** is written only by `setEbcUpdateScheme` (`### EUS, s N c M`, always logged):
  3 from `afterStartHandwriting`; 2/4 from `clearHandwritingFBSchema`, `setFBSchema` (a fixed slot
  changed) and `syncFBSchema`. **SF has no path from a pen state to scheme 4**; scheme 4 always
  means a fast mode sits in a fixed slot (07 §1) [code]. A fixed-slot change arriving while the
  scheme is 3 is stored but does not switch; the switch happens at the next exit from handwriting.
- **Clearing the transient slot** (`CLEAR_TRANSIENT_UPDATE`): if the slot held a fast mode, scheme 2
  (after 100 ms), a one-shot GC only when the clear flag is 1, and in every case a full-screen update
  entry in mode 5 (07 §5.3) [code]. Stock always passes 0. How the panel shows the mode-5
  full-screen update (driver `update_mode[0]` per 05 §4.4, waveform chosen by the kernel) is
  **[unknown]** (09 C10).
- **Driver-log decoding** (05 §4.4) [code]: `update_mode[1]` means bit `0x20` of the mode word; of the
  modes stock uses, only 98 (`UI_GC_MODE`) sets it. Mode nibbles outside 1–6 (e.g. 9, `REGAL_PLUS`)
  log `waveform_mode  wrong!` and go out with waveform 0.

### 2.3 How the stock app avoids pen input into a paused session

Stock keeps render on (state D) whenever the active tool draws or erases through SF (pen, eraser
types 0/1/2, geometric shape, lasso waiting). Every pause comes from a UI event or an erase-finish,
and resumes after 150–800 ms. The documented states in which stock does have pen input with the
session paused are: the ~300 ms after an erase (state R), the 500 ms resume delay after any menu,
the fill-colour tool (R), a shown selection (R, brush flag 0) and the rectangle lasso (P)
(01 §1 item 7, 03 §3.7, 04 §5.3, §5.5) [code]. What SF and the framework do with pen input in those
states on a `com.onyx` package is the central open question (09 C11, C12).

## 3. Per action: stock call sequence and SF-visible effect

Timings are the code's delays; real latency adds scheduling [inferred]. "SF:" lists what SF
receives or does. Log lines expected in the driver log are collected in 10.

### 3.1 Open a note page

1. `OpenDocumentAction`; the view is bound by `RestoreNoteZoomAction` (with a view) or by the layout
   path `RenderToEditorViewAction` → `NoteManager.bindPenHostView(view, focused)` (01 §4.3) [code].
   Which binder runs first on a cold open is **[unknown]**.
2. SF: limit = view's local visible rect (at `TouchHelper` construction); open sequence; park
   (U → P) (§2.1) [code]. With an unfocused split-screen pane the park is skipped (state O).
3. `RestoreNoteZoomAction` resets the exclude registry, sets `scribbleRect = (0,0,w,h)`, displays
   the page and posts `resumePenDelay(600)` → RS(600) → state D (01 §4.3) [code].
4. No GC and no update-mode request of its own on open [code, 05 §3].

### 3.2 Write (pen tool, tip)

(02 §5) [code unless marked]

1. Down: SF reader 2 draws; if `penTriggered == 0`, schema 7 and `EUS s 3`. App: shape model only,
   `RawPenDownEvent` (cancels a pending fast-mode exit). No transaction.
2. Moves: SF inks; app feeds the in-memory model. No transaction.
3. Up: the app flushes the stroke into its page bitmap, `View.invalidate()` (mode 5), then
   `AddShapesBackgroundAction` (not a pausing request) and a second invalidate after the slice
   re-render. No pen state, no `ENABLE_POST`, no update mode.
4. +500 ms: `onPenUpRefresh` → `EpdShapeHandler.P0()` → the per-stroke reconcile
   (`GrayscaleRefreshAction`, `HAND_WRITING_REPAINT_MODE`) is gated off on this device by
   `vendor.onyx.tablet=true`; only the meeting scene reacts (5 s later `ENABLE_POST(1)`).
5. SF stays in schema 7 and scheme 3; the panel keeps SF's ink until the next hand-back (pause,
   `ENABLE_POST(1)`, fast-mode exit with a pen-up) [inferred from the schema-7 output rule].
6. Live-ink waveform and whether colour pens ink in colour: **[unknown]**.

### 3.3 Erase with the eraser tool (tip), types 0 stroke, 1 partial ("move"), 2 area

Selecting the tool from the eraser menu (03 §3.2) [code]:

- `EraseHandler.onActivate`: `ENABLE_COLOR_CU(1)`, PR, RS(150).
- `EraseByTypeAction.t()` → INV (PR, invalidate, RS(500)). The two cycles overlap on the pen thread
  (order [inferred]). Changing only the type while the eraser is active runs the INV alone.
- The resume under `SCRIBBLE_ERASE_PROVIDER` makes style 8 (types 0/1) or 5 (type 2) the main
  stroke style and enables SF eraser raw drawing with the same style.

Per gesture (03 §3.3–§3.4, §4.1) [code]:

1. Down: the tip is a brush touch → SF reader 2 draws with main style 8 (soft eraser, parameters
   `[width, 0.5, 0.1]`, default widths 5.9 / 59 px) or 5 (dash); schema 7 / `EUS s 3` on the first
   sample after a pause. The app reroutes drawing callbacks to erasing (`usePenTipErase`). No pause
   at pen-down (`EraseBeginAction` pauses only non-raw-rendered types, none exist in the menu).
   `ENABLE_COLOR_CU(1)`.
2. During: the app erases in the model on 20 ms (type 0) / 30 ms (types 1, 2) batches; draws
   nothing. Pen state stays 2.
3. Up: `ENABLE_COLOR_CU(0)`; `EraseFinishAction` renders the changed slices and, only if a visible
   shape changed, sends `ENABLE_POST(1)` while still DRAWING (→ SF leaves schema 7: `Post enabled!`,
   `EUS s 2`); after `eraserAreaDisplayDelayMillSecond` = 0 ms a render-only pause (state R), a
   whole-view invalidate (mode 5), and RS(300) → state D with the full re-push.
4. The pen-up refresh timer fires for tip strokes but its consumer is empty.

### 3.4 Erase with the stylus eraser end

- **Pen tool active** (03 §3.3, §4.1; 06 §3.11) [code]: identical pipeline to 3.3 with the currently
  selected eraser type; SF selects reader 4 (`EraseReader`), which writes the eraser style into
  SF's single style slot. `ENABLE_COLOR_CU(1)` at begin and `(0)` at the point list. After lift:
  optional `ENABLE_POST(1)`, render-only pause, invalidate, RS(300); that resume re-sends the pen's
  own style. The session stays running during the gesture. No pen-up refresh for eraser-end strokes.
- **Geometric-shape tool**: as the pen tool (03 §3.7) [code for the gates, SF inferred].
- **Fill-colour tool**: render refused (`canPenRawRender()` false), input on → erasing happens in
  state R; neither side draws live (03 §3.7) [code app side].
- **Universal-shape tool**: render is refused until eraser pen-down, which posts a 0 ms resume while
  the eraser is already on the glass (03 §3.7) [code app side; SF reaction unknown].
- **Selection, HWR, zoom-selection, text, span, layer-lock tools**: no erase, no post-erase pause or
  resume; if SF inks the trace (render on) nothing removes it or restores the style slot
  (03 §3.7) [code app side, inferred SF].

### 3.5 Lasso (smart, the default; free)

(04 §5.1–§5.5) [code unless marked]

1. Tool entry: `ENABLE_COLOR_CU(1)`, pause (PR), invalidate, RS(500) with the selection provider:
   style 5 (dash) ×2, eraser raw drawing on, brush on, pen state 2.
2. Stroke: SF draws the dashed trace; the app collects the lasso path from stylus `MotionEvent`s and
   keeps raw points only for the free-lasso outline.
3. Lift, nothing hit: `SelectShapesRequest` (pausing) → PR; full page-slice re-render; RS(500) → D.
4. Lift, shapes hit: PR (SelectShapesRequest), a second pause from `StartTransformRequest` (only the
   brush/eraser reset reaches SF), dirty-rect and overlay frames, FM on (transient 2308 → scheme 4,
   possibly deferred until the pause ends schema 7), the floating selection menu (does not pause),
   then a reader-only resume after 500 ms that sends `SET_BRUSH_RAW_DRAWING_ENABLED(0)` and eraser
   raw drawing on; pen state stays 3 (state R).
5. How the dashed SF trace disappears when shapes were hit is **[unknown]** (04 E2).

- **Rectangle lasso**: render and input refused (state P); the app draws a dashed rectangle overlay
  every ≥ 30 ms inside an FM on/off pair (04 §5.3) [code].
- **Pen tap without movement in SELECTION**: +300 ms INV (04 §5.6) [code].

### 3.6 Drag, scale, rotate a selection

(04 §5.7–§5.9) [code unless marked]

1. The session stays in state R (pen state 3, reader on, brush flag 0, eraser flag 1); fast mode
   (2308, turbo 5, dither 160) is on since the selection appeared.
2. Stylus or finger `MotionEvent`s drive `SelectionManager`; each step is a non-pausing request plus
   an overlay `View.invalidate()`, batched every 10 ms. No pen-state change, no update mode per step.
3. Lift: popup re-shown; nothing else.
4. Tap on empty space (auto-switch to pen, default on): named pause, `QuitTransformAction`
   (non-pausing; dirty-rect re-render), provider back to pen (`ENABLE_COLOR_CU(0)`), a
   pausing `QuitSelectionRequest`, named resume → PR + RS(500) with pen style → D. Fast mode is
   released by the handler change; 5 s later `clearTransientUpdate(false)` → scheme 2 and the SF
   full-screen mode-5 entry (07 §5.3); turbo is never restored.
5. Delete: as 4 without the named pause first; the removed area is repainted while the transient A2
   mode is still active [inferred].

### 3.7 Open or close a menu, popup, dialog or overlay

(01 §5.4, 05 §5.2, 06 §3.10) [code unless marked]

- **Popup show** (`BasePopWindow` family): popup counter +1 → PR; `PopupChangeAction` also runs an INV
  whose render resume is disabled (input only). `ColorModeEventHandler` turns colour CU on while a
  popup is open if the active tool does not own CU.
- **Popup dismiss**: INV with the `REGAL_PLUS` one-shot (mode 9 → `waveform_mode  wrong!`, waveform
  0), counter −1 → RS(500) (`POPUP_RESUME_PEN_TIME_MS` = 500 on colour, 09 C2). The REGAL one-shot may
  be consumed by any other invalidate that runs first [inferred].
- **Floating selection menu**: no pause (empty `windowChangeEvent`).
- **Settings fragments**: named `PausePenEvent` on create, unnamed `ResumePenEvent()` on destroy
  (clears every named pauser) plus a REGAL repaint.
- **Dialogs** (Onyx `OnyxBaseDialog`): `disableRegal()` / `enableRegal()` ioctls; a dialog that takes
  window focus goes through the focus path (§3.12).
- **Overlays inside the canvas** (float toolbar, zoom preview, page navigator, scroll bars, clipboard,
  link-jump, audio menu): typed exclude rects pushed with `UpdateRawDrawRectAction` (limit ×2,
  exclude ×2), no pen-state change. Pressing a floating-toolbar item pauses (PR + colour CU) and the
  release does not resume by itself; drags of float menus run FM on/off and pause/resume.
- **Toast, status bar, system float button, no-focus system dialog**: flag + pause, gated resume on
  close (01 §7.2) [code].
- **Soft keyboard** (text input): flag + PR; hide → RS(500); text-box editing blocks both gates for
  the whole text session (01 §7.2) [code].

### 3.8 Finger scroll and pinch zoom

(06 §3.5–§3.7) [code unless marked]

- A plain finger tap does not pause. A finger-down while the status-bar flag is set resumes
  (RS(500)).
- **Scroll or pinch begin** (`ViewportScrollBeginEvent` only when the viewport can scroll;
  `ViewportScaleBeginEvent`): FM on (transient 2308 → `EUS s 4` if not in scheme 3) and
  PR + RS(500). The resume does not wait for the fingers; the session returns to D about 500 ms after
  the gesture began [inferred].
- **During pinch**: `UpdateRawPenWidthAction` per frame (`SET_STROKE_WIDTH` ×2).
- **End**: limit/exclude re-push, FM off → immediate resume attempt (no-op when already on), 5 s
  timer → `clearTransientUpdate(false)` → `EUS s 2` + SF full-screen mode-5 entry. Zoom finish also
  runs `TranslateViewPortRequest` (render and input off on the note thread) followed by a throttled
  RS(500).
- While the pen hovers (and 100 ms after), finger moves, pans and pinches are ignored in software
  (`RxGesture` pen-active filter) (06 §3.5) [code].

### 3.9 Page turn (swipe, button, add, delete, goto)

(01 §7.2, 05 §5.3) [code]

1. `BasePageAction` → display of the new page with `pauseArgs` (PR, no resume), then
   `PenEvent.resumeRawDrawing(max(600 − elapsed, 500))`.
2. `EpdDeviceManager.applyGCWithInterval()`: every 10th tick (default `enableFullRefresh =
   isColorDevice()`, count 10; stored value **[unknown]**) `byPass(0)`, `applyGCOnce()` (one-shot 98,
   or `[0xaca250]` while scheme 4) and an INV. The GC lands on whichever app frame composes next
   [inferred].
3. If fast mode is on: `onBeforePageChangeEvent` clears it immediately (dither 128,
   `clearTransientUpdate(false)`).
4. Page-turn ticks are shared with selection-menu clicks, text-menu clicks and rename; the counter
   resets at every fast-mode exit (04 §5.10) [code].

### 3.10 Undo / redo

`UndoRedoAction`: `pauseRawPen(pauseArgs)` (PR), `forceRenderVisibleDirtyScreenRectToDisplay` (mode-5
invalidate of the changed rect), `resumePen()` → RS(500)
(`note/note/action/undoredo/UndoRedoAction.java:199, 265-294, 361-367`) [code, read in this pass].
No session close, no GC. Inside the lasso tool the selection is quit first (04 §5.10).

### 3.11 Leave the page (back to the library, switch document)

- Close: `SaveDocumentAction` with close → pausing `SaveDocumentRequest` → `NoteManager.reset()`
  (close sequence §2.1) → `ScribbleFragment.onDestroy` and `EpdEventHandler.destroy` →
  `resetEpdPost()` each (01 §4.6) [code]. If fast mode is on, `CloseDocumentEvent` clears it at once.
  No GC request; SF regions stay.
- Switching to another document: `OpenDocumentStatusEvent(true)` blocks resumes, save + close,
  open + bind (park), `OpenDocumentStatusEvent(false)` + `PenEvent.resumeRawDrawing()` (01 §4.3)
  [code].

### 3.12 Background / foreground and window focus

- Focus lost (recents, Home, dialog activity, lock screen): `ActivityFocusChangedEvent` → INV whose
  resume is refused (`hasFocus` false) → state P; touches are dropped while unfocused (01 §5.4)
  [code].
- `ScribbleMainFragment.onPause` → `SaveDocumentAction` (pausing request, then a gated resume that the
  focus flag blocks) [code + inferred].
- Activity `onPause/onStop/onResume` make no other pen call; the session is **not closed** when the
  app goes to the background [code].
- Focus regained: INV → RS(500) → D [code]. Screen off/on has no dedicated handling (01 §7.2)
  [code]; the effect through keyguard focus is **[unknown]**.

### 3.13 Tool switch (pen ↔ eraser ↔ lasso ↔ shape)

`HandlerManager.activeProvider` → old `onDeactivate`, new `onActivate` (colour CU per handler,
`EraseHandler` PR + RS(150), lasso PR + invalidate + RS(500)); most menu entry points also run an INV
(01 §7.2, 03 §3.2, 04 §5.1) [code]. Whether every switch between two `ScribbleHandler`s adds an INV
was not fully traced **[unknown]**.

## 4. Side by side: stock and Tangleaf (from 08)

"PS" = pen-state transactions. Tangleaf details: 08 §5–§6.

| action                   | stock                                                                                                                                          | Tangleaf                                                                                                                                                                                                                                     |
| ------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| open page                | open (1,1, `ENABLE_POST(0)`), park (3), RS(600) → 2; limit = view, exclude = one degenerate rect at the view origin; no GC                     | 3 (session cycle), limit = whole WebView, open (1,1, `ENABLE_POST(0)`), limit = sheet, eraser raw (0,0), resume → 2 (pen tool) or stays 1 (eraser, rectangle lasso); exclude n=0; WebView mode GU; whole-limit reconcile frame (mode 524290) |
| write, per stroke        | no transaction; plain mode-5 invalidates; per-stroke reconcile gated off on this device                                                        | no transaction; after pen-up refresh (500 ms) or 120 ms, whole-WebView invalidate in mode 524290 (`HAND_WRITING_REPAINT_MODE`)                                                                                                               |
| eraser end, pen tool     | render stays on (2); SF eraser reader draws style 8/5; CU 1/0; after lift `ENABLE_POST(1)` (if erased), render-only 3, invalidate, RS(300) → 2 | render off at down after a main-thread hop (3); SF eraser raw drawing off; JS draws the eraser circle and erased canvas during the stroke; render back (2) after the reconcile frame presents                                                |
| eraser tool, per gesture | render on (2); SF draws the eraser with main style 8/5; after lift as above                                                                    | steady 3 for the whole time the tool is selected (1 if the session opened with it); no transaction per gesture; JS draws                                                                                                                     |
| lasso freehand, drawing  | SF dash trace, session running (2)                                                                                                             | SF dash trace, session running (2)                                                                                                                                                                                                           |
| lasso, lift with hit     | PR, second pause, FM on (2308, turbo 5, dither 160), reader-only resume (brush 0, eraser 1), steady 3                                          | configure after the selection changes: 3, input off/on, re-push, stays 3                                                                                                                                                                     |
| lasso, lift without hit  | PR, page repaint, RS(500) → 2                                                                                                                  | no configure; render stays on with dash style [inferred]                                                                                                                                                                                     |
| rectangle lasso          | steady 3, app overlay under FM on/off                                                                                                          | steady 3 (or 1), JS polygon, no transient                                                                                                                                                                                                    |
| drag selection (stylus)  | steady 3, fast mode already on, overlay frames, no transaction per step                                                                        | steady 3, `APPLY_TRANSIENT_UPDATE(2308)` at down, cleared 5 s after the last up, reconfigure after lift                                                                                                                                      |
| drag selection (finger)  | same as stylus                                                                                                                                 | no native call during the drag; reconfigure after lift                                                                                                                                                                                       |
| menu / popup open        | PR (+ INV input-only); floating selection menu does not pause                                                                                  | 3, then a second 3 from `cycleScribbleSession` (not deduplicated by SF once paused, 07 §4.1)                                                                                                                                                 |
| menu / popup close       | INV with `REGAL_PLUS` (mode 9), RS(500)                                                                                                        | GC (mode 98, `update_mode[1]` [inferred]) 300 ms after `onOpenChange(false)`, resume after 300 + 500 ms                                                                                                                                      |
| soft keyboard hide       | RS(500)                                                                                                                                        | resume after 500 ms + GC after 300 ms                                                                                                                                                                                                        |
| finger scroll            | FM on (2308), PR + RS(500), 5 s later clear; only when the viewport can scroll                                                                 | one configure per scroll event whose args change: 3 → 2 cycle, limit re-push, whole reconcile frame; framework scroll-A2 (`OnyxScrollRefreshManager`) applies to Tangleaf's package [inferred, 07 §6]                                        |
| pinch zoom               | FM on, PR + RS(500), width per frame, 5 s later clear                                                                                          | not covered in 08 **[unknown]**                                                                                                                                                                                                              |
| page turn                | PR, display, RS(≥500), GC every 10th tick                                                                                                      | full close (3,3,3,0,0,3), GC 300 ms later, new session (3,1,1,2)                                                                                                                                                                             |
| undo / redo              | PR, dirty-rect repaint, RS(500)                                                                                                                | full close + GC 300 ms later + new session                                                                                                                                                                                                   |
| tool switch              | handler-specific PR + RS(150 or 500), colour CU, often an INV                                                                                  | 3 (if on), input off/on, re-push, 2 if the tool renders                                                                                                                                                                                      |
| leave page               | close (3, 0), `resetEpdPost` ×1–2, no GC, regions left                                                                                         | close (3,3,3,0,0,3), exclude n=0, GC 300 ms later                                                                                                                                                                                            |
| background               | focus loss → INV (3), save pause; session kept                                                                                                 | close + GC, plus a 3 from focus loss and one from `onPause`                                                                                                                                                                                  |
| foreground               | focus gain → INV → RS(500) → 2                                                                                                                 | new session (3,1,1,2), whole reconcile                                                                                                                                                                                                       |
| app start                | nothing pen-related until a note opens                                                                                                         | pen state 3 and exclude n=0 at plugin load                                                                                                                                                                                                   |

## 5. Divergences

Neutral list of every difference found, grouped. Each item names the documents that hold the
evidence.

**Session lifetime**

1. Stock keeps one session for the life of the open document and only pauses it; Tangleaf closes and
   reopens the session on undo/redo, page switch, leave, background and `pagehide` (01 §4.6, 08 §5.12–§5.14).
2. Stock never closes the session on backgrounding; Tangleaf does (§3.12 above, 08 §5.14).
3. Tangleaf sends pen state 3 at plugin load and through `cycleScribbleSession` on every full pause,
   named pause and close, bypassing the SDK guard; stock sends pen states only through `TouchHelper`
   (08 §3, 01 §4).
4. Tangleaf can leave pen state 1 (START) as the last state when a session opens with the eraser or
   rectangle lasso; stock always parks with 3 after open unless the pane is unfocused (08 §5.2, 01 §4.2).

**Render during erase and other pen input**

5. Stock erases with render on for the pen, eraser and shape tools and lets SF draw the eraser trace
   (eraser raw drawing on, style 8/5); Tangleaf erases with render off and SF eraser raw drawing off,
   and draws the eraser in the WebView (03 §6, 08 §5.4–§5.5).
6. Stock issues no pause at pen-down in its pen, eraser and shape tools; Tangleaf pauses at eraser-end
   down with the pen tool and at a pen-down on the selection menu, and keeps render off for the whole
   time the eraser tool, the rectangle lasso or a shown selection is active. Stock also keeps render
   off for the rectangle lasso and a shown selection, but not for the eraser tool (03 §4.1, 04 §5.3,
   §5.5, 08 §1 item 3).
7. Stock returns to state 2 after an erase through a 300 ms timed resume with a full re-push;
   Tangleaf returns after the reconcile frame presents, without a re-push (03 §3.4, 08 §5.4).
8. Stock brackets eraser gestures and some tools with `ENABLE_COLOR_CU`; Tangleaf never sends it
   (03, 05 §5.6, 08 §3).
9. Stock sends `ENABLE_POST(1)` after an erase that changed a visible shape while still DRAWING;
   Tangleaf sends `ENABLE_POST(1)` only as part of render-off and session cycles (01 §1 item 7, 08 §3).

**Per-stroke refresh**

10. Stock issues no per-stroke update mode on this device (reconcile gated off by
    `vendor.onyx.tablet`); Tangleaf marks a whole-WebView reconcile frame with `HAND_WRITING_REPAINT_MODE`
    after every stroke and erase, which is exactly the entry SF lets through while in schema 7
    (02 §5.6, 08 §5.3) [code for both; the SF commit of Tangleaf's frame is inferred].
11. Stock's view default update mode is 5 (AUTO); Tangleaf sets the WebView's default to GU (2)
    through the display profile and the session layer (05 §5.1, 08 §6.3).

**Full refreshes**

12. Stock's full GCs come from the counted GC (every 10th page action / menu click), the Refresh
    button and a text-input fast-mode exit; Tangleaf's come 300 ms after every session close, keyboard
    hide and overlay close (05 §3, 08 §6.2).
13. Stock's popup dismiss repaint is a `REGAL_PLUS` frame (mode 9, rejected nibble); Tangleaf's overlay
    close is a mode-98 GC (05 §5.1, 08 §5.10).

**Fast mode and transient A2**

14. Stock applies transient A2 (2308) with turbo 5 and dither 160 for scroll, pinch, drags,
    transforms and rectangle lasso, and releases it 5 s later with flag 0; Tangleaf applies 2308 only for a
    pen-down inside the selection grab box (including an eraser-end touch there), without turbo or
    dither, and clears it 5 s after the last such gesture (05 §5.4, 08 §5.7).
15. Stock's package is exempt from the framework's scroll-A2 manager and its canvas is not a WebView;
    Tangleaf's package is not exempt and its editor is a WebView, so both framework entries into
    `ScrollHelper` apply to it (07 §6) [code for the gates; that they fire for Tangleaf is inferred].
16. Stock writes the dither threshold (160/128) into the EinkWise store on every fast-mode entry/exit;
    Tangleaf writes an EinkWise refresh profile once (05 §7.3, 08 §5.1).

**Regions**

17. Stock pushes limit and exclude on every resume and overlay change, with the exclude list never
    empty (at least one degenerate rect at the view origin) and typed overlay rects; Tangleaf pushes an
    empty exclude (clearing SF's table) on every resume and uses no firmware excludes for its overlays
    (it swallows pen-downs on DOM overlays instead) (01 §4.4, 06 §3.10, 08 §3; 09 C4, C23).
18. Stock never clears the SF limit/exclude store on close; Tangleaf resets the exclude table at start,
    every resume and every close (01 §4.6, 08 §8).
19. Stock's limit is the `EditorView` bounds; Tangleaf's is the visible sheet clipped to the viewport,
    re-pushed on every scroll that changes the args, and can become empty when the sheet scrolls out
    (06 §3.10, 08 §5.11).

**Input routing**

20. Stock enables `setPostInputEvent(true)` and uses hover (`PenActiveEvent`) to suppress finger
    gestures; Tangleaf disables it (06 §3.3, 08 §3).
21. Stock scrolls and zooms by its own gesture engine with an explicit fast mode and pause; Tangleaf
    relies on native WebView scrolling and reconfigures per scroll event (06 §3.6, 08 §5.11).
22. Stock's resumes wait 500 ms (colour) inside a request on a single pen thread, re-pushing all
    parameters; Tangleaf's resumes also wait 500 ms but through its own pause registry (01 §4.4, 08 §5.10).

**SDK and backend**

23. Stock runs its bundled SDK copy (with `ResumeRawDrawingRequest`, `PenEventHandler`); Tangleaf
    links the public `onyxsdk-pen 1.5.4.3` / `device 1.3.5.2`. The lifecycle methods compared match
    (01 §2); both use `SDMDevice` on `lito` (08 §0).

## 6. Device-specific branches that decide the stock path on the Note Air 4C

| branch                                   | value here                                                                                                                       | basis                                                                                                                                                                  |
| ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Device` class                           | `SDMDevice`                                                                                                                      | `lito` in `vendor/build.prop` [code] + `Device.b()` map [code]                                                                                                         |
| `isColorDevice()` (`getColorType() > 0`) | true                                                                                                                             | [inferred]: Kaleido panel; the SF answer was not read. It sets the 500 ms resume delay, popup 500 ms, dither 160, `enableFullRefresh` default, colour-original strokes |
| `vendor.onyx.tablet`                     | true                                                                                                                             | `vendor/build.prop` [code]; runtime value [inferred]. Turns off the per-stroke reconcile; meeting hand-back 5 s                                                        |
| `vendor.onyx.htcon`                      | true                                                                                                                             | `vendor/build.prop` [code]. SF uses `HWEpdcManager`; gesture intervals 0; pen-up-refresh setting hidden                                                                |
| `note_noteair4c.json`                    | eraser buffers 20/30 ms, `showPenUpRefresh:false`, `epdTurbo:5`, `isTabDevice:true` (unused), no `transformUpdateModel` (→ 2308) | [code] file; `Build.MODEL` mapping [inferred]                                                                                                                          |
| `config_note_noteair4c.json`             | absent → `REGAL_PLUS` one-shot, no tilt                                                                                          | [code]                                                                                                                                                                 |
| `eraserAreaDisplayDelayMillSecond`       | 0                                                                                                                                | default [code]                                                                                                                                                         |
| `supportEAC`                             | false (device store)                                                                                                             | handoff fact; effects 05 §7.4                                                                                                                                          |
| `isOnyxApp`                              | true                                                                                                                             | package name; exempts stock from the framework scroll-A2 path (07 §6) [code]                                                                                           |
| rich-text `TouchHelper` mask             | 2 if `hasStylus`                                                                                                                 | [inferred]                                                                                                                                                             |

## 7. What is still open

The full list with resolutions is in 09; the observation plan in 10. The questions that decide the
comparison:

1. Whether stock produces the scheme-4 → full-update sequence when pen input lands in a paused
   session (post-erase window, fill colour, selection drag, rectangle lasso, 500 ms after a menu).
   The documents disagree on the prediction (09 C12).
2. Where the observed full update in Tangleaf comes from: the framework scroll-A2 path (07) or
   `ExtBufferSchema::onDeactivate` (05), and why it logs `update_mode[1]` when the scroll-A2 exit
   queues a mode-5 update (09 C10, C11).
3. Whether plain app frames are withheld during schema 7 (02) and therefore whether Tangleaf's
   `0x80002` reconcile frames are the only ones committed there.
4. What `ENABLE_COLOR_CU`, SF style 8 and the degenerate exclude rect do on the panel.
5. Which firmware the tablet runs.
