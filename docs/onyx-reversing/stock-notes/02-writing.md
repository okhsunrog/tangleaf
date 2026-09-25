# Stock Notes: writing (pen strokes)

Subsystem document for the stock BOOX Notes app (`com.onyx.android.note`, "knote2") on a Note Air 4C.
It describes the full path of one pen stroke and of a series of strokes: what the firmware draws
live, what the app draws, when the app's own rendering reaches the panel, and every refresh-relevant
call issued along the way. Research only: no fixes and no design for Tangleaf.

## Conventions

- Every claim is tagged **[code]** (read in source or disassembly), **[inferred]** (reasoned from
  code, not read directly) or **[unknown]** (not settleable statically).
- Paths are abbreviated:
  - `app/` = `~/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/note/` (stock app code)
  - `sdk/` = `~/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/sdk/` (the SDK copy
    **bundled inside the stock APK**; this is what stock actually runs)
  - `pubsdk/` = `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-pen-1.5.4.3/sources/com/onyx/android/sdk/`
  - `fw/` = `~/tmp_zfs/onyx_framework/out/sources/android/` (framework.jar)
  - `sf` = `~/tmp_zfs/onyx_framework/native/surfaceflinger` (addresses are virtual addresses; symbol
    names come from its `.gnu_debugdata` MiniDebugInfo, extracted with
    `objcopy --dump-section .gnu_debugdata` + `xz -d` + `nm | c++filt`)
  - `res/` = `~/tmp_zfs/boox-notes-inspect/decoded/resources/res/`
  - `vendor.img` = `~/tmp_zfs/boox_fw/imgs/vendor.img` (read with `debugfs -R "cat /build.prop"`)
- Line numbers are jadx line numbers of the file named.
- Transaction codes are the `fw/onyx/ViewUpdateHelper.java` constants (e.g. `SET_STROKE_STYLE` =
  16711688). "SF" = SurfaceFlinger.

**Build identity.** The firmware images analysed carry
`ro.vendor.build.fingerprint=Onyx/NoteAir4C/NoteAir4C:11/2026-04-28_17-50_4.2-rel_04282_555977efe/8548`
(vendor partition Android 11, firmware 4.2) [code: `vendor.img:/build.prop`]; the system partition is
Android 13 (SDK 33). The tablet runs exactly this build, and its SurfaceFlinger, `framework.jar` and pen
native libraries are byte-identical to the analysed copies (verified 2026-09-25, see
[00-overview §0](00-overview.md)). The "firmware 4.1" in the original research brief was wrong.

---

## 1. Summary

1. **One handler writes.** Every "pen" tool whose shape type is one of 2, 3, 4, 5, 15, 21, 22, 60, 61
   is served by `EpdShapeHandler` (`Provider.SCRIBBLE_EPD_PROVIDER`). Geometric shapes use
   `NormalShapeHandler` and do **not** get firmware ink (`canBrushRawRender()` is false there)
   [code: `sdk/scribble/shape/ShapeFactory.java:362-364` `isDFBShape`,
   `sdk/notecore/editor/handler/HandlerManager.java:211-213` `getShapeProvider`,
   `app/note/handler/common/EpdShapeHandler.java:293-295`].
2. **The firmware draws the live stroke; the app draws nothing on screen while the pen moves.**
   SurfaceFlinger reads the pen itself, renders the ink with its own neo-pen engine and pushes it to
   the panel while its handwriting schema (id 7) is active. The app receives the same points through
   a separate, in-process reader (`libonyx_pen_touch_reader.so`) and only feeds them to an in-memory
   stroke model (`Shape.onLowLatencyTouchData`), which draws nothing
   [code: `EpdShapeHandler.java:239-246, 318-340, 377-383`; `sdk/pen/NeoPenRender.java:151-167`].
3. **Per-stroke pen parameters are not sent per stroke.** Style, width, colour, per-style stroke
   parameters, brush/eraser raw-drawing flags, region mode and pen-up timeout are pushed when the
   pen session is (re)enabled by `ResumeRawDrawingRequest`, plus width on zoom changes
   [code: `app/note/request/pen/ResumeRawDrawingRequest.java:84-138`,
   `app/note/action/pen/UpdateRawPenWidthAction.java`]. Other, non-writing call sites also touch
   them outside a resume: every full pause resets brush raw drawing to on and eraser raw drawing
   to off (`TouchHelper.setRawDrawingEnabled` → `resetPenDefaultRawDrawing`,
   `sdk/pen/TouchHelper.java:182-185, 276-284`), `PenShapeChangeAction` (pen-tip eraser tool menu)
   sets style and colour, `StartSelectionRequest` sets width and style 2, span requests set width,
   and `HandWritingHandler` (text handwriting input) sets its own colour/width/style 1
   [code: grep of the setters over `app/`, see Verification notes].
4. **At pen-up the app renders the stroke into its own page bitmap and calls a plain
   `View.invalidate()`** on the `EditorView`, then persists the shape and re-renders the page slice in
   the background, which invalidates again. No update mode, no `ENABLE_POST`, no pen-state change, no
   `handwritingRepaint` is issued [code: `EpdShapeHandler.java:179-217`,
   `sdk/notecore/editor/display/InkRenderManager.java:79-96, 139-141`,
   `sdk/data/note/DisplayArgs.java:19-20`].
5. **The per-stroke hardware reconcile (`GrayscaleRefreshAction`, the only user of
   `HAND_WRITING_REPAINT_MODE`) never runs on this device.** Its gate
   `OnyxSystemConfig.isUseHWUpdateForPenUpRefresh()` returns `false` when `vendor.onyx.tablet` is
   true, and the Note Air 4C's `/vendor/build.prop` sets `vendor.onyx.tablet=true`; independently, its
   default is `!isColorDevice()` [code: `sdk/data/config/system/OnyxSystemConfig.java:145-150`,
   `sdk/data/config/system/data/SystemConfigBean.java:21-22`, `vendor.img:/build.prop`].
6. **Consequence: on this device stock never hands the stroke region back from firmware ink to app
   content per stroke.** While SF is in the handwriting schema, `HandwritingSchema::output` commits app
   layer updates only if they carry a handwriting-repaint entry (or a queued SF update entry exists);
   stock's plain invalidates carry none, so the app frame is composed but not pushed to the panel, and
   the firmware ink simply stays [code: `sf` `HandwritingSchema::output` @ `0x55b828`;
   `UpdateEntryDetector::hasHWRepaint` @ `0x556b48` returns true only if some collected layer entry
   has the mode word `0x80002` (= `HAND_WRITING_REPAINT_MODE`) exactly; that the app's view default
   update mode is what lands in that word is **[inferred]**; what feeds the queued-entry fallback
   (`EpdcManager::next`) is **[unknown]**, so "not committed" stays **[inferred]**]. The panel is
   handed back to app content only when something ends the SF handwriting session: pen state 3
   (pause), or an `ENABLE_POST(v)` with `v >= 0 && v != androidDrawing` [code: `sf` direct callers of
   `EpdcManager::stopHandwriting`]. In stock that happens through `InvalidateScreenAction` /
   pause-resume cycles triggered by UI events (menus, popups, focus, tool switches, page turns, the
   system "refresh screen" broadcast), by erase actions, by the fast-mode exit timer and, in the
   meeting scene only, by a 5 s timer armed at every pen-up refresh (§5.5) — in the normal note scene
   never by a pen stroke on its own.
7. **SF-side scheme during writing.** When the first pen-down after a resume enters the handwriting
   schema, SF runs `afterStartHandwriting`, which calls `setEbcUpdateScheme(3, 0)` → the driver-log line
   `### EUS, s 3 c 0`. Nothing in SF leaves the handwriting schema at pen-up; the schema and scheme 3
   persist until the next pause / `ENABLE_POST`, where `afterStopHandwriting` →
   `clearHandwritingFBSchema` selects scheme 4 (fast/A2-class app mode) or scheme 2 (anything else)
   [code: `sf` `0x562c08`, `0x562ec0`, `0x566134`, `0x565c78`, `0x565de0`].
8. **Pen-up refresh timer still runs** (500 ms default, debounced across strokes, rect = union of the
   strokes since the last firing plus the previous firing's rect), but on this device its only
   effects are draining the bookkeeping list and the menu hooks: the audio hook is a no-op for the
   panel, while the meeting hook, in the meeting scene with media open, arms a 5000 ms timer
   (`isTablet()` → 5000, else 0) that sends `ENABLE_POST(1)` and so ends the SF session
   [code: `sdk/pen/RawInputReader.java:200-205, 285-302, 329-341, 418-432`;
   `EpdShapeHandler.java:248-263, 365-374`;
   `app/note/menu/viewmodel/meeting/MeetingViewModel.java:82-93, 240-244, 263-268`;
   `app/note/action/render/EnablePostAction.java`].
9. **Colour.** The firmware stroke colour is the pen's raw ARGB when "colour strokes show original"
   is on, which defaults to `isColorDevice()` (true on a colour panel); otherwise non-neutral colours
   are sent as black with the pen's alpha [code: `sdk/scribble/shape/RenderContext.java:357-363`,
   `sdk/pen/PenUtils.java:56-61`, `OnyxSystemConfig.java:100-105`]. Whether SF's live ink appears in
   colour on the Kaleido panel is **[unknown]**.

---

## 2. Central classes and entry points

