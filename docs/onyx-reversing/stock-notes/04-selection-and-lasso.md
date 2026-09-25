# Stock Notes: selection, lasso and transforms

Research document, static analysis only (2026-09-25). Subject: how the stock Onyx Notes app
(`com.onyx.android.note`, module `knote2`) runs the lasso/selection tool on a BOOX Note Air 4C, and
what SurfaceFlinger (SF) sees at every step. No fixes and no design proposals for Tangleaf.

Markers: **[code]** read in source; **[inferred]** reasoned from code that was read, not read
directly; **[unknown]** not determinable statically. Unless stated otherwise, paths are relative to
the stock app's jadx output `~/tmp_zfs/boox-notes-inspect/decoded/sources/`, and the bundled SDK
copy inside that APK is the one described (not the public 1.5.4.3). Abbreviations used below:

| short  | path                                                                             |
| ------ | -------------------------------------------------------------------------------- |
| `SH`   | `com/onyx/android/note/note/handler/scribble/SelectionHandler.java`              |
| `ScH`  | `com/onyx/android/note/note/handler/scribble/ScribbleHandler.java`               |
| `PEH`  | `com/onyx/android/note/note/eventhandler/PenEventHandler.java`                   |
| `EEH`  | `com/onyx/android/note/note/eventhandler/EpdEventHandler.java`                   |
| `RRDR` | `com/onyx/android/note/note/request/pen/ResumeRawDrawingRequest.java`            |
| `BNR`  | `com/onyx/android/sdk/notecore/editor/request/BaseNoteRequest.java` (bundled)    |
| `NM`   | `com/onyx/android/sdk/notecore/editor/NoteManager.java` (bundled)                |
| `TH`   | `com/onyx/android/sdk/pen/TouchHelper.java` (bundled pen SDK)                    |
| `SFTR` | `com/onyx/android/sdk/pen/touch/SFTouchRender.java` (bundled)                    |
| `ATR`  | `com/onyx/android/sdk/pen/touch/AppTouchRender.java` (bundled)                   |
| `EPM`  | `com/onyx/android/sdk/pen/EpdPenManager.java` (bundled)                          |
| `SDM`  | `com/onyx/android/sdk/device/SDMDevice.java` (bundled device SDK)                |
| `act/` | `com/onyx/android/note/note/action/selection/`                                   |
| `req/` | `com/onyx/android/note/note/request/selection/`                                  |
| `VUH`  | `~/tmp_zfs/onyx_framework/out/sources/android/onyx/ViewUpdateHelper.java`        |
| `sf`   | `~/tmp_zfs/onyx_framework/native/surfaceflinger` (symbols from `.gnu_debugdata`) |

SF pen states (`EPM` constants, `EpdPenManager.java:7-11` [code]): 0 STOP, 1 START, 2 DRAWING,
3 PAUSE, 4 ERASING. "Render on" below means pen state 2 was last sent; "render off / paused" means
state 3 (plus `ENABLE_POST(-1,1,pid)`).

---

## 1. Summary

1. **The lasso trace is drawn by SurfaceFlinger, not by the app.** While the lasso tool is waiting
   for a stroke (`SelectionMode.SELECTION`), `SelectionHandler.canPenRawRender()` returns true for
   the free/smart lasso (`SH:1903`) [code]; the pen resume path therefore turns SF render on
   (pen state 2) and, because the active provider is `SCRIBBLE_SELECTION_PROVIDER`,
   `ResumeRawDrawingRequest` sets **stroke style 5 = `STROKE_STYLE_DASH`** with stroke parameters
   `{5.0}` (`RRDR:52-59, 84-96`; `TH:25`) [code]. SF renders it with its `PainterDash` painter
   (`sf` symbols `PainterDash::*`, `MemoryPainter::setStrokeStyle`) [inferred from names]. The
   stroke colour is the current drawing colour (`NM.applyDrawingArgs`, `NM:196-202`) [code]. The
   app itself never paints the lasso path for the free/smart lasso: `NormalSelectionDrawingProxy`
   only collects points (`handler/drawing/NormalSelectionDrawingProxy.java`) [code].
2. **The rectangle lasso is the exception**: `canPenRawRender()` and `isUsePenInput()` are false
   for `SelectionType.RECTANGLE` (`SH:1898-1905, 1974-1976`) [code]; the app draws a dashed
   rectangle (black, width 4.5, dash 10/10) into an overlay renderer
   (`act/DrawRectangleSelectionAction.java:54-60`) every ≥30 ms, inside an `ApplyFastModeEvent(true)`
   window (`handler/drawing/RectangleSelectionDrawingProxy.java:61-75, 110-132`) [code].
3. **The lasso path used for hit-testing comes from ordinary MotionEvents**, not from the SF raw
   reader: `onTouchDown/Move/Up` feed `NormalSelectionDrawingProxy` (`SH:2681-2745`), while raw
   points from the SF reader are only kept to draw the dashed outline of a FREE lasso afterwards
   (`SH:2285-2295`, `req/StartTransformRequest.java:61-80`) [code].
4. **Every "note request" pauses the pen by default.** `BaseNoteRequest.execute()` posts
   `RawPenPauseEvent(pauseRender=true, pauseReader=true)` unless the request opted out
   (`BNR:17-18, 34-47`) [code]. `SelectShapesRequest`, `StartTransformRequest`,
   `QuitSelectionRequest`, `StartRotateRequest`, `PasteCopyShapesRequest`, `MoveShapesToLayerRequest`
   do not opt out; `TranslateRequest`, `ScaleRequest`, `RotateRequest`, `MirrorRequest`,
   `RemoveSelectedShapesRequest`, `AlignRequest`, `CutShapesRequest` do (`setPauseRawDraw(false)`)
   [code]. So the lasso's pen-up immediately pauses the SF session (pen state 3) through the
   selection hit-test request.
5. **While a selection is shown (`SelectionMode.TRANSFORM`) the SF session stays paused.**
   `canPenRawRender()` is false in TRANSFORM (`SH:1903`, `i1()` at `SH:1398`), so every later
   `PenEvent.resumeRawDrawing()` only re-enables the raw input reader (`PEH:105-107, 155-168`)
   [code]. Because `canBrushRawRender()` is also false in TRANSFORM and `ResumeRawDrawingRequest.h()`
   forwards that value, the reader-only resume sends **`SET_BRUSH_RAW_DRAWING_ENABLED(0)`** while it
   forces **eraser raw drawing on** (`RRDR:48-50, 61-65`; `PEH.q()` `setCanBrushRawRender(...)`)
   [code]. Dragging, scaling and rotating are therefore done from MotionEvents with the session
   paused and the reader running [inferred from the two facts above]. Per the proven driver-log
   fact (pen input while paused ⇒ SF scheme 4/A2 and a GC16 ~1.5 s after lift) a stylus drag of a
   selection is expected to trigger the same SF path — **not yet observed on hardware** (§9, E3).
6. **The app, not SF, draws the selection and the moving strokes.** `StartTransformAction`
   re-renders the page slices under the selected shapes (`act/RenderSelectionDirtyRectToDisplayAction`)
   and installs an overlay renderer `RENDER_KEY_TRANSFORM` that draws the selected shapes plus the
   selection rectangle/handles (`act/RenderSelectionShapesAction`, `RenderVarietyShapesAction`);
   every drag step re-installs that renderer, which calls `View.invalidate()`
   (`RenderManager.java:543-548`, `InkRenderManager.java:79-95`) [code]. No per-view update mode is
   set on these invalidates (default `DisplayArgs`) [code].
7. **"Fast mode" (a transient A2 update) covers the whole TRANSFORM phase.** `StartTransformAction`
   posts `ApplyFastModeEvent(true)` (`act/StartTransformAction.java:225-229`); `EpdEventHandler`
   then calls `setDitherThreshold(160)` (colour device), `setEpdTurbo(5)` and
   `EpdController.applyTransientUpdate(ANIMATION_QUALITY)` = `UI_A2_QUALITY_MODE` 2308
   (`EEH:81, 109-116, 176-179`; `SDM:392, 477, 870-872`) [code]. It is cleared 5 s after fast mode
   is released, with `clearTransientUpdate(false)` = clear flag 0, no GC (`EEH:118-150`) [code].
   Fast mode can only be released when the active handler no longer reports `renderInFastMode()`
   (= TRANSFORM, `SH:2861-2863`) (`EEH:185-190`) [code] — see §5.9 for the consequence.
8. **The selection menu popup does not pause the pen.** `SelectionPopupMenu` overrides
   `windowChangeEvent()` with an empty body (`menu/popup/SelectionPopupMenu.java:188`) [code], so
   unlike every other `BaseEditPopWindow`/`BaseNotePopWindow` it never posts `PopupWindowChangeEvent`
   and never increments `PenEventHandler`'s popup counter. Other popups opened from it (colour,
   more, align, move-to-layer, link) do pause (`PopupChangeAction`) [code].
9. **Deselecting returns to the pen tool by default.** `NoteMMKVHelper.isAutoSwitchToPenMode()`
   defaults to `true` (`NoteMMKVHelper.java:1076-1077`) [code]; tapping empty space in TRANSFORM
   (and finishing a delete) goes `M4 → C4 → D4 → H4` which quits the selection tool entirely,
   swaps the handler back to the pen provider and resumes the pen ~500 ms later
   (`SH:338-383, 533-562, 728-740`; `act/QuitSelectionAction.java`) [code].
10. **Stock never issues a full refresh of its own in the selection flow**, except through the
    selection-menu click counter: every selection-menu click calls
    `EpdDeviceManager.applyGCWithInterval()` (`menu/SelectionMenuHandler.java:346`), which every
    N-th click does `byPass(0)` + `applyGCOnce()` + `InvalidateScreenAction`
    (`note/common/utils/EpdDeviceManager.java:16-40`) [code]. The counter is a process-wide
    singleton that is also advanced by page actions and other menus, and it is **reset to 0 every
    time fast mode is released** (`EEH.k()` → `resetRefreshCount()`), so N counts clicks since the
    last release, not since the last GC [code; practical effect inferred] (§5.10).

---

## 2. Central classes and entry points

