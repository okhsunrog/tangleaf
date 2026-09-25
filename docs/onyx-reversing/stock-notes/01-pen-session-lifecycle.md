# Stock Notes: pen session lifecycle

How the stock Onyx Notes app (`com.onyx.android.note`, module name `knote2`) creates, opens, pauses,
resumes and closes its pen session on a BOOX Note Air 4C, traced from the app down to the
SurfaceFlinger transactions. Static analysis only; nothing here was observed on the device.

Every claim carries a tag:

- **[code]**: read in the source or binary at the cited place.
- **[inferred]**: reasoned from code that was read, not read directly.
- **[unknown]**: not settled statically. Where it matters, it becomes an experiment in section 10.

Path prefixes used below:

| prefix | path                                                                                                       |
| ------ | ---------------------------------------------------------------------------------------------------------- |
| `N/`   | `~/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/` (stock Notes, jadx, with bundled SDK)     |
| `P/`   | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-pen-1.5.4.3/sources/com/onyx/android/sdk/` (public Pen SDK)     |
| `FW/`  | `~/tmp_zfs/onyx_framework/out/sources/android/onyx/` (firmware `framework.jar`)                            |
| `SF`   | `~/tmp_zfs/onyx_framework/native/surfaceflinger`, summarised in `docs/onyx-reversing/08-surfaceflinger.md` |

Line numbers are those of the jadx output. Kotlin classes carry a long `@Metadata` line near the top,
so their numbers look shifted when compared with other tools.

Firmware version: the tablet runs firmware 4.2, build `2026-04-28_17-50_4.2-rel_04282_555977efe`,
system partition Android 13 (SDK 33), vendor partition Android 11 (SDK 30). The SurfaceFlinger,
`framework.jar` and pen native libraries on the device are byte-identical to the analysed copies, so
every framework or SF claim below applies to the device (verified 2026-09-25, see
[00-overview §0](00-overview.md)). The "Android 11 / firmware 4.1" in the original research brief was
wrong.

---

## 1. Summary

1. **One session per document view, owned by `NoteManager`.** The stock app keeps a single
   `TouchHelper` in `NoteManager.touchHelper`, created on the first bind of the note view (`EditorView`)
   with feature mask **3** (`SFTouchRender` + `AppTouchRender`) and `touchListenerEnabled=false`
   (`N/sdk/notecore/editor/NoteManager.java:221-256`, `bindPenHostView`) **[code]**. It is closed only
   when the document closes (`NoteManager.reset()` → `closeRawDrawing()`, `:765-779`), and
   `ScribbleFragment.onDestroy()` then calls `EpdController.resetEpdPost()` **[code]**.
2. **Opening the session parks it paused.** `bindPenHostView(view, windowFocused=true)` runs
   `openRawDrawing()` and immediately `forceSetRawDrawingEnabled(false)`. On the wire that is pen state
   START(1) twice, `ENABLE_POST(0)`, then `ENABLE_POST(1)`, PAUSE(3), `ENABLE_POST(1)`
   (section 4.2) **[code]**. The first real resume comes from a delayed `PenEvent`:
   `RestoreNoteZoomAction` → `resumePenDelay(600)` after a restore that binds **[code]**. The
   `OpenDocumentStatusEvent(true/false)` + `PenEvent.resumeRawDrawing()` pair in
   `ScribbleMainFragment` belongs only to the _switch to another document_ path
   (`onReopenDocumentRequest` → `l0()`), not to the first open (section 4.3; corrected during
   verification) **[code]**.
3. **Every pause goes through one of three actions, and every resume through one request.** Pauses are
   `PauseRawPenAction` (render+input, or either one), `PauseRawPenRenderAction` (render only) and the
   direct calls in two requests. Resumes always go through `PenEventHandler` → `ResumeRawDrawingRequest`
   on the single-threaded `pen` scheduler. That request sleeps the resume delay (500 ms on a colour
   device), re-pushes limit rect, exclude rects, region mode, colour, style, width, stroke parameters,
   eraser and brush raw-drawing flags, then enables input and then render (section 4.4) **[code]**.
   Exception: the rich-text view owns a **second, separate `TouchHelper`** in the same process
   (`HandWritingHandler`), which opens, pauses and resumes itself without any of this machinery
   (section 4.7) **[code]**.
4. **Resume is gated, pause is not.** `PenEventHandler.A()` (render) and `z()` (input) are predicates
   over about 15 UI flags: keyboard, popup counter, toast, status bar, float button, window focus, no-focus
   system dialog, screenshot, named pause registry, "opening", render dialog, float-menu pressing,
   recognising, scene, host bound, current layer, and the active tool's `canPenRawRender()` /
   `isUsePenInput()` (`N/note/note/eventhandler/PenEventHandler.java:105,271`) **[code]**. Every pause
   trigger pauses unconditionally. A resume only goes through when the predicate is true at the time
   the `PenEvent` is handled.
5. **The standard repaint pattern is pause → invalidate → gated resume.** `InvalidateScreenAction` →
   `InvalidateViewWithPenControlAction` does `PauseRawPenAction(pauseResumeArgs)`, then
   `InkRenderManager.invalidate(DisplayArgs)` (a normal `View.invalidate()` with the render flag off), then
   `ResumeRawPenAction` → `PenEvent(render, input, delay)` **[code]**. More than 40 call sites use it:
   tool switch, focus change, menu changes, popups, rotation, erase by type, layer changes.
   Two gates sit in front of it **[code]**: `InvalidateScreenAction.y()` posts
   `InvalidateViewWithPenControlEvent` only if `WorkBundlesKt.isDocumentOpened(...)`, and that event
   is handled only by `ScribbleHandler.onInvalidateScreenEvent`, i.e. only when the _active_ handler
   is a `ScribbleHandler` subclass (handlers register on the bus in `BaseHandler.onActivate` and
   unregister in `onDeactivate`). With an `HWRHandler` family handler or `RichTextHandler` active,
   an INV does nothing to the pen. `SelectionHandler.onInvalidateViewWithPenControl` also drops it
   while a selection exists (`SelectionHandler.java:2175-2180`).
6. **Every `BaseNoteRequest` pauses the pen by default.** `BaseNoteRequest.execute()` posts
   `RawPenPauseEvent(pauseRender=true, pauseInput=true)` before its body unless the request opted out
   with `setPauseRawDraw(false)`. By grep, 101 files contain `extends BaseNoteRequest` directly (more
   extend it through intermediates such as `BaseEditNoteRequest`), and there are 28
   `setPauseRawDraw(false)` call sites.
   The pause runs asynchronously: the EventBus delivers it on the main thread, and that thread then
   dispatches a `PauseRawPenAction` to the `pen` thread (section 5.2) **[code]**.
7. **The stock app never erases with the render flag off on this device.** All three user-selectable
   stroke erasers (stroke 0, move 1, area 2) satisfy `EraseUtils.isEraseTrackByRawRender`, so
   `EraseHandler.canPenRawRender()` is true and `EraseBeginAction` skips its
   `PauseRawPenRenderAction` **[code]**. The SF-side eraser trail is enabled on every resume via
   `setEraserRawDrawingEnabled(true, 8 or 5)` **[code]**. After the erase ends, `EraseFinishAction` pauses
   the **render only** (input stays on) and resumes after 300 ms **[code]**. That window, with pen state
   PAUSE(3) and the input reader still delivering points, is the one stock state that matches the proven
   flash trigger. Whether stock flashes when the pen lands inside it is **[unknown]** (experiment E3).
   Two SF-visible calls around the erase were missing from the first draft **[code]**:
   `ScribbleHandler.a()` sends `EpdController.enableColorCU()` (SF `ENABLE_COLOR_CU` 1048693, value 1)
   right after `EraseBeginAction` on eraser-down, and `ScribbleHandler.l0()` sends `disableColorCU()`
   (value 0) when the erasing point list arrives; and `EraseFinishAction.render()` builds
   `DisplayArgs(invalidate=false, enablePost=true)` and passes it to
   `forceRenderVisibleDirtyScreenRectToDisplay`, whose completion calls
   `InkRenderManager.invalidate(displayArgs)` → `EpdController.enablePost(view,1)`, i.e.
   `ENABLE_POST(-1,1,pid)` **while render is still on (DRAWING)**, before the render-only pause is
   posted. What SF does with `ENABLE_COLOR_CU`, and with `ENABLE_POST(1)` during DRAWING, is
   **[unknown]**.
8. **`PenEvent.pauseDrawingRender()` does nothing.** `NormalShapeHandler.onActivate`,
   `UniversalShapeHandler.onEndRawErasing` and `DeviceReceiverEventHandler.e()` (split screen) post
   `PenEvent(false)`. `PenEventHandler.onPenEvent` → `s(false,false,…)` → `q()` returns `just(false)`
   without submitting anything, because `A(false)` is false and `z3 = z() && false` (section 5.5) **[code]**.
9. **The `AppTouchRender` half only matters for `ENABLE_POST`.** With `touchListenerEnabled=false` it
   never delivers callbacks, and pen `MotionEvent`s are intercepted before its `moveTo`/`quadTo`. It
   contributes `ENABLE_POST(0)` on open, `ENABLE_POST(1)` on every render-off, `ENABLE_POST(1)` + STOP on
   close, and a **second** limit/exclude transaction for every region push, including an exclude push of
   `[Rect(0,0,0,0)]` when no exclude is registered (sections 4.1 and 6) **[code]**.
10. **Activity lifecycle hooks touch the pen only indirectly.** `ScribbleActivity.onPause/onStop/onResume`
    and `ScribbleFragment.onPause/onResume` contain no pen calls (`HandlerManager.onPause()` →
    `ScribbleHandler.onPause()` is empty; `ScribbleActivity.onStop/onStart` only unregister/register the
    activity on the global bus) **[code]**. **Correction:** `ScribbleMainFragment.onPause()` →
    `q0()` → `n().execute()` runs a `SaveDocumentAction` (not closing). That action executes
    `SaveDocumentRequest` (extends `BaseEditNoteRequest` → `BaseNoteRequest`, no opt-out, so the
    implicit `RawPenPauseEvent` of 5.2 applies) and ends by posting `PenEvent.resumeRawDrawing()`
    (`SaveDocumentAction.java:213`) **[code]**. So leaving the note with the activity pausing produces a
    pause and a gated resume attempt; the focus flag normally blocks that resume **[inferred]**.
    Leaving the app is otherwise handled through **window focus**:
    `ScribbleActivity.onWindowFocusChanged` → `ActivityFocusChangedEvent` → `PenEventHandler` sets
    `hasFocus` and runs `InvalidateScreenAction` (pause, invalidate, resume attempt that the focus flag
    blocks) **[code]**. `ScribbleTouchDistributor.onTouchEvent` also drops every `MotionEvent` while the
    activity has no window focus **[code]**.

---

## 2. Central classes and entry points

| role                     | class (file)                                                                                                                                                                            | what it does for the session                                                                                                                                                                                                                                                                             |
| ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| session owner            | `NoteManager` (`N/sdk/notecore/editor/NoteManager.java`)                                                                                                                                | holds `touchHelper`; `bindPenHostView`, `reset`, thin wrappers `setRawDrawingEnabled/RenderEnabled/InputReaderEnable`, `setDrawLimitRect/ExcludeRect`, `setStroke*`, `applyPenArgs`, `setErasePathDrawing`, `setBrushRawDrawing`, `setSingleRegionMode`, `setPenUpRefreshTimeMs` (`:196-923`) **[code]** |
| SDK facade               | `TouchHelper` (`N/sdk/pen/TouchHelper.java`)                                                                                                                                            | fans every call out to a list of `TouchRender`; keeps `created`/`renderEnabled`/`inputEnabled` flags (`a`/`b`/`c`) that deduplicate calls **[code]**                                                                                                                                                     |
| SF half                  | `SFTouchRender` → `RawInputManager` → `RawInputReader`; `EpdPenManager` (`N/sdk/pen/touch/SFTouchRender.java`, …)                                                                       | pen states 0/1/2/3, `leaveScribbleMode`, in-process native pen reader (`libonyx_pen_touch_reader.so`), limit/exclude/region-mode to both native reader and SF **[code]**                                                                                                                                 |
| app half                 | `AppTouchRender` → `AppTouchInputReader` (`N/sdk/pen/touch/AppTouchRender.java`)                                                                                                        | `ENABLE_POST(0/1)`, pen state 1/0, second copy of limit/exclude to SF **[code]**                                                                                                                                                                                                                         |
| pause/resume policy      | `PenEventHandler` (`N/note/note/eventhandler/PenEventHandler.java`)                                                                                                                     | owns all UI-state flags, the resume predicates, the named pause registry, and submits `ResumeRawDrawingRequest` **[code]**                                                                                                                                                                               |
| pause executors          | `PauseRawPenAction`, `PauseRawPenRenderAction`, `PauseResumeRawPenAction`, `ResumeRawPenAction` (`N/note/note/action/pen/`)                                                             | run on the `pen` scheduler; resume actions only post `PenEvent` **[code]**                                                                                                                                                                                                                               |
| resume executor          | `ResumeRawDrawingRequest` (`N/note/note/request/pen/ResumeRawDrawingRequest.java`)                                                                                                      | sleeps, re-pushes all parameters, enables input then render **[code]**                                                                                                                                                                                                                                   |
| repaint with pen control | `InvalidateScreenAction` → `InvalidateViewWithPenControlEvent` → `ScribbleHandler.onInvalidateViewWithPenControl` → `InvalidateViewWithPenControlAction` (`N/note/note/action/render/`) | pause → `InkRenderManager.invalidate` → resume **[code]**                                                                                                                                                                                                                                                |
| region registry          | `NoteDocViewInfo` (`N/sdk/notecore/editor/data/NoteDocViewInfo.java`), `ExcludeRectType`, `EditorBundlesKt.getTouchLimitRectList/getTouchExcludeRectList`                               | typed exclude slots; limit = `scribbleRect` **[code]**                                                                                                                                                                                                                                                   |
| tool policy              | `HandlerManager` + each `Handler` (`canPenRawRender`, `isUsePenInput`, `canBrushRawRender`)                                                                                             | decides whether render and input may be on for the active tool **[code]**                                                                                                                                                                                                                                |
| host view binding        | `RenderToEditorViewAction`, `RestoreNoteZoomAction`, `AttachHwrNoteViewAction` → `BindPenHostViewAction`                                                                                | call `NoteManager.bindPenHostView` when the pen host view differs from the current one **[code]**                                                                                                                                                                                                        |
| hardware abstraction     | `EpdController` → `Device.currentDevice()` = `SDMDevice` → reflection on `android.onyx.ViewUpdateHelper`                                                                                | Binder transactions to SurfaceFlinger (section 3) **[code]**                                                                                                                                                                                                                                             |
| schedulers               | `SchedulerManager.penScheduler()` = single thread "pen"; `NoteManager.getNoteScheduler()` = `RxScheduler.sharedSingleThreadManager()`                                                   | pen calls are serialised on "pen"; note requests on the shared note thread **[code]** (`N/sdk/notecore/editor/scheduler/SchedulerManager.java:112-132`, `NoteManager.java:437-447`)                                                                                                                      |

The host view is `EditorView`, a plain `android.view.View` (`N/sdk/notecore/editor/display/EditorView.java:20`,
`extends View`), held by `ScribbleFragment.n` **[code]**. It is not a `SurfaceView`.

The bundled pen SDK matches the public 1.5.4.3 in every lifecycle method compared:
`TouchHelper.openRawDrawing/closeRawDrawing/setRawDrawingEnabled/setRawDrawingRenderEnabled`,
`SFTouchRender.openDrawing/closeDrawing/setDrawingRenderEnabled`,
`RawInputReader.start/resume/pause/quit`, `EpdPenManager.*`. Only the obfuscated names differ (compare
`P/pen/TouchHelper.java:152-198`, `P/pen/touch/SFTouchRender.java:187-284`,
`P/pen/RawInputReader.java:319-351`) **[code]**. The second decompilation,
`~/tmp_zfs/reversed_onyx_notes_app/src`, has identical class and method layout at the places spot-checked
(for example `ScribbleFragment.onAttachHostViewEvent` at `:1321` in both) **[code]**.

---

## 3. From SDK call to SurfaceFlinger

### 3.1 Device selection on the Note Air 4C

`Device.detectDevice()` maps `ro.board.platform` through a table in which `lito` maps to
`DeviceIndex.SDM`, so it returns `SDMDevice.createDevice()`
(`N/sdk/device/Device.java:72-135`, `b()` puts `BOARD_PLATFORM_LITO → SDM`) **[code]**. The platform
string `lito` comes from `docs/onyx-reversing/README.md`, so `SDMDevice` on this tablet is **[inferred]**
from code plus a recorded device fact.

`SDMDevice` resolves every pen method reflectively on `android.onyx.ViewUpdateHelper`
(`N/sdk/device/SDMDevice.java:453`, class lookup; bindings at `:526-556` and `:689-697`) **[code]**:

| SDK call (`EpdController.*`)                                             | `SDMDevice` binding                                                                                         | framework method (`FW/ViewUpdateHelper.java`)                                                      | transaction code                            |
| ------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| `setScreenHandWritingPenState(view, s)`                                  | `x0 = setScreenHandWritingPenState(int)`; view ignored (static)                                             | `:1195-1200` writes `s`, `Process.myPid()`                                                         | `SET_SCREEN_HANDWRITING_PEN_STATE` 16711693 |
| `enablePost(view, n)` / `leaveScribbleMode(view)` = `enablePost(view,1)` | `t0 = enablePost(int)`                                                                                      | `:602-608` writes `-1, n, pid`                                                                     | `ENABLE_POST` 16711692                      |
| `setScreenHandWritingRegionLimit(view, Rect[])`                          | `z0 = setScreenHandWritingRegionLimit(View,int[])`                                                          | `:1223-1238`: adds `view.getLocationOnScreen` to every x/y, writes `view!=null?1:0`, the int array | `…_REGION_LIMIT` 16711694                   |
| `setScreenHandWritingRegionExclude(view, Rect[])`                        | `A0 = setScreenHandWritingRegionExclude(View,int[])` (rects normalised min/max, `SDMDevice.java:2593-2610`) | `:1202-1217`, same offsetting                                                                      | `…_REGION_EXCLUDE` 16711714                 |
| `setScreenHandWritingRegionMode(view, m)`                                | `y0 = setScreenHandWritingRegionMode(int)`                                                                  | `:1240-1244`                                                                                       | `…_REGION_MODE` 1048620                     |
| `setStrokeStyle(i)` / `setStrokeWidth(f)` / `setStrokeColor(c)`          | `e0` / `f0` / `d0`                                                                                          | `:1266`, `:1272`, `:1246`                                                                          | 16711688 / 16711687 / 16711686              |
| `Device.setStrokeParameters(style, float[])`                             | `n3`                                                                                                        | `:1252-1264`                                                                                       | `SET_STROKE_PARAMETERS` 1049089             |
| `Device.setBrushRawDrawingEnabled(b)`                                    | `k3`                                                                                                        | `:1110-1114`                                                                                       | 1048834                                     |
| `Device.setEraserRawDrawingEnabled(b, style)`                            | `l3` (two-arg form preferred)                                                                               | `:1167-1172`                                                                                       | 1048833                                     |
| `EpdController.resetEpdPost()`                                           | `u0`                                                                                                        | `:1071-1078`: `ENABLE_POST(-1,1,pid)` then pen state 0                                             | 16711692 + 16711693                         |

Region transactions carry no pid, so they are global to SF. Pen state and `ENABLE_POST` carry the pid
**[code]**. SF keeps a single pen-owner pid slot and a global region store
(`08-surfaceflinger.md` §2 and "Question 2") **[code per 08, not re-verified here]**.

### 3.2 What SurfaceFlinger does with each pen state

From `08-surfaceflinger.md` §3 (`HWEpdcManager::setHandWritingPenState` @ `0x5666bc`)
**[code per 08, not re-verified here]**:

| pen state | SF action                                                                                                                        | driver-log expectation                                                             |
| --------- | -------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| 0 STOP    | `enableExtBufAutoSync(0)`, `TouchReader::stop()`, `enableCytpLoFilter(0)`                                                        | `SurfaceFlinger: Pid N State change to: stop` [inferred from `ctrlStateStr` table] |
| 1 START   | `enableExtBufAutoSync(EBAS)`, `TouchReader::start()`                                                                             | `… start`                                                                          |
| 2 DRAWING | `TouchReader::ensureStart(); resume()`; schema not restored                                                                      | `… running` (`toResume` first, per the string table)                               |
| 3 PAUSE   | `TouchReader::pause()`, then `EpdcManager::stopHandwriting()`: leaves handwriting schema 7, `androidDrawing=1`, full recomposite | `… toPause`                                                                        |

The strings `Pid %d State change to: %s`, `toPause` and `toResume` are present in `SF` (checked with
`strings`) **[code]**. `ENABLE_POST` also routes into `stopHandwriting` when `androidDrawing != 1`
(`08` §1 table, `handleEnablePost` @ `0x560bf8`) **[code per 08]**. What `ENABLE_POST(0)` does inside SF
beyond "do not post" is **[unknown]**. `03-raw-drawing-render-flag.md` §1.3 infers a per-pid suppression
of composited output.

The in-process reader is separate. `RawInputReader.pause()` = `nativePausePen()`,
`resume()` = `nativeSetPenState(4)`, `quit()` = `nativeSetPenState(4)` + `nativeRawClose()`
(`N/sdk/pen/RawInputReader.java:573-602`) **[code]**. These act on `android::PenManager` /
`android::TouchReader` inside `libonyx_pen_touch_reader.so` in the app's own process (symbols
`Java_…_nativeSetPenState`, `_ZN7android10PenManager8pausePenEv` found with `strings`) **[code]**. They
do not produce SF transactions **[inferred, consistent with 03 §1.5]**. What value 4 means to the native
`PenManager` is **[unknown]**. Pause and resume in the reader also toggle the Java flag `reportData`
(`f4564d`), which gates every callback except hover/`PenActiveEvent` (`onTouchPointReceived`, `:538-571`)
**[code]**.

---

## 4. The mechanism step by step

### 4.1 `TouchHelper` construction (first bind only)

`NoteManager.bindPenHostView` (`NoteManager.java:221-256`) **[code]**:

1. Throws if the view has zero size. Returns early ("not attach for note view not changed") if the view
   is already the host and the render bitmap is valid.
2. First time: `TouchHelper.create(view, 3, rawInputCallback, false)` then `setPostInputEvent(true)`.
   Later times: `touchHelper.bindHostView(view, callback)`.
3. `debugLog(Debug.getDebug())`, then `openRawDrawing()`.
4. If `windowFocused`, `forceSetRawDrawingEnabled(false)`.
5. `rendererHelper.createRendererBitmap(0,0,w,h)`.

`TouchHelper` constructor with mask 3 (`TouchHelper.java:349-366`) **[code]**: adds `SFTouchRender`
first, then `AppTouchRender`. It calls `setTouchListenerEnabled(false)`, which sets `AppTouchRender.e=false`,
and then `bindHostView(view, cb)`.

`bindHostView` side effects, in order **[code]**:

- `SFTouchRender.bindHostView` (`SFTouchRender.java:300-307`) wraps the callback so that every callback is
  `hostView.post(...)`ed to the UI thread. Then `RawInputManager.setHostView(view)`
  (`RawInputManager.java:144-150`) does `RawInputReader.setHostView` and **immediately
  `setLimitRect(view.getLocalVisibleRect())`**. That is `nativeSetLimitRegion` plus
  `EpdController.setScreenHandWritingRegionLimit(view, [rect])` → **SF 16711694** (`RawInputReader.java:622-628`).
  Then `EpdPenManager.setHostView`.
- `AppTouchRender.bindHostView` (`AppTouchRender.java:186-198`) creates an `AppTouchInputReader`. It does
  not install an `OnTouchListener`, because `e=false`.

`setPostInputEvent(true)` makes `RawInputReader` post `PenActiveEvent`/`PenDeactivateEvent` on hover
(`RawInputReader.java:214-223`, 100 ms deactivate timer) **[code]**. `ScribbleTouchDistributor` consumes
them for gesture arbitration (`N/note/note/touch/ScribbleTouchDistributor.java:358-366`) **[code]**. That
belongs to the input-routing subsystem.

**`AppTouchRender` is inert for pen strokes.** Its callbacks forward only `if (AppTouchRender.this.e)`
(`AppTouchRender.java:44-77`), and `e` is false. Its `moveTo`/`quadTo` path is skipped for pen
`MotionEvent`s (`onInterceptTouchEvent` = `TouchUtils.isPenTouchType`, `:235-237`; `s()`/`o()`/`u()`,
`:131-172`) **[code]**. It is reached at all only through `NoteManager.onTouchEvent`, which
`ScribbleTouchDistributor` calls only when `handlerManager().supportFingerDraw() &&
scene.canFingerTouch()` (`ScribbleTouchDistributor.java:368-398`) **[code]**. Its SF-visible
contributions are listed in 4.2 to 4.6.

### 4.2 Open: `openRawDrawing()` and the immediate park

`TouchHelper.openRawDrawing()` (`TouchHelper.java:165-173`) clears the three flags, calls `openDrawing()`
on each render, then sets `created=true` **[code]**.

Wire sequence for one `bindPenHostView(view, true)` **[code]** (SF effects per 3.2):

| #   | origin                                                                           | SDK / native call                                                                                                                                                                  | SF transaction / effect                                                |
| --- | -------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| 1   | `SFTouchRender.openDrawing` → `j()`                                              | `EpdController.setStrokeStyle(0)`                                                                                                                                                  | `SET_STROKE_STYLE(0)`                                                  |
| 2   | → `c()` → `RawInputManager.startRawInputReader`                                  | `RawInputReader.start()`: `nativeRawClose()`, flags reset (`reportData=false`), native reader thread submitted on `raw_input` executor (`RawInputReader.java:669-676`, `:290-292`) | none (in-process)                                                      |
| 3   | → `EpdPenManager.startDrawing`                                                   | `setScreenHandWritingPenState(view, 1)`                                                                                                                                            | pen state **START(1)**                                                 |
| 4   | `AppTouchRender.openDrawing` (`:252-257`)                                        | `setStrokeStyle(0)`                                                                                                                                                                | `SET_STROKE_STYLE(0)` again                                            |
| 5   |                                                                                  | `enablePost(view, 0)`                                                                                                                                                              | `ENABLE_POST(-1,0,pid)`                                                |
| 6   |                                                                                  | `setScreenHandWritingPenState(view, 1)`                                                                                                                                            | pen state START(1) again                                               |
| 7   | `forceSetRawDrawingEnabled(false)` → SF `setDrawingRenderEnabled(false)` → `h()` | `leaveScribbleMode(hostView)` = `enablePost(1)`; `pauseDrawing()`                                                                                                                  | `ENABLE_POST(-1,1,pid)`; pen state **PAUSE(3)** → SF `stopHandwriting` |
| 8   | → SF `setInputReaderEnable(false)` → `i()`                                       | `RawInputReader.pause()` → `nativePausePen()`, `reportData=false`                                                                                                                  | none                                                                   |
| 9   | → App `setDrawingRenderEnabled(false)` (`:267-274`)                              | `enablePost(view, 1)`                                                                                                                                                              | `ENABLE_POST(-1,1,pid)` again                                          |
| 10  | → App `setInputReaderEnable(false)`                                              | flag only                                                                                                                                                                          | none                                                                   |

`forceSetRawDrawingEnabled` bypasses the `TouchHelper` dedupe and sets both flags to false
(`TouchHelper.java:115-126`) **[code]**. After step 10 the session is **created, render off, input off**.

`windowFocused` comes from the caller. `BindPenHostViewAction` defaults it to `true`
(`N/note/note/action/pen/BindPenHostViewAction.java`, constructor sets `r=true`) **[code]**.
`RestoreNoteZoomAction.p()` (`N/note/note/action/zoom/RestoreNoteZoomAction.java:309-329`) and
`AttachHwrNoteViewAction` keep that default. `RenderToEditorViewAction.p()`
(`N/note/note/action/render/RenderToEditorViewAction.java:103-121`) passes
`!noteDocViewInfo.userSplitFocusStatus.notFocused()` **[code]**. In split-screen, when the note pane is
not the focused split, the park (steps 7-10) is skipped. The session then stays at START(1) with
`ENABLE_POST(0)` in force until something pauses it **[code]**. `AttachNoteViewRequest` also calls
`bindPenHostView`, but nothing constructs it (grep for `new AttachNoteViewRequest` finds nothing)
**[code]**.

`AppTouchRender.setDrawingRenderEnabled(true)` returns without a transaction (`:267-274`) **[code]**. After
the first render-off, **`ENABLE_POST(0)` is not sent again until the next `openRawDrawing()`**, so for
the whole working life of the session posting stays enabled from the SDK's side **[code]**. Only steps 5
to 7 run with `ENABLE_POST(0)`, and those happen back to back on the `pen` thread **[inferred: step 7
follows step 6 in the same method]**.

### 4.3 When the host is bound (open, reopen, layout change)

- **First open of a document.** The first open goes through `ScribbleMainFragment.c0()`/`d0()` →
  `OpenDocumentAction` (`ScribbleMainFragment.java:285-312`) and does **not** post
  `OpenDocumentStatusEvent` **[code]**. The view is bound and restored by `RestoreNoteZoomAction` with an
  editor view set (`ScribbleFragment.s1()` `:1082-1089`, `onReopenNoteDocumentEvent` `:1814-1829`), or by
  `RenderToEditorViewAction` from the layout path below **[code]**. `RestoreNoteZoomAction.p()` binds only
  if its view `q` is non-null and differs from `penHostView()` (`RestoreNoteZoomAction.java:309-329`);
  `RenderScribbleContentAction.render()` (`:221`) and `RenderMeetingContentAction` (`:84`) construct it
  without a view, so they never bind **[code]**. On a binding restore: bind (4.2), `p0()` resets the
  exclude registry and sets `scribbleRect = (0,0,w,h)` (`:333-347`), `concatDisplayVisibleBox` runs, then
  `resumePenDelay(600)` (`:395`) **[code]**. Which of the two binders runs first on a cold open is
  **[unknown]**. `InvalidateScreenAction`s issued before `isDocumentOpened` is true are dropped (summary
  item 5) **[code]**.
- **Switch to another document** (`ScribbleMainFragment.onReopenDocumentRequest` with a different
  `documentId` → `l0()`, `:915-925`, `:441-451`). `l0()` posts `OpenDocumentStatusEvent(true)` (sets
  `PenEventHandler.k`, blocking both resume predicates), saves and closes the current document
  (`SaveDocumentAction.setCloseAfterSave(true)`, which runs `NoteManager.reset()`, 4.6), then `V()`
  opens the new one and posts `ReopenNoteDocumentEvent` (`m0()`), which makes `ScribbleFragment`
  run a binding `RestoreNoteZoomAction` **[code]**. In `doFinally`, `B()` posts
  `OpenDocumentStatusEvent(false)` then `PenEvent.resumeRawDrawing()` (`:493-497`) **[code]**. Whether the
  `resumePenDelay(600)` event lands before or after `isOpening` clears is timing dependent
  **[inferred]**.
- **Layout change.** `ScribbleFragment.onAttachHostViewEvent` subscribes to the `EditorView` global
  layout observable and calls `r1()` → `RenderToEditorViewAction` on every layout
  (`ScribbleFragment.java:1321-1332`, `:1071-1076`, `:1097-1099`) **[code]**. So do
  `onContentViewSizeChangedEvent` → `h1()` and `onRenderToEditorViewEvent` (`:1442-1450`, `:1809-1811`)
  **[code]**. `RenderToEditorViewAction.p()` binds only if `penHostView() != view`
  (`RenderToEditorViewAction.java:103-108`). Otherwise it stores the new `scribbleRect` and either
  re-renders or runs `InvalidateScreenAction` (`L()`) **[code]**. Nothing in the source posts
  `AttachHostViewEvent`; how it is delivered is **[unknown]**.
- **Reopen of an already-open document.** When
  `scene.shouldRestartRawDrawingWhenReopenDocument(...)` holds, `ScribbleMainFragment.n0()` runs
  `RestartRawDrawingAction(resumePen=false)` = `touchHelper.restartRawDrawing()` = `closeRawDrawing()` +
  `openRawDrawing()` **without** the park (`N/note/note/action/hwr/RestartRawDrawingAction.java`,
  `TouchHelper.java:187-190`) **[code]**.

### 4.4 Resume: `PenEvent` → `PenEventHandler` → `ResumeRawDrawingRequest`

Every resume starts as a `PenEvent(render, input, delayMs)` (`N/note/note/event/PenEvent.java`) **[code]**.
It is posted directly or through `ResumeRawPenAction.n()` (`ResumeRawPenAction.java:44-48`), which posts
`PenEvent(args.resumeRender, args.resumeInput, args.resumeDelayTime)` when either flag is set.

`PenEventHandler.onPenEvent` (`:418-421`, main thread) → `s(render, input, delay)` (`:193-219`)
**[code]**:

1. On the main thread, read `EditorBundlesKt.getTouchLimitRectList()` = `[scribbleRect]`
   (`N/sdk/notecore/editor/extension/EditorBundlesKt.java:341-344`) **[code]**.
2. `q(limitList, render, input, delay)` (`:155-186`): `zA = A(render)`, `z3 = z() && input`. If both are
   false, return `just(false)`: **no SDK call at all**. Otherwise build a `ResumePenBean` holding both
   flags, the delay, the limit list, a **copy of `getTouchExcludeRectList()`**, the active quick pen, the
   active provider, the graphics colour, `canBrushRawRender()` and a copy of the eraser args. Then submit
   `ResumeRawDrawingRequest` on `penScheduler` (`BaseEditActionKt.submitRequest`,
   `N/note/common/base/BaseEditActionKt.java:51-70`) **[code]**.
3. After it completes: `printExcludeRectLog()` and `GlobalNoteBundles.updateCurPenDrawArgs` **[code]**.

The two predicates (`PenEventHandler.java:105-107`, `:271-273`) **[code]**:

```
A(render) = render && !recognizing && !keyboard && !popupCount>0 && !statusBar && !toast
            && !noFocusSysDialog && hasFocus && !floatButton && !screenshot
            && scene.supportScribble() && noteInfo.isBindPenHostView()
            && handler.canPenRawRender() && currentLayerEnabled
            && !pauseRegistryNonEmpty && !opening && !renderDialog && !pressingFloatMenu