| Layer         | Class / symbol                                                                                     | Role in writing                                                                                                                   | Source                                                                                                             |
| ------------- | -------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| app           | `EpdShapeHandler`                                                                                  | The pen tool: receives raw callbacks, builds `Shape`, flushes to bitmap, schedules save                                           | `app/note/handler/common/EpdShapeHandler.java`                                                                     |
| app           | `ScribbleHandler`                                                                                  | Base of pen/erase handlers; `updatePenRawTouch`, fast-mode helpers, `onInvalidateViewWithPenControl`                              | `app/note/handler/scribble/ScribbleHandler.java`                                                                   |
| app           | `TouchEventHandler`                                                                                | EventBus subscriber that turns `NoteManager` events into `Handler` calls on the main thread, normalises points                    | `app/note/eventhandler/TouchEventHandler.java:135-328`                                                             |
| app           | `PenEventHandler`                                                                                  | Pause/resume predicate; builds `ResumePenBean` and submits `ResumeRawDrawingRequest`                                              | `app/note/eventhandler/PenEventHandler.java:105, 155-190, 271`                                                     |
| app           | `ResumeRawDrawingRequest`                                                                          | The only place pen parameters reach the firmware during normal writing                                                            | `app/note/request/pen/ResumeRawDrawingRequest.java:40-138`                                                         |
| app           | `EpdEventHandler`                                                                                  | Fast (A2 transient) mode; reacts to `RawPenDownEvent`/`RawPenUpEvent`                                                             | `app/note/eventhandler/EpdEventHandler.java:109-290`                                                               |
| app           | `GrayscaleRefreshAction`                                                                           | Per-stroke HW reconcile; disabled on this device                                                                                  | `app/note/action/render/GrayscaleRefreshAction.java:57-80`                                                         |
| app           | `InvalidateScreenAction` → `InvalidateViewWithPenControlAction`                                    | Pause pen → invalidate → resume pen (the cycle that actually hands the panel back)                                                | `app/note/action/render/InvalidateScreenAction.java:29-39, 83-87`; `InvalidateViewWithPenControlAction.java:31-52` |
| app           | `AddShapesBackgroundAction`                                                                        | Adds the shape to the document, undo stack, HWR; re-renders page slices                                                           | `app/note/action/shape/AddShapesBackgroundAction.java`                                                             |
| app           | `AutoSaveEventHandler`                                                                             | Timed auto-save after the pen leaves proximity                                                                                    | `app/note/eventhandler/AutoSaveEventHandler.java:72-106`                                                           |
| sdk (bundled) | `NoteManager`                                                                                      | Owns the `TouchHelper`; raw callback → EventBus; pen setters                                                                      | `sdk/notecore/editor/NoteManager.java:196-256, 449-508, 800-900`                                                   |
| sdk           | `TouchHelper` (mask 3 = SF + App renders)                                                          | Fan-out to `SFTouchRender` and `AppTouchRender`                                                                                   | `sdk/pen/TouchHelper.java:165-173, 276-308, 349-366`                                                               |
| sdk           | `SFTouchRender`                                                                                    | Wraps the raw reader; posts every callback to the host view (main thread); pen state via `EpdPenManager`                          | `sdk/pen/touch/SFTouchRender.java:27-223, 264-289, 368-374, 401-407`                                               |
| sdk           | `AppTouchRender`                                                                                   | MotionEvent-based renderer; with `touchListenerEnabled=false` it forwards no callbacks, but its render toggles emit `ENABLE_POST` | `sdk/pen/touch/AppTouchRender.java:37-78, 200-205, 253-274`                                                        |
| sdk           | `RawInputReader`                                                                                   | Java side of `libonyx_pen_touch_reader.so`: point state machine, pen-up refresh timer                                             | `sdk/pen/RawInputReader.java`                                                                                      |
| sdk           | `EpdPenManager`                                                                                    | `setScreenHandWritingPenState(view, 0..3)`                                                                                        | `sdk/pen/EpdPenManager.java:9-52`                                                                                  |
| sdk           | `InkRenderManager` / `RenderManager` / `EditorView`                                                | App-side display of the page bitmap                                                                                               | `sdk/notecore/editor/display/*.java`                                                                               |
| sdk           | `SDMDevice`                                                                                        | Reflection to `android.onyx.ViewUpdateHelper` (static methods; the `View` receiver is ignored)                                    | `sdk/device/SDMDevice.java:453-700, 1013, 2230-2294, 2337`                                                         |
| framework     | `ViewUpdateHelper`                                                                                 | Parcel + `transact` to the `SurfaceFlinger` service                                                                               | `fw/onyx/ViewUpdateHelper.java:602-608, 1195-1276`                                                                 |
| SF            | `SurfaceFlingerHelper::handle*`, `HWEpdcManager`, `HandwritingSchema`, `PenManager`, `TouchReader` | Pen session, live ink, schema/scheme switching                                                                                    | `sf` symbols, see §5.8                                                                                             |

---

## 3. Before the first stroke: what a resume pushes to the firmware

Writing only works while render and input are enabled. Stock enables them through one request,
submitted on the pen scheduler (a worker thread) by `PenEventHandler.q()`
[code: `PenEventHandler.java:155-190`]. The bean carries the limit/exclude rects, the active quick pen
(`getActiveQuickPen()`), the active provider, the graphics shape colour, `canBrushRawRender()` of the
active handler and a copy of the eraser args [code: `PenEventHandler.java:166-176`].

`ResumeRawDrawingRequest.execute()` [code: `ResumeRawDrawingRequest.java:121-138`]:

1. Returns if nothing is to be resumed, or render and input are already on (`f()`/`g()`, `:40-46`).
2. `ThreadUtils.mySleep(delay)` — 500 ms on a colour device, 150 ms on mono, unless the caller set
   another delay (`RawPenArgs.java:68-70`; popups 500 ms on colour, `app/note/event/PenEvent.java:8-21`).
3. `o()` (`:107-113`):
   - `p()` → `NoteManager.setDrawLimitRect` + `setDrawExcludeRect` (`:115-118`).
   - `setPenUpRefreshTimeMs(OnyxSystemConfig.getPenUpRefreshTimeMs())` — local to the Java reader only
     (`RawInputReader.setPenUpRefreshTimeMs`, `:640-642`); no transaction.
   - `setSingleRegionMode()` → `nativeSetRegionMode(1)` + `SET_SCREEN_HANDWRITING_REGION_MODE(1)`
     (`RawInputReader.java:652-656`).
   - `applyDrawingArgs(drawingArgs)` → `NoteManager.setStrokeColor(drawingArgs.getStrokeColor())`,
     which itself applies `getRenderContext().getRenderColor(color)` before handing it to
     `TouchHelper` (`NoteManager.java:196-202, 896-903`).
   - `n()` (`:84-105`), for the EPD provider:
     - `NoteManager.applyPenArgs(pen)` (`NoteManager.java:205-219`):
       `setStrokeStyle(ShapeFactory.getStrokeStyle(type, attrs))`,
       `setStrokeWidth(ShapeUtils.getRenderStrokeWidth(width, type, renderContext))`,
       `setStrokeColor(renderContext.getRenderColor(pen.color))`.
     - `m(pen)` (`:71-82`): per-type stroke parameters (table in §6.2).
     - `i()` (`:52-55`): `setStrokeParameters(5, [5.0])` (dash style) and
       `setStrokeParameters(8, [eraserWidth, 0.5, 0.1])` (soft-eraser style).
     - `k()` (`:61-65`): `setEraserRawDrawingEnabled(isEraseTrackByRawRender(eraserType) || provider==SELECTION, isMoveStrokeErase(eraserType) ? 8 : 5)`
       (`sdk/scribble/utils/EraseUtils.java:22-28`: types 0/1/2 render a trace; 0/1 use style 8).
     - `h()` (`:48-50`): `setBrushRawDrawingEnabled(canBrushRawRender)` — `true` for `EpdShapeHandler`.
4. Input first: `setRawInputReaderEnable(true)` (`:130-132`) → `RawInputReader.resume()` =
   `nativeSetPenState(4)` + report flag on (`RawInputReader.java:597-602`); App render sets a flag.
5. Render last: `setRawDrawingRenderEnabled(true)` (`:133-135`) → `SFTouchRender.k()` →
   `EpdPenManager.resumeDrawing()` → `SET_SCREEN_HANDWRITING_PEN_STATE(2, pid)`
   (`SFTouchRender.java:283-285, 368-374`; `EpdPenManager.java:37-39`). `AppTouchRender` only sets
   `c=true` (`AppTouchRender.java:268-274`).

Because `TouchHelper` holds both renders (SF first, App second, `TouchHelper.java:349-366`), each
style/width/colour setter goes out **twice**: once via `SFTouchRender` and once via `AppTouchRender`
[code: `TouchHelper.java:318-340`; `SFTouchRender.java:440-453`; `AppTouchRender.java:329-342`]. The
same holds for the limit rect list, and in a different way for the exclude list: `SFTouchRender`
→ `RawInputReader.setLimitRect(List)` / `setExcludeRect` send nothing for an empty list, but
`AppTouchRender` → `AppTouchInputReader.setLimitRectList` / `setExcludeRectList` send the
transaction for any non-null list, **including an empty one** (`sdk/pen/touch/AppTouchInputReader.java:239-254`).
`PenEventHandler.q()` always passes a (possibly empty) copy of the exclude list
(`PenEventHandler.java:169`), so every resume sends at least one `SET_SCREEN_HANDWRITING_REGION_EXCLUDE`
[code]; what SF does with an empty exclude array (presumably clears the exclude set) is **[inferred]**.
Brush/eraser raw-drawing flags and region mode go through `SFTouchRender` only (the App versions are
empty), and `SET_STROKE_PARAMETERS` is issued directly via `Device.currentDevice()`, once per call
[code: `AppTouchRender.java:263-278, 324-326`; `ResumeRawDrawingRequest.java:52-55`;
`NoteRenderUtils.java:64-137`].

### Wire trace of one resume (EPD provider, both flags off before)

| #   | transaction (code)                                                             | payload                                                            | emitted by                                                                                        |
| --- | ------------------------------------------------------------------------------ | ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------- |
| 1   | `SET_SCREEN_HANDWRITING_REGION_LIMIT` (16711694) ×2 (SF leg only if non-empty) | view!=null, rects offset to screen in `ViewUpdateHelper`           | `RawInputReader.setLimitRect(List)` `:678-685`; `AppTouchInputReader.setLimitRectList` `:251-254` |
| 2   | `SET_SCREEN_HANDWRITING_REGION_EXCLUDE` (16711714) ×1-2                        | SF leg only if non-empty; App leg always (empty array if no rects) | `RawInputReader.setExcludeRect` `:604-611`; `AppTouchInputReader.setExcludeRectList` `:239-245`   |
| 3   | `SET_SCREEN_HANDWRITING_REGION_MODE` (1048620)                                 | 1                                                                  | `setSingleRegionMode` (SF leg only)                                                               |
| 4   | `SET_STROKE_COLOR` (16711686) ×2                                               | `getRenderColor(drawingArgs.strokeColor)`                          | `applyDrawingArgs`                                                                                |
| 5   | `SET_STROKE_STYLE` (16711688) ×2                                               | §6.2                                                               | `applyPenArgs`                                                                                    |
| 6   | `SET_STROKE_WIDTH` (16711687) ×2                                               | px, zoom-scaled                                                    | `applyPenArgs`                                                                                    |
| 7   | `SET_STROKE_COLOR` ×2                                                          | render colour of the pen                                           | `applyPenArgs`                                                                                    |
| 8   | `SET_STROKE_PARAMETERS` (1049089)                                              | 0-2 calls, per type, each preceded by a `getStrokeParameters` read | `m()`                                                                                             |
| 9   | `SET_STROKE_PARAMETERS`, two calls                                             | (5,[5]), (8,[w,0.5,0.1])                                           | `i()`                                                                                             |
| 10  | `SET_ERASER_RAW_DRAWING_ENABLED` (1048833) ×1                                  | (bool, 8 or 5)                                                     | `k()`                                                                                             |
| 11  | `SET_BRUSH_RAW_DRAWING_ENABLED` (1048834) ×1                                   | 1                                                                  | `h()`                                                                                             |
| 12  | `SET_SCREEN_HANDWRITING_PEN_STATE` (16711693)                                  | 2, pid                                                             | render resume                                                                                     |

[code for the Java/transaction side; the order of 4-11 follows `o()`/`n()`. The input resume between
11 and 12 (`RawInputReader.resume()` = `nativeSetPenState(4)`) is in-process only.] What SF does with 4-11
beyond storing them in its painter/pen objects is **[unknown]** at this level of detail
(`SurfaceFlingerHelper::handleSetStroke*`, `MemoryPainter::setStroke*` exist in `sf`; colour is passed
through to every painter without conversion in `MemoryPainter::setStrokeColor` @ `0x52541c` [code]).

---

## 4. Threading of a stroke