| Class (stock APK)                                                                | Role                                                                                                                            |
| -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `SelectionHandler` (`SH`, extends `ScribbleHandler`)                             | The handler for `Provider.SCRIBBLE_SELECTION_PROVIDER`; owns selection state, touch handling, all event subscribers             |
| `SelectionManager` (`note/manager/SelectionManager.java`)                        | Hit-test of the touch against the selection rect (translate / scale X/Y/XY / rotate / corner menus), dispatch of transforms     |
| `NormalSelectionDrawingProxy`, `RectangleSelectionDrawingProxy`                  | Collect lasso points (normal) or draw the rectangle overlay (rectangle)                                                         |
| `SelectShapesAction` / `CutShapesAction` (`act/`)                                | Hit-test after lasso pen-up (SMART/RECTANGLE vs FREE, which splits strokes)                                                     |
| `StartTransformAction` + `StartTransformRequest`                                 | Enter TRANSFORM: build selection rect, set variety shapes, render overlay, apply fast mode                                      |
| `TranslateAction`/`ScaleAction`/`RotateAction`/`MirrorAction`                    | One transform step: request on note thread (no pen pause) + overlay re-render                                                   |
| `QuitTransformAction` + `QuitTransformRequest`                                   | Leave TRANSFORM: write shapes back to pages, undo command, force-render dirty rect, remove overlay, `ApplyFastModeEvent(false)` |
| `StartSelectionAction` / `QuitSelectionAction`                                   | Enter / leave the lasso tool (provider switch, pen pause/resume)                                                                |
| `PenEventHandler` (`PEH`)                                                        | The only place that resumes the pen; predicate `A()` (render) and `z()` (reader)                                                |
| `EpdEventHandler` (`EEH`)                                                        | Fast mode: transient A2 update, dither threshold, turbo, 5 s release timer                                                      |
| `ResumeRawDrawingRequest` (`RRDR`)                                               | Applies provider-specific raw-drawing config (dash style for selection) then enables reader/render                              |
| `SelectionPopupMenu` (`menu/popup/SelectionPopupMenu.java`)                      | The floating selection menu; positioned above/below the selection rect; does not pause the pen                                  |
| `ScribbleFragment.onSelectionShapesEvent` (`note/ui/ScribbleFragment.java:1896`) | Shows/updates/dismisses the popup menu                                                                                          |

Entry points into the lasso tool [code]:

- Toolbar "selection" button: `ToolbarMenuModel.onSelection()` (`menu/viewmodel/scribble/ToolbarMenuModel.java:1056-1076`)
  → `StartSelectionAction(LASSO)`; a second click while already in the tool opens `SelectionTypePopup`
  (choose SMART/FREE/RECTANGLE).
- `SelectionTypeHandler` (`menu/SelectionTypeHandler.java:58`) activates the provider directly.
- Programmatic entries with `SelectionAction.MOVE`: long-press on a stroke with a finger in any
  scribble tool (`ScH:207-225, 619-627`; `TouchEventHandler.onLongPressEvent` only passes finger
  events), after inserting an image/attachment/link/timestamp, HWR surround gesture
  (`HWR_GESTURE`), paste from clipboard (`action/clipboard/PasteClipboardCopyShapeAction.java:64`).

---

## 3. State model and the predicates SF depends on

`SelectionMode` (`note/model/SelectionMode.java`): `SELECTION` (waiting for a lasso), `TRANSFORM`
(a selection exists), `TEXT_INPUT`. Stored in `menuModel.selectionMenuModel`, set via `u5()`
(`SH:1741-1743`) [code].

`SelectionType` (`sdk/data/note/SelectionType.java`): `SMART 0` (default,
`NoteMMKVHelper.getSelectionType()` default 0 at `NoteMMKVHelper.java:1005-1006`), `FREE 1`,
`RECTANGLE 2` [code]. SMART and RECTANGLE go through `SelectShapesAction`; FREE goes through
`CutShapesAction`, which cuts strokes at the lasso boundary (`SH:1651-1669`) [code].

`SelectionAction` (`note/model/SelectionAction.java`): `LASSO`, `MOVE`, `HWR_GESTURE` [code].

Handler predicates and what they gate [code, `SH` line numbers]:

| Predicate                     | Value                                                 | Consumer                                                                                                                                     |
| ----------------------------- | ----------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `canPenRawRender()` 1903      | `SELECTION && !popupShowing && !RECTANGLE && !(t&&u)` | `PEH.A()` (resume SF render), `PEH.onActivityFocusChangedEvent`, `PEH.onNoFocusSystemDialogChangeEvent`                                      |
| `canBrushRawRender()` 1898    | same as above                                         | `RRDR.h()` → `setBrushRawDrawingEnabled` (SF `SET_BRUSH_RAW_DRAWING_ENABLED` 1048834)                                                        |
| `isUsePenInput()` 1974        | `!RECTANGLE`                                          | `PEH.z()` (resume raw input reader)                                                                                                          |
| `renderInFastMode()` 2861     | `TRANSFORM`                                           | `EEH.k()`, `onHandlerChangedEvent`, `onResumeScribbleComponentEvent`, `onQuitFastModeImmediatelyEvent`                                       |
| `supportFingerDraw()` 2882    | `SELECTION && !popupShowing`                          | `ScribbleTouchDistributor.java:387` → `NoteManager.onTouchEvent` → `AppTouchRender` (finger lasso)                                           |
| `isEnableColorCU()` 1958      | `true`                                                | `ScH.onActivate()` → `EpdController.enableColorCU()` (`VUH` `ENABLE_COLOR_CU` 1048693); `ColorModeEventHandler` ignores popup-driven toggles |
| `isTouchEventConsumed()` 1963 | TRANSFORM and touch hits the selection                | consumed by touch routing (other subsystem)                                                                                                  |

`i1()` = SELECTION and not `PenEventHandler.isPopupWindowShowing()` (`SH:1398-1400`); `g1()` = popup
counter > 0 (`SH:1323-1329`); `t`/`u` = "touch started while a popup was open" / "touch active"
(`SH:1250-1252, 2681-2688`) [code]. A touch that starts while a pausing popup is open is swallowed
(`w5()`, `SH:1779-1781`) and its up event only resumes the pen (`o5()`, `SH:1566-1571`) [code].

`PenEventHandler` gates [code, `PEH:105-107, 271-273`]: render resume `A(z)` requires, among others,
no keyboard, no popup (`c == 0`), no toast/status bar, window focus, not paused by name
(`PausePenEvent` registry `g`), scribble scene, host view bound, current layer enabled, and
`canPenRawRender()`. Reader resume `z()` requires the same set with `isUsePenInput()` instead.

---

## 4. The SF-facing primitives used by the selection flow

All transactions go to `SurfaceFlinger` via `VUH.transactData` (see `03-raw-drawing-render-flag.md`
§1.1) [code in earlier doc; re-checked codes below in `VUH`].

| App call (bundled)                                                                 | Chain                                                                                                                                                                                                                                                                               | SF transaction (`VUH`)                                                                                                             |
| ---------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `NM.setRawDrawingEnabled(false)` (used by `PauseRawPenAction` when both flags set) | `TH.setRawDrawingEnabled` (`TH:276-284`) → render off + reader off + `resetPenDefaultRawDrawing()` (`TH:182-185`)                                                                                                                                                                   | see next three rows, plus `SET_BRUSH_RAW_DRAWING_ENABLED(1)` 1048834, `SET_ERASER_RAW_DRAWING_ENABLED(0,5)` 1048833                |
| render off                                                                         | `SFTR.setDrawingRenderEnabled(false)` → `h()`: `EpdController.leaveScribbleMode` (= `enablePost(view,1)`) + `EPM.pauseDrawing()` (`SFTR:264-273, 368-373`; `EPM:28-30`); `ATR.setDrawingRenderEnabled(false)` → `enablePost(1)`                                                     | `ENABLE_POST(-1,1,pid)` 16711692 ×2, `SET_SCREEN_HANDWRITING_PEN_STATE(3,pid)` 16711693                                            |
| render on                                                                          | `SFTR` → `EPM.resumeDrawing()` (`EPM:37-39`)                                                                                                                                                                                                                                        | `SET_SCREEN_HANDWRITING_PEN_STATE(2,pid)` 16711693                                                                                 |
| reader on/off                                                                      | `RawInputManager.resume/pauseRawInputReader` (app-side reader)                                                                                                                                                                                                                      | none identified [unknown whether the reader touches SF]                                                                            |
| `setStrokeStyle(5)`                                                                | `TH.setStrokeStyle` fans out to both renders: `SFTR.setStrokeStyle` → `EPM.setStrokeStyle` → `EpdController.setStrokeStyle`, and `ATR.setStrokeStyle` → `EpdController.setStrokeStyle` directly (`ATR:334-336`); then `SDM.setStrokeStyle` (`SDM:2286-2291`) → `VUH.setStrokeStyle` | `SET_STROKE_STYLE` 16711688, **sent twice** per call (once per render) [code] (`sf`: `SurfaceFlingerHelper::handleSetStrokeStyle`) |
| `setBrushRawDrawing(b)` / `setErasePathDrawing(b, s)`                              | `NM` → `TH.setBrushRawDrawingEnabled` / `setEraserRawDrawingEnabled` → `SFTR` → `Device.setBrushRawDrawingEnabled` / `setEraserRawDrawingEnabled` (`SFTR:363-379`; `ATR` versions are empty, `ATR:264-278`)                                                                         | `SET_BRUSH_RAW_DRAWING_ENABLED` 1048834 / `SET_ERASER_RAW_DRAWING_ENABLED` 1048833                                                 |
| `Device.setStrokeParameters(5,{5.0})`, `(8,{w,0.5,0.1})`                           | `SDM:2281-2283` → `VUH.setStrokeParameters`                                                                                                                                                                                                                                         | `SET_STROKE_PARAMETERS` 1049089 (`sf`: `MemoryPainter::setStrokeParameters`)                                                       |
| `EpdController.applyTransientUpdate(ANIMATION_QUALITY)`                            | `SDM:870-872`, `h(mode)` case 10 → `A = UI_A2_QUALITY_MODE` (`SDM:392, 477, 1593-1633`)                                                                                                                                                                                             | `APPLY_TRANSIENT_UPDATE(2308)` 16711782; SF logs `### applyTransientUpdateMode, updateMode %d`                                     |
| `EpdController.clearTransientUpdate(false)`                                        | `SDM:902-904` → `VUH.clearTransientUpdate(0)`                                                                                                                                                                                                                                       | `CLEAR_TRANSIENT_UPDATE(0)` 16711783; SF logs `### clearTransientUpdateMode, clearFlag %d`                                         |
| `EpdController.setEpdTurbo(5)`                                                     | `SDM:2140-2142`                                                                                                                                                                                                                                                                     | `SET_EPD_TURBO` 1048661                                                                                                            |
| `Device.setDitherThreshold(160/128)`                                               | `SDM:2119-2121` → `EInkHelper.setDitherThreshold`                                                                                                                                                                                                                                   | `SET_DITHER_THRESHOLD` 1048704 (values < 128 dropped, see `05-panel-refresh-levers.md`)                                            |
| `EpdController.enableColorCU(bool)`                                                | `SDM:998-1000` → `VUH.enableColorCU`                                                                                                                                                                                                                                                | `ENABLE_COLOR_CU` 1048693 (`sf`: `EpdcManager::setEnableColorCU`); meaning [unknown]                                               |
| `EpdController.resetViewUpdateMode(view)`                                          | `SDM:2034-2036` → `View.resetUpdateMode()`                                                                                                                                                                                                                                          | per-view default update mode reset (framework View field) [inferred]                                                               |
| `View.invalidate()` on the note view                                               | `InkRenderManager.invalidate(DisplayArgs())` (`InkRenderManager.java:79-95`)                                                                                                                                                                                                        | normal frame; waveform chosen by the view/app default and any active transient update [inferred]                                   |

The two strings `### applyTransientUpdateMode, updateMode %d` and
`### clearTransientUpdateMode, clearFlag %d` are present in `sf` (checked with `strings`) [code]; if
they are logged at a visible level they are a direct log marker for fast mode (§9).

---