z()       = !keyboard && !pauseRegistryNonEmpty && !recognizing && !floatButton && !popupCount>0
            && !statusBar && hasFocus && !screenshot && scene.supportScribble()
            && noteInfo.isBindPenHostView() && !noFocusSysDialog && handler.isUsePenInput()
            && currentLayerEnabled && !toast && !opening && !renderDialog && !pressingFloatMenu
```

`ResumeRawDrawingRequest.execute` (`ResumeRawDrawingRequest.java:121-138`, runs on the `pen` thread)
**[code]**:

```
if (touchHelper == null) return
if (!(bean.resumeRender || bean.resumeInput)) return
f() = bean.resumeRender && !touchHelper.renderEnabled
g() = bean.resumeInput  && !touchHelper.inputEnabled
if (f() || g()):
    ThreadUtils.mySleep(bean.delay)          // blocks the single "pen" thread
    o():
      p(): setDrawLimitRect(limitList); setDrawExcludeRect(excludeList)
      setPenUpRefreshTimeMs(OnyxSystemConfig.penUpRefreshTimeMs)   // Java only, default 500
      setSingleRegionMode()
      applyDrawingArgs(drawingArgs)          // setStrokeColor(renderColor)
      n():
        EPD provider (pen tool):  applyPenArgs(pen) [style, width, colour] + per-type stroke params
        NORMAL_SHAPE provider:    setStrokeColor(graphicsColour)
        SELECTION provider:       setStrokeStyle(5)
        ERASE provider:           eraser type 0/1 → setStrokeStyle(8); type 2 → setStrokeStyle(5)
        i(): Device.setStrokeParameters(5, [5.0]); Device.setStrokeParameters(8, [eraserWidth, 0.5, 0.1])
        k(): setErasePathDrawing(isEraseTrackByRawRender(eraserType) || provider==SELECTION,
                                 isMoveStrokeErase(eraserType) ? 8 : 5)
        h(): setBrushRawDrawing(bean.canBrushRawRender)
    if (g()) setRawInputReaderEnable(true)
    if (f()) setRawDrawingRenderEnabled(true)