1. **`raw_input` thread** (single-thread executor, `RawInputReader.java:115-120, 290-292`) runs the
   native loop `nativeRawReader()`, which calls back
   `onTouchPointReceived(x, y, pressure, tiltX, tiltY, erasing, shortcutDraw, shortcutErase, state, ts)`
   [code: `:538-571`; parameter meaning inferred from use: `TouchPoint(x,y,pressure,size=0,tiltX,tiltY,ts)`,
   `sdk/data/note/TouchPoint.java:362-367`].
2. Each callback reaches `RawInputManager.a` (drops everything if `isUseRawInput()` is false,
   `sdk/pen/RawInputManager.java:16-89`) and then `SFTouchRender.b`, which **posts a `Runnable` to the
   host view** for every event (`SFTouchRender.java:90-218`). From here everything runs on the **main
   thread**, in the order the reader produced it.
3. `NoteManager`'s callback posts an EventBus event per callback (`NoteManager.java:449-508`);
   `TouchEventHandler` subscribers (`ThreadMode.MAIN`, so delivered synchronously when posted from main)
   normalise the point(s) to page coordinates when the handler asks for it (`EpdShapeHandler.normalizePoint()`
   is true) and call the active `Handler` (`TouchEventHandler.java:73-91, 135-328`).
4. Stroke persistence (`AddShapesBackgroundAction`) and HWR run on the note worker scheduler; the
   `ResumeRawDrawingRequest` runs on the pen scheduler [code: `AddShapesBackgroundAction.create()`,
   `PenEventHandler.java:177`].

`AppTouchRender` is created too (mask 3), but the stock app passes `touchListenerEnabled=false`
(`NoteManager.java:232`), so `AppTouchRender.e == false`: it installs no `OnTouchListener` and its
callback wrapper forwards **nothing** to the app (`AppTouchRender.java:37-78, 187-198`). It would only
draw via `EpdController.moveTo/quadTo(DU)` if `TouchHelper.onTouchEvent` were fed MotionEvents, which
happens only when the active handler `supportFingerDraw()` (false for `EpdShapeHandler`, default in
`BaseHandler`) [code: `app/note/touch/ScribbleTouchDistributor.java` `onTouchEvent`,
`sdk/notecore/editor/handler/BaseHandler.java:300`].

---

## 5. The mechanism, step by step

### 5.1 The reader's state machine (bundled `RawInputReader`)

`onTouchPointReceived(..., state i4, ...)` [code: `RawInputReader.java:538-571`]:

| state | meaning (inferred)                          | handling                                                                                                                                                                                                                                                  |
| ----- | ------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 5     | hover                                       | `H()` → `PenActiveEvent` on the touch EventBus (if `setPostInputEvent(true)`), re-armed 100 ms `PenDeactivateEvent` timer — both regardless of input enable; `onPenActive` callback only while input is enabled (`q()`, `:418-423`) (`:193-198, 214-223`) |
| 0     | down                                        | only if reporting (`z()`, i.e. input enabled): reset list, cancel pen-up timer, grow dirty rect, `onBeginRawDrawing(shortcutDraw, p)` (`:225-237, 434-445`)                                                                                               |
| 1     | move                                        | if down was reported: append, cancel pen-up timer, grow rect, `onRawDrawingTouchPointMoveReceived(p)`; if not, synthesise a down (`PenDownPointLostEvent`) (`:179-191, 239-247`)                                                                          |
| 2 / 3 | up (3 = up with "out of limit region" flag) | `onRawDrawingTouchPointListReceived(list)` **then** `onEndRawDrawing(outLimit, p)`; then (drawing only) start the pen-up timer (`:249-265`)                                                                                                               |
| 6     | forced release                              | same as up (`:564-569`)                                                                                                                                                                                                                                   |

Details that matter:

- Nothing is reported unless input is enabled (`f4564d`, set by `resume()`, cleared by `pause()`)
  except state 6 (`:542`).
- Repeated slow moves are dropped (`l()`, speed < 0.005 px/ms and |Δpressure| ≤ 2, `:353-363`);
  `setFilterRepeatMovePoint` is never called by stock (grep), so the filter is on.
- Coordinates: raw → screen through `EpdController.getRawTouchPointToScreenMatrix()`, then minus the
  host view's `getLocationOnScreen` (`:159-177, 304-306, 406-416`).
- The point list for the up callback is allocated with capacity 600 and handed over whole (`:313-325`).

### 5.2 Pen down

`EpdShapeHandler.onBeginRawDraw(boxInfo, shortcutDrawing, point)` [code: `EpdShapeHandler.java:318-340`]:

1. `UpdatePenHapticTypeAction(false)` — filtered out unless `DeviceConfig.supportPenHapticType`, which
   defaults to false and is not set in `res/raw/note_noteair4c.json` [code:
   `app/note/action/pen/UpdatePenHapticTypeAction.java`, `sdk/note/ui/config/DeviceConfig.java:59, 87`].
2. `postDocEvent(RawPenDownEvent)` → `EpdEventHandler.onRawPenDownEvent` cancels the fast-mode exit
   timer and the wake lock (`EpdEventHandler.java:118-128, 279-282`).
3. `updatePenRawTouch(true)` → `TouchStateInfo.setPenRawTouch(true)` (`ScribbleHandler.java:1293-1295`).
4. Zoom-preview popup hook, HWR bookkeeping (`setRecBoxInfo`, reset point lists), remember page id,
   `y0()` logs "last raw input not finished" if the previous up never arrived, meeting-menu hook.
5. `A0(boxInfo)`: a fresh `RenderContext` with the box's render matrix and the colour config of §6.3
   (`:118-124`).
6. `ShapeUtils.createShape(loadPenDrawingArgs(drawingArgs, activeQuickPen), renderContext)` → a free
   shape of the current shape type (`sdk/scribble/utils/ShapeUtils.java:823-829`).
7. `O0(toDownTouchData(point))` → `shape.onLowLatencyTouchData(...)` → for pencil/charcoal types
   `NeoPenRender.onTouchData` (pure computation of the pen geometry; no canvas, no invalidate)
   [code: `EpdShapeHandler.java:239-246`; `sdk/scribble/shape/pen/PencilScribbleShape.java:317-327`;
   `sdk/pen/NeoPenRender.java:151-167`; `BaseShape.onLowLatencyTouchData` returns true and does nothing,
   `sdk/scribble/shape/BaseShape.java:791-795`].

No EPD call, no invalidate. [code]

In parallel, and independent of the app, SF's own `TouchReader`/`PenManager` see the same pen-down
and, if the point is in a valid region and `canScribble()` holds, `PenManager::moveTo` →
`handlePenTriggerImpl` → `EpdcManager::startHandwriting` (only when `penTriggered` is 0) → schema 7 →
`HandwritingSchema::onActivate` → `afterStartHandwriting` [code: `sf` `0x52e840` (`bl 0x562f24`),
`0x562f24-0x562f80`, `0x55b814`; see §5.6]. The only other direct caller of
`EpdcManager::startHandwriting` is a lambda of `SurfaceFlingerHelper::handleMoveTo` (`bl` at
`0x564a6c`), i.e. the `MOVE_TO` transaction used by `EpdController.moveTo` (the `AppTouchRender`
path, not fed in stock) [code: `sf` call-site scan].

### 5.3 Moves

`EpdShapeHandler.onRawDrawingPointsMoveReceived(point)` [code: `:377-383`]: `updatePenRawTouch(true)`,
append to the HWR point list, `O0(toMoveTouchData(point))`. No EPD call, no invalidate.

The stylus MotionEvents that Android also delivers go through `ScribbleTouchDistributor` to
`TouchEventHandler.onTouchDown/Move/Up` → `EpdShapeHandler.onTouchDown/Move/Up`, which only feed the
HWR touch list and log "raw point is not report" if a stylus up arrives with no raw down while render
is on (`EpdShapeHandler.java:452-485`); pen gestures go to `RxPenGesture` [code:
`ScribbleTouchDistributor.onTouchEvent`]. None of this draws.

### 5.4 Pen up

The reader emits the point list first, then the end event (§5.1).

`EpdShapeHandler.onRawDrawingPointsReceived(pointList)` [code: `:386-450`]:

1. `updatePenRawTouch(false)`, `RawPenUpEvent` (→ `EpdEventHandler.onRawPenUpEvent`: if fast mode is on,
   `k()` resets the GC counter and arms the 5 s fast-mode exit, and `f3199d=true` so that the exit also
   runs a pause/resume cycle, `EpdEventHandler.java:185-190, 285-290, 145-155`), zoom-preview hook.
2. Empty list → return (the firmware ink, if any, stays; nothing is saved).
3. Feed the up point to the low-latency model.
4. **Early returns that leave firmware ink on screen without a shape** [code]:
   - no `NOTE_SCRIBBLE` permission → toast, return (`:401-404`);
   - HWR "hold to convert" in progress for this time stamp → mark ignored, return (`:405-416`);
   - float button or float-button menu active → clear flag, return (`:417-421`).
     How that stray ink is later cleared is **[unknown]** here; the float-button path arms a full-region
     repaint (`DeviceReceiverEventHandler.java:89`) that only matters when `P0()` is enabled.
5. Pick the box (page) the stroke belongs to, `shape.addPoints(pointList)`, `shape.updatePoints()`,
   `F0(shape, box)`.

`F0(shape, box)` [code: `:179-207`]:

1. Unless the page changed between down and up (`z0()`), build a `FlushShapeReference`.
2. Deep-copy the shape, append a new `ShapeGrayscaleBean` to `this.m`.
3. For every cross-page piece and for the main box: `RenderManager.flushShapeToPage(ShapeFlushInfo)`
   → every page render handler draws the shape **synchronously on the main thread** into its cached
   slice bitmap with `shape.lowLatencyRender(renderContext)`
   (`sdk/notecore/editor/display/RenderManager.java:777-796`;
   `PageNormalRenderHandler.java:441-466, 661-672` and lambda `c` `:110-126`).
4. `I0().invalidate()` → `InkRenderManager.invalidate()` → `invalidate(new DisplayArgs())`
   (`InkRenderManager.java:139-141, 79-96`). Defaults: `enablePost=false`, `invalidate=true`,
   `enableRegalModeOnce=false` (`DisplayArgs.java:19-20`), and the one-shot view mode is `None` unless a
   page action armed REGAL for once (`InkRenderManager.java:32-33, 49-51, 98-119`). So the call is a
   bare `View.invalidate()` on `EditorView`, whose `onDraw` blits the page cache
   (`EditorView.java:65-76`). **No update mode, no transaction.**
5. `AddShapesBackgroundAction(...).build().map { bean.setRenderToBitmap(true); P0() }.subscribe()`
   (`:200-217`).
6. `C0(shapeCopy)`: a 10 s debounced emitter (`WorkRequest.MIN_BACKOFF_MILLIS`) that, 10 s after the
   last stroke, disposes itself and starts background handwriting recognition if the setting is on
   (`:140-177, 233-237`).

`EpdShapeHandler.onEndRawDrawing(outLimitRegion, point)` [code: `:352-357`]: haptic reset (no-op here)
and `j = true`. Nothing visual.