## 5. The mechanism, step by step

### 5.1 Entering the lasso tool

1. `ToolbarMenuModel.onSelection()` → `ensureInScribbleMode()`, `closeAllSubMenu()`,
   `new StartSelectionAction(LASSO)` (`ToolbarMenuModel.java:1056-1066`) [code].
2. `StartSelectionAction.create()` (`act/StartSelectionAction.java`) [code]:
   - `p()` → `HandlerManager.activeProvider(SCRIBBLE_SELECTION_PROVIDER)`
     (`sdk/notecore/editor/handler/HandlerManager.java:80-104`): previous handler `onDeactivate()`,
     `SelectionHandler.onActivate()`, then `HandlerChangedEvent`.
   - `SelectionHandler.onActivate()` (`SH:2009-2019`): clears selection (`B0`), then
     `ScribbleHandler.onActivate()` (`ScH:908-918`): `enableColorCU()` (because
     `isEnableColorCU()` is true) → SF `ENABLE_COLOR_CU(1)`, `ActiveRenderModeAction(SCRIBBLE)`
     (app renderer only). Resets the transform bean, subscribes the throttled corner hit-test.
   - `HandlerChangedEvent` → `EEH.onHandlerChangedEvent` (`EEH:252-258`): selection mode is
     SELECTION, `renderInFastMode()` false → `k()`: if fast mode was on, reset the GC counter and
     start the 5 s release timer [code].
   - `A()` → `NoteManager.postNoteInfo()`.
   - `InvalidateViewWithPenControlAction(pauseResumeArgs, DisplayArgs())`
     (`act/StartSelectionAction.java:77`; `action/render/InvalidateViewWithPenControlAction.java`):
     pause render + reader (`setRawDrawingEnabled(false)` → pen state 3), `View.invalidate()`,
     then `ResumeRawPenAction` → `PenEvent(true, true, resumeDelayTime)` [code]. The default
     `resumeDelayTime` of `RawPenArgs` is `DELAY_ENABLE_RAW_DRAWING_MILLS` = 500 ms on a colour
     device (`RawPenArgs.java:69, 232`) [code].
   - `C()` / `B()`: menu events, `StartSelectionEvent`, `SelectionActionChangeEvent(LASSO)`.
3. `PEH.onPenEvent` → `s()` → `q()` on the main thread (`PEH:155-213, 418-421`) [code]: render
   allowed (SELECTION mode, no popup) and reader allowed → `ResumeRawDrawingRequest` with
   `activeProvider = SCRIBBLE_SELECTION_PROVIDER`, `canBrushRawRender = true`, limit rect =
   scribble rect, exclude rects = current touch-exclude list.
4. `RRDR.execute()` (`RRDR:121-135`) on the pen scheduler [code]: sleep 500 ms, then `o()`:
   `setDrawLimitRect`, `setDrawExcludeRect` (SF regions, see `02-handwriting-regions.md`),
   `setPenUpRefreshTimeMs(OnyxSystemConfig.getPenUpRefreshTimeMs())`, `setSingleRegionMode()`,
   `applyDrawingArgs` (stroke colour = current colour), `n()`:
   - provider is SELECTION → `j()` = `setStrokeStyle(5)` (DASH);
   - `i()` = `setStrokeParameters(5, {5.0})` and `setStrokeParameters(8, {eraserWidth, 0.5, 0.1})`;
   - `k()` = `setErasePathDrawing(true, isMoveStrokeErase(type) ? 8 : 5)` — **eraser raw drawing
     is forced on for the selection provider** (`RRDR:61-64`);
   - `h()` = `setBrushRawDrawing(resumePenBean.canBrushRawRender)` (`RRDR:48-50`), where
     `PEH.q()` filled that field from `HandlerManager.canBrushRawRender()` at resume time — true
     here (SELECTION, non-rectangle, no popup), **false in TRANSFORM** (§5.5) [code].

   Then reader on, render on (pen state 2). `RRDR.execute()` only does any of this when
   `f() || g()` (render or reader actually needs enabling); the sleep is
   `resumePenBean.delayResumePenTimeMs`, i.e. the `PenEvent`'s delay (`RRDR:121-138`) [code].

SF-visible net effect of entering the tool: `ENABLE_COLOR_CU(1)`; one pause (state 3 + enablePost 1,
brush 1, eraser 0/5); ~500 ms later style 5, parameters, eraser-raw 1, brush 1, state 2 [code for
each call; ordering on the wire inferred from the Rx chains].

### 5.2 Drawing a lasso (free / smart, stylus)

Who does what while the pen moves [code unless marked]:

- **SF** draws the trace in the handwriting overlay with style 5 (dash) and the current colour,
  because the session is in state 2 with brush raw drawing on (§5.1). The stroke width is whatever
  was last set with `setStrokeWidth`; the selection resume path does not set a width (`RRDR.n()`
  sets width only for `SCRIBBLE_EPD_PROVIDER` via `applyPenArgs`) [inferred: width = last pen width].
- **Stylus MotionEvents** still reach the note view (the note `TouchHelper` is created with
  `FEATURE_ALL_TOUCH_RENDER` = 3 and `setPostInputEvent(true)`, `NM:232-235` [code]; that this flag is
  what keeps stylus MotionEvents flowing to the view while SF renders is [inferred]):
  `ScribbleTouchDistributor.onTouchEvent`
  → `ScribbleTouchDetector` posts `TouchDownEvent/TouchMoveEvent/TouchUpEvent`
  (`note/touch/ScribbleTouchDetector.java:13-31`) → `TouchEventHandler.onTouchDownEvent/...`
  (`note/eventhandler/TouchEventHandler.java:241-307`) → `SelectionHandler.onTouchDown/Move/Up`.
  - `onTouchDown` (`SH:2681-2700`): new `m` list, proxy `onTouchDown`, in SELECTION mode posts
    `SelectionStartEvent` (only `AudioMenuModel` listens).
  - `onTouchMove` (`SH:2703-2718`): `r = MOVING`; proxy collects the point. Nothing is drawn by the
    app for SMART/FREE.
- **SF raw reader callbacks** (`RawInputCallback` → `NoteManager` posts `RawDrawingPointsReceivedEvent`)
  → `SelectionHandler.onRawDrawingPointsReceived` (`SH:2285-2295`): stores the points (with the
  render matrix) in `m` = `selectionTrackPoints`, used later only for the FREE outline.
- The raw pen-up callback → `EndRawDrawingEvent` → `SH.onEndRawDrawing` (`SH:2156-2162`): if still in
  SELECTION and the MotionEvent path did **not** see a move (`r != MOVING`), schedule
  `InvalidateScreenAction` with 300 ms delay (§5.6). After a real lasso nothing happens here.

Finger lasso: `supportFingerDraw()` is true in SELECTION, so `ScribbleTouchDistributor`
(`:387-389`, additionally gated by `activeNoteScene.canFingerTouch()`) also feeds finger events into
`NoteManager.onTouchEvent` → `TouchHelper` → `AppTouchRender.onTouchEvent` (only while its reader flag
is set, `ATR:240-243`) → `AppTouchInputReader.processMotionEvent`. Finger events pass the reader's
filter because `enableFingerTouch` defaults to true (`AppTouchInputReader.java:36, 83-88`) and the app
never calls `TouchHelper.enableFingerTouch` [code: grep]. `ATR` then draws with
`EpdController.moveTo(view, x, y, width[, pressure])` on down (`ATR:122-127`) and
`EpdController.quadTo(view, x, y, UpdateMode.DU[, pressure])` on moves (`ATR:174-179`), only while
its render flag `c` is set (`ATR:97-99, 131-160`) — the framework's direct-draw path, style also 5
because `TouchHelper.setStrokeStyle` fans out to both renders — while the handler path collects the
lasso from the same MotionEvents [code for calls; visual result inferred]. `AppTouchRender` does not
draw stylus events (`onInterceptTouchEvent` = `TouchUtils.isPenTouchType`, `ATR:235-237`) [code].

### 5.3 Rectangle lasso

- `canPenRawRender()` false, `isUsePenInput()` false → no resume of either render or reader while the
  rectangle type is active; the session stays in whatever state the last pause left it, normally
  paused (state 3) [inferred: every tool/menu action goes through a pausing request or popup].
- `RectangleSelectionDrawingProxy.onTouchDown` → `ApplyFastModeEvent(true)` (transient A2 2308, §6.3);
  `onTouchMove` → at most every `selectionDrawTrackTimeMs` (30 ms default, no override in
  `note_noteair4c.json`) a `DrawRectangleSelectionAction` which installs a
  `RENDER_KEY_RECTANGLE_SELECTION` renderer (black dashed rect) → `View.invalidate()`;
  `onTouchUp` → abort pending draws, `ClearSelectionPathRequest` (= `EpdController.resetViewUpdateMode`),
  `ApplyFastModeEvent(false)` (`RectangleSelectionDrawingProxy.java:61-75, 110-132`,
  `act/DrawRectangleSelectionAction.java:54-86`) [code].
- The pen moves over the panel with the SF session paused during a rectangle drag [inferred] — the
  same condition as the proven erase flash (§9, E6).

### 5.4 Lasso pen-up → hit test

`SH.onTouchUp` (`SH:2721-2745`) in SELECTION mode [code]:

1. Clone the proxy's MotionEvent points, apply the render matrix → `r5(l)` (`SH:1651-1669`):
   - current layer invisible → `SelectShapeInvisibleLayerEvent`;
   - SMART/RECTANGLE → `SelectShapesAction(points).setSelectionType(type)`;
   - FREE → `CutShapesAction(points)` (cut strokes along the lasso, `CutShapesRequest` does not pause
     the pen, then a `SelectShapesRequest` that does).
2. `W0().clearPoints(); reset(true)`; `b1()` (TRANSFORM-only logic, no-op here); `y0()`; `z0()` =
   `ClearSelectionPathRequest` → `EpdController.resetViewUpdateMode(penHostView)`
   (`req/ClearSelectionPathRequest.java:19`); `SelectionEndEvent`.

`SelectShapesAction.create()` (`act/SelectShapesAction.java`): per visible page box a
`SelectShapesRequest` (selection pen width 10) runs on the note thread. `SelectShapesRequest extends
BaseNoteRequest` without opting out, so `execute()` first posts `RawPenPauseEvent(true,true)`
(`BNR:34-47`) → `PEH.onRawPenPauseEvent` (`PEH:449-452`) → `PauseRawPenAction.n()`
(`action/pen/PauseRawPenAction.java:47-57`) → `NM.setRawDrawingEnabled(false)` → **pen state 3,
enablePost 1, reader off, brush 1, eraser-raw 0/5** [code].

Timing caveat: `BaseNoteRequest.b()` only _posts_ the `RawPenPauseEvent` (`noteManager.post`,
`NM:616-618`); `PEH.onRawPenPauseEvent` is `ThreadMode.MAIN` and `PauseRawPenAction` then runs on the
pen scheduler (`createPenObservable()`). The request body (`execute(noteManager)`) runs immediately on
the note thread after the post. So the SF pause is **not** guaranteed to precede the request's work;
it is queued behind whatever the main looper is doing [code for thread modes; relative order on the
wire inferred]. The same applies to every "pauses the pen" statement below.