```

`f()` and `g()` are re-evaluated after the sleep, against the live `TouchHelper` flags **[code]**. The UI
predicates are **not** re-evaluated. They were sampled on the main thread when the event arrived
**[code]**.

SF-visible sequence of a full resume (render and input both off before; pen tool; no exclude slots
registered) **[code]**:

| step  | call                                                                                                                          | SF transaction(s)                                                                         |
| ----- | ----------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| sleep | `mySleep(500)` on colour (`RawPenArgs.f`, `PenEvent.DELAY_ENABLE_RAW_DRAWING_MILLS`)                                          | none                                                                                      |
| a     | `TouchHelper.setLimitRect(List)` → SF render → `RawInputReader.setLimitRect(list)`                                            | native limit + `REGION_LIMIT([scribbleRect], view)`                                       |
| b     | → App render → `AppTouchInputReader.setLimitRectList`                                                                         | `REGION_LIMIT` again (same rect)                                                          |
| c     | `setExcludeRect([Rect(0,0,0,0)])` → SF render                                                                                 | `nativeSetExcludeRegion(mapToRawTouchPoint(0,0,0,0))` + `REGION_EXCLUDE([0,0,0,0], view)` |
| d     | → App render → `setExcludeRectList`                                                                                           | `REGION_EXCLUDE` again                                                                    |
| e     | `setSingleRegionMode()` → SF render                                                                                           | `nativeSetRegionMode(1)` + `REGION_MODE(1)`; App render: no-op                            |
| f     | `setStrokeColor(c)` (applyDrawingArgs) ×2 renders                                                                             | `SET_STROKE_COLOR` ×2                                                                     |
| g     | `applyPenArgs`: `setStrokeStyle(s)` ×2 renders, `setStrokeWidth(w)` (SF render: native + SF; App: SF), `setStrokeColor(c)` ×2 | `SET_STROKE_STYLE` ×2, `SET_STROKE_WIDTH` ×2, `SET_STROKE_COLOR` ×2                       |
| h     | per-type stroke params (`NoteRenderUtils.*`, brush 5, charcoal 22, calligraphy 60/61)                                         | `SET_STROKE_PARAMETERS` (only for those pen types)                                        |
| i     | `setStrokeParameters(5,[5])`, `(8,[w,0.5,0.1])`                                                                               | `SET_STROKE_PARAMETERS` ×2                                                                |
| j     | `setErasePathDrawing(true, 8 or 5)` → SF render only                                                                          | `SET_ERASER_RAW_DRAWING_ENABLED(1, 8/5)`                                                  |
| k     | `setBrushRawDrawing(true)` (EpdShapeHandler) → SF render only                                                                 | `SET_BRUSH_RAW_DRAWING_ENABLED(1)`                                                        |
| l     | `setRawInputReaderEnable(true)` → SF render → `RawInputReader.resume()`                                                       | none; reader starts reporting                                                             |
| m     | `setRawDrawingRenderEnabled(true)` → SF render → `resumeDrawing()`                                                            | pen state **DRAWING(2)**; App render: flag only                                           |

The `[Rect(0,0,0,0)]` in step c is not an accident. `NoteDocViewInfo.getExcludeRectList()` returns a
list holding one empty `Rect()` when no slot is registered (`NoteDocViewInfo.java:51-65`), so the SF
render's "skip empty list" guard (`RawInputReader.java:604-611`) never triggers **[code]**. Because
`view != null`, the framework adds the view's screen origin to each coordinate, turning `(0,0,0,0)` into
`(x0,y0,x0,y0)` (`FW/ViewUpdateHelper.java:1204-1213`) **[code]**. How SF treats that one degenerate
rect (a reset, or an inflated box at the view origin) is disputed between `02-handwriting-regions.md` and
`08-surfaceflinger.md` and is **[unknown]** (experiment E7). `getTouchExcludeRectList` also appends the
parts of the view outside the visible page boxes when `handlerManager.touchInVisibleRect()`
(`EditorBundlesKt.java:315-338`) **[code]**.

### 4.5 Pause

`PauseRawPenAction.n()` (`PauseRawPenAction.java:47-58`, runs on `pen` via `createPenObservable`,
`N/note/common/base/BasePenAction.java:27-36`) **[code]**:

- both flags → `NoteManager.setRawDrawingEnabled(false)` → `TouchHelper.setRawDrawingEnabled(false)`
  (`TouchHelper.java:276-284`): `setRawDrawingRenderEnabled(false)`, then
  `setRawInputReaderEnable(false)`, then **`resetPenDefaultRawDrawing()` =
  `Device.setBrushRawDrawingEnabled(true)` + `Device.setEraserRawDrawingEnabled(false, 5)`**;
- render only → `setRawDrawingRenderEnabled(false)`;
- input only → `setRawInputReaderEnable(false)`.

`setRawDrawingRenderEnabled` and `setRawInputReaderEnable` do nothing if not created or already in the
requested state (`TouchHelper.java:286-308`) **[code]**. **Correction:** `TouchHelper.setRawDrawingEnabled`
checks only `created` (`a`) and then calls `resetPenDefaultRawDrawing()` unconditionally
(`TouchHelper.java:276-284`) **[code]**. So a both-flags pause (`PauseRawPenAction(pauseArgs)`, every
implicit `BaseNoteRequest` pause, every INV) issued while already paused still sends
`SET_BRUSH_RAW_DRAWING_ENABLED(1)` and `SET_ERASER_RAW_DRAWING_ENABLED(0,5)` to SF; only the pen-state
and `ENABLE_POST` parts are deduplicated. A render-only or input-only pause issued while already in that
state emits nothing. The eraser raw-drawing flag therefore stays off from any full pause until the
next `ResumeRawDrawingRequest` whose `f()` or `g()` is true re-sends it (4.4 step j) **[code]**; a resume
that finds both flags already on returns without re-pushing anything (`ResumeRawDrawingRequest.java:127`)
**[code]**.

SF-visible pause of both (render and input were on) **[code]**:

| #   | call                                                   | SF effect                                                                    |
| --- | ------------------------------------------------------ | ---------------------------------------------------------------------------- |
| 1   | SF render `h()`: `leaveScribbleMode` → `enablePost(1)` | `ENABLE_POST(-1,1,pid)` (→ `stopHandwriting` if `androidDrawing!=1`, per 08) |
| 2   | `pauseDrawing()`                                       | pen state **PAUSE(3)** → SF `TouchReader::pause` + `stopHandwriting`         |
| 3   | App render: `enablePost(1)`                            | `ENABLE_POST(-1,1,pid)`                                                      |
| 4   | SF render `i()`: `RawInputReader.pause()`              | none (native `pausePen`)                                                     |
| 5   | `Device.setBrushRawDrawingEnabled(true)`               | 1048834(1)                                                                   |
| 6   | `Device.setEraserRawDrawingEnabled(false, 5)`          | 1048833(0,5)                                                                 |

`PauseRawPenRenderAction.n()` = `setRawDrawingRenderEnabled(false)` only (steps 1-3)
(`PauseRawPenRenderAction.java:38-40`) **[code]**.

### 4.6 Close

`NoteManager.reset()` (`NoteManager.java:765-779`), called from `ResetNoteManagerRequest` inside
`SaveDocumentAction.r()` when the document is closed after save
(`N/note/note/action/doc/SaveDocumentAction.java:396-406`) and from `ResetNoteManagerAction`
(`GlobalNoteBundle.java:315`) **[code]**:

| #   | call                                                                                                      | SF effect                                        |
| --- | --------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| 1   | SF render `closeDrawing` → `j()` `setStrokeStyle(0)`                                                      | `SET_STROKE_STYLE(0)`                            |
| 2   | `g()`: `leaveScribbleMode` → `enablePost(1)`                                                              | `ENABLE_POST(-1,1,pid)`                          |
| 3   | `RawInputReader.pause()`                                                                                  | none                                             |
| 4   | `pauseDrawing()`                                                                                          | pen state PAUSE(3)                               |
| 5   | `d()`: `RawInputReader.quit()` (timer cancel, `nativeSetPenState(4)`, callback cleared, `nativeRawClose`) | none                                             |
| 6   | `quitDrawing()`                                                                                           | pen state **STOP(0)**                            |
| 7   | App render `closeDrawing` (`:200-205`): `setStrokeStyle(0)`, `enablePost(1)`, pen state 0                 | `SET_STROKE_STYLE(0)`, `ENABLE_POST(1)`, STOP(0) |

Then `touchHelper=null` and the renderer context is reset **[code]**. `ScribbleFragment.onDestroy()`
(`ScribbleFragment.java:1483-1497`) additionally calls `EpdController.resetEpdPost()`, which sends
`ENABLE_POST(-1,1,pid)` + STOP(0) once more **[code]**. A third `resetEpdPost()` is in
`EpdEventHandler.destroy()` (`N/note/note/eventhandler/EpdEventHandler.java:192-199`), reached through
`EventHandlerManager.destroy()` when the document's editor bundle is torn down
(`NoteEditBundles.java:71`, `UIBundle.java:134`) **[code]**; its timing relative to the other two is
**[inferred]** to be on document teardown. `PenEventHandler.destroy()` → `quit()` resets every UI flag
(`hasFocus=true`, counters 0, registry cleared) **[code]**. `SaveDocumentAction` also posts
`PenEvent.resumeRawDrawing()` after the close (`:213`); with `touchHelper == null`
`ResumeRawDrawingRequest.execute` returns at its first line, so this has no SF effect **[inferred from
code order]**. **The stock app never clears the SF limit or exclude
store on close.** No `setScreenHandWritingRegionExclude(null, …)` or empty-array call appears anywhere in
`N/note` or `N/sdk/notecore` **[code, by grep]**.

### 4.7 The second session: rich-text handwriting (`HandWritingHandler`)

Added during verification. A grep for `openRawDrawing`/`setRawDrawingEnabled` over the whole app finds a
second `TouchHelper` owner besides `NoteManager` **[code]**:

- `HandWritingHandler.g()` (`N/note/note/handler/text/HandWritingHandler.java:149-160`) creates it with
  `TouchHelper.create(view, rawInputCallback)`, i.e. the two-argument constructor: feature mask
  `DeviceFeatureUtil.hasStylus(ctx) ? 2 : 1` and `touchListenerEnabled = true`
  (`N/sdk/pen/TouchHelper.java:38-40`) **[code]**. On a stylus device the mask is 2, `SFTouchRender`
  only, so no `ENABLE_POST(0)` is sent on its open **[inferred: `hasStylus` is true on the Note Air 4C,
  not read]**.
- It is owned by `RichTextBundle` and driven by `FragmentRichText` (`restart()`, `deactivate()`,
  `resumeOrPauseRawDrawing(!floatButtonActive)`) and `RichTextHandler` (`resumeOrPauseRawDrawing`)
  **[code]**. `attachHostView` → `ensureTouchHelper()` → `active()` → `s()` = `openRawDrawing()` then
  `resumeRawDrawing()` (`:218-225`, `:328-371`) **[code]**.
- Its pause/resume call `TouchHelper.setRawDrawingEnabled(false/true)` directly (`:471-515`), so each
  call ends in `resetPenDefaultRawDrawing()` (brush 1, eraser 0/5), including the resume **[code]**. None
  of `PenEventHandler`'s predicates, delays or region re-pushes apply. Before each resume, `C()`
  (`:139-147`) pushes limit = the host view's local visible rect and exclude = `z()`; before open, `A()`
  (`:127-129`) sets colour black, width 4.0, style 1 (fountain) **[code]**.
- `deactivate()` → `j()` pauses and `closeRawDrawing()`s it (`:176-181`, `:361-365`) **[code]**.

Both sessions live in the same pid, and SF keeps one pen-owner pid slot and one global region store
(3.1), so this session's START/PAUSE/DRAWING/STOP and its limit/exclude pushes overwrite what the note
session last sent **[inferred]**. Whether the two are ever open at the same time on this device (the
rich-text fragment next to an open note canvas) is **[unknown]** (experiment E15).

---

## 5. The request/action pipeline that serialises these calls

### 5.1 Threads

| thread                                          | what runs there                                                                                                                                                                                                                               |
| ----------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| main                                            | all `@Subscribe(threadMode=MAIN)` handlers, including every `PenEventHandler` method, `InvalidateScreenAction`'s first step, provider switches                                                                                                |
| `pen` (single, `SchedulerManager.penScheduler`) | `PauseRawPenAction`, `PauseRawPenRenderAction`, `ResumeRawPenAction` (posting only), `UpdateRawDrawRectAction` (region push), `UpdateRawPenWidthAction`, `ResumeRawDrawingRequest` including its sleep, first half of `BindPenHostViewAction` |
| note (`RxScheduler.sharedSingleThreadManager`)  | `BaseNoteRequest`s via `NoteManager.enqueue` / `submitRequestToNote`; `TranslateViewPortRequest` and `SavePullShapeListRequest` call render/input setters from here; `ShapeChangeAction`/`PenShapeChangeAction` style changes                 |
| `raw_input` (single)                            | the native reader loop (`RawInputReader.J`)                                                                                                                                                                                                   |

Sources: `BasePenAction.createPenObservable` observes on `penScheduler()`; `BaseEditAction.noteScheduler()`,
`penScheduler()` (`N/note/common/base/BaseEditAction.java:98-105`) **[code]**. Because `pen` is single
threaded and `ResumeRawDrawingRequest` sleeps on it, **a pause issued during a resume delay runs only
after the resume has completed**. The session then goes to DRAWING briefly and back to PAUSE
**[inferred from the threading, not observed]**. `TouchHelper`'s flags are `volatile` but calls from the
note thread (`TranslateViewPortRequest`, `SavePullShapeListRequest`) and from the main thread
(`ShapeChangeAction.z()`) are not synchronised with the pen thread **[code]**.

### 5.2 Implicit pause of every note request

`BaseNoteRequest.execute()` (`N/sdk/notecore/editor/request/BaseNoteRequest.java:34-48`) **[code]**:

```
b(noteManager):  if (pauseRender || pauseInput) noteManager.post(RawPenPauseEvent(args))   // defaults true/true
execute(noteManager)
a(noteManager):  clearPageCache / renderHWRShapesToBitmap if requested
```

`RawPenPauseEvent` → `PenEventHandler.onRawPenPauseEvent` (`:449-452`) → `PauseRawPenAction(args)` on
`pen` **[code]**. The event is posted from the note thread, handled on main, and executed on `pen`, so
the request body normally runs **before** the pause lands **[inferred from EventBus MAIN delivery]**.
Requests that opt out with `setPauseRawDraw(false)` include `AddShapesToPageRequest`,
`RenderVarietyShapesRequest`, the transform requests (`Mirror`, `Scale`, `Translate`, `Rotate`),
`AreaErasingRequest`, `StrokesEraseFinishedRequest`, `OverlayEraseFinishedRequest`,
`EraseShapeTagRequest`, `EraseShapeLinkRequest`, `RemoveSelectedShapesRequest`, `CutShapesRequest`,
`AlignRequest`, `HitTestShapeRequest`, `RenderInputTextShapeRequest`, `Start/QuitSpanRequest`,
`WaitViewUpdateRequest`, `TranslateViewPortRequest`, `RenderFillBitmapRequest`, `SaveHWRPageShapeRequest`
and `BaseHwrNoteRequest` (grep for `setPauseRawDraw(false)`) **[code]**. The live stroke commit uses
`AddShapesBackgroundRequest`, which extends `RxBaseRequest` rather than `BaseNoteRequest`, so writing a
stroke never goes through this implicit pause **[code]**
(`N/sdk/notecore/editor/request/AddShapesBackgroundRequest.java:28`).

There is no matching implicit resume. A resume comes only from an explicit `PenEvent`: the action's own
`resumePen*()`, a display action's `RawPenArgs`, or a later `InvalidateScreenAction` **[code]**.
`ResumeRawDrawingRequest` itself extends `BaseRxNoteRequest` (→ `RxBaseBenchmarkRequest`), not
`BaseNoteRequest`, so it does not pause itself **[code]**. `SaveDocumentRequest` extends
`BaseEditNoteRequest` → `BaseNoteRequest` without opt-out, so a save that runs it
(`SaveDocumentAction.p0()`, `:370-374`) pauses the pen, and `SaveDocumentAction` then posts
`PenEvent.resumeRawDrawing()` (`:213`) **[code]**. That `p0()` runs on every save branch was not traced
**[inferred]**.

### 5.3 Display actions and their default pen args

| entry                                                                          | default `RawPenArgs`                                       | effect                                                                                                            |
| ------------------------------------------------------------------------------ | ---------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `DisplayVisibleBoxAction(…, null, 2, null)` (its own default)                  | `pauseResumeArgs()` = pause both, resume both after 500 ms | pause → display → `PenEvent(true,true,500)` (`N/note/note/action/render/DisplayVisibleBoxAction.java`) **[code]** |
| `ConcatDisplayVisibleBoxAction` own default                                    | `pauseResumeArgs()`                                        | same **[code]**                                                                                                   |
| `BaseNoteAction.displayVisibleBox$default` / `concatDisplayVisibleBox$default` | **`pauseArgs()`** = pause both, **no resume**              | caller must resume (`N/note/common/base/BaseNoteAction.java:35-53`) **[code]**                                    |
| `InvalidateScreenAction` (constructor)                                         | pause both, resume both, delay 500                         | via `InvalidateViewWithPenControlAction` **[code]**                                                               |
| `InvalidateViewWithPenControlAction` own default                               | `pauseResumeArgs()`                                        | **[code]**                                                                                                        |

`RawPenArgs` factory values and the default delay: `pauseArgs` (true,true,false,false),
`pauseResumeArgs` (true,true,true,true), `resumeArgs` (false,false,true,true), `nothingArgs` (all false).
The Kotlin default-argument masks leave `resumeDelayTime = f = isColorDevice ? 500 : 150`
(`N/sdk/notecore/editor/data/RawPenArgs.java:53-69`) **[code]**. The app's `PenEvent` reuses that value
as `DELAY_ENABLE_RAW_DRAWING_MILLS` and sets `POPUP_RESUME_PEN_TIME_MS = isColorDevice ? 500 : 300`,
`SELECTION_RESUME_PEN_TIME_MS = 500`, `SHAPE_CHANGE_RESUME_PEN_TIME_MS = 100`
(`N/note/note/event/PenEvent.java:8-21`) **[code]**.

### 5.4 The named pause registry and the generic flags

- `PausePenEvent(className)` → `onPausePenEvent` (`:406-416`): adds the name to the set `o`, sets
  `g=true` (pause flag), runs `PauseResumeRawPenAction`, which pauses both and then posts a resume that
  the new `g` blocks **[code]**.
- `ResumePenEvent(className?)` → `onResumePenEvent` (`:462-486`): removes that name, or **clears the whole
  set if the name is null**, recomputes `g = !set.isEmpty()`, then runs `PauseResumeRawPenAction` (with
  the event's delay if given) **[code]**. So "resume" also begins with a pause. Several producers send an
  unnamed `ResumePenEvent()`: `FragmentMenuSettingBase.onDestroyView`, `SwipeOpenToolbarGuideFragment`,
  `FragmentNoteAISetting`, `FloatToolMenuHandler`. Each of them clears every other pauser, for example
  a selection pause **[code]**.
- Popups: `onPopupWindowChange` / `onAudioMenuPopupChangeEvent` keep the counter `c`. Show: `c++` and
  `PauseRawPenAction(pauseArgs)`. Dismiss: `c--` and `s(true,true,POPUP_RESUME_PEN_TIME_MS)` **[code]**.
- Keyboard, toast: set the flag; show → `PauseRawPenAction(pauseArgs)`; hide → `s(true,true,500)` **[code]**.
- Status bar: if the rich editor is not shown → `PauseResumeRawPenAction`; else as keyboard **[code]**.
  `onTouchDownEvent` clears a stale status-bar flag and resumes **[code]**.
- No-focus system dialog: set flag; if `canPenRawRender()` → `PauseResumeRawPenAction` **[code]**.
- Window focus: `onActivityFocusChangedEvent` (`:335-348`) sets `h`, clears the status-bar flag on gain,
  and if `canPenRawRender() || isUsePenInput()` runs `InvalidateScreenAction` **[code]**.
- Float button (system floating button): set `f`, `InvalidateScreenAction.setDelayResumePenTimeMs(500)` **[code]**.
- `OpenDocumentStatusEvent`, `NoteRenderDialogEvent`, `ScreenShotChangeEvent`, `FloatMenuPressingEvent`:
  set flags only, no call **[code]**.
- `PenPauseEvent(args)` / `RawPenPauseEvent(args)` → `PauseRawPenAction(args)`; `PenResumeEvent(args)` →
  `ResumeRawPenAction(args)` **[code]**.
- `ZoomPreviewPopMoveEnableEvent(true/false)` → `onPausePenEvent(null)` / `onResumePenEvent(null)` **[code]**.
- Keyboard hide, toast hide, status-bar close with the rich editor shown, and `onTouchDownEvent` call
  `t(this, false, false, 0, 7, null)`. Mask 7 replaces all three arguments with their defaults, so the
  call is `s(true, true, DELAY_ENABLE_RAW_DRAWING_MILLS)` despite the literal `false, false, 0`
  (`PenEventHandler.java:222-233`) **[code]**.
- `PenEvent(true,true,true)` (`waitAndResumeRawDrawing`) sets `isWaitViewUpdate`, but
  `onPenEvent` ignores that flag (`:418-421`) **[code]**.

Handler-specific pen calls outside `PenEventHandler`, added during verification **[code]**:

- `EraseHandler.onActivate()` (`N/note/note/handler/scribble/EraseHandler.java:60-66`):
  `PauseResumeRawPenAction` with resume delay **150 ms** (not 500).
- `UniversalShapeHandler.onDeactivate()` (`:597-603`): `PenEvent.resumeRawDrawingImmediately()` (delay 0);
  `onEndRawErasing()` posts the no-op `pauseDrawingRender()` (5.5).
- `NormalShapeHandler.onActivate()` / `onDeactivate()`: `pauseDrawingRender()` (no-op) /
  `resumeDrawingRender()` = `PenEvent(true)` (render only, delay 500).
- `ZoomSelectionHandler` (`:52`): `PenEvent.resumeRawDrawing(800)`.
- `EpdEventHandler` (fast mode, `N/note/note/eventhandler/EpdEventHandler.java`):
  `onApplyFastModeEvent(enable=false)` posts `PenEvent.resumeRawDrawingImmediately()` (delay 0) when fast
  mode was on; `onQuitFastModeImmediatelyEvent` posts `PenEvent.resumeRawDrawing()`; and if a raw pen-up
  happened while fast mode was on (`onRawPenUpEvent` sets `d`), the 5-second fast-mode timer `e()` ends
  with a `PauseResumeRawPenAction` (`e()` `:145-155`, `j()` `:181-183`, `onRawPenUpEvent` `:285-291`). `TranslateViewPortRequest` and
  `ViewportEventHandler.onViewportScaleBeginEvent` post `ApplyFastModeEvent(true)`.
- `ScribbleHandler.onActivate()` / `onDeactivate()` send `ENABLE_COLOR_CU(1/0)` when the handler's
  `isEnableColorCU()` is true: `EraseHandler`, `NormalShapeHandler`, `SelectionHandler`,
  `UniversalShapeHandler` (`EpdShapeHandler`, the pen tool, keeps the `BaseHandler` default false).
- `MeetingViewModel` (timer and `toggleRecord`) and `MeetingRecognizerViewModel.f0()` run
  `EnablePostAction` = `EpdController.enablePost(1)` → `ENABLE_POST(-1,1,pid)` outside any pause.

### 5.5 `PenEvent.pauseDrawingRender()` is a no-op

`PenEvent.pauseDrawingRender()` = `new PenEvent(false)`: render false, input false, delay 500
(`PenEvent.java`) **[code]**. In `s(false,false,500)` → `q()`: `A(false)` is false because its first
term is `!z`, and `z3 = z() && false` is false, so the method returns `just(false)` without a request
(`PenEventHandler.java:155-164`) **[code]**. The only other `PenEvent` subscriber,
`N/sdk/scribble/data/pen/BasePenEventHandler`, has no subclass in the app and handles a different class
(`sdk.scribble.event.pen.PenEvent`) **[code, by grep]**. The producers
`NormalShapeHandler.onActivate` (`:174-179`), `UniversalShapeHandler.onEndRawErasing` (`:626-631`) and
`DeviceReceiverEventHandler.e()` (`onPreEnterSplitScreen`, `onPreExchangeSplitScreen`) therefore have no
SF-visible effect **[code]**. Whether this is intended is **[unknown]**.

---

## 6. Limit and exclude regions: when set, updated, cleared

| event                                            | call chain                                                                                                                                                           | limit pushed                                 | exclude pushed                                                                 |
| ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------- | ------------------------------------------------------------------------------ |
| `TouchHelper` creation or `bindHostView`         | `RawInputManager.setHostView` (`:144-150`)                                                                                                                           | `view.getLocalVisibleRect()`, SF render only | none                                                                           |
| every resume                                     | `ResumeRawDrawingRequest.p()`                                                                                                                                        | `[scribbleRect]` ×2 (SF render + App render) | `getTouchExcludeRectList()` ×2, which is `[Rect(0,0,0,0)]` when no slot is set |
| menu with an exclude slot shown, moved or hidden | `BaseMenuHandler.addExcludeRect/removeExcludeRect(type, rect, update=true)` → `UpdateRawDrawRectAction` (`N/note/note/menu/view/handler/BaseMenuHandler.java:51-96`) | `[scribbleRect]` ×2                          | full list ×2                                                                   |
| scale end, scroll end                            | `ViewportEventHandler.onViewportScaleEndEvent`, `onViewportScrollEndEvent` → `UpdateRawDrawRectAction` (`:78-90`, `:121-133`)                                        | ×2                                           | ×2                                                                             |
| document restore                                 | `RestoreNoteZoomAction.p0()` → `resetExcludeRect()` (registry only) and `setScribbleRect`                                                                            | at next resume                               | at next resume                                                                 |
| close                                            | nothing                                                                                                                                                              | stays in SF                                  | stays in SF                                                                    |

Exclude slot owners (`ExcludeRectType`): `AUDIO_MENU_RECT 1` (FloatAudioMenuHandler),
`ZOOM_PREVIEW_RECT 2` (ZoomPreviewContainerHandler, ZoomWithoutPreviewMenuHandler), `FLOAT_MENU_RECT 3`
(FloatToolMenuHandler), `LINK_NOTE_JUMP_MENU_RECT 4`, `HORIZONTAL_SCROLL_BAR 5`,
`VERTICAL_SCROLL_BAR 6`, `NOTE_PAGE_NAVIGATOR_MENU_RECT 7`, `ZOOM_TOOL_MENU 8` (CanvasSeekbarMenuHandler),
`CLIPBOARD_ACTION_MENU_RECT 9`, `FLOAT_COMPACT_MENU_RECT 10` (ScribbleMenuModel) **[code]**. Most use
the Kotlin default `update=true` (the `$default` mask 4 or 2), so they push immediately.
`CanvasSeekbarMenuHandler` passes `false` explicitly, so scroll-bar and zoom-tool rects take effect only
at the next resume or rect update **[code]**. `ScribbleMenuModel` edits slot 10 directly on
`NoteDocViewInfo` without pushing **[code]**.

Region mode is set to single (1) on every resume (`setSingleRegionMode`) and never to multi by the app
**[code]**.

---

## 7. Tables

### 7.1 State machine

States are named by the triple (pen state last sent to SF, `TouchHelper.renderEnabled`,
`TouchHelper.inputEnabled`). "Posting" is the last `ENABLE_POST` value.

| state | SF pen state | render | input | posting | meaning                                                                                                                                            |
| ----- | ------------ | ------ | ----- | ------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| U     | 0 or none    | –      | –     | 1       | no `TouchHelper` (before first bind, after close)                                                                                                  |
| O     | 1 START      | off    | off   | 0       | opened, not parked (split-screen unfocused bind, or `restartRawDrawing`)                                                                           |
| P     | 3 PAUSE      | off    | off   | 1       | parked or fully paused (the common idle-while-UI state)                                                                                            |
| R     | 3 PAUSE      | off    | on    | 1       | render paused, input live (erase-finish window, input-only resumes, fill tool, some lasso states)                                                  |
| D     | 2 DRAWING    | on     | on    | 1       | active: firmware draws ink                                                                                                                         |
| Dr    | 2 DRAWING    | on     | off   | 1       | render on, input off: reachable only via `PenEvent(true,false)` (`resumeDrawingRender`, e.g. `NormalShapeHandler.onDeactivate`) when input was off |

Transitions **[code]**, each with its SF-visible effect:

```
U --bind(windowFocused=true)--> [START,START,POST0] --> P      (4.2 steps 1-10)
U --bind(windowFocused=false)--> O
O --any render-off--> P                                        POST1, PAUSE(3), POST1
P --ResumeRawDrawingRequest(render,input)--> D                 regions, style..., input on, DRAWING(2)
P --ResumeRawDrawingRequest(input only)--> R                   regions, style..., input on (no SF pen state)
R --ResumeRawDrawingRequest(render)--> D                       regions..., DRAWING(2)
D --PauseRawPenAction(both) / setRawDrawingEnabled(false)--> P POST1, PAUSE(3), POST1, brush(1), eraser(0,5)
D --PauseRawPenRenderAction / render-only pause--> R           POST1, PAUSE(3), POST1
D/R/P --closeRawDrawing--> U                                   style0, POST1, PAUSE(3), STOP(0), style0, POST1, STOP(0)
any --restartRawDrawing--> O                                   close sequence then open sequence without park
```

### 7.2 Trigger → SDK calls in order → SF-visible effect

"PR" means `PauseRawPenAction(pauseArgs)` (4.5: POST1, PAUSE(3), POST1, brush 1, eraser 0/5). "RS(d)"
means a gated `ResumeRawDrawingRequest` after sleeping `d` ms (4.4: region pushes ×2, mode, colour,
style, width, stroke params, eraser/brush flags, then DRAWING(2) if render passes). "INV" means
`InvalidateScreenAction` = PR, then `InkRenderManager.invalidate` (`View.invalidate()` with
`EpdDeviceManager.setUpdateMode` around it when a one-shot mode is set), then RS(500).

| trigger                                                                                            | call chain (file)                                                                                                                                                                                                                                                                                | SDK calls in order                                                                   | SF-visible effect                                                                            | tag                                                                    |
| -------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| open document (first open)                                                                         | `OpenDocumentAction` (`ScribbleMainFragment.d0`); bind via `ScribbleFragment.s1`/`onReopenNoteDocumentEvent` → `RestoreNoteZoomAction` (view set) or layout → `RenderToEditorViewAction` → `BindPenHostViewAction` → `NoteManager.bindPenHostView`; `resumePenDelay(600)`                        | create (limit=local visible rect), open, park; later RS(600)                         | START ×2, POST0, POST1, PAUSE, POST1; then regions…, DRAWING                                 | code (which binder runs first: unknown)                                |
| switch to another document                                                                         | `ScribbleMainFragment.onReopenDocumentRequest` → `l0`: `OpenDocumentStatusEvent(true)`, save+close current (4.6), open new, `ReopenNoteDocumentEvent` → binding `RestoreNoteZoomAction`; `B`: `OpenDocumentStatusEvent(false)` + `PenEvent.resumeRawDrawing()`                                   | close, create, open, park; RS(600) blocked while opening; RS(500)                    | …STOP(0); START…PAUSE; DRAWING                                                               | code                                                                   |
| host view layout / size change                                                                     | `ScribbleFragment.onAttachHostViewEvent`/`onContentViewSizeChangedEvent` → `RenderToEditorViewAction`                                                                                                                                                                                            | bind only if host changed; else scribbleRect update + render or INV                  | usually INV                                                                                  | code                                                                   |
| window focus lost (dialog activity, app switch, recents, lock screen)                              | `ScribbleActivity.onWindowFocusChanged(false)` → `ActivityFocusChangedEvent` → `PenEventHandler.onActivityFocusChangedEvent`                                                                                                                                                                     | `hasFocus=false`; INV (resume blocked)                                               | POST1, PAUSE(3), POST1; app frame posted; no DRAWING                                         | code                                                                   |
| window focus gained                                                                                | same with `true`                                                                                                                                                                                                                                                                                 | `hasFocus=true`; INV                                                                 | PAUSE…, invalidate, then RS(500) → DRAWING                                                   | code                                                                   |
| Activity `onPause`/`onStop`/`onResume`                                                             | `ScribbleActivity`, `ScribbleFragment`                                                                                                                                                                                                                                                           | none for the pen (`RxFrameHandler`, `NoteStatus`, touch-control status)              | none                                                                                         | code                                                                   |
| `ScribbleMainFragment.onPause`                                                                     | `q0()` → `SaveDocumentAction` (no close) → `SaveDocumentRequest` (BaseNoteRequest: implicit PR) → end: `PenEvent.resumeRawDrawing()` (`SaveDocumentAction.java:213`)                                                                                                                             | PR (async); gated RS(500)                                                            | PAUSE; DRAWING only if the predicates pass (focus usually lost)                              | code / inferred                                                        |
| any document save                                                                                  | `SaveDocumentAction` (menu save, auto-save paths)                                                                                                                                                                                                                                                | PR via `SaveDocumentRequest`; `PenEvent.resumeRawDrawing()` at the end               | PAUSE → DRAWING                                                                              | code                                                                   |
| close document                                                                                     | `SaveDocumentAction.r()` → `ResetNoteManagerRequest` → `NoteManager.reset`; `ScribbleFragment.onDestroy` → `resetEpdPost`; `EpdEventHandler.destroy` → `resetEpdPost`                                                                                                                            | close (4.6); `resetEpdPost` (up to twice)                                            | …STOP(0); POST1+STOP(0) per reset; regions left in SF                                        | code (order of the resets: inferred)                                   |
| tool switch pen ↔ eraser (menu or `TogglePenTypeAction`)                                           | `HandlerManager.activeProvider` → old `onDeactivate`, new `onActivate`; `ScribbleHandler.onActivate` → `ActiveRenderModeAction`; `EraseHandler.onActivate` → `PauseResumeRawPenAction` with resume delay 150 ms; colour-CU handlers send `ENABLE_COLOR_CU(1/0)`                                  | PR, then gated RS(150) when entering the eraser; other entries as their `onActivate` | PAUSE → DRAWING with the new style; `ENABLE_COLOR_CU`                                        | code (whether an INV is also issued on every switch was not re-traced) |
| pen type/shape change                                                                              | `ShapeChangeAction`: `prepareShapeChangeObservable`, provider switch, INV(resume render=false) → `setStrokeStyle`/`setStrokeColor` on main → `resumePenDelay(100)`                                                                                                                               | PR, invalidate, input-only resume attempt, style ×2, colour ×2, RS(100)              | PAUSE; possibly R; then DRAWING                                                              | code                                                                   |
| pen shape change (`PenShapeChangeAction`)                                                          | provider switch → style/colour on note thread → `RendererToScreenRequest` (BaseNoteRequest: implicit PR) → `resumePen()`                                                                                                                                                                         | style, colour, PR, render, RS(500)                                                   | PAUSE → DRAWING                                                                              | code                                                                   |
| eraser type change                                                                                 | `EraseByTypeAction.applyEraserConfig` → INV (layer erasers → `EraseLayerAction`)                                                                                                                                                                                                                 | INV                                                                                  | PAUSE → DRAWING                                                                              | code                                                                   |
| stylus-eraser stroke (any scribble tool)                                                           | `ScribbleHandler.onBeginRawErasing` → `a()` → `EraseBeginAction`, `enableColorCU()`; points: `ErasingShapeAction` (display with `DisplayArgs.enablePost=true`); list received: `l0()` → `disableColorCU()`                                                                                       | no pen-state call for types 0/1/2 (render stays on); `ENABLE_COLOR_CU(1)`…`(0)`      | stays DRAWING; SF draws eraser trail (style 8/5); app redraws erased areas during the stroke | code (whether the mid-stroke display reaches `enablePost`: inferred)   |
| erase stroke finished                                                                              | `EraseFinishAction`: requests, `render()` → `InkRenderManager.invalidate(DisplayArgs(invalidate=false, enablePost=true))`; delay `eraserAreaDisplayDelayMillSecond` (0); `EraseFinishEvent(pauseResume, pauseInput=false, delay 300)` → `onEraseFinished` → `InvalidateViewWithPenControlAction` | POST1 while DRAWING; render-only pause, invalidate, RS(300)                          | POST1 (in DRAWING); POST1, PAUSE(3), POST1 → ~300 ms+ in state R → DRAWING                   | code                                                                   |
| lasso start/stop (selection)                                                                       | `SelectionHandler`: `PausePenEvent(name)` / `ResumePenEvent(name)` (`:536`, `:692`); `canPenRawRender` = lasso and not rectangle                                                                                                                                                                 | registry pause, then gated resume                                                    | PAUSE → DRAWING (lasso rendered with style 5)                                                | code                                                                   |
| rectangle selection active                                                                         | `SelectionHandler.canPenRawRender()` false, `isUsePenInput()` false                                                                                                                                                                                                                              | resumes blocked                                                                      | stays PAUSE                                                                                  | code                                                                   |
| text tool (`TextInputHandler`), rich text, HWR placement/reflow, zoom selection, layer lock        | handler returns false from both predicates                                                                                                                                                                                                                                                       | resumes blocked                                                                      | stays PAUSE                                                                                  | code                                                                   |
| fill-colour tool                                                                                   | `FillColorHandler.canPenRawRender()` false, `isUsePenInput()` default true                                                                                                                                                                                                                       | input-only resumes                                                                   | state R: PAUSE with live input                                                               | code                                                                   |
| popup shown (`BasePopWindow`, `BaseNotePopWindow`, `BaseEditPopWindow`)                            | `PopupWindowChangeEvent(true)` → PR; `PopupChangeAction` also INV(resume render=false)                                                                                                                                                                                                           | PR (twice)                                                                           | PAUSE                                                                                        | code                                                                   |
| popup dismissed                                                                                    | `PopupChangeAction`: INV(regal once, resume render=false), then `PopupWindowChangeEvent(false)` → `c--`, `s(true,true,500)`                                                                                                                                                                      | PR, invalidate, gated RS(500)                                                        | PAUSE → DRAWING if no other flag                                                             | code                                                                   |
| settings/menu fragments (`FragmentMenuSettingBase`, background, draft template, AI setting, guide) | `PausePenEvent(name)` on view create; `ResumePenEvent()` (unnamed: clears all) + `RegalInvalidateWithPenControlEvent` on destroy                                                                                                                                                                 | registry pause / resume                                                              | PAUSE … DRAWING                                                                              | code                                                                   |
| float tool menu drag                                                                               | `FloatToolMenuHandler.onTransformToFloatTypeEvent`: `ApplyFastModeEvent(true)` + `PausePenEvent`; on touch-up `ResumePenEvent()`                                                                                                                                                                 | registry pause/resume                                                                | PAUSE → DRAWING                                                                              | code                                                                   |
| float menu press (drag bar)                                                                        | `FloatDraggableMenuBar`: `FloatMenuPressingEvent(true)` + `RawPenPauseEvent(pauseArgs)`; release `FloatMenuPressingEvent(false)`                                                                                                                                                                 | PR; flag blocks resumes                                                              | PAUSE                                                                                        | code                                                                   |
| menu with exclude slot shown/moved/hidden                                                          | `BaseMenuHandler.add/removeExcludeRect` → `UpdateRawDrawRectAction` (pen thread)                                                                                                                                                                                                                 | limit ×2, exclude ×2                                                                 | region transactions only; pen state unchanged                                                | code                                                                   |
| toast shown / hidden (system)                                                                      | `DeviceReceiver` → `ToastChangeAction` → INV(resume render=false) → `ToastChangeEvent` → PR / `s(…,500)`                                                                                                                                                                                         | PR, invalidate, …                                                                    | PAUSE; → DRAWING after hide                                                                  | code                                                                   |
| status bar pulled / closed                                                                         | `DeviceReceiver` → `StatusBarChangeAction` (regal once on close) → `StatusBarChangeEvent`                                                                                                                                                                                                        | pause-resume or PR / RS                                                              | PAUSE → DRAWING                                                                              | code                                                                   |
| system float button activated                                                                      | `FloatButtonChangedEvent` → `f=active`; INV(delay 500)                                                                                                                                                                                                                                           | INV                                                                                  | PAUSE; DRAWING only when inactive                                                            | code                                                                   |
| no-focus system dialog                                                                             | `NoFocusSystemDialogChangeEvent` → flag; `PauseResumeRawPenAction` if `canPenRawRender`                                                                                                                                                                                                          | pause, gated resume                                                                  | PAUSE → DRAWING after close                                                                  | code                                                                   |
| region screenshot start / end                                                                      | `DeviceReceiver.onRegionScreenShot` → `ScreenShotChangeAction`: `ScreenShotChangeEvent` (flag `j = !end`), then `renderScreen()` = `RendererToScreenRequest` (BaseNoteRequest: implicit PR), then `PenEvent.resumeRawDrawing()`                                                                  | PR; gated RS(500), blocked while the screenshot flag is set                          | PAUSE during the screenshot; DRAWING after it ends                                           | code                                                                   |
| Onyx soft keyboard (meeting recogniser path)                                                       | `MeetingRecognizerViewModel` → `KeyboardChangeEvent` → PR / `s(true,true,500)`                                                                                                                                                                                                                   | PR / RS(500)                                                                         | PAUSE → DRAWING                                                                              | code                                                                   |
| text box editing                                                                                   | `StartTextInputAction` → INV(resume render=false); `TextInputHandler` blocks both predicates                                                                                                                                                                                                     | PR                                                                                   | PAUSE for the whole text session                                                             | code                                                                   |
| page turn (next/prev/goto/add/remove)                                                              | `BasePageAction` → `concatDisplayVisibleBox$default` (**pauseArgs**) → display → `PenEvent.resumeRawDrawing(max(600−elapsed, 500))` + `EpdDeviceManager.applyGCWithInterval` (`BasePageAction.java:117-126`)                                                                                     | PR, display, RS(≥500)                                                                | PAUSE → DRAWING; GC per the refresh subsystem                                                | code                                                                   |
| finger scroll                                                                                      | `ScrollGestureHandlerAction` → `ScrollByRequest` (core) → `displayScrollingBoxList`; end: `ViewportScrollEndEvent` → `UpdateRawDrawRectAction`                                                                                                                                                   | no pause found during scrolling; regions pushed at end                               | pen state unchanged (**[unknown]** whether it stays DRAWING during the scroll)               | code / unknown                                                         |
| pinch zoom begin                                                                                   | `ViewportEventHandler.onViewportScaleBeginEvent` → `ApplyFastModeEvent(true)` + `PauseResumeRawPenAction`                                                                                                                                                                                        | pause, gated resume                                                                  | PAUSE → DRAWING                                                                              | code                                                                   |
| zoom end (`ZoomFinishAction`)                                                                      | `TranslateViewPortRequest.execute` (note thread): `setRawDrawingRenderEnabled(false)`, `setRawInputReaderEnable(false)` directly; `ZoomFilter.e()` → `PenEvent.resumeRawDrawing()` (throttled 1 s)                                                                                               | render off, input off; later RS(500)                                                 | POST1, PAUSE, POST1 → DRAWING                                                                | code                                                                   |
| scale end                                                                                          | `onViewportScaleEndEvent` → `UpdateRawDrawRectAction`                                                                                                                                                                                                                                            | regions ×2                                                                           | regions only                                                                                 | code                                                                   |
| split screen enter/exchange                                                                        | `DeviceReceiverEventHandler.e()` → `PenEvent.pauseDrawingRender()` (**no-op**, 5.5); post: `f()` → `PenEvent.resumeRawDrawing()`                                                                                                                                                                 | none; then gated RS(500)                                                             | none; then DRAWING                                                                           | code                                                                   |
| user split handle drag                                                                             | `DeviceReceiverEventHandler.d()`: `RawPenPauseEvent(pauseArgs)` or `PenEvent.resumeRawDrawing()`; focus status stored                                                                                                                                                                            | PR / RS                                                                              | PAUSE / DRAWING                                                                              | code                                                                   |
| rotation                                                                                           | `DeviceReceiverEventHandler.c()` → INV if the rotation changed                                                                                                                                                                                                                                   | INV                                                                                  | PAUSE → DRAWING                                                                              | code                                                                   |
| shutdown broadcast                                                                                 | `PausePenEvent` + handler pause + save                                                                                                                                                                                                                                                           | registry pause                                                                       | PAUSE                                                                                        | code                                                                   |
| screen off / on                                                                                    | `SCREEN_ON` and `SYSTEM_WAKE_UP` ignored; no pen handling in `DeviceReceiverEventHandler`                                                                                                                                                                                                        | none directly; focus loss if the keyguard takes focus                                | **[unknown]** (see E9)                                                                       | code / inferred                                                        |
| any `BaseNoteRequest` without opt-out                                                              | `BaseNoteRequest.b()` → `RawPenPauseEvent` → PR                                                                                                                                                                                                                                                  | PR (async)                                                                           | PAUSE; resume only from a later `PenEvent`                                                   | code                                                                   |
| cloud sync pull into the open document                                                             | `SavePullShapeListRequest`: `setRawDrawingRenderEnabled(false)`, `setRawInputReaderEnable(false)` (note thread)                                                                                                                                                                                  | render off, input off                                                                | PAUSE                                                                                        | code                                                                   |
| stroke width change (zoom)                                                                         | `UpdateRawPenWidthAction` (pen thread) → `setStrokeWidth`                                                                                                                                                                                                                                        | width ×2 renders                                                                     | `SET_STROKE_WIDTH` ×2                                                                        | code                                                                   |
| span shapes                                                                                        | `StartSpanRequest`/`QuitSpanRequest`: `setStrokeWidth`                                                                                                                                                                                                                                           | width                                                                                | `SET_STROKE_WIDTH`                                                                           | code                                                                   |
| fast mode off / quit                                                                               | `EpdEventHandler.onApplyFastModeEvent(false)` → `PenEvent.resumeRawDrawingImmediately()`; `onQuitFastModeImmediatelyEvent` → `PenEvent.resumeRawDrawing()`                                                                                                                                       | gated RS(0) / RS(500)                                                                | DRAWING                                                                                      | code                                                                   |
| fast-mode timeout after a pen-up                                                                   | `EpdEventHandler` 5 s timer → `e()` → `j()` = `PauseResumeRawPenAction` when a raw pen-up happened during fast mode                                                                                                                                                                              | PR, gated RS(500)                                                                    | PAUSE → DRAWING about 5 s after lift                                                         | code                                                                   |
| selection handler with a live selection                                                            | `SelectionHandler.onInvalidateViewWithPenControl` returns without acting                                                                                                                                                                                                                         | INV dropped                                                                          | none                                                                                         | code                                                                   |
| INV while an HWR or rich-text handler is active, or before the document is opened                  | `InvalidateScreenAction.y()` guard / no `ScribbleHandler` subscriber                                                                                                                                                                                                                             | nothing                                                                              | none                                                                                         | code                                                                   |
| meeting recording                                                                                  | `MeetingViewModel` timer, `toggleRecord`; `MeetingRecognizerViewModel.f0` → `EnablePostAction`                                                                                                                                                                                                   | `enablePost(1)`                                                                      | `ENABLE_POST(-1,1,pid)` without a pen-state change                                           | code                                                                   |
| rich-text handwriting view                                                                         | `FragmentRichText`/`RichTextHandler` → `HandWritingHandler` (second `TouchHelper`, 4.7)                                                                                                                                                                                                          | open, `setRawDrawingEnabled(true/false)`, close                                      | START/DRAWING/PAUSE/STOP from the same pid; brush 1 + eraser 0/5 on every call               | code / inferred                                                        |

### 7.3 Stroke parameters and styles per resume

| active provider / tool                                | stroke style sent at resume                    | eraser raw drawing (`setEraserRawDrawingEnabled`)                                                                   | brush raw drawing                                   |
| ----------------------------------------------------- | ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------- |
| `SCRIBBLE_EPD_PROVIDER` (pen tool, `EpdShapeHandler`) | `ShapeFactory.getStrokeStyle(pen.type, attrs)` | `(isEraseTrackByRawRender(eraser), move/stroke ? 8 : 5)` = `(true, 8)` for stroke/move eraser, `(true, 5)` for area | `true` (`EpdShapeHandler.canBrushRawRender`)        |
| `SCRIBBLE_SELECTION_PROVIDER`                         | 5 (`STROKE_STYLE_DASH`)                        | `(true, …)`                                                                                                         | `SelectionHandler.canBrushRawRender()`              |
| `SCRIBBLE_ERASE_PROVIDER` (eraser tool)               | 8 for type 0/1, 5 for type 2                   | `(true, 8 or 5)`                                                                                                    | `EraseHandler.canBrushRawRender()` = true for 0/1/2 |
| `SCRIBBLE_NORMAL_SHAPE_PROVIDER`                      | unchanged (colour only)                        | as eraser type                                                                                                      | `false` (BaseHandler default)                       |

The same resume also sends `setStrokeParameters(5, [5.0])` and `setStrokeParameters(8, [eraserWidth, 0.5,
0.1])` **[code]** (`ResumeRawDrawingRequest.i()`, `:52-55`). The 0.5 and 0.1 match
`ResumeRawDrawingRequest.ERASE_PEN_OPACITY` and `ERASE_PEN_BLACK_OPACITY` **[code]**. What SF does with
style 8 is **[unknown]**. A previous attempt to copy it in Tangleaf produced a pale sheet on device
(`pen-research-handoff.md`), which suggests style 8 depends on the rest of this sequence, but that is
**[inferred]**.

---

## 8. Device-specific branches on the Note Air 4C

| branch                                                                                    | where                                                                                                                                                                                                       | value on Note Air 4C and how determined                                                                                                                                                                                                                                                                                                                                                                                           |
| ----------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| device class                                                                              | `Device.a()`/`b()` (`N/sdk/device/Device.java:72-107`)                                                                                                                                                      | `SDMDevice`, from `ro.board.platform=lito` (README) **[inferred: code + recorded fact]**                                                                                                                                                                                                                                                                                                                                          |
| `isColorDevice()`                                                                         | `DeviceInfoUtil.isColorDevice()` = `SDMDevice.getColorType() > 0` → `ViewUpdateHelper.getColorType()` → SF `GET_COLOR_TYPE` (`N/sdk/utils/DeviceInfoUtil.java:136-138`, `FW/ViewUpdateHelper.java:648-655`) | colour: 08 finds `isHWTconColorDevice` true on this build **[inferred]**. Every resume delay is then 500 ms (`RawPenArgs.f`), popup resume 500 ms                                                                                                                                                                                                                                                                                 |
| `ColorDeviceUtils.refreshScreen`                                                          | `N/note/note/utils/ColorDeviceUtils.java`                                                                                                                                                                   | colour only: `InvalidateScreenAction` with `setPauseRawDrawingRender(false)`, i.e. an input-only pause around a refresh. No callers found in the app (grep) **[code]**                                                                                                                                                                                                                                                            |
| per-model config                                                                          | `DeviceConfig.readConfig` → raw resource `note_` + `Build.MODEL.lower()` (`N/sdk/note/ui/config/DeviceConfig.java:27`, `:110-134`)                                                                          | `res/raw/note_noteair4c.json`: `isTabDevice:true`, `showPenUpRefresh:false`, `canDisableCTPTouch:true`, `overlayEraserBufferTimeMs:30`, `strokeEraserBufferTimeMs:20`, `epdTurbo:5`, `loadingShapePointThreshold:20000`, `largeObjectSize:0`. Defaults kept: `canPenTipErase=true`, `supportPenHapticType=false`, `eraserAreaDisplayDelayMillSecond=0` **[code]**. `Build.MODEL = NoteAir4C` comes from the README **[inferred]** |
| `TouchHelper.create(view, rawInputCallback)` (feature from `DeviceFeatureUtil.hasStylus`) | only used by `HandWritingHandler` (rich-text handwriting)                                                                                                                                                   | not the note canvas; the canvas always uses mask 3 **[code]**. It is still a second pen session in the same pid (4.7). Mask 2 (SF only) if `hasStylus` is true on this device **[inferred, `hasStylus` not traced]**                                                                                                                                                                                                              |
| `isEnableColorCU()` per handler                                                           | `ScribbleHandler.onActivate/onDeactivate`, erase begin/points (`ScribbleHandler.a()`, `l0()`)                                                                                                               | not device-gated in the app: `ENABLE_COLOR_CU` (1048693) is sent on every device; what SF does with it on a mono panel or on this Kaleido panel is **[unknown]** **[code for the calls]**                                                                                                                                                                                                                                         |
| `eraserAreaDisplayDelayMillSecond`                                                        | `EraseFinishAction.E()` delay before posting `EraseFinishEvent`                                                                                                                                             | 0 on this model (default, not in `note_noteair4c.json`) **[code + config file as recorded above]**                                                                                                                                                                                                                                                                                                                                |
| pen haptics                                                                               | `UpdatePenHapticTypeAction` filters on `supportPenHapticType`                                                                                                                                               | false → never runs **[code]**                                                                                                                                                                                                                                                                                                                                                                                                     |
| `setEnablePenSideButton` / `enableSideBtnErase`                                           | `SFTouchRender.enableSideBtnErase` → `Device.setEnablePenSideButton`                                                                                                                                        | never called by the app (grep) **[code]**. The stylus has no side button anyway                                                                                                                                                                                                                                                                                                                                                   |
| eraser end of the stylus                                                                  | native reader reports `erasing` → `onBeginRawErasing` (`RawInputReader.onTouchPointReceived`, `:538-571`)                                                                                                   | handled in every `ScribbleHandler` through `a()` → `EraseBeginAction` (render kept on for types 0/1/2) **[code]**. The native eraser detection itself is **[unknown]**                                                                                                                                                                                                                                                            |
| `supportEAC:false` for stock (firmware config)                                            | not read by the pen-session code                                                                                                                                                                            | irrelevant to this subsystem **[code, by grep: no EAC reads in the pen path]**; see the refresh subsystem                                                                                                                                                                                                                                                                                                                         |
| multi-window on Rockchip                                                                  | `RenderToEditorViewAction.r()` checks `DeviceUtils.isRk32xxDevice()`                                                                                                                                        | false on `lito` **[inferred]**                                                                                                                                                                                                                                                                                                                                                                                                    |
| pen-up refresh                                                                            | `setPenUpRefreshTimeMs(OnyxSystemConfig.penUpRefreshTimeMs)` (default 500, `SystemConfigBean:21`); reader's `penUpRefreshEnabled` default true                                                              | `onPenUpRefresh` fires 500 ms after lift → `PenUpRefreshEvent` → `EpdShapeHandler.onPenUpRefresh`; its action is the writing/refresh subsystem's                                                                                                                                                                                                                                                                                  |

---

## 9. Interactions with other subsystems

- **Writing.** The session must be in D for firmware ink. Strokes arrive via `RawInputCallback` →
  `NoteManager` → EventBus (`BeginRawDrawEvent`, `RawDrawingPointsReceivedEvent`, `EndRawDrawingEvent`,
  `NoteManager.java:449-509`), are handled by the active handler, and are committed by
  `AddShapesBackgroundRequest` without a pen pause **[code]**. Stroke style, width and colour are
  re-pushed on every resume, so a style change made while paused takes effect at the next resume
  **[code]**.
- **Erasing.** The render flag stays on throughout an eraser stroke (types 0/1/2). The firmware draws the
  eraser trail itself (`setEraserRawDrawingEnabled(true, 8/5)` pushed at resume). After the stroke comes
  the render-only pause, an invalidate that paints the erased result through ordinary composition, and a
  gated resume after 300 ms **[code]**. The only pen-state change is that post-erase pause. Before it,
  still in DRAWING, `EraseFinishAction.render()` sends `ENABLE_POST(-1,1,pid)` via
  `InkRenderManager.invalidate(DisplayArgs(enablePost=true))`, and the stroke is bracketed by
  `ENABLE_COLOR_CU(1)`/`(0)` (summary item 7) **[code]**.
- **Selection / lasso.** A lasso is drawn as raw ink with style 5 while render is on. A rectangle
  selection keeps both off. Transforms use requests that opt out of the implicit pause. Pause and resume
  go through the named registry **[code]**.
- **Refresh.** Every INV does a `View.invalidate()` **while the render flag is off**, i.e. after SF has
  left the handwriting schema (`stopHandwriting` on PAUSE, per 08). App frames therefore reach the
  panel through normal composition. `InkRenderManager.invalidate` can wrap it in a one-shot update mode
  (`REGAL`/`REGAL_PLUS`, `EpdDeviceManager.setUpdateMode/resetUpdateMode`) and can add an extra
  `enablePost(1)` when `DisplayArgs.enablePost` is set
  (`N/sdk/notecore/editor/display/InkRenderManager.java:44-94`) **[code]**. Page turns add
  `applyGCWithInterval` **[code]**.
- **Input routing.** `ScribbleTouchDistributor` drops all touches without window focus and routes finger
  touches to gestures. Hover posts `PenActiveEvent`/`PenDeactivateEvent` for palm rejection **[code]**.
  The session's input flag gates only the native reader's callbacks, not Android `MotionEvent`s **[code]**.
- **Firmware.** Region state is global in SF and outlives the app. Stock never clears it and only
  overwrites it on each resume. Pen state and posting are per-pid, and the last writer owns the SF pen
  slot (08) **[code per 08]**.

---

## 10. Open questions as hardware experiments

All of these need `adb logcat -b all -v time` with the driver lines (`onyx_tcon_send_cmd`,
`onyx_tcon_set_upd_scheme` / `HWEpdcManager: ### EUS`, `SurfaceFlinger: Pid N State change to: …`),
plus the stock app's own `PenLogger` and `Debug` lines where noted. Get the stock pid with
`pidof com.onyx.android.note`.