`AddShapesBackgroundAction` [code: `app/note/action/shape/AddShapesBackgroundAction.java`]: per box, on
the note worker, `AddShapesBackgroundRequest` (adds to the page model; `addToCmdStack=false`), logging,
then `forceRenderAddShapeDirtyRect` over the stroke's bounding page rect with `invalidate=true`
(`app/common/extension/ForceRenderSliceListActionsKt.java:92-105`; field meaning from
`sdk/notecore/editor/data/ForceRenderNoteDirtyArgs.java` getter names); afterwards the shapes go to the
command manager (undo), an HWR "Gesture" context-less stroke event is posted and the zoom thumbnail is
cleared. When the stroke's page is not the current page, the page's slices are invalidated
(`:B()/C()`). The second invalidate reaches `EditorView` the same way as step 4 (plain): at the end of
`ForceRenderNoteDirtyAction`, if the slice bitmaps were updated and the args' invalidate flag is set,
it calls `renderManager.invalidate()` = `InkRenderManager.invalidate(new DisplayArgs())`
[code: `sdk/notecore/editor/action/ForceRenderNoteDirtyAction.java:134-136`; the thread it runs on
was not traced].

### 5.5 The pen-up refresh timer

After a drawing up, `RawInputReader.I()` starts a one-shot timer of `penUpRefreshTimeMs` (default 500)
unless disabled (`RawInputReader.java:200-205`; `n = 500` at `:65`; `isPenUpRefreshEnabled()` default
true, `:66`, never changed by stock). Any new down or move cancels it (`S()` in `L()`/`G()`,
`:185-188, 230-233`), so it fires only after 500 ms without new pen contact. On firing, `r(i())`
[code: `:104-113, 329-341, 426-432`]:

- `i()` returns the dirty rect accumulated since the last firing (every down/move/up point of every
  stroke in between), expanded by the stroke width, **unioned with the rect of the previous firing**;
  it then stores the current rect as "previous" and clears the accumulator.
- `r()` delivers `onPenUpRefresh(rect)` only if input reporting is still on (`z()`).

Delivery: → `SFTouchRender.b.onPenUpRefresh` (posted to main) → `NoteManager` → `PenUpRefreshEvent`
→ `TouchEventHandler.penUpRefreshEvent` → `EpdShapeHandler.onPenUpRefresh(rect)` [code: `:365-374`]:

1. `ScribbleMenuModel.onPenUpRefresh()` → meeting and audio-record view models only
   (`app/note/menu/viewmodel/scribble/ScribbleMenuModel.java:435-438`) [code]:
   - `MeetingViewModel.onPenUpRefresh()` (`:263-268`): if the active scene is the meeting scene
     (`getActiveScene().getB() == 2`, `:111-113`) and the audio media is not closed, it starts
     `RxTimerUtil.timer(SystemPropertiesUtil.isTablet() ? 5000 : 0, n)`; the observer `n`
     (`:82-93`) runs `EnablePostAction`, i.e. `EpdController.enablePost(1)` with no view →
     `SDMDevice.enablePost(int)` → `ENABLE_POST(1)` (`EnablePostAction.java`,
     `sdk/device/SDMDevice.java:2487-2492`). The next pen-down cancels the timer
     (`EpdShapeHandler.onBeginRawDraw` → `getMenuModel().onBeginDrawing()` →
     `MeetingViewModel.onBeginDrawing`, `:240-244`). On the Note Air 4C (`isTablet()` true) this is a
     hand-back 5 s after the last pen-up refresh, **in the meeting scene only**.
   - `AudioMenuModel.onPenUpRefresh()` (`app/note/menu/viewmodel/scribble/AudioMenuModel.java:422-427`)
     calls `repaint.notifyChange()`; the `repaintView` binding adapter would issue
     `EpdController.handwritingRepaint(view, rect)` (`app/common/utils/DataBindingUtils.java:69-74`),
     but no generated binding class references `repaint` or `repaintView` (grep of
     `app/databinding/`), so this is a no-op for the panel [code for the absence].
2. The **last** bean in `this.m` gets `grayscaleRect = rect`; `P0()`.

`P0()` [code: `:248-263`] takes all beans that are both rendered to bitmap and have a non-empty rect
(`B0()`, `:126-138`, which also **removes** them from the list), and only if
`isUseHWUpdateForPenUpRefresh()` runs `GrayscaleRefreshAction` on their union (or on the whole
`scribbleRect` once after `setEnabledPenDirtyRect(false)`). On the Note Air 4C that predicate is false
(§7), so `P0()` only drains the list. `P0()` is called from both ends (pen-up refresh and bitmap
completion); whichever comes last supplies the missing half. Only the **last** bean is ever stamped
with a rect, so a bean belonging to a stroke that was followed by another stroke within 500 ms never
gets a rect and is never taken by `B0()` [code: `:126-138, 368-373`]; it stays in `this.m` for the
life of the handler [inferred]. With the reconcile enabled this would not lose area, because the
stamped rect of the next firing already covers every stroke since the previous firing (§5.5); on this
device it has no visible effect.

What `GrayscaleRefreshAction` would do (not on this device) [code:
`GrayscaleRefreshAction.java:57-80`]: `View.setDefaultUpdateMode(HAND_WRITING_REPAINT_MODE = 524290 =
UI_GU_MODE | 0x80000)` on the editor view, `displayNoteDirtyRect(...)` (a view invalidate on the main
thread), `View.resetUpdateMode()` (back to 5) in `doFinally` (`fw/view/View.java:13718-13720,
14577-14579`). That flag is what `HandwritingSchema::output` looks for (§5.8).

### 5.6 What the firmware draws, and when app content reappears

Live ink is SF's. SF has its own pen reader (`android::TouchReader` in `sf`, started/resumed by the pen
state transactions: state 1 → `TouchReader::start`, 2 → `ensureStart`+`resume`, 3 → `pause` + tail call
`EpdcManager::stopHandwriting`, 0 → `stop`) and its own stroke engine (`neo::pen::NeoBrushPen`,
`NeoFountainPen`, … symbols in `sf`) [code: `docs/onyx-reversing/08-surfaceflinger.md` §3, re-checked:
`HWEpdcManager::setHandWritingPenState` is vtable slot +0xe8 at `0xa355b8`, and `stopHandwriting` is
tail-called from `0x5667f4`]. Re-checked from its jump table (bytes at `0x34152a`, base `0x5666f4`)
[code: `sf` `HWEpdcManager::setHandWritingPenState` @ `0x5666bc`]: state 0 →
`FBDev::enableExtBufAutoSync(0)`, `TouchReader::stop`, `enableCytpLoFilter(0)`; state 1 →
`enableExtBufAutoSync(flag@+0x25d)`, `TouchReader::start`, `enableCytpLoFilter(1)`; state 2 →
`TouchReader::ensureStart`, `TouchReader::resume`, and if the byte at +0x25d is set a tail call to
`FBDev::syncDataToExtendedBuffer`; state 3 → `TouchReader::pause` then tail call
`EpdcManager::stopHandwriting` (unconditional); states > 3 return. Only state 3 ends the handwriting
schema.

Entering the handwriting schema on pen-down [code: `sf`]:

- `PenManager::moveTo` calls `handlePenTriggerImpl` (`0x52e840`); it returns immediately if
  `penTriggered` (`0xaca244`) is already set, else logs `HandlePenTrigger, sid: %d value is: %d pid %d`
  and tail-calls `EpdcManager::startHandwriting` (`0x562f24-0x562f80`), which logs
  `### startHandwriting` and activates schema 7.
- `HandwritingSchema::onActivate` tail-calls `afterStartHandwriting` (`0x55b814`), which sets
  `penTriggered=1`, `androidDrawing=0`, `addToWatchApp(pid)` and calls vtable +0x78 =
  `HWEpdcManager::setEbcUpdateScheme(3, 0)` (`0x562c18-0x562c6c`), which logs `### EUS, s 3 c 0`
  (`0x565e10`, format `### EUS, s %d c %d`) and programs the scheme via `FBDev::setEbcUpdateScheme`.

While schema 7 is active, `HandwritingSchema::output` (`0x55b828-0x55ba48`) [code, disassembly]:

1. `collectLayerEpdcList` gathers the app layers' pending updates.
2. If the byte at `0xac9e60` (the "HW repaint pending" flag in `08-surfaceflinger.md`) is 0 **and**
   `inFastMode()` (vtable +0x28 = `HWEpdcManager::inFastMode`) is true, the layer updates are dropped
   (debug log `### warning: sys/app is in fast mode, ignoring refresh.`, string at `0x801ce`).
3. Else, if `UpdateEntryDetector::hasHWRepaint(list)` is true, the layer updates are committed
   (`EpdcManager::enableExtBufferSyncFB(1)` + `BaseEpdcSchema::commitEpdc`).
4. Else it takes the next queued SF update entry (`EpdcManager::next` @ `0x55908c` →
   `UpdateEntryDetector::add/output`); if the detector's output entry has a field (+8) outside
   {-1, 0} it builds an EPDC update from it (`BaseEpdcSchema::createEpdcByUpdateEntry`) and commits it
   together with the collected layer list (debug log `### warning: commit epdc in handwriting mode.`,
   string at `0xf8764`), otherwise nothing is committed (debug log `### disable refresh.`, string at
   `0xdee2d`). Both debug logs are gated by the byte at `0xac9ea0`.

[code] `hasHWRepaint` (`0x556b48`) walks the collected `hwc_epdc_llist` vector (stride 0x14) and
returns true only if the 32-bit word at offset +0x10 of some entry equals `0x80002` exactly
(`HAND_WRITING_REPAINT_MODE` = `UI_GU_MODE | 0x80000`), not merely if the 0x80000 bit is set.
[inferred] That word is the layer's requested update mode, fed by the view default update mode
(`GrayscaleRefreshAction`) or by a `HANDWRITING_REPAINT` transaction. Stock's per-stroke invalidates
carry neither on this device. What enqueues entries for `EpdcManager::next` was not traced
**[unknown]**; if a plain invalidate never produces such an entry, **the re-rendered stroke does not
reach the panel while schema 7 is active and the firmware ink stays in its place [inferred; see
experiments 3 and 12]**.

Leaving schema 7 [code: `sf`]: the only direct callers of `EpdcManager::stopHandwriting` (`0x558630`)
are `handleEnablePost` (`0x560c30`), `handleToggleDream` (`0x5621fc`), `showImage` (`0x5630f0`),
`HWEpdcManager::setHandWritingPenState(3)` (`0x5667f4`) and the unused-here
`SWEpdcManager::setHandWritingPenState` (`0x56787c`) (scan for `bl`/`b 0x558630`; an indirect call
through a function pointer would not show up, **[unknown]** but no vtable slot for it is known). `handleEnablePost(value)` receives the **second** parcel int (`0x55fef4-0x55ff14`: the
leading -1 and the trailing pid are read and discarded) and calls `stopHandwriting` only if
`value >= 0` and `value != androidDrawing` (`0x560c18-0x560c30`). `stopHandwriting` →
`HandwritingSchema::onDeactivate` → `afterStopHandwriting` (`0x55b824`): logs `Post enabled!`,
`penTriggered=0`, `androidDrawing=1`, then vtable +0x70 = `HWEpdcManager::clearHandwritingFBSchema`
(call made inside `afterStopHandwriting`, `0x562ec0-0x562f20`; the callee is at `0x566134`), which picks scheme **4** if the app's current EPD mode is one of
{1, 4, 2305, 2308, 2312, 0x1000001, 0x1000004, 0x2000004} (DU/A2-class) and scheme **2** otherwise,
calls `setEbcUpdateScheme(scheme, 0)`, toggles enhance and `signalRefresh()` (`0x566134-…`, same
decision table as `setFBSchema` `0x565cfc-0x565d78`). **Nothing in `sf` calls `stopHandwriting` at
pen-up** (the only caller of `TouchReader::reportUpEvent` is the `handlePenUp` lambda, `0x564e80`, and
`handlePenUp` is reached only through the `PEN_UP` transaction that the pen path does not send).