Multi-box detail: `SelectShapesAction.N()` force-renders a page box whenever **that box's** request
selected nothing (`act/SelectShapesAction.java:54-73`), so with several visible page boxes the
unselected boxes are re-rendered even when another box produced a selection [code].

- Nothing hit: `N()` → `forceRenderNoteToDisplay(page)` — a full re-render of the page slice to
  wipe the SF trace — and `Q()` → `resumePen()` → `PenEvent.resumeRawDrawing()` (delay 500) →
  render and reader back on with dash style (`act/SelectShapesAction.java:54-96`) [code].
- Something hit: `SH.N3()` / `o3()` → `D5()` → `z5()/y5()` → `StartTransformAction` (§5.5).

How the SF-drawn dashed trace disappears when something was selected: the app re-renders the dirty
rect of the selected shapes and the full view is invalidated when the overlay renderer is added
(§5.5), but no code path re-renders the **whole lasso path area** [code: dirty rect =
`RenderSelectionDirtyRectToDisplayAction.x()/z()` = union of selected shapes' bounds]. Whether SF
removes its own overlay pixels when the session goes to state 3 or when the next app frame for that
pid is composed is [unknown] (§9, E2).

### 5.5 StartTransform — how the selection is shown

`SH.y5()` (`SH:1831-1867`) builds a `StartTransformBean` (track points, shapes, selection type,
`provider = SCRIBBLE_SELECTION_PROVIDER`, `interactiveMode = SCRIBBLE_SELECTION`, render matrix) and
runs `StartTransformAction` (`act/StartTransformAction.java:381-440`) [code]:

1. `beforeRenderObservable`, `updateRenderMeasure`.
2. `G()`: if the note manager is already in the same interactive mode with variety shapes, quit them
   first (`QuitTransformRequest` + force-render their dirty rect).
3. Copy the visible box list; continue only if the active provider is still the bean's provider.
4. `FindTransformBoxInfoAction` (which page box the selection belongs to).
5. `o0()` → `StartTransformRequest.execute()` synchronously (`act/StartTransformAction.java:208-217`;
   `req/StartTransformRequest.java:94-133`). As a `BaseNoteRequest` it **pauses the pen again**
   (`RawPenPauseEvent(true,true)`) [code]. Inside: `activeRenderMode(SCRIBBLE_SELECTION)`, mark shapes
   selected, for FREE type (and not select-all) build a dashed free shape from the raw track points
   (stroke 5 × scale, dash `{6s, 6s}`, line-paint colour; `req/StartTransformRequest.java:61-80`),
   `setVarietyShapes(...)`, build `SelectionRect` (expanded for SCRIBBLE_SELECTION), set rotation
   handle, `postSelectionBundle()`.
6. `drawSelectionShapes` (app renderer), store the selection render context.
7. `q0()`: add universal-link targets.
8. `l0()` = `RenderSelectionDirtyRectToDisplayAction`: re-render the page slices under the union of
   the selected shapes' original bounds (so the page underneath no longer shows them) and
   `invalidate(DisplayArgs())` [code: `ForceRenderSliceListActionsKt.java:463-483, 541-560`].
9. `m0()` = `RenderSelectionShapesAction`: install `RENDER_KEY_TRANSFORM` renderer drawing the
   selected shapes + selection rect in `SCRIBBLE_SELECTION` mode; `addRenderer()` invalidates the
   view (`RenderManager.java:543-548`).
10. `p0()` → `ApplyFastModeEvent(true)` if shapes non-empty (`:225-229`) → §6.3 (transient A2 2308,
    dither 160, turbo 5).

Back in `SelectionHandler` (`SH:291-295, 318-322, 1308-1317`) [code]:

- `f4()`: `j = shapes`, `SelectionMode = TRANSFORM`; if any shape is in a visible box →
  `J5(true)` → `I5()` → `SelectionShapesEvent(visible = true when no image edit rect)` →
  `ScribbleFragment.onSelectionShapesEvent` (`ScribbleFragment.java:1896-1911`) creates/updates and
  `showAtLocation`s `SelectionPopupMenu` above the selection rect (below it if there is no room;
  `note/utils/SelectionPopupMenusKt.java`). No pen pause (overridden `windowChangeEvent`).
- `B5` → `resumePen()` (`SH:2866-2874`) → `PenEvent.resumeRawDrawing()` → `PEH.q()`: `A()` false
  (TRANSFORM) → render **stays paused**; `z()` true → `ResumeRawDrawingRequest` enables only the
  reader (it re-applies regions, style 5, parameters, eraser-raw and brush first because `g()` is
  true) (`RRDR:61-135`) [code]. Because `PEH.q()` copies `HandlerManager.canBrushRawRender()` into
  the bean and that is **false in TRANSFORM**, `RRDR.h()` sends `setBrushRawDrawing(false)` →
  `SET_BRUSH_RAW_DRAWING_ENABLED(0)`, while `RRDR.k()` still forces eraser raw drawing **on**
  (style 5, or 8 for move-stroke erasers) because the provider is the selection provider [code].

SF-visible net effect of lasso-up with a hit (default SMART, stylus) [code for calls, order
inferred]: state 3 + enablePost 1 ×2 + brush 1 + eraser-raw 0/5 (SelectShapesRequest) → second
pause from StartTransformRequest: `setRawDrawingRenderEnabled` and `setRawInputReaderEnable` are both
guarded by `TH.b` / `TH.c` (`TH:286-307`), so the second pause sends **only**
`resetPenDefaultRawDrawing()` = brush 1 + eraser-raw 0/5 (`TH:182-185`) → app frame(s) for the dirty
rect and overlay → `APPLY_TRANSIENT_UPDATE(2308)`, `SET_DITHER_THRESHOLD(160)`, `SET_EPD_TURBO(5)` →
popup window appears → ~500 ms later regions + style 5 (×2) + params + **eraser-raw 1** + **brush 0**
(reader on, render still paused, pen state stays 3).

### 5.6 Pen tap without movement in SELECTION mode

`onEndRawDrawing` with `r != MOVING` → `InvalidateScreenAction.setDelayRefreshTime(300)`
(`SH:2156-2162`) → after 300 ms `InvalidateViewWithPenControlEvent(pause both, resume both, delay 500)`
(`action/render/InvalidateScreenAction.java:29-37, 83-87`) → `SH.onInvalidateViewWithPenControl`
runs the base behaviour only if no shapes are selected (`SH:2175-2180`) →
`InvalidateViewWithPenControlAction`: pause (state 3), `View.invalidate()`, resume after 500 ms
(state 2) [code]. This wipes the dot SF drew for the tap [inferred purpose]. Separately the tap goes
through `onSingleTapUp` (`SH:2612-2638`): hit-test at the point; if a shape is hit it is selected
(`V3 → x4 → E5 → StartTransformAction`); if nothing is hit in SELECTION mode →
`PasteClipboardCopyShapeAction` (paste menu if the clipboard has shapes) [code].

### 5.7 Dragging, scaling, rotating (TRANSFORM)

Touch path (stylus or finger; `TouchEventHandler` delivers both unless `filterFingerTouch`) [code]:

1. `onTouchDown` (`SH:2681-2700`) → not SELECTION → `g4(point)` (`SH:1341-1364`): `r = DOWN`,
   `SelectionManager.onDown()` hit-tests the point against the selection rect
   (`SelectionManager.java:330-337`: `noteTouchPointHitTest` → Translate / ScaleX / ScaleY / ScaleXY /
   Rotation / Undefined). If a transform was hit, an `ObservableHolder` buffers move points for
   `SELECTION_TRANSFORM_TIME` = `DeviceConfig.selectionTransformTimeMs` (10 ms default, not
   overridden for NA4C) on a single-thread scheduler and forwards only the **last** point of each
   batch (`a1()` `SH:1149-1155`).
2. `F5()` → `SelectionManager.transform()` (`SelectionManager.java:414-445`):
   - Translate → `TranslateAction` (`act/TranslateAction.java`): `TranslateRequest` on the note
     thread (`setPauseRawDraw(false)`, `sdk/notecore/editor/request/transform/TranslateRequest.java:28`)
     then `renderSelectionShapesAction` → `RenderVarietyShapesAction` re-installs
     `RENDER_KEY_TRANSFORM` → `View.invalidate()`; then `UpdateSelectionPopupEvent` →
     `ScribbleFragment.onUpdateSelectionPopupEvent` moves the popup (`:2005-2011`).
   - ScaleXY/X/Y → `ScaleAction` (`ScaleRequest`, no pen pause) + overlay re-render.
   - Rotation → `RotateAction` (`RotateRequest`, no pen pause) + overlay re-render.
   - Image edit rect corners → `CroppingImageAction`.
3. No pen pause/resume, no explicit update mode per step; each step is one app frame drawn under the
   transient A2 quality mode applied at StartTransform [code for absence of calls; waveform inferred].
   The SF session is paused (state 3) and the raw reader is on during the drag [inferred, §5.5].
4. `onTouchUp` (`SH:2721-2745`) → `b1()` (`SH:1172-1190`): in TRANSFORM, if the down hit nothing and
   it was a quick tap → only `HWRTransformEvent`; otherwise `J5(true)` → `SelectionShapesEvent` →
   popup re-shown/updated. `z0()` again resets the view update mode.

Menu-driven transforms: "rotate" (`SelectionStartRotate` → `StartRotateAction`: `StartRotateRequest`
pauses the pen, overlay re-render, `J5(true)`), "rotate 90° CW/CCW" (`SelectionRotateEvent` →
`SelectionManager.onRotate` → `RotateAction`), mirror X/Y (`MirrorAction`, no pause), align
(`AlignRequest`, no pause, then `y4()` re-starts transform) [code].

The `onScalingDisplayPrepareEvent`, `onScrollingDisplayPrepareEvent`, `onViewportScrollEndEvent`
subscribers only update the selection render matrix and popup position while the page is zoomed or
scrolled with a selection present (`SH:2396-2418, 2791-2795`) [code].

### 5.8 Leaving TRANSFORM: `QuitTransformAction`

Called from almost every selection action through `U4()` (`SH:960-990`). `U4` skips the quit only
when `i1()` is true (SELECTION mode **and** no pausing popup counted by `PenEventHandler`) or the
bean is already marked quited (`Y0().getN()`); in SELECTION mode with a popup open it still runs
`QuitTransformAction` [code]. `QuitTransformAction.create()` (`act/QuitTransformAction.java:667-744`):

1. Find the box of the transform page; copy the visible box list.
2. `QuitTransformRequest.execute2()` (`req/QuitTransformRequest.java:81-161`) — a
   `BaseRxNoteRequest`, so **no pen pause**: `activeRenderMode(SCRIBBLE)`, apply the accumulated
   transform matrix to each shape (with undo command), unselect, add back to the page, save the page,
   update links, `clearSelectionRect()`, post change event.
3. Cross-page moves (`AddShapesToOtherPageAction`, `addCrossPageShapes`), tags, universal links.
4. Remove `RENDER_KEY_TRANSFORM` **without** invalidate (`:635-637`).
5. If `renderChangeShape` (default true): `forceRenderDirtyScreenRectToDisplay(changeShapes)` → page
   slices re-rendered for the old and new positions → `invalidate(DisplayArgs())` [code].
6. `ExpandShapeDraftPageAction`, add undo/redo command.
7. `T0()` (`:543-547`): `UpdateSelectionMenuEvent`, `ApplyFastModeEvent(false)`.

`ApplyFastModeEvent(false)` → `EEH.onApplyFastModeEvent` (`EEH:203-218`): cancel timers; since fast
mode is on, `k()` — but `k()` does nothing while `renderInFastMode()` is true, i.e. while the handler
is still in TRANSFORM (`EEH:185-190`) — then posts `PenEvent.resumeRawDrawingImmediately()` (delay 0)
[code]. In the `U4` chain the selection mode is reset to SELECTION only **after**
`QuitTransformAction` completes (`W4 → X4(next observable) → Y4 → Z4`), so at `T0` time the mode is
still TRANSFORM and fast mode is **not** released there [code for ordering; the main-thread
synchronous delivery of `postDocEvent` is inferred from EventBus `ThreadMode.MAIN` semantics]. The
resume event is processed later on the main looper (`PEH.s()` uses `observeOn(mainThread)`), by which
time the mode may already be SELECTION, so render can come back with delay 0 [inferred].

Fast mode is released later by any of [code]: `SelectionHandler.onDeactivate()` →
`exitFastMode()` (`SH:2142-2153`, `ScH:817-819`; mode already reset to SELECTION by `g5()` so
`k()` runs), `HandlerChangedEvent` (`EEH:252-258`), `onRawPenUpEvent` after an eraser stroke
(`EEH:285-291`, `ScH:962-967`), `QuitTransformAction` from `onChangeShapeAttributesEvent` (which sets
SELECTION first, `SH:2055-2082`), or any other `ApplyFastModeEvent(false)` source (scroll, zoom, menus).
Release = `resetRefreshCount()` + 5 s timer (plus a 5 s wakelock) → `e()` → `d()`:
`setDitherThreshold(128)`, `clearTransientUpdate(false)` (the argument is the final field `EEH.c`,
never assigned, so `false`), `FastModeStateChangedEvent(false)`, wakelock release; if a raw pen-up
happened meanwhile, a `PauseResumeRawPenAction` (`EEH:118-150`) [code]. Turbo is **not** restored by
`d()` [code].

Additional release/cancel conditions not listed above [code, `EEH`]:

- `e()` returns **without releasing and without rescheduling** if `touchStateInfo.isPenRawTouch()` is
  true when the 5 s timer fires (`EEH.e()` at `EEH:145-153`; flag set by `ScribbleHandler.updatePenRawTouch`,
  `ScH:1293-1295`). Fast mode then stays on until the next `k()` caller [code; how often the pen is
  down exactly at the 5 s mark is unknown].
- `onRawPenDownEvent` cancels a pending release timer (`c()`); only a following `RawPenUpEvent`
  restarts it. `RawPenDownEvent/RawPenUpEvent` are posted by `ScribbleHandler.onBeginRawErasing /
onEndRawErasing` (`ScH:931-936, 962-967`), `EraseHandler` (`:72, :92`) and `EpdShapeHandler`
  (`:324, :390`) — i.e. eraser strokes and shape-tool strokes, not plain pen strokes [code: grep].
- `onBeforePageChangeEvent` and `onCloseDocumentEvent` call `d()` **immediately** (no 5 s delay)
  when fast mode is on [code].
- `onQuitFastModeImmediatelyEvent` (posted by `TextInputHandler`, `TextInputHelper`,
  `FloatMenuDragHandler`, not by the selection code) releases immediately, posts
  `PenEvent.resumeRawDrawing()` and either `applyGCImmediately()` or `resetRefreshCount()` [code].
- `onHandlerChangedEvent` re-**applies** fast mode (`h()` → `a()`) if the newly active handler
  reports `renderInFastMode()`, and `onResumeScribbleComponentEvent` re-applies it unconditionally
  while in TRANSFORM (`EEH:252-258, 300-305`) [code].
- `onZoomPreviewPopMoveEnableEvent` maps to `ApplyFastModeEvent(enable)` [code].
- `destroy()` calls `EpdController.resetEpdPost()` [code].

### 5.9 Deselection

Paths [code]:

| Trigger                                                                               | Handler path                                                                                                                                              | Result with default `isAutoSwitchToPenMode() == true`                                           |
| ------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| Tap on empty space in TRANSFORM                                                       | `onSingleTapUp` → hit test empty → `V3` → `M4()` → `C4()` → `D4()` (`SH:998-1026, 728-740, 338-383`)                                                      | `toolbarMenuModel.setSelectionMode(false)` + `H4()`: leave the lasso tool, back to the pen tool |
| Same tap, selection came from an HWR surround gesture (`SelectionAction.HWR_GESTURE`) | `M4()` with `e1()` true → `UndoAction.setResumePen(false)` instead of `C4()`, then `h5()` resets the action to LASSO (`SH:728-740, 1269-1271, 1392-1396`) | undo of the gesture result; no deselect chain [code; resulting pen/SF sequence not traced]      |
| Multi-finger tap on empty space                                                       | `V3`: `j1(event)` (finger and pointerCount > 1) suppresses `M4()` (`SH:1004, 1420-1422`)                                                                  | nothing                                                                                         |
| Tap on another stroke                                                                 | `V3` → `f5()` (drop overlay), `x4(shapes)` → new `StartTransformAction`                                                                                   | selection replaced                                                                              |
| Back key                                                                              | `onBackPressed` (`SH:2040-2047`): TRANSFORM → `S4()` (quit transform, stay in tool); else `B4()`                                                          | —                                                                                               |
| Page change / layer / template / margin                                               | `O4()` / `S4()` / `B4()` (`SH:2050-2052, 2204-2235, 2675-2678`)                                                                                           | quit transform (stays in tool) or quit tool                                                     |
| Toolbar: pick another tool                                                            | `ToolbarMenuModel.quitSelection()` (`:1320-1326`) → `QuitSelectionEvent` → `SH.onQuitSelectionEvent` → `H4()`                                             | leave tool                                                                                      |

`H4()` (`SH:533-562`) [code]:

1. `PausePenEvent("…SelectionHandler")` → `PEH.onPausePenEvent` (`PEH:406-416`): named pause `g = true`,
   `PauseResumeRawPenAction` → pause both (state 3 etc.) and a resume `PenEvent(true,true,500)` that is
   rejected because of the named pause.
2. `U4()` → `QuitTransformAction` (§5.8) if still in TRANSFORM (no `resumePen`, `z` = true but blocked).
3. `j5()` (SELECTION), `QuitSelectionAction(docBundle)` (`act/QuitSelectionAction.java`):
   `activeProviderByShapeType(currentShapeType)` → `SelectionHandler.onDeactivate()` (quit transform if
   needed, reset, `SelectionEndEvent`, `exitFastMode()` → release fast mode, §5.8) → pen handler
   `onActivate` (its own logic, other subsystem) → `HandlerChangedEvent`; `ScribbleHandler.onDeactivate`
   → `disableColorCU()` → `ENABLE_COLOR_CU(0)` [code]. Then `QuitSelectionRequest`
   (`clearSelectionRect`, and as a `BaseNoteRequest` another pause), `resumePenDelay(500)` (blocked),
   `SelectionQuitEvent`.
4. `doFinally` → `ResumePenEvent("…SelectionHandler")` → `PEH.onResumePenEvent` (`PEH:462-481`):
   remove the name, `g = false`, `PauseResumeRawPenAction` → pause both, then `PenEvent(true,true,500)`
   → now evaluated against the **pen** handler → `ResumeRawDrawingRequest` with the pen provider's
   args (pen stroke style etc.) → state 2 after 500 ms [code for the chain; the pen-handler side belongs
   to the writing/session documents].

If the user disabled auto-switch, `D4()` instead runs `U4(… D0(true))`: quit transform, clear the
selection, stay in the lasso tool in SELECTION mode; `Y4` → `resumePen()` and the delay-0 resume from
`T0` bring render back with dash style. Fast mode then stays on until one of the release triggers in
§5.8 fires [code for the absence of an `ApplyFastModeEvent(false)` in that path; consequence inferred].

### 5.10 Actions on the selection

| Action (menu id)                           | Event → SelectionHandler                                                                                                                              | Main calls                                                                                                                                                        | Pen / SF effect [code unless noted]                                                                                                                                                                                                          | Refresh afterwards                                                                                                                                                                                                                                       |
| ------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Delete (`SELECTION_DELETE`)                | `RemoveSelectedShapeEvent` → `onRemoveSelectedShape` (`SH:2350-2368`)                                                                                 | `U4(…, z=false)` → `QuitTransformAction`; `b5()` → SELECTION, `RemoveSelectedShapesAction.setRender(true)` (`RemoveSelectedShapesRequest`, no pause); then `D4()` | no resume from `U4` (`z=false`); with auto-switch `H4()` leaves the tool (§5.9)                                                                                                                                                              | `forceRenderVisibleDirtyScreenRectToDisplay(removed)` → invalidate; waveform under still-active transient A2 until release [inferred]                                                                                                                    |
| Copy (`SELECTION_COPY`, Ctrl+C)            | `CopyShapesEvent` → `H0(false)` (`SH:506-525`)                                                                                                        | `U4` quit transform → `L0`: `CopyShapesToClipboardAction`; `C0(false, COPY)` (clear, SELECTION, popup hidden); `K0` → `resumePen()`                               | resume in SELECTION → render back on (dash style)                                                                                                                                                                                            | quit-transform force render                                                                                                                                                                                                                              |
| Cut (`SELECTION_CUT`, Ctrl+X)              | `CutShapesEvent` → `H0(true)`                                                                                                                         | as copy, then `RemoveSelectedShapeEvent(mode CUT)`                                                                                                                | as copy, then as delete                                                                                                                                                                                                                      | as delete                                                                                                                                                                                                                                                |
| Paste (`SELECTION_PASTE`, Ctrl+V)          | `PasteSelectionShapesEvent` → `s4()` → `o4()`                                                                                                         | `ReadClipboardDataAction`, `U4` + `C0(true)`, `PasteCopyShapesAction` (`PasteCopyShapesRequest` pauses), then `r3()` → select pasted → `StartTransformAction`     | pause, new TRANSFORM (§5.5)                                                                                                                                                                                                                  | pasted slices force-rendered                                                                                                                                                                                                                             |
| Clone                                      | `CloneShapesEvent` → `E0()`                                                                                                                           | paste of a deep copy                                                                                                                                              | as paste                                                                                                                                                                                                                                     | as paste                                                                                                                                                                                                                                                 |
| Colour (`SELECTION_COLOR`)                 | `ShowColorPopupEvent` → `onShowColorPopupEvent` (`SH:2594-2609`)                                                                                      | `U4` (quit, `resumePen` in TRANSFORM → reader only), restart transform, `ShowChangeShapeAttrsPopupEvent` → `ChangeShapeAttributesPopup` (a `BaseNotePopWindow`)   | popup → `PopupChangeAction(true)` → `PopupWindowChangeEvent(true)` → `PauseRawPenAction(pauseArgs)`; plus `InvalidateScreenAction(resumeRender=false)` whose pen-controlled invalidate is skipped while shapes are selected (`SH:2175-2180`) | colour applied by `ChangeShapeAttributesAction` → `ChangeShapeAttributesEvent` → SELECTION, `QuitTransformAction(renderChangeShape=false)`, `resumePen`, restart transform (overlay redraw)                                                              |
| Rotate / mirror / align                    | see §5.7                                                                                                                                              |                                                                                                                                                                   | only `StartRotateRequest` pauses                                                                                                                                                                                                             | overlay redraw                                                                                                                                                                                                                                           |
| AI recognise / convert                     | `SelectionAIRecognizeEvent` → `SelectionManager.handleRightTopMenuClick` → `AIRecognizeScribbleEvent` → `onAIRecognizeScribbleEvent` (`SH:1979-2006`) | `BeforeHWRConvertTextAction` (loading dialog), `ConvertHWRToRichTextAction`, `U4`, `ConvertHWRShapeToTextAction`                                                  | loading dialog is a `Dialog` → focus change → `PEH.onActivityFocusChangedEvent` → `InvalidateScreenAction` (pause/resume) [inferred for the dialog-focus link]                                                                               | text shape render                                                                                                                                                                                                                                        |
| Move to layer / link / more / align popups | `K(menuClickEvent, popup)` in `SelectionMenuModel`                                                                                                    | popups are `BaseNotePopWindow`s                                                                                                                                   | pause while open, resume `POPUP_RESUME_PEN_TIME_MS` (500 ms on colour) after close — render only if back in SELECTION                                                                                                                        | —                                                                                                                                                                                                                                                        |
| Undo / redo                                | `onUndo/onRedo` (`SH:2298-2324, 2748-2774`)                                                                                                           | `U4` + `B0`/`l5(false)` then `UndoAction`/`RedoAction`                                                                                                            | `resumePen` from `U4`                                                                                                                                                                                                                        | undo render                                                                                                                                                                                                                                              |
| Every selection-menu click                 | `SelectionMenuHandler` (`:340-346`)                                                                                                                   | `ClickSelectionMenuEvent` (→ `h5()`), `EpdDeviceManager.applyGCWithInterval()`                                                                                    | every N-th click: `byPass(0)`, `applyGCOnce()`, `InvalidateScreenAction` (pause/resume)                                                                                                                                                      | one full GC on the N-th click; counter shared with `BasePageAction:124`, `FuncBarMenuModel:229`, `TextInputMenuHandler:258` and reset by `resetRefreshCount()` on every fast-mode release (`EEH:185-190`) and in `onQuitFastModeImmediatelyEvent` [code] |

### 5.11 Eraser end while the lasso tool is active

The selection provider forces eraser raw drawing on with style 5 (dash) or 8 (soft eraser for
move-stroke erasers) (`RRDR:61-64`) [code]. Eraser input goes `BeginRawErasingEvent` →
`TouchEventHandler.beginRawErasing` (`:159-171`) → `SelectionHandler` (inherits
`ScribbleHandler.onBeginRawErasing`, `ScH:931-936`) → `RawPenDownEvent` + normal erase path; on end
`RawPenUpEvent` (`ScH:962-967`) [code]. In SELECTION mode this runs with the session on (state 2);
in TRANSFORM mode with the session paused (state 3) [inferred from §5.5]. In TRANSFORM the last
resume left brush raw drawing **off** and eraser raw drawing **on** (§5.5); what SF does with eraser
raw drawing enabled while the pen state is 3 is [unknown] (E11). The erasing subsystem document
covers what the erase itself renders.

### 5.12 Further entry paths and subscribers found by the re-check

Grep over `SelectionHandler`'s `@Subscribe` methods and the `act/` and `req/` directories turned up
these paths that the sections above do not trace [code for the calls listed; SF effects follow from
the primitives in §4 and are not separately traced]:

- **Select all** (`SelectAllShapesEvent` → `SH.onSelectAllShapesEvent`, `SH:2421-2444`): outside
  TRANSFORM → `p5()` → `SelectAllShapesAction`, which issues `SelectShapesRequest` (pauses, as in
  §5.4), `forceRenderNoteToDisplay` for boxes with nothing selected (`act/SelectAllShapesAction.java:76`)
  and `resumePen()` when nothing was selected (`:81`); inside TRANSFORM it quits the transform first
  via `V4`.
- **Programmatic starts**: `onSelectShapesEvent` (`SH:2454-2486`), `onSelectShapesToStartEvent`
  (`SH:2488-2504`, sets TRANSFORM before `z5`, or calls `S4()` if no shape is in a visible box),
  `onSelectEditFinishShapesEvent` (`SH:2447-2452`), `onReselectShapesEvent`. All end in
  `z5/y5 → StartTransformAction` (§5.5).
- **Text transform**: `StartTextTransformAction` calls `startTransformAction.setApplyFastMode(false)`
  (`act/StartTextTransformAction.java:90`), so that path does **not** post `ApplyFastModeEvent(true)`
  (`StartTransformAction.p0()` checks the flag `s`, default true, `act/StartTransformAction.java:51, 60, 225-229, 479-481`).
  `StartHWRPlacementTransformAction` and `QuitTextInputAction` belong to the TEXT_INPUT mode and were
  not traced.
- **Keyboard**: `SH.onKeyEvent` maps Ctrl+C / Ctrl+X / Ctrl+V to the copy / cut / paste handlers
  (`SH:2183-2201`).
- **Long press** inside the lasso tool: `SH.onLongPress` forwards to `ScribbleHandler.onLongPress`
  only in SELECTION mode with no pausing popup and no touch started under a popup (`SH:2212-2218`).
- **Audio popup**: `AudioMenuPopupChangeEvent(show)` both increments `PenEventHandler`'s popup
  counter (pause, `PEH:350-360`) and makes `SH` quit the transform (`S4()`, `SH:2032-2037`).
- **Content view changed** (`ContentViewChangedEvent`) → `S4()` when shapes are selected
  (`SH:2100-2105`).
- **Focus loss/gain**: `PEH.onActivityFocusChangedEvent` runs `InvalidateScreenAction` whenever
  `canPenRawRender() || isUsePenInput()` (`PEH:334-347`); in TRANSFORM the resulting
  `InvalidateViewWithPenControlEvent` is swallowed by `SH.onInvalidateViewWithPenControl`
  (`SH:2175-2180`), so a dialog over a live selection neither pauses nor resumes through that path
  [code].
- `StartSelectionRequest` (`req/StartSelectionRequest.java`) exists but has no caller in the APK
  [code: grep].

---

## 6. Tables

### 6.1 Per step: SDK calls and SF-visible effect (stylus, SMART lasso, default settings)

| #   | Step                            | Calls reaching SF (in order)                                                                                                                                                                                                        | Pen state after | Style       | Transient update          | App-rendered frames                             |
| --- | ------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------- | ----------- | ------------------------- | ----------------------------------------------- |
| 1   | Select lasso tool               | `ENABLE_COLOR_CU(1)`; pause (enablePost 1 ×2, state 3, brush 1, eraser 0/5); +500 ms: regions, style 5, params (5) (8), eraser 1/5-or-8, brush 1, state 2                                                                           | 2               | 5 dash      | none                      | one full `View.invalidate()`                    |
| 2   | Pen down + move                 | none from the app; SF draws the dashed trace itself                                                                                                                                                                                 | 2               | 5           | none                      | none                                            |
| 3a  | Pen up, nothing hit             | pause (SelectShapesRequest); full page slice render; +500 ms resume → state 2                                                                                                                                                       | 2               | 5           | none                      | page force-render                               |
| 3b  | Pen up, shapes hit              | pause (SelectShapesRequest), second pause (StartTransformRequest: only brush 1 + eraser 0/5 reach SF), `APPLY_TRANSIENT_UPDATE(2308)`, dither 160, turbo 5, +500 ms: regions, style 5, params, eraser-raw 1, **brush 0**, reader on | 3               | 5 (latched) | A2 quality 2308           | dirty-rect page render, overlay (selection)     |
| 4   | Tap (no move) in SELECTION      | +300 ms: pause, invalidate, +500 ms resume                                                                                                                                                                                          | 2               | 5           | none                      | full invalidate                                 |
| 5   | Drag / scale / rotate           | none (brush-raw 0 and eraser-raw 1 left over from 3b)                                                                                                                                                                               | 3               | —           | 2308 (active)             | one overlay frame per ≥10 ms batch; popup moves |
| 6   | Lift after drag                 | none                                                                                                                                                                                                                                | 3               | —           | 2308                      | popup update                                    |
| 7   | Tap empty (auto-switch default) | named pause (state 3); quit transform (dirty-rect page render); provider → pen; `ENABLE_COLOR_CU(0)`; pause ×2; +5 s `CLEAR_TRANSIENT_UPDATE(0)`, dither 128; +~500 ms pen-provider resume → state 2                                | 2               | pen style   | cleared 5 s after release | force-render of old+new areas                   |
| 8   | Delete                          | quit transform (no pause), remove (no pause), then as 7                                                                                                                                                                             | 2               | pen style   | as 7                      | dirty-rect render of removed shapes             |

Pen-state values in rows 3b–6 are derived from the predicates (§3, §5.5) [inferred]; each call in the
"Calls" column is [code].

### 6.2 Who draws what

| Visual                                 | Drawn by                                                                             | Evidence                                                                |
| -------------------------------------- | ------------------------------------------------------------------------------------ | ----------------------------------------------------------------------- |
| Lasso trace (SMART/FREE, stylus)       | SF raw drawing, `PainterDash` [inferred name mapping], current colour                | `SH:1903`, `RRDR:84-96`, `TH:25`, `sf` symbols                          |
| Lasso trace (finger)                   | framework direct draw via `EpdController.moveTo/quadTo(…, DU)` from `AppTouchRender` | `ATR:122-127, 174-179`, `ScribbleTouchDistributor.java:387`             |
| Rectangle lasso                        | app overlay renderer, black dash 10/10, width 4.5                                    | `act/DrawRectangleSelectionAction.java:54-86`                           |
| FREE lasso outline after selection     | app, dashed free shape (width 5·s, dash 6·s), part of variety shapes                 | `req/StartTransformRequest.java:61-80, 100-104`                         |
| Selection rect, handles, moving shapes | app overlay `RENDER_KEY_TRANSFORM` (`RenderVarietyShapesAction`)                     | `act/RenderSelectionShapesAction.java`, `SelectionActionsKt.java:38-68` |
| Page without the selected shapes       | app page slices, re-rendered for the selection's dirty rect                          | `act/RenderSelectionDirtyRectToDisplayAction.java`                      |
| Selection menu                         | separate `PopupWindow` (`SelectionPopupMenu`)                                        | `ScribbleFragment.java:1896-1911`                                       |

### 6.3 Update modes used in the selection flow

| Situation                              | Mode                                                                                                                   | Source                                                         |
| -------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------- | ---------------------------------------------------- | ------------------------------------------------------------------------- |
| TRANSFORM phase (and rectangle drag)   | transient `UI_A2_QUALITY_MODE` 2308 = `ANIM                                                                            | DITHER                                                         | Y1` for all app frames [inferred scope from SF name] | `EEH:109-116, 176-179`; `SDM:392, 477`; `05-panel-refresh-levers.md` §1.2 |
| Fast-mode dither                       | threshold 160 on colour devices (255 mono), back to 128 on release                                                     | `EEH:81, 136-143`                                              |
| Turbo                                  | `setEpdTurbo(deviceConfig.epdTurbo)` = 5 on NA4C, never restored by the release path                                   | `EEH:109-116`, `res/raw/note_noteair4c.json`                   |
| App invalidates                        | default `DisplayArgs` → no per-view mode; `REGAL_PLUS` once only if `enableRegalModeOnce` (not used in selection code) | `InkRenderManager.java:79-128`                                 |
| Selection-menu GC cadence              | `applyGCOnce()` + `byPass(0)` every N clicks                                                                           | `EpdDeviceManager.java:16-40`, `SelectionMenuHandler.java:346` |
| After rectangle lasso / every lasso up | `resetViewUpdateMode(view)`                                                                                            | `req/ClearSelectionPathRequest.java:19`                        |

No `HAND_WRITING_REPAINT_MODE`, `handwritingRepaint`, full-screen `invalidate(view, GC)` or
`applyGCOnce` appears in `SelectionHandler`, the selection actions or requests [code: grep over
`act/`, `req/`, `SH`].

---

## 7. Device-specific branches on the Note Air 4C

| Branch                                                                  | Value on NA4C                                                                            | How determined                                                                                                                                                                                                   |
| ----------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `DeviceInfoUtil.isColorDevice()` (`getColorType() > 0`)                 | true                                                                                     | [inferred]: `epdColorMode: 1`, CFA device (`05-panel-refresh-levers.md` §0); `SF` uses `HWEpdcManager` (`08-surfaceflinger.md`)                                                                                  |
| `DELAY_ENABLE_RAW_DRAWING_MILLS` / default `RawPenArgs.resumeDelayTime` | 500 ms (150 mono)                                                                        | `RawPenArgs.java:69, 232` [code] + colour [inferred]                                                                                                                                                             |
| `PenEvent.POPUP_RESUME_PEN_TIME_MS` (app copy)                          | 500 ms (300 mono)                                                                        | `note/note/event/PenEvent.java:20` [code]                                                                                                                                                                        |
| Fast-mode dither threshold                                              | 160 (255 mono)                                                                           | `EEH:81` [code]                                                                                                                                                                                                  |
| DeviceConfig json                                                       | `res/raw/note_noteair4c.json` (`Build.MODEL` "NoteAir4C" lower-cased)                    | `DeviceConfig.java:111-129` [code]; model string from `01-sources-and-firmware.md` [code in doc]                                                                                                                 |
| `epdTurbo`                                                              | 5                                                                                        | `note_noteair4c.json` [code]; the `DeviceConfig` default is also 5 (`DeviceConfig.java:82`), so the json entry does not change the value [code]                                                                  |
| `transformUpdateModel`                                                  | not set → `ANIMATION_QUALITY` (2308)                                                     | json lacks key; `EEH:176-179` [code]. 13 other models set `"DU_QUALITY"` (e.g. `note_noteair2.json`) [code]                                                                                                      |
| `selectionTransformTimeMs` (drag batching)                              | 10 ms                                                                                    | default `DeviceConfig.java:76`, not in NA4C json [code]; merge of json over defaults [inferred]                                                                                                                  |
| `selectionDrawTrackTimeMs` (rectangle)                                  | 30 ms                                                                                    | default `DeviceConfig.java:77` [code]                                                                                                                                                                            |
| `isTablet()` / `hasHTCon()`                                             | both true [inferred]                                                                     | `vendor.onyx.tablet`, `vendor.onyx.htcon` (`SystemPropertiesUtil.java`); htcon from SF provider choice (`08-surfaceflinger.md`); effect here: gesture timing constants in `ScribbleTouchDistributor` static init |
| Selection type default                                                  | SMART                                                                                    | `NoteMMKVHelper.java:1005-1006` [code]; the user's current setting [unknown]                                                                                                                                     |
| Auto-switch to pen after quitting selection                             | true by default                                                                          | `NoteMMKVHelper.java:1076-1077` [code]; user's setting [unknown]                                                                                                                                                 |
| Selection-menu GC interval N                                            | from `noteSetting.getFullRefreshCount()` or `Integer.MAX_VALUE` when full refresh is off | `EpdDeviceManager.setGcInterval`, `LoadAndApplySettingAction` (per `~/tmp_zfs/reversed_onyx_notes_app/REPORT.md` line 204) [code in report]; current value [unknown]                                             |
| `supportEAC`                                                            | false for stock (from `/onyxconfig/mmkv/onyx_config`)                                    | handoff; no selection code reads it [code: grep]                                                                                                                                                                 |
| Colour CU                                                               | enabled for the whole time the lasso tool is active                                      | `SH:1958`, `ScH:908-918, 946-954` [code]; SF meaning [unknown]                                                                                                                                                   |

---

## 8. Interactions with other subsystems

- **Session lifecycle**: the lasso tool never calls `openRawDrawing/closeRawDrawing`; it only toggles
  render/reader via `PenEventHandler`. Its specific contributions are (a) the dash style and forced
  eraser raw drawing in `ResumeRawDrawingRequest`, (b) `canPenRawRender()` false in TRANSFORM, which
  keeps the session paused for the entire time a selection is visible, (c) a named pause
  (`PausePenEvent/ResumePenEvent("…SelectionHandler")`) around quitting the tool [code].
- **Pausing by requests**: the default pause in `BaseNoteRequest` means that most note-thread work
  (hit tests, start transform, paste, quit selection) pauses the session as a side effect; this is
  the mechanism that turns the SF session off at lasso pen-up [code].
- **Writing**: after leaving the tool the pen provider's own `ResumeRawDrawingRequest` restores the
  pen style; the lasso's dash style is never reset explicitly (`closeDrawing` would reset style to 0,
  but is not called) [code].