| id  | question                                                                                          | action on the device (stock Notes)                                                                                                                                                                                                                        | confirms if …                                                                                                                          | refutes if …                                                             |
| --- | ------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| E1  | Open sequence (4.2): is the session parked in PAUSE right after bind, and when does it first run? | Open a note; do not touch it for 3 s; then draw one stroke                                                                                                                                                                                                | `State change to: … toPause` for the stock pid on open, then `toResume`/`running` about 0.5-0.6 s after the open completes             | no `toPause` at open, or `running` before the open finishes              |
| E2  | Stylus-eraser stroke keeps render on                                                              | With the pen tool active, erase a stroke with the eraser end, lift, wait 2 s                                                                                                                                                                              | no `toPause` while erasing; one `toPause` right after lift, `running` ~300-800 ms later; **no** `EUS, s 4` and no `update_mode[1]`     | `toPause` at eraser-down, or scheme 4 during the erase                   |
| E3  | Does pen input in the post-erase window (state R) trigger the flash in stock?                     | Erase with the eraser end, and put the pen tip down again **within 200 ms** of lift and draw                                                                                                                                                              | `EUS, s 4` + `waveform[4]` during the second stroke, then `update_mode[1]` ~1.5 s after lift (stock flashes too)                       | no scheme switch: stock has another guard; look for the extra calls      |
| E4  | Fill-colour tool leaves the session in R (PAUSE with input)                                       | Select fill colour, tap and drag with the pen inside a closed shape                                                                                                                                                                                       | pen state stays `toPause`/paused, scheme 4 appears on pen contact, GC16 ~1.5 s after lift                                              | `running` while the tool is active                                       |
| E5  | Popup show/dismiss sequence                                                                       | Open the pen-width popup, wait 2 s, dismiss it                                                                                                                                                                                                            | `toPause` on show; on dismiss another pause (INV) then `running` ~500 ms later                                                         | `running` without a preceding pause after dismiss                        |
| E6  | Window focus loss and app switch                                                                  | Press recents, return; separately open a system dialog (for example the brightness panel) and close it                                                                                                                                                    | `toPause` on focus loss, no `running` until focus returns; after return `running` ~500 ms later                                        | `running` while unfocused                                                |
| E7  | What does the stock exclude push `[Rect(0,0,0,0)]` + view offset do in SF?                        | In stock, after any resume, draw a stroke that starts exactly at the `EditorView` top-left corner (just below the toolbar), with the widest pen. If a way to dump the SF `PenManager` exclude table is found (root; 08 names `applySFDebug`), dump it too | ink is missing in a small box of about half the stroke width at the view origin (a degenerate rect was stored and inflated)            | ink starts normally at the corner (the one-rect push behaves as a reset) |
| E8  | Is `PenEvent.pauseDrawingRender()` truly a no-op? (5.5)                                           | Switch to a geometric shape tool (line/circle)                                                                                                                                                                                                            | only the INV pause/resume of the tool switch; no extra `toPause` from `NormalShapeHandler.onActivate`                                  | an extra `toPause` without a following `running`                         |
| E9  | Screen off/on                                                                                     | With a note open, press power, wait 5 s, press power, unlock                                                                                                                                                                                              | pause on keyguard focus loss (`toPause`), resume after unlock                                                                          | no pen-state lines at all (session stays DRAWING across screen off)      |
| E10 | Page turn                                                                                         | Turn a page with the page button or gesture                                                                                                                                                                                                               | `toPause`, GC per the refresh settings, then `running` ≥500 ms after the display                                                       | `running` without a pause                                                |
| E11 | Finger scroll on an expanded page or zoomed view                                                  | Zoom in, scroll with one finger, stop                                                                                                                                                                                                                     | no pen-state lines during the scroll; region transactions at the end (not visible in the driver log, so pair with `PenLogger`)         | `toPause` at scroll start                                                |
| E12 | Close sequence                                                                                    | Exit the note to the library                                                                                                                                                                                                                              | `toPause`, then STOP (`State change to: stop`), twice (reset, then `resetEpdPost`)                                                     | no `stop` line                                                           |
| E13 | Resume sleep blocks later pauses (5.1)                                                            | Dismiss a popup and within 200 ms open another popup                                                                                                                                                                                                      | `running` appears briefly and is followed by `toPause`, instead of the second pause cancelling the resume                              | no `running` between the two pauses                                      |
| E14 | Split screen unfocused bind leaves START with posting 0                                           | Put Notes in split screen with another app focused, open a note on the Notes side                                                                                                                                                                         | `State change to: start` for Notes without a following `toPause` until Notes gets focus                                                | `toPause` right after `start`                                            |
| E15 | Does the rich-text `HandWritingHandler` session (4.7) ever run alongside the note session?        | Open a note, open the rich-text editor (if HWR in rich text is enabled), write in it, close it, write on the canvas again                                                                                                                                 | a second `start`/`running` sequence for the same pid while the rich-text view is up, then the canvas session resumes                   | no pen-state lines while the rich-text view is up                        |
| E16 | Does leaving Notes pause through the `onPause` save before the focus loss arrives?                | With a note open and the pen tool active, press Home                                                                                                                                                                                                      | a `toPause` from the save path (pair with the stock `PenLogger` "pause" call stack of `PauseRawPenAction`) and no `running` afterwards | `running` after the save while the app is in the background              |
| E17 | Eraser tool activation resumes after 150 ms, not 500 ms                                           | Switch from pen to the eraser tool in the toolbar                                                                                                                                                                                                         | `toPause` then `running` about 150 ms later (plus INV timing if one is also issued)                                                    | only a ~500 ms gap                                                       |
| E18 | Effect of the post-erase `ENABLE_POST(1)` sent while DRAWING (`EraseFinishAction.render`)         | Erase with the eraser end on a stroke, lift, do not touch for 2 s; record `onyx_tcon_send_cmd` lines between lift and the `toPause`                                                                                                                       | panel updates for the erased area appear **before** the `toPause`, i.e. while the session is still DRAWING                             | the erased area updates only after `toPause`                             |