So on the Note Air 4C the hand-back from firmware ink to app content happens only at the next:

| trigger in stock                                                                                                   | why it ends the SF session                                                                                                                                                            | source                                                                                                                                                                              |
| ------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `InvalidateScreenAction` (menus, popups, focus, float button, tool switch, page GC, refresh)                       | pause render+input → `SFTouchRender.h()` = `ENABLE_POST(1)` + pen state 3; `AppTouchRender` `ENABLE_POST(1)`; `renderManager.invalidate()`; resume after 500 ms                       | `InvalidateScreenAction.java:29-39, 83-87`; `InvalidateViewWithPenControlAction.java:31-52`; `SFTouchRender.java:270-273`; `AppTouchRender.java:268-274`                            |
| `PauseResumeRawPenAction` (e.g. fast-mode exit 5 s after a pen-up while fast mode was on; no-focus system dialogs) | same pause legs                                                                                                                                                                       | `EpdEventHandler.java:145-155, 181-183`; `PenEventHandler` `onNoFocusSystemDialogChangeEvent`                                                                                       |
| Erase actions                                                                                                      | `DisplayArgs.setEnablePost(true)` → `InkRenderManager.enablePost()` = `ENABLE_POST(1)`                                                                                                | `app/note/action/erase/ErasingShapeAction.java:125`, `EraseFinishAction.java:354`; `InkRenderManager.java:56-61, 88-90`                                                             |
| Any keyboard/popup/dialog pause                                                                                    | pen state 3                                                                                                                                                                           | `PenEventHandler` (session subsystem)                                                                                                                                               |
| System "refresh screen" broadcast `onyx.android.intent.action.REFRESH_SCREEN`                                      | `KNoteCommonReceiver.i()` posts `FullScreenRefreshEvent` on the global bus → `EpdShapeHandler.onFullScreenRefreshEvent` → `InvalidateScreenAction` (pause/invalidate/resume as row 1) | `app/common/receiver/KNoteCommonReceiver.java:62-64` and its action switch; `EpdShapeHandler.java:359-362`; delivery of the global event to the handler's subscriber **[inferred]** |
| Meeting scene: 5 s after the last pen-up refresh (tablet)                                                          | `EnablePostAction` → `ENABLE_POST(1)` without pen state change                                                                                                                        | `MeetingViewModel.java:82-93, 263-268` (§5.5)                                                                                                                                       |
| Zoom finish viewport translate                                                                                     | `TranslateViewPortRequest.execute` pauses render + input (`ENABLE_POST(1)` ×2, pen state 3) and enters fast mode                                                                      | `app/note/request/zoom/TranslateViewPortRequest.java:178-186`; only caller `ZoomFinishAction.java:76`                                                                               |
| Cloud sync pull that reloads the open page                                                                         | `SavePullShapeListRequest.f()` pauses render + input                                                                                                                                  | `app/note/request/ksync/SavePullShapeListRequest.java:111-117`                                                                                                                      |
| Re-opening the document from another scene                                                                         | `RestartRawDrawingAction` → `TouchHelper.restartRawDrawing()` = close (pen state 3 then 0, `ENABLE_POST(1)` ×2) + open (pen state 1 ×2, `ENABLE_POST(0)`)                             | `app/note/action/hwr/RestartRawDrawingAction.java:46-51`; `ScribbleMainFragment.java:473`; `SFTouchRender.java:309-314, 348-352`; `AppTouchRender.java:200-205, 252-257`            |

Between two such events, every stroke stays as firmware ink on the panel **[inferred from the above;
to be confirmed with the driver log, §10]**.

### 5.7 Fast mode during writing

`EpdEventHandler` turns on an A2-quality transient update ("fast mode") for scroll, zoom, viewport
moves, selection transforms, float-menu drags and similar (`ApplyFastModeEvent(true)` call sites, e.g.
`app/note/eventhandler/ViewportEventHandler.java:82, 116`, `app/note/request/zoom/TranslateViewPortRequest.java:185`):
`setDitherThreshold(160 on colour)`, `setEpdTurbo(deviceConfig.epdTurbo)` (= 5 from
`res/raw/note_noteair4c.json`), `applyTransientUpdate(transformUpdateModel ?: ANIMATION_QUALITY)`
(`EpdEventHandler.java:79-82, 109-116`). While it is on:

- pen-down cancels the exit timer (`:279-282`);
- pen-up (if the active handler does not `renderInFastMode()`, which `EpdShapeHandler` does not)
  resets the GC counter and arms a 5 s exit with a wake lock, and marks `f3199d` (`:185-190, 285-290`);
- the exit (`e()`, `:145-155`) clears the transient update (`d()`: `clearTransientUpdate`, dither
  128, `:136-142`) and, because a pen-up happened, runs `PauseResumeRawPenAction` — a pen-state 3/2
  cycle, i.e. a hand-back of the panel. If at that moment `touchStateInfo.getG()` (= "pen raw touch",
  set true by `updatePenRawTouch(true)` on raw down/move and false on raw up,
  `sdk/notecore/editor/data/TouchStateInfo.java:105, 193-195`) is true, `e()` returns **before doing
  anything**: neither the transient update is cleared nor the pause/resume runs, and the timer is not
  re-armed; fast mode then stays on until the next pen-up re-arms it (`onRawPenUpEvent` → `k()`) or
  another fast-mode event arrives [code: `EpdEventHandler.java:118-129, 145-155, 185-190, 285-290`].

Also `HWEpdcManager::inFastMode()` gates `HandwritingSchema::output` step 2 above; whether the app's
`applyTransientUpdate` makes SF's `inFastMode()` true is **[unknown]**.

---

## 6. Tables

### 6.1 Raw callbacks during a stroke (EPD provider, render and input on)

| #   | reader event             | callback                                         | `EpdShapeHandler` work                                                                                            | EPD / SF effect from the app                                                                |
| --- | ------------------------ | ------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| 1   | hover (5)                | `onPenActive`; `PenActiveEvent` on the touch bus | — (`AutoSaveEventHandler` cancels its timer, `ScribbleTouchDistributor` marks pen active)                         | none                                                                                        |
| 2   | down (0)                 | `onBeginRawDrawing`                              | `onBeginRawDraw`: shape + render context, HWR, `RawPenDownEvent`                                                  | none                                                                                        |
| 3   | move (1) ×N              | `onRawDrawingTouchPointMoveReceived`             | `onRawDrawingPointsMoveReceived`: low-latency model                                                               | none                                                                                        |
| 4   | up (2/3)                 | `onRawDrawingTouchPointListReceived`             | `onRawDrawingPointsReceived` → `F0`: bitmap flush, `View.invalidate()`, background add + re-render (+ invalidate) | none (plain invalidates)                                                                    |
| 5   | (same up)                | `onEndRawDrawing`                                | haptic reset, flag                                                                                                | none                                                                                        |
| 6   | +500 ms idle             | `onPenUpRefresh(rect)`                           | meeting/audio hooks, stamp last bean, `P0()` (no-op here)                                                         | none in the note scene; in the meeting scene (media open) a 5 s timer then `ENABLE_POST(1)` |
| 7   | +100 ms after last hover | `PenDeactivateEvent`                             | `AutoSaveEventHandler` arms `autoSaveTime` timer                                                                  | none                                                                                        |

SF effects produced by the pen itself (not by app calls): first down after a resume → schema 7,
`### startHandwriting`, `### EUS, s 3 c 0`; live ink updates **[code for the scheme; waveform of the
ink updates [unknown]]**.

### 6.2 Brush type → SDK stroke style, SF parameters and app shape

Shape types from `sdk/scribble/shape/ShapeFactory.java:89-120`; style from `getStrokeStyle`
(`:616-630`); style names from `sdk/pen/TouchHelper.java:23-30`; app shape from `createShape`
(`:632-700`); per-type parameters from `ResumeRawDrawingRequest.m()` (`:71-82`) and
`sdk/scribble/utils/NoteRenderUtils.java:64-137`.

| shape type | constant                               | firmware style (`SET_STROKE_STYLE`)                  | extra `SET_STROKE_PARAMETERS` on resume                                              | app shape class            |
| ---------- | -------------------------------------- | ---------------------------------------------------- | ------------------------------------------------------------------------------------ | -------------------------- |
| 2          | `SHAPE_PENCIL_SCRIBBLE`                | 0 PENCIL                                             | —                                                                                    | `NormalPencilShape`        |
| 3          | `SHAPE_OILY_PEN_SCRIBBLE`              | 0 PENCIL                                             | —                                                                                    | `NormalPencilShape`        |
| 4          | `SHAPE_FOUNTAIN_PEN_SCRIBBLE`          | 0 PENCIL                                             | —                                                                                    | `NormalPencilShape`        |
| 5          | `SHAPE_BRUSH_SCRIBBLE`                 | 1 FOUNTAIN                                           | style 1: [0] = pressure sensitivity (default 0.3), [1] = smooth level (clamped 0..1) | `FountainScribbleShape`    |
| 15         | `SHAPE_MARKER_SCRIBBLE`                | 2 MARKER                                             | —                                                                                    | `MarkerScribbleShape`      |
| 21         | `SHAPE_NEO_BRUSH`                      | 3 NEO_BRUSH                                          | —                                                                                    | `NeoBrushShape`            |
| 22         | `SHAPE_CHARCOAL_SCRIBBLE`              | 4 CHARCOAL, or 6 CHARCOAL_V2 if `attrs.texture == 2` | style 4/6: [0] set to 1 (tilt) only if it was 0 and `DeviceNoteConfig.supportTilt`   | `CharcoalScribbleShape`    |
| 60         | `SHAPE_LATIN_CALLIGRAPHY_PEN_SCRIBBLE` | 7 SQUARE_PEN                                         | style 7: [1] = min(width, 10), [2] = 45°, [3] = smooth level                         | `LatinCalligraphyPenShape` |
| 61         | `SHAPE_ASIA_CALLIGRAPHY_PEN_SCRIBBLE`  | 7 SQUARE_PEN                                         | style 7: [1] = min(width, 10), [2] = −45°, [3] = smooth level                        | `AsiaCalligraphyPenShape`  |

Notes [code]:

- The parameter writes are read-modify-write: `getStrokeParameters(style)` from SF, patch, write back;
  they are skipped if SF returns too short an array.