- **Erasing**: see §5.11; plus `RawPenUpEvent` from an eraser stroke is one of the few events that
  can release fast mode while in SELECTION (`EEH:285-291`) [code].
- **Refresh / EinkWise**: fast mode issues `setEpdTurbo(5)` and a transient A2 mode that overrides
  whatever app-scope refresh profile applies to the app's frames until cleared [inferred scope];
  the only full refresh is the menu-click GC counter; `resetRefreshCount()` is called when fast mode
  is released (`EEH:185-190`) [code].
- **Input routing**: stylus MotionEvents are used for the lasso path and for all transforms even
  while the SF raw reader runs; finger input draws the lasso only in SELECTION (`supportFingerDraw`);
  a touch starting while a pausing popup is open is swallowed (`w5`) [code].
- **Popups**: `SelectionPopupMenu` does not pause; its sub-popups do (and bump the colour-CU counter,
  which is ignored while the lasso handler is active) [code].
- **Firmware**: all transient/turbo/dither/colour-CU state is SF-global or per-app state not reset
  by the app on release except dither and transient (§5.8) [code for which calls exist].

---

## 9. Open questions as hardware experiments

All with `adb logcat -b all -v time` running, filtering for `onyx_tcon_send_cmd`,
`onyx_tcon_set_upd_scheme`, `HWEpdcManager: ### EUS`, `SurfaceFlinger: Pid N State change to`,
`### applyTransientUpdateMode`, `### clearTransientUpdateMode`. Use the stock Notes app, default
settings (SMART lasso, auto-switch on) unless stated. Note the Notes pid first.