For every experiment, run the same action in Tangleaf and compare the two sequences line by line, as
planned in `pen-research-handoff.md`.

---

## Verification notes

An independent pass re-read the cited sources (bundled SDK and app under
`~/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/`, framework `ViewUpdateHelper`) and grepped
the whole decompiled app for the session entry points (`openRawDrawing`, `closeRawDrawing`,
`restartRawDrawing`, `setRawDrawingEnabled`, `setRawDrawingRenderEnabled`, `setRawInputReaderEnable`,
`forceSetRawDrawingEnabled`, `bindPenHostView`, `TouchHelper.create`, `setScreenHandWritingPenState`,
`enablePost`, `leaveScribbleMode`, `resetEpdPost`, `setScreenHandWritingRegion*`,
`set{Brush,Eraser}RawDrawingEnabled`, `setStrokeParameters`) and for every pause/resume producer
(`PauseRawPenAction`, `PauseRawPenRenderAction`, `PauseResumeRawPenAction`, `ResumeRawPenAction`,
`PenEvent.*`, `PausePenEvent`, `ResumePenEvent`, `RawPenPauseEvent`, `InvalidateScreenAction`,
`resumePen*`; 107 files).

Confirmed as written (re-read in source):

- `NoteManager.bindPenHostView` (mask 3, `touchListenerEnabled=false`, open, park when `windowFocused`) and
  `reset()`; `TouchHelper` constructor order (SF render, then App render), `openRawDrawing`,
  `forceSetRawDrawingEnabled`, dedupe in `setRawDrawingRenderEnabled`/`setRawInputReaderEnable`.