- `supportTilt` comes from `DeviceNoteConfigBean` (default false, `sdk/scribble/data/config/DeviceNoteConfigBean.java`);
  there is **no** `config_note_noteair4c.json` in `res/raw/` (the loader looks up
  `config_note_<Build.MODEL lowercased>`, `sdk/scribble/device/DeviceNoteConfig.java`), so the Note
  Air 4C runs with bean defaults: `supportTilt=false`, `editorViewUpdateModel="REGAL_PLUS"`,
  `scribblePenType=5` [code: the loader returns `""` when the resource id is 0, and
  `DeviceNoteConfig.ensureConfig()` returns `new DeviceNoteConfigBean()` whenever the parsed bean is
  null, `sdk/scribble/device/DeviceNoteConfig.java:38-56, 78-91`; whether `parseObject("")` returns
  null or an empty bean, both give the defaults; `supportTilt` is field `a`, getter `getA()`,
  `DeviceNoteConfigBean.java:11, 100-102`].
- Firmware width = `min(width × strokeWidthDisplayScale, mmToPx(maxWidth(type)) × scale)`, max 8 mm or
  2 mm depending on type (`ShapeUtils.java:1113-1116`, `getRenderStrokeWidth(float,int,float)`;
  `NotePenInfoUtils.getMaxStrokeWidth`). On pinch/zoom and page-layout changes the width is re-sent by
  `UpdateRawPenWidthAction` without a resume (`app/note/gesture/ScaleGestureHandlerAction.java:307, 439`,
  `app/note/action/page/UpdatePageLayoutModeAction.java:95`).
- `setStrokeWidth` also calls `nativeSetStrokeWidth` in the in-process reader, which inflates the
  exclude test by half the width (`RawInputReader.java:662-667`; `02-handwriting-regions.md` §4).

### 6.3 Colour handling

| step                                                             | value                                                                                                                                             | source                                                                                                                  |
| ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| Colour config of `NoteManager`'s render context at creation      | 1 (RAW)                                                                                                                                           | `sdk/notecore/editor/render/RendererHelper.java:58`                                                                     |
| Updated by `UpdateRendererArgsRequest` (fit/zoom/render actions) | `isColorStrokesShowOriginal() ? 1 : 0`                                                                                                            | `app/note/request/render/UpdateRendererArgsRequest.java:22-33`; `sdk/note/ui/common/utils/RenderContextUtils.java:8-10` |
| Per-stroke context in `EpdShapeHandler.A0`                       | same rule                                                                                                                                         | `EpdShapeHandler.java:118-124`                                                                                          |
| `isColorStrokesShowOriginal()` default                           | `isColorDevice()`, written back to the system config on first read                                                                                | `OnyxSystemConfig.java:100-105`                                                                                         |
| `isColorDevice()`                                                | `Device.getColorType() > 0` (SF `GET_COLOR_TYPE`, 1048659)                                                                                        | `sdk/utils/DeviceInfoUtil.java:136-138`; `fw/onyx/ViewUpdateHelper.java:648-655`                                        |
| `getRenderColor(c)` config 1                                     | `c` unchanged                                                                                                                                     | `RenderContext.java:357-363`                                                                                            |
| `getRenderColor(c)` config 2                                     | `getSpecifiedRenderColor()` (not set on the writing path)                                                                                         | `RenderContext.java:357-363`                                                                                            |
| `getRenderColor(c)` config 0                                     | alpha of `c` + (c if R=G=B else black)                                                                                                            | `RenderContext.java:82-84, 357-363`; `PenUtils.java:56-61`; `sdk/utils/ColorUtils.java:30-34, 104-106`                  |
| Value sent as `SET_STROKE_COLOR`                                 | `getRenderColor(pen.color)` (after `getRenderColor(drawingArgs.strokeColor)` was sent first; `NoteManager.setStrokeColor` applies the conversion) | `NoteManager.java:196-219, 896-903`                                                                                     |

On the Note Air 4C: `getColorType() > 0` is **[inferred]** (Kaleido panel, `isHWTconColorDevice` in SF
per `08-surfaceflinger.md` §Q3); hence config 1, raw colour to SF, unless the user switched the
setting off (`SettingViewModel.java:121`). How SF renders non-grey ink on the CFA panel live is
**[unknown]**.

### 6.4 Refresh / update calls per stroke on the Note Air 4C

| moment                                    | call                                   | update mode / waveform requested                                                | region                                   | SF-visible effect                                                           |
| ----------------------------------------- | -------------------------------------- | ------------------------------------------------------------------------------- | ---------------------------------------- | --------------------------------------------------------------------------- |
| down, move                                | none                                   | —                                                                               | —                                        | (SF inks on its own)                                                        |
| up                                        | `EditorView.invalidate()` (flush)      | view default (5, `UI_DEFAULT_MODE`), unless a page action armed REGAL_PLUS once | whole view (`invalidate()` without rect) | app frame composed; not committed while schema 7 [inferred from §5.6]       |
| up + background render                    | second `invalidate()`                  | same                                                                            | same                                     | same                                                                        |
| +500 ms                                   | none (`P0()` gated off)                | —                                                                               | —                                        | —                                                                           |
| +500 ms + 5 s, meeting scene only         | `EnablePostAction`                     | —                                                                               | —                                        | `ENABLE_POST(1)` → `stopHandwriting` → scheme 2/4 (§5.5)                    |
| fast-mode exit (only if fast mode was on) | `clearTransientUpdate` + pen 3/2 cycle | —                                                                               | —                                        | `ENABLE_POST(1)`, pen state 3 → `stopHandwriting` → scheme 2/4, recomposite |

There is no `GrayscaleRefreshAction`, no `HAND_WRITING_REPAINT_MODE`, no `handwritingRepaint`,
no `refreshScreen`, no `repaintEverything`, no `applyGCOnce` and no counted GC on the stroke path
[code: grep of `HAND_WRITING_REPAINT_MODE`/`handwritingRepaint(` in the app finds only
`GrayscaleRefreshAction.java:60`, `DataBindingUtils.java:72` (UI views) and a test activity;
`EpdDeviceManager.applyGCWithInterval` is called from page actions and menus only,
`app/common/utils/EpdDeviceManager.java:16-39` and its callers].

### 6.5 Timing constants that shape writing

| constant                                        | value on Note Air 4C                                                                                                                 | source                                                                             |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------- |
| Resume delay (`DELAY_ENABLE_RAW_DRAWING_MILLS`) | 500 ms (colour)                                                                                                                      | `sdk/notecore/editor/data/RawPenArgs.java:68-70`                                   |
| Popup resume delay                              | 500 ms (colour)                                                                                                                      | `app/note/event/PenEvent.java:18-21`                                               |
| Selection resume / shape change resume          | 500 / 100 ms                                                                                                                         | `PenEvent.java:10-11`                                                              |
| Pen-up refresh timeout                          | `SystemConfigBean.penUpRefreshTimeMs`, default 500 (value on device **[unknown]**: persisted system config; settings UI hidden here) | `SystemConfigBean.java:21`; `OnyxSystemConfig.java:81-83`                          |
| Pen deactivate after last hover                 | 100 ms                                                                                                                               | `RawInputReader.java:221`                                                          |
| HWR background recognition debounce             | 10 s after last stroke                                                                                                               | `EpdShapeHandler.java:148` (`WorkRequest.MIN_BACKOFF_MILLIS`)                      |
| Fast-mode exit after pen-up                     | 5 s                                                                                                                                  | `EpdEventHandler.java:124`                                                         |
| Auto-save                                       | `NoteSetting.autoSaveTime` seconds after `PenDeactivateEvent`; 0 = off; default of the field 0, stored value **[unknown]**           | `AutoSaveEventHandler.java:72-106`; `sdk/note/ui/setting/bean/NoteSetting.java:21` |
| Eraser buffer times (for reference)             | overlay 30 ms, stroke 20 ms                                                                                                          | `res/raw/note_noteair4c.json`                                                      |

---

## 7. Device-specific branches on the Note Air 4C

| branch                           | condition                                                                               | value here              | how determined                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | effect on writing                                                                                                                          |
| -------------------------------- | --------------------------------------------------------------------------------------- | ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Per-stroke HW reconcile          | `OnyxSystemConfig.isUseHWUpdateForPenUpRefresh()` = `!isTablet() && enablePenUpRefresh` | **false**               | `vendor.onyx.tablet=true` in `vendor.img:/build.prop` [code]; `isTablet()` reads that property (`sdk/utils/SystemPropertiesUtil.java:10, 57-59`) through `SDMDevice.getSystemProperties` → `android.onyx.hardware.DeviceController.getSystemProperties` → `OnyxDeviceService.getSystemProperties` = `SystemProperties.get` [code: `sdk/device/SDMDevice.java:615, 1448-1451`; `fw/onyx/hardware/DeviceController.java:246-252`; `fw/onyx/services/OnyxDeviceService.java:218-220`]. The property is not `ro.`, so its runtime value equalling the build.prop value is **[inferred]** | `GrayscaleRefreshAction` never runs                                                                                                        |
| Meeting-scene hand-back delay    | `MeetingViewModel.onPenUpRefresh`: `isTablet() ? 5000 : 0` ms                           | 5000 ms                 | same property                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | meeting scene only: `ENABLE_POST(1)` 5 s after the last pen-up refresh (§5.5)                                                              |
| Default of `enablePenUpRefresh`  | `!isColorDevice()`                                                                      | false [inferred colour] | `SystemConfigBean.java:22`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | redundant with the above                                                                                                                   |
| Pen-up-refresh setting shown     | `DeviceConfig.showPenUpRefresh && !hasHTCon()`                                          | **hidden**              | `note_noteair4c.json` `"showPenUpRefresh": false`; `vendor.onyx.htcon=true` in `/vendor/build.prop`; `SettingViewModel.java:83`, `LoadSettingsDataAction.java:71`                                                                                                                                                                                                                                                                                                                                                                                                                    | user cannot enable it from Notes settings                                                                                                  |
| Resume delays                    | `isColorDevice()`                                                                       | 500 ms                  | `RawPenArgs.java:68-70`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | pen stays paused 500 ms after each cycle                                                                                                   |
| Stroke colour to firmware        | `isColorStrokesShowOriginal()` default `isColorDevice()`                                | raw colour [inferred]   | §6.3                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | colour pens send their colour to SF                                                                                                        |
| Fast-mode dither threshold       | `isColorDevice() ? 160 : 255`                                                           | 160 [inferred]          | `EpdEventHandler.java:79-82`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | only while fast mode                                                                                                                       |
| EPD turbo in fast mode           | `DeviceConfig.epdTurbo`                                                                 | 5                       | `note_noteair4c.json`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | only while fast mode                                                                                                                       |
| Tilt for charcoal                | `DeviceNoteConfigBean.supportTilt`                                                      | false                   | no `config_note_noteair4c.json`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | charcoal tilt not enabled by Notes                                                                                                         |
| Editor view "REGAL once" mode    | `DeviceNoteConfigBean.editorViewUpdateModel`                                            | `REGAL_PLUS`            | bean default                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | used only when a page action arms `enableRegalModeOnce` (page goto/insert, `InvalidateScreenAction.setCheckUseRegalMode`) — not per stroke |
| Auto full refresh (page-turn GC) | `NoteSetting.enableFullRefresh` default `isColorDevice()`                               | on [inferred]           | `sdk/note/ui/setting/bean/NoteSetting.java:47`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | GC on page turns, not strokes                                                                                                              |
| Touch-distributor menu interval  | `hasHTCon() ? 0 : …`                                                                    | 0                       | `app/note/touch/ScribbleTouchDistributor.java:130-138`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | not writing-related                                                                                                                        |
| SF EPDC backend                  | `vendor.onyx.htcon`                                                                     | `HWEpdcManager`         | `08-surfaceflinger.md` (setSurfaceFlingerProvider @ `0x55eba4`); property from build.prop                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | scheme switches via `HWEpdcManager::setEbcUpdateScheme`                                                                                    |
| `Device` implementation          | board platform `lito`                                                                   | `SDMDevice`             | `sdk/device/Device.java:98-103` maps `lito` → `DeviceIndex.SDM` [code]; platform `lito` per `docs/onyx-reversing/README.md`                                                                                                                                                                                                                                                                                                                                                                                                                                                          | all EPD calls reflect into `ViewUpdateHelper`                                                                                              |