- **E1 — Tool entry.** Tap the lasso tool, wait 2 s. Expect one `toPause` then `running` ~500 ms
  later for the Notes pid. Refuted if no state change (would mean the tool entry does not cycle the
  session) or if `running` comes without the delay.
- **E2 — Who draws and who wipes the lasso trace.** Draw a lasso around nothing. Expect SF-style pen
  updates during the stroke (same waveform/flags as a pen stroke, compare with a plain pen stroke),
  then `toPause`, a partial update covering the page (the `forceRenderNoteToDisplay`), then `running`
  ~500 ms later. Repeat with a lasso that selects a stroke: expect `toPause` and **no** `running`
  while the selection is shown. Check whether any `onyx_tcon_send_cmd` covers the whole lasso path
  area (rect larger than the selected stroke's bounds) — that answers the [unknown] in §5.4.
- **E3 — Drag with the stylus while paused.** Select a stroke, drag it with the stylus for ~2 s,
  lift, wait 3 s. Hypothesis H3 (from the proven erase mechanism): `EUS s 4` / A2 during the drag and
  a full GC16 (`update_mode[1]`) ~1.5 s after lift. Refuted if the drag updates carry waveform 4 but
  no scheme switch and no full update follows. Repeat the same drag with a **finger** — if the finger
  drag produces no scheme switch, the trigger is stylus input to a paused session, not the frames.
- **E4 — Transient A2 during TRANSFORM.** In E3, check the waveform of the app frames drawing the
  moving selection. Expect `### applyTransientUpdateMode, updateMode 2308` when the selection appears
  and A2-class waveform (`waveform[4]`) on the drag frames. Refuted if the drag frames are GC16/GU
  (then the transient update is not applied to these frames).
- **E5 — Release of fast mode on deselect.** After E3, tap empty space. Expect: the quit-transform
  partial update(s), the provider switch back to the pen (`toPause` again), `running` ~500 ms later,
  and `### clearTransientUpdateMode, clearFlag 0` about 5 s after the tap. Record the waveform of the
  update that repaints the moved stroke at its new position: A2 (if the transient mode still applies,
  as the code order suggests) vs GC16. Check whether any refresh follows the clear (code says no GC).
  Repeat with auto-switch disabled in the lasso settings: expect **no** clear until something else
  releases fast mode (e.g. switching tools or erasing).
- **E6 — Rectangle lasso.** Switch to rectangle selection, drag a rectangle with the stylus. Expect no
  `running` during the drag (render never resumed for rectangle), A2 frames for the dashed rectangle,
  and (if H3 holds) a scheme-4 switch plus a delayed full GC16 after lift.
- **E7 — Popup vs pen.** With a selection shown, open the colour popup and close it. Expect `toPause`
  on open (or no change, since already paused) and no `running` after close while the selection stays
  (TRANSFORM). With the selection popup alone, expect no state change when it appears/moves.
- **E8 — Tap in SELECTION.** In the lasso tool with nothing selected, tap the pen once. Expect
  ~300 ms later `toPause`, a full-view partial update, then `running` ~500 ms later.
- **E9 — Selection-menu GC cadence.** Perform many selection-menu actions (e.g. copy repeatedly).
  Expect exactly one full update (`update_mode[1]`) every N clicks where N matches the device's
  "Full Refresh Frequency" setting, and none if full refresh is off.
- **E10 — Delete.** Select a coloured stroke, delete it. Record the waveform of the update that
  clears it (transient A2 still active vs GC16) and whether a `clearTransientUpdateMode` follows ~5 s
  later; check whether any flash (`update_mode[1]`) occurs.
- **E11 — Eraser end in the lasso tool.** (a) In SELECTION mode erase with the back end: expect a
  running session (no `toPause` before the stroke), eraser trace style 5 or 8. (b) With a selection
  shown (TRANSFORM, session paused) erase with the back end: if H3 holds, expect scheme 4 and a full
  GC16 ~1.5 s after lift — this would be a stock flash-after-erase case, contradicting "stock never
  flashes after erasing" only for this state.
- **E12 — Colour CU.** Enter and leave the lasso tool while watching SF logs for any line tied to
  `ENABLE_COLOR_CU` / `setEnableColorCU`, and compare the rendering of coloured strokes inside the
  selection overlay with and without the tool active. Determines what colour CU changes [unknown].
- **E13 — Turbo persistence.** After E5, write a normal pen stroke and compare driver-log timing or
  `turbo` in any log line against a fresh app start; the release path never restores turbo, so a
  lasso session may leave turbo 5 behind [code for absence; effect unknown].
- **E14 — Brush raw drawing off in TRANSFORM.** Code sends `SET_BRUSH_RAW_DRAWING_ENABLED(0)` with the
  reader-only resume after a selection appears (§5.5). During E3, check whether a stylus drag over a
  selection produces any SF pen-trace pixels or any `EUS`/scheme switch at all; compare with a stylus
  drag in the rectangle-lasso tool (E6), where no resume is issued and the last pause left brush 1.
  A difference between the two would indicate that the brush flag, not only pen state 3, decides
  whether SF reacts to pen input.
- **E15 — Fast-mode release skipped while the pen is down.** Deselect (tap empty space), then put the
  pen tip on the screen about 4.5 s later and hold it past the 5 s mark. Code (`EEH.e()` checks
  `isPenRawTouch`) predicts no `### clearTransientUpdateMode` at 5 s and none after lift unless
  another release trigger fires. Refuted if the clear still arrives ~5 s after the tap.
- **E16 — Page turn with a selection.** Select a stroke, then turn the page. Code predicts an
  immediate `### clearTransientUpdateMode, clearFlag 0` (no 5 s delay, `EEH.onBeforePageChangeEvent`).

---

## 10. Things not resolved statically

- Exact SF behaviour of `PainterDash` (dash length, whether parameter `5.0` is the dash length or gap)
  [unknown; symbol names only].
- Whether SF clears its own handwriting overlay pixels when a paused session's app frame arrives
  (decides how the lasso trace vanishes when the selection is non-empty) [unknown] — E2.
- Whether the transient update applies to all layers of the app (including the `SelectionPopupMenu`
  window) or only to the note view's layer [unknown].
- The meaning of `ENABLE_COLOR_CU` [unknown] — E12.
- The public Pen SDK 1.5.4.3 was not diffed against the bundled copy for the classes used here; all
  statements refer to the bundled copy. The second decompilation
  (`~/tmp_zfs/reversed_onyx_notes_app/src`) was not re-read line by line; its report agrees on the
  `SelectionHandler` named pause (`:536/692`) and the menu GC counter (`SelectionMenuHandler:346`).

---

## Verification notes

Independent re-check against the jadx output of the stock APK (bundled SDK copies), the firmware
`ViewUpdateHelper` and the NA4C device json, static only (2026-09-25).

Checked and confirmed as written [code]:

- `SelectionHandler` predicates: `canPenRawRender()` / `canBrushRawRender()` = `i1() && !h1() && !d1()`
  (`SH:1897-1905`), `i1()` = SELECTION && no pausing popup (`SH:1398-1400`), `h1()` = rectangle type
  (`SH:1374-1376`), `d1()` = `t && u` (`SH:1250-1252`), `k1()` = TRANSFORM (`SH:1444-1446`),
  `isUsePenInput()` = `!h1()`, `supportFingerDraw()` = `i1()`, `renderInFastMode()` = `k1()`,
  `isEnableColorCU()` = true. `isTouchEventConsumed` additionally requires a non-empty selection.
- `ResumeRawDrawingRequest`: dash style 5 only for the selection provider (and erase type 2),
  `setStrokeParameters(5,{5.0})` and `(8,{eraserWidth,0.5,0.1})` for every provider, eraser raw drawing
  forced on for the selection provider, sleep = the `PenEvent` delay (`RRDR:40-138`).
- `PenEventHandler.A()` / `z()` gate lists and the `q()` bean construction (`PEH:105-107, 155-181, 271-273`).
- `BaseNoteRequest` default pause and the opt-out list: `AlignRequest`, `CutShapesRequest`,
  `RemoveSelectedShapesRequest`, `TranslateRequest`, `ScaleRequest`, `RotateRequest`, `MirrorRequest`
  call `setPauseRawDraw(false)`; `SelectShapesRequest`, `StartTransformRequest`, `StartRotateRequest`,
  `PasteCopyShapesRequest`, `MoveShapesToLayerRequest`, `QuitSelectionRequest` pause;
  `QuitTransformRequest` is a `BaseRxNoteRequest` (no pause); `ClearSelectionPathRequest` is a plain
  `RxRequest`.
- `TouchHelper` guards (`TH:276-307`), `resetPenDefaultRawDrawing` (`TH:182-185`), `SFTouchRender`
  render off = `leaveScribbleMode` (= `enablePost(view,1)`, `SDMDevice.java:1875-1877`) + pen state 3,
  `EpdPenManager` state constants and calls, `AppTouchRender.setDrawingRenderEnabled(false)` =
  `enablePost(1)`.
- SF transaction codes in `ViewUpdateHelper.java:113-216`; `SDMDevice` reflects
  `applyTransientUpdate`/`clearTransientUpdate` directly on `android.onyx.ViewUpdateHelper`
  (`SDMDevice.java:453, 586-587`), which sends `APPLY_TRANSIENT_UPDATE` / `CLEAR_TRANSIENT_UPDATE`
  (`ViewUpdateHelper.java:404-407, 488-495`).
- `EpdEventHandler`: dither 160 on colour devices, `setEpdTurbo(deviceConfig.epdTurbo)`,
  `applyTransientUpdate(transformUpdateModel ?: ANIMATION_QUALITY)`, 5 s release, `k()` blocked while
  `renderInFastMode()`; NA4C json has no `transformUpdateModel`.
- `SelectionPopupMenu.windowChangeEvent` is empty; `BaseNotePopWindow.onShow` otherwise only
  registers events, and `BasePopWindow` has no EPD calls.
- `QuitTransformAction.T0()` posts `UpdateSelectionMenuEvent` + `ApplyFastModeEvent(false)`;
  `RENDER_KEY_TRANSFORM` is removed without invalidate.
- `InkRenderManager.invalidate(DisplayArgs)`: default `DisplayArgs` has `enablePost = false`,
  `invalidate = true`, no regal; per-view update mode is set only if `viewUpdateModelOnce` is not
  `None`, which only `enableRegalModeOnce()` changes (callers: page actions and
  `InvalidateScreenAction` with `checkUseRegalMode`, not the selection code).
- `RectangleSelectionDrawingProxy` fast-mode window and 30 ms throttle; `DeviceConfig` defaults
  (`selectionTransformTimeMs` 10, `selectionDrawTrackTimeMs` 30, `epdTurbo` 5) and model-based json
  lookup (`DeviceConfig.java:111-133`).

Corrections made:

1. §5.5 / §6.1 row 3b (and new text in §1 item 5): the reader-only resume after a selection appears was
   described as re-applying "brush 1". `PEH.q()` passes `HandlerManager.canBrushRawRender()`, which is
   false in TRANSFORM, so the request sends **brush raw drawing 0** (eraser raw drawing still forced
   on). Changed to brush 0.
2. §5.5: the second pause (from `StartTransformRequest`) was said to re-send "reader/brush/eraser
   resets". The reader toggle is guarded by `TH.c` just like render by `TH.b`, so only
   `resetPenDefaultRawDrawing()` (brush 1, eraser 0/5) reaches SF. Corrected.
3. §5.1: `h()` was written as `setBrushRawDrawing(true)`; it forwards the bean's
   `canBrushRawRender`. Corrected, with the value per mode.
4. §4: `setStrokeStyle` reaches `EpdController.setStrokeStyle` through both `SFTouchRender` and
   `AppTouchRender`, i.e. twice per call. Added, plus a row for the brush/eraser raw-drawing calls.
5. §5.2: finger lasso gating was incomplete (`canFingerTouch()` of the active scene, `ATR` reader and
   render flags, finger filter default) and the `ATR` down call is `moveTo` without `DU`; the
   `onInterceptTouchEvent` location was "around line 230", now `ATR:235-237`. Stylus-MotionEvent
   delivery now cites `setPostInputEvent(true)` (`NM:232-235`) and is marked [inferred] as to cause.
6. §5.8: the `U4()` guard was "unless already in SELECTION mode"; it is `i1()` (SELECTION **and**
   no pausing popup) or the bean's quited flag. Corrected.
7. §5.8: release path details were missing: pen-down-at-timer skip (`isPenRawTouch`), timer cancel on
   `RawPenDownEvent`, which handlers post `RawPenDown/UpEvent`, immediate release on page change and
   document close, `QuitFastModeImmediatelyEvent`, re-apply on `HandlerChangedEvent` /
   `ResumeScribbleComponentEvent`, `resetEpdPost()` on destroy. Added.
8. §1 item 10 / §5.10: the GC counter is shared across the app and reset at every fast-mode release.
   Added.
9. §5.9: added the HWR-gesture tap-empty branch (`UndoAction` instead of deselect) and the
   multi-finger tap suppression.

Items added: §5.4 timing caveat (pause is posted to the main thread, not synchronous with the
request body) and multi-box force-render detail; §5.11 note on brush/eraser flags in TRANSFORM;
§5.12 (select all, programmatic starts, text transform without fast mode, keyboard, long press,
audio popup, content-view change, focus change, unused `StartSelectionRequest`); §7 `epdTurbo`
default; experiments E14-E16.

Not confirmed / left as is:

- Line numbers in `SelectionManager`, `ScribbleFragment`, `SelectionPopupMenusKt`,
  `HandlerManager`, `ForceRenderSliceListActionsKt`, `RenderSelectionDirtyRectToDisplayAction`,
  `StartTransformRequest` (FREE outline dash sizes) and `NoteMMKVHelper` defaults were not re-opened;
  they are carried over from the original author.
- `UI_A2_QUALITY_MODE` = 2308 is taken from `05-panel-refresh-levers.md`, not re-derived here.
- The `sf` symbol mapping (`PainterDash`, `MemoryPainter::setStrokeParameters`) was not re-checked.
- `isColorDevice()` = true on NA4C remains [inferred]; every colour/mono constant in this document
  depends on it.
- Whether `RawInputManager.pause/resumeRawInputReader` has any SF-visible effect remains [unknown].
- The main-thread synchronous delivery assumption behind "fast mode is not released at `T0`" (§5.8)
  still rests on the caller of `T0` running on the main thread, which was not traced through every Rx
  `observeOn` in `QuitTransformAction.create()`.
- The HWR-gesture undo branch and `SelectAllShapesAction` were located but their full pen/SF
  sequences were not traced.