- `SFTouchRender.openDrawing/closeDrawing/setDrawingRenderEnabled/setInputReaderEnable` (`j/c/g/h/i/k/l`),
  `EpdPenManager` pen states 1/2/3/0, `AppTouchRender.openDrawing` (`ENABLE_POST(0)` + START) and its
  render-off `ENABLE_POST(1)`, `closeDrawing` (style 0, `ENABLE_POST(1)`, STOP).
- `RawInputManager.setHostView` limit push; `RawInputReader.setExcludeRect/setLimitRect` skip empty lists;
  `pause/resume/quit` native calls and `reportData`.
- `SDMDevice` reflective bindings and `ViewUpdateHelper` parcels (`enablePost` writes `-1,n,pid`;
  `resetEpdPost` = `ENABLE_POST(-1,1,pid)` + pen state 0; region calls add `getLocationOnScreen`;
  pen state carries the pid, regions do not). `Device` maps `lito` to `SDMDevice`.
- `ResumeRawDrawingRequest.execute` and helpers (sleep, `o()`, `n()`, `i()`, `k()`, `h()`, input then
  render), `PenEventHandler` predicates `A()`/`z()` and `q()`/`s()`, `RawPenArgs` factories and
  `f = isColorDevice ? 500 : 150`, `PenEvent` constants and the `pauseDrawingRender` no-op,
  `PauseRawPenAction.n()`, `PauseResumeRawPenAction`, `ResumeRawPenAction`,
  `InvalidateViewWithPenControlAction` order, `BaseNoteRequest.b()` defaults (pause both),
  `EraseUtils.isEraseTrackByRawRender` (types 0/1/2), `EraseBeginAction` skip,
  `EraseFinishAction.E()` (render-only pause, resume 300 ms), `ScribbleActivity.onWindowFocusChanged`,
  `ScribbleTouchDistributor` focus check (`:375`), `TranslateViewPortRequest` and
  `SavePullShapeListRequest` direct setters, `note_noteair4c.json` values (the file also contains
  `showReadNote: true`, not listed in section 8).