Phone vs tablet: apart from the two `isTablet()` rows above, nothing else on the writing path
branches on `isTablet()` or on the model name. `note_noteair4c.json` also sets `"isTabDevice": true`,
but `DeviceConfig.isTabDevice()` has no caller in the app or bundled SDK (grep) [code]. The per-model
JSON files are selected by `Build.MODEL` lowercased (`DeviceConfig.java:110-133`,
`DeviceNoteConfig.java:38-76`); `Build.MODEL == "NoteAir4C"` is **[inferred]** from
`ro.product.vendor.model` / `ro.product.odm.model = NoteAir4C` in the firmware images and the value
recorded in `docs/onyx-reversing/README.md`.
Mono vs colour: only the entries above. [code: grep for `isColorDevice()`, `isTablet()`,
`hasHTCon()` over `app/`, `sdk/notecore`, `sdk/scribble`, `sdk/pen`, `sdk/note`.]

---

## 8. Across strokes, pauses and saves

**Consecutive strokes (within one SF session).** Each stroke repeats §5.2-5.4. SF stays in schema 7
and scheme 3 after the first stroke; `penTriggered` blocks a second `startHandwriting`. The app keeps
flushing each stroke into its cache and invalidating; the panel keeps showing SF's ink. The pen-up
timer is restarted at every up and cancelled at every down/move, so a fluent sequence of strokes
produces one `onPenUpRefresh` after the last one, with a rect covering all of them plus the previous
firing's rect (§5.5). `y0()` warns if a down arrives before the previous up was processed; `j`
detects a repeated up [code: `EpdShapeHandler.java:275-280, 422-424`].

**Pauses in writing.** When the pen leaves proximity: 100 ms later `PenDeactivateEvent` → auto-save
timer (if configured) [code]. In the note scene nothing touches the panel or the SF session [code: no
refresh call on these paths]; in the meeting scene with media open, `ENABLE_POST(1)` follows 5 s after
the pen-up refresh (§5.5) [code]. Ten seconds after the last stroke the debounced HWR recognition may start (setting)
[code]. SF-side, no idle timeout ends the handwriting schema [code: callers of `stopHandwriting`,
§5.6]; whether SF's own debouncer / "post-input quality repaint" (`05-panel-refresh-levers.md` §3.7)
acts inside schema 7 is **[unknown]**.

**Page save / auto-save.** `AutoSaveEvent` → `ScribbleMainFragment.onAutoSaveDocumentEvent` →
`SaveDocumentAction` with a progress dialog when the scene's `showAutoSaveProgress()` is true (base
scene: true), and at the end `PenEvent.resumeRawDrawing()` [code:
`app/note/ui/ScribbleMainFragment.java:673-681`; `app/note/action/doc/SaveDocumentAction.java:212`;
`BaseNoteScene.showAutoSaveProgress`]. The dialog is expected to pause the pen through the dialog/popup
registry, and the resume re-runs §3 — i.e. an auto-save during a writing session is one pause/resume
cycle and therefore one hand-back **[inferred; the dialog → pause link is in the session subsystem]**.
The shape itself is already in the document model after `AddShapesBackgroundAction`; save writes the
document (`NoteManager.saveNoteDocument`).

**Page turn while writing.** Page actions run `EpdDeviceManager.applyGCWithInterval`, which every
`gcInterval` turns does `byPass(0)`, `applyGCOnce()` and an `InvalidateScreenAction`
(`EpdDeviceManager.java:16-39`; `BasePageAction.java:124`); `EpdEventHandler.onBeforePageChangeEvent`
leaves fast mode. Not stroke-driven [code].

---

## 9. Interactions with other subsystems

- **Session lifecycle.** Writing depends entirely on the resume path (§3). Every pause
  (`setRawDrawingEnabled(false)` or render-only pause) that actually changes the render state
  (`TouchHelper.setRawDrawingRenderEnabled` returns early if `b == z`, `TouchHelper.java:286-296`)
  emits `ENABLE_POST(1)` (SF leg, `SFTouchRender.h()`), pen state 3, then `ENABLE_POST(1)` (App leg),
  which is what ends SF's handwriting schema and lets the app's already-rendered strokes appear. A
  full pause through `PauseRawPenAction` with both flags uses `setRawDrawingEnabled(false)` and so
  also sends `SET_BRUSH_RAW_DRAWING_ENABLED(1)` and `SET_ERASER_RAW_DRAWING_ENABLED(0, 5)`
  (`app/note/action/pen/PauseRawPenAction.java:47-58`). The
  app never re-sends stroke parameters without a resume except width on zoom. After
  `setRawDrawingEnabled(false)` the SDK also resets brush raw drawing to on and eraser raw drawing to
  off (style 5) (`TouchHelper.java:182-185, 276-284`).
- **Erasing.** In the pen tool the eraser end is configured by `k()` at resume (`SET_ERASER_RAW_DRAWING_ENABLED`
  with style 8 or 5). Erase actions invalidate with `enablePost=true`, i.e. they end the SF session
  (§5.6). The stroke callbacks carry the erasing flag; `usePenTipErase()` routes drawing callbacks to
  the erase path for handlers that want it (`TouchEventHandler.java:141-148, 175-181, 301-306, 317-322`;
  `EpdShapeHandler.useDrawErase()` true, `:497-500`). Details belong to the erasing document.
- **Selection / lasso.** A resume with the selection provider sends style 5 (dash) and eraser trace on
  (`ResumeRawDrawingRequest.java:92-94, 61-65`). Not part of writing.
- **Refresh.** No per-stroke refresh; the app relies on the cycles listed in §5.6 and on page-turn GC.
- **Input routing.** Stylus MotionEvents still reach the app (HWR point capture, pen gestures) because
  the SDK installs no touch listener; finger events are independent (`NoteManager.java:232`).
- **Firmware.** SF owns live ink, the handwriting schema, scheme 3, and the limit/exclude/region-mode
  state (`08-surfaceflinger.md`). The in-process reader owns only the app's callbacks and its own
  region copy.

---

## 10. Corrections to earlier notes

- `reversed_onyx_notes_app/REPORT.md` cites `onyxsdk-pen/RawInputReader.java:747-758` for the
  pen-up rect union; in the **bundled** SDK it is `RawInputReader.i()` at `sdk/pen/RawInputReader.java:329-341`.
  Same behaviour.
- The same report says `setRawDrawingRenderEnabled(false)` pauses the input reader. In the bundled
  `SFTouchRender`, `setDrawingRenderEnabled(false)` is `h()` = `leaveScribbleMode` + pen state 3 only;
  the input reader is paused only by `setInputReaderEnable(false)` (`SFTouchRender.java:264-277, 368-374, 401-407`).
  `closeDrawing()` uses `g()`, which does both.
- `08-surfaceflinger.md` states `ENABLE_POST` calls `stopHandwriting` "if androidDrawing != 1". More
  precisely: the handler takes the second parcel int and calls `stopHandwriting` if that value is
  non-negative and differs from `androidDrawing`; so `ENABLE_POST(0)` (sent by `AppTouchRender.openDrawing`)
  also calls it when `androidDrawing==1` (it then logs `### not in handwriting schema.` and does
  nothing if the schema is not 7) [code: `sf` `0x55fef4-0x55ff14`, `0x560c18-0x560c30`].
- Android 13 (`docs/onyx-reversing/README.md`) and Android 11 (the Onyx fingerprint) are both right:
  system and vendor partitions respectively (verified on the tablet, 00-overview §0).

---

## 11. Open questions as hardware experiments

All with `adb logcat -b all -v time` running, filtered for `onyx_tcon_send_cmd`,
`onyx_tcon_set_upd_scheme`, `HWEpdcManager`, `SurfaceFlinger`, `startHandwriting`, `stopHandwriting`,
`Post enabled`, `HandlePenTrigger`, `EUS`.

1. **Scheme during a stock writing session.** Open a stock note, wait 2 s after the last UI action,
   write one stroke. Expect `HandlePenTrigger …`, `### startHandwriting`, `### EUS, s 3 c 0` once at
   the first pen-down, and no `stopHandwriting`/`EUS` at pen-up or after it. Refuted if a scheme
   change or `stopHandwriting` appears after the stroke without any UI action.
2. **Waveform of live ink.** In the same capture, read the `onyx_tcon_send_cmd` lines emitted while
   the pen moves: record `waveform[W] update_mode[M] flags[F]` and region sizes. Unknown today.
3. **Does the app's post-stroke invalidate reach the panel?** Write ten strokes in a row without
   touching the UI. Expect no `onyx_tcon_send_cmd` burst ~0-100 ms after each pen-up beyond the ink
   updates, and no update ~500 ms after the last pen-up. A GU-class update over the stroke rect at
   pen-up or +500 ms would refute §5.6 step 3-4 (and would mean SF commits app layers in schema 7).
4. **Hand-back moment.** After the ten strokes, open and close a menu (or tap the toolbar). Expect
   `ENABLE_POST`-driven `stopHandwriting`, `Post enabled!`, `### EUS, s 2 …` (or `s 4` if the app is
   in an A2-class mode), one update covering the app content, and 500 ms later a resume with
   `SET_SCREEN_HANDWRITING_PEN_STATE(2)`. Record whether the update is full (`update_mode[1]`) or
   partial and its waveform.
5. **Is there any SF idle timeout?** Write one stroke and then do nothing for 60 s. If
   `stopHandwriting` or a scheme change appears, SF has an idle path not found statically.
6. **Colour ink.** Select a red pen (colour strokes "show original" on) and write. Observe whether the
   live trace is coloured or grey/black before the next hand-back, and whether the colour changes at
   the hand-back update. Repeat with the setting off.
7. **Fast mode interaction.** Scroll the page with a finger (fast mode on), then write a stroke within
   5 s. Expect, 5 s after the pen-up, `clearTransientUpdate` effects and a pause/resume cycle
   (`stopHandwriting`, `EUS`, pen state 3 then 2). Also check whether the ink during fast mode uses a
   different waveform than in experiment 2.
8. **Writing during the 500 ms resume gap.** Close a menu and start writing immediately (< 500 ms).
   Expect no app stroke (input still paused) and, per the proven driver-log behaviour for a paused
   session, SF scheme 4 / A2 for the input and a GC16 full update ~1.5 s after lift. This tells us
   whether stock itself can produce the "flash after pause" in normal use.
9. **Stray ink paths.** With the float button active (or during HWR hold-to-convert), draw a stroke
   that stock discards (§5.4 step 4). Observe when and by which update the stray firmware ink
   disappears.