Corrected in place:

1. Section 4.5: "A pause issued while already paused therefore emits nothing" was wrong for full
   pauses. `TouchHelper.setRawDrawingEnabled` always calls `resetPenDefaultRawDrawing()` after the
   `created` check, so every both-flags pause emits brush(1) + eraser(0,5) even when already paused.
2. Summary item 2 and section 4.3: `ScribbleMainFragment.l0()` / `B()` (`OpenDocumentStatusEvent` and the
   trailing `PenEvent.resumeRawDrawing()`) run only on the switch-to-another-document path
   (`onReopenDocumentRequest`), not on the first open. The first open binds through
   `RestoreNoteZoomAction` with a view or `RenderToEditorViewAction`; `RenderScribbleContentAction`'s
   `RestoreNoteZoomAction` has no view and never binds.
3. Summary item 10: "Activity lifecycle hooks do not touch the pen" was incomplete.
   `ScribbleMainFragment.onPause()` runs a `SaveDocumentAction`, whose `SaveDocumentRequest` pauses the pen
   implicitly and which ends with `PenEvent.resumeRawDrawing()`.
4. Summary item 6: counts changed from "142 classes / about 25 opt out" to what grep shows (101 direct
   `extends BaseNoteRequest`, 28 `setPauseRawDraw(false)` sites).
5. Section 7.2 tool-switch row: the eraser tool's entry is `EraseHandler.onActivate` →
   `PauseResumeRawPenAction` with a 150 ms resume delay; the previous row claimed a plain INV without
   citing it. Downgraded the INV part to "not re-traced".
6. Section 4.6: the close sequence has a third `resetEpdPost()` source (`EpdEventHandler.destroy`) and
   `PenEventHandler.destroy` resets all flags.

Added:

- Section 4.7 and rows in 7.2/8: a second `TouchHelper` in the same process (`HandWritingHandler`,
  rich-text handwriting), with its own open/pause/resume/close and a `resetPenDefaultRawDrawing` on every
  enable/disable.
- Summary item 5 and 7.2 rows: `InvalidateScreenAction` is a no-op before the document is opened and
  whenever the active handler is not a `ScribbleHandler` (HWR family, `RichTextHandler`);
  `SelectionHandler` drops it while a selection exists.
- Summary item 7 and 7.2 erase rows: `ENABLE_COLOR_CU(1/0)` around each eraser stroke, and
  `ENABLE_POST(1)` sent by `EraseFinishAction.render()` while the session is still DRAWING, before the
  render-only pause.
- Section 5.4: `t(..., 7, ...)` default-mask behaviour, handler-specific pen calls (`EraseHandler` 150 ms,
  `UniversalShapeHandler` immediate resume, `NormalShapeHandler`, `ZoomSelectionHandler` 800 ms),
  `EpdEventHandler` fast-mode resumes and its 5 s pause-resume, colour-CU handlers, meeting
  `EnablePostAction`.
- Section 5.2: `ResumeRawDrawingRequest` is not a `BaseNoteRequest`; a save that runs
  `SaveDocumentRequest` pauses the pen and ends with a gated resume.
- Experiments E15-E18.

Not confirmed / remaining doubts:

- Everything tagged "per 08" about SF internals (`stopHandwriting`, `androidDrawing`, scheme changes) was
  not re-verified against the SurfaceFlinger binary in this pass.
- Whether `ErasingShapeAction`'s mid-stroke `DisplayArgs(enablePost=true)` actually reaches
  `InkRenderManager.invalidate` (it is passed into `ForceRenderNoteDirtyArgs`; the render path through
  `RenderManager.forceRenderDirtyRect` was not followed to the end).
- `DeviceFeatureUtil.hasStylus` on this device (decides mask 2 vs 1 for the rich-text session) was not
  traced.
- Which binder (layout-driven `RenderToEditorViewAction` or `ScribbleFragment.s1()`) runs first on a cold
  open, and who posts `AttachHostViewEvent` (no `new AttachHostViewEvent` in the jadx output).
- Whether a tool switch between two `ScribbleHandler`s also issues an INV in addition to the handler's
  own `onActivate` calls (the `activeProvider` callers, e.g. `TogglePenTypeAction`, were not all re-read).
- The 107 producer files were listed but not every one was opened; rows in 7.2 that the first draft took
  from them and that are not named above were not individually re-checked.
- The second decompilation (`~/tmp_zfs/reversed_onyx_notes_app/src`) was not compared in this pass.