10. **Pen-up timeout value.** Read the persisted system config (root: the file named
    `ONYX_SYSTEM_CONFIG_KEY` under the Onyx system-config prefix, likely `/onyxconfig/` per
    `07-einkwise.md`) and check `penUpRefreshTimeMs` and `enablePenUpRefresh`. Only the timeout
    matters on this device (the reconcile is gated by `vendor.onyx.tablet` regardless).
11. **Auto-save during writing.** Set auto-save to the shortest interval, write, lift the pen away,
    wait. Expect a progress dialog, a pause (pen state 3, `stopHandwriting`) and a resume after the
    save; record the updates.
12. **What `HandwritingSchema::output` does with stock's post-stroke frames.** With SF debug logging
    on (`applySFDebug`, root), write strokes normally and during fast mode, and count the
    `### disable refresh.`, `### warning: sys/app is in fast mode, ignoring refresh.` and
    `### warning: commit epdc in handwriting mode.` lines per pen-up. Expect `disable refresh` (app
    frame withheld) for normal writing; `commit epdc in handwriting mode` would mean SF found a queued
    update entry and pushed something.
13. **Meeting-scene hand-back.** Open a meeting note (meeting scene, recording or playback active),
    write one stroke and keep the pen away. Expect, about 0.5 s + 5 s after the pen-up, `Post enabled!`,
    `stopHandwriting` and `### EUS, s 2 …` with no pen-state change, and one app-content update.
    Repeat in a normal note: nothing should appear.
14. **System refresh broadcast.** While writing in a stock note, trigger the system "refresh screen"
    action (the one that broadcasts `onyx.android.intent.action.REFRESH_SCREEN`; which UI element
    sends it on this firmware is **[unknown]**). Expect the `InvalidateScreenAction` sequence of
    experiment 4.
15. **Empty exclude list on resume.** On a page with no toolbars/popups overlapping the canvas,
    close a menu (resume) and capture the transactions, e.g. with SF debug logging. Expect one
    `SET_SCREEN_HANDWRITING_REGION_EXCLUDE` with an empty array (from the App leg); check whether SF
    logs clearing its exclude regions and whether pen input in a previously excluded area now draws.

---

## Verification notes

Independent re-check of this document against the primary sources (static analysis only, no
device access). Line numbers are jadx lines of the bundled copy under
`~/tmp_zfs/boox-notes-inspect/decoded/sources/`.

### Checked and confirmed

- `OnyxSystemConfig.isUseHWUpdateForPenUpRefresh()` (`sdk/data/config/system/OnyxSystemConfig.java:145-150`)
  and `SystemConfigBean` defaults (`penUpRefreshTimeMs=500`, `enablePenUpRefresh=!isColorDevice()`);
  `vendor.onyx.tablet=true`, `vendor.onyx.htcon=true`, `ro.board.platform=lito` read from
  `vendor.img:/build.prop`; the `isTablet()` chain down to `SystemProperties.get` (added to §7).
- `ResumeRawDrawingRequest` order `p()` → pen-up timeout → region mode → `applyDrawingArgs` → `n()`
  (`applyPenArgs`, `m()`, `i()`, `k()`, `h()`) → input → render (`:40-138`); `PenEventHandler.q()` bean
  contents (`:154-183`).
- `TouchHelper` fan-out and guards; `SFTouchRender.h()` = `leaveScribbleMode` (`SDMDevice.leaveScribbleMode`
  → `enablePost(view, 1)`, `SDMDevice.java:1875-1877`) + pen state 3; `AppTouchRender.setDrawingRenderEnabled(false)`
  = `ENABLE_POST(1)`; `EpdPenManager` state constants.
- `RawInputReader` state machine (`:538-571`), repeat-move filter (`:353-363`), pen-up timer and rect
  union (`:200-205, 285-341, 426-432`); `setFilterRepeatMovePoint` and `setPenUpRefreshEnabled` never
  called from stock app code (grep; `TouchHelper.setFilterRepeatMovePoint` actually forwards to
  `setPenUpRefreshEnabled`, `TouchHelper.java:216-221`, an SDK bug that does not matter here because
  nothing calls it).
- `EpdShapeHandler` down/move/up/pen-up-refresh (`:318-450`), `F0`/`G0`/`P0`/`B0` (`:126-263`);
  `RenderManager` returned by `I0()` is the `InkRenderManager` built in `DisplayBundle.java:42`;
  `InkRenderManager.invalidate(DisplayArgs)` (`:79-97`); `EditorView.onDraw` (`:64-77`).
- `GrayscaleRefreshAction.x()` (`:58-79`) is the only app use of `HAND_WRITING_REPAINT_MODE` outside
  test activities; `setViewDefaultUpdateMode`/`applyAppScopeUpdate` are not called by app code on the
  writing path (only `GrayscaleRefreshAction`, test activities and `MaterialCenterFragment`).
- `ShapeFactory.isDFBShape` (`:362-364`), `HandlerManager.getShapeProvider` (`:211-213`);
  `supportFingerDraw()` is true only for `SelectionHandler` in one mode, so no writing handler feeds
  `AppTouchRender` MotionEvents.
- `note_noteair4c.json` contents; absence of `config_note_noteair4c.json`; `NotePenInfoUtils`
  defaults (pressure 0.3, smooth 0.6); `NoteRenderUtils` read-modify-write guards.
- `sf`: symbol addresses of `startHandwriting`, `stopHandwriting`, `HandwritingSchema::output`,
  `handleEnablePost`, `afterStartHandwriting` (sets `0xaca244=1`, clears `androidDrawing`, vtable +0x78
  with (3, 0)), `handlePenTriggerImpl`; `handleEnablePost` condition (`tbnz w19,#31` / compare with
  `androidDrawing` byte, `0x560c18-0x560c30`); the three branches of `HandwritingSchema::output` and
  the three debug strings; direct call sites of `start/stopHandwriting`.

### Changed

| where                                          | old claim                                                             | new claim                                                                                                                                                                                                      |
| ---------------------------------------------- | --------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| §1 item 3                                      | pen parameters pushed **only** at resume (+ width on zoom)            | also touched outside a resume by full pauses (brush on / eraser off), `PenShapeChangeAction`, selection, span and `HandWritingHandler` paths (none on the pen-writing path itself)                             |
| §1 item 6, §5.6                                | `hasHWRepaint` semantics inferred from its name (0x80000 flag)        | [code]: exact match of the entry word to `0x80002`; mapping of that word to the view mode stays [inferred]; source of queued entries [unknown]                                                                 |
| §1 item 6, §1 item 8, §5.5, §6.1, §6.4, §7, §8 | pen-up refresh hooks have no panel effect; no stroke-driven hand-back | meeting scene with media open: `ENABLE_POST(1)` 5000 ms after the pen-up refresh (`isTablet()` branch), cancelled by the next pen-down; audio hook confirmed no-op (no binding uses `repaint`)                 |
| §3, wire trace rows 1-2                        | LIMIT once; EXCLUDE only if the list is non-empty                     | LIMIT ×2 (SF leg only if non-empty); EXCLUDE: SF leg only if non-empty, App leg always, i.e. an empty EXCLUDE on every resume without exclude rects                                                            |
| §3, wire trace row 4, §6.3                     | first colour is `drawingArgs.strokeColor`                             | `getRenderColor(drawingArgs.strokeColor)`: `NoteManager.setStrokeColor` converts                                                                                                                               |
| §3, rows 9-11                                  | multiplicity implicit                                                 | `SET_STROKE_PARAMETERS` direct via `Device` (one each); brush/eraser raw flags and region mode SF leg only                                                                                                     |
| §5.2                                           | `startHandwriting` only via pen trigger                               | also called from the `handleMoveTo` lambda (`MOVE_TO` transaction; unused by stock)                                                                                                                            |
| §5.4                                           | second invalidate path [inferred]                                     | [code]: `ForceRenderNoteDirtyAction.java:134-136` → `renderManager.invalidate()`                                                                                                                               |
| §5.6                                           | pen states summarised from `08-surfaceflinger.md`                     | decoded jump table: per-state actions incl. `enableExtBufAutoSync`, `enableCytpLoFilter`, `syncDataToExtendedBuffer`                                                                                           |
| §5.6                                           | "only callers" of `stopHandwriting`                                   | "only direct callers", plus the SW backend; indirect calls not excluded                                                                                                                                        |
| §5.6 table                                     | four triggers                                                         | added the system refresh broadcast, meeting timer, zoom-finish viewport translate, sync pull reload, document re-open restart; fixed line refs (`EpdEventHandler.java:181-183`, `InkRenderManager.java:56-61`) |
| §5.6                                           | `clearHandwritingFBSchema` at `0x562ec0-0x562f20`                     | that range is `afterStopHandwriting`; the callee is at `0x566134`                                                                                                                                              |
| §5.7                                           | exit skips only the pause/resume while the pen touches                | `e()` returns before clearing the transient update, and nothing re-arms the timer until the next pen-up or fast-mode event                                                                                     |
| §5.1                                           | hover callback unconditional                                          | `onPenActive` only while input enabled; EventBus events regardless                                                                                                                                             |
| §6.2 notes                                     | bean defaults via `parseObject("")` [inferred]                        | [code] via `ensureConfig()` null fallback                                                                                                                                                                      |
| §7                                             | `isTablet()` reads the property                                       | chain traced to `SystemProperties.get`; runtime value [inferred]; `isTabDevice` JSON flag unused; `Build.MODEL` [inferred]                                                                                     |
| §9                                             | every pause emits `ENABLE_POST(1)` twice + state 3                    | only when the render state actually changes; order SF `ENABLE_POST`, state 3, App `ENABLE_POST`; full pause also resets brush/eraser raw flags                                                                 |
| §11                                            | 12 experiments                                                        | added 13-15 (meeting hand-back, refresh broadcast, empty exclude)                                                                                                                                              |

### Could not confirm

- What SF does with `SET_STROKE_*`, an empty `SET_SCREEN_HANDWRITING_REGION_EXCLUDE`, and which
  producer enqueues entries for `EpdcManager::next` inside schema 7 (so whether a plain app invalidate
  can be committed during schema 7 is still open).
- That the collected layer entry's mode word is fed by the view's default update mode (the
  framework → SF path for per-view update modes was not traced here).
- The live-ink waveform and scheme-3 behaviour, colour of live ink on the Kaleido panel, and the
  runtime values of `vendor.onyx.tablet`, `penUpRefreshTimeMs`, `Build.MODEL`.
- That `handlePenUp` is reachable only via the `PEN_UP` transaction (not re-checked).
- That `FullScreenRefreshEvent` posted with `postGlobalEvent` reaches `EpdShapeHandler`'s subscriber
  (the handler's bus registration was not traced), and which system UI element sends the
  `REFRESH_SCREEN` broadcast.
- Firmware-level per-app refresh settings (EinkWise/EAC app config) that could change the app's
  default update mode outside the app; stock Notes has `supportEAC:false` per the brief, so they are
  presumably inactive, **[inferred]**.
- Paths not covered by this document: text handwriting input (`HandWritingHandler`, own `TouchHelper`
  use with style 1, width 4, black, `setRawDrawingEnabled` toggles, `app/note/handler/text/HandWritingHandler.java:128, 182, 229, 474, 514`)
  and HWR hold-to-convert refresh behaviour.
