# Notable — pen and panel path, described in the stock documents' terms

Written 2026-09-25 from static reading of Notable (`com.ethran.notable`) at commit `6da250e`
(2026-08-08, "Merge pull request #310 from Ethran/sync") in `~/tmp_zfs/reference_notes_apps/notable`,
of the Onyx SDK it links, and of the extracted framework and SurfaceFlinger. No device was touched.
The purpose is comparison, action by action, with the stock Notes documents in
[`../stock-notes/`](../stock-notes/00-overview.md) (especially 07 for the SF side) and with
Tangleaf ([08](../stock-notes/08-tangleaf-current-path.md)). Nothing here proposes a change to
Tangleaf.

## 0. Notation, sources, build and SDK identity

Tags:

- **[code]** read directly in the cited source.
- **[inferred]** reasoned from code that was read, not read directly.
- **[doc]** taken from an earlier document in `docs/onyx-reversing/`, not re-verified here.
- **[unknown]** not determinable statically.

Path shorthands:

| shorthand | root                                                                                      |
| --------- | ----------------------------------------------------------------------------------------- |
| `nb/`     | `~/tmp_zfs/reference_notes_apps/notable/app/src/main/java/com/ethran/notable/`            |
| `pen/`    | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-pen-1.5.4.3/sources/com/onyx/android/sdk/pen/` |
| `dev/`    | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-device-1.3.5.2/sources/com/onyx/android/sdk/`  |
| `fw/`     | `~/tmp_zfs/onyx_framework/out/sources/android/onyx/` (framework.jar)                      |
| `fwv/`    | `~/tmp_zfs/onyx_framework/out/sources/android/view/` (framework.jar)                      |

**Build identity.** The tablet runs firmware 4.2, `ro.build.display.id =
2026-04-28_17-50_4.2-rel_04282_555977efe`, system partition Android 13 (SDK 33), vendor partition
Android 11 (SDK 30, hence the `:11/` in Onyx fingerprints), `vendor.onyx.tablet=true`. The device's
`/system/bin/surfaceflinger` and `/system/framework/framework.jar` are byte-identical to the
extracted copies, so every SF and framework statement from 07 applies to this device [verified on
the device by the coordinator, 2026-09-25].

**SDK actually linked.** `gradle/libs.versions.toml:38-40` declares `onyxsdk-base 1.8.5.2`,
`onyxsdk-device 1.3.5.2`, `onyxsdk-pen 1.5.4.1`; `app/build.gradle:151-169` pulls all three plus
`hiddenapibypass` [code]. The pen 1.5.4.1 POM (fetched from `repo.boox.com`) depends on base 1.8.5.1,
and the base 1.8.5.2 POM on device 1.3.5.2, so Gradle resolves pen 1.5.4.1, base 1.8.5.2,
device 1.3.5.2 [code: POMs]. Device 1.3.5.2 is the decompiled copy used here. Pen 1.5.4.1 is not
decompiled locally; its AAR was fetched and compared with 1.5.4.3 by `javap -c -p`:
`SFTouchRender`, `EpdPenManager`, `RawInputManager`, `RawInputReader`, `RawInputCallback` are
bytecode-identical; `TouchHelper` differs only in a log line in `printTouchInfo`; the arm64
`libonyx_pen_touch_reader.so` has the same MD5 (`d489e2da…`) [code]. So `pen/` citations (1.5.4.3)
describe what Notable runs. Base 1.8.5.2 was not compared with the decompiled 1.8.5.4; only
`DeviceFeatureUtil.hasStylus` and `DeviceInfoUtil.isColorDevice` are used from it [inferred same].
This differs from stock (bundled SDK copy) and from Tangleaf (pen 1.5.4.3) [doc].

**Device class.** `SDMDevice` on `lito` [doc: 00 §0], reached by reflection on
`android.onyx.ViewUpdateHelper`, synchronous Binder transactions to SF [doc: 08 §0].

**Notable's own notes.** Code comments cite `docs/onyx-sdk/*.md` (for example
`nb/editor/canvas/CanvasRefreshManager.kt:78`); that directory does not exist in this checkout
[code: `ls docs`]. The comments' claims (for example "a bare unlockCanvasAndPost is swallowed")
are the authors' device observations, not verified here.

## 1. Summary

1. **One `SFTouchRender`, no app render.** `TouchHelper.create(drawCanvas, inputCallback)`
   (`nb/editor/canvas/OnyxInputHandler.kt:63-74`) uses the two-argument constructor, which picks
   feature mask `hasStylus ? 2 : 1` (`pen/TouchHelper.java:45-47`) [code]. `hasStylus` matches input
   device names containing `onyx_emp`, `Wacom I2C Digitizer` or `hanvon_tp`
   (`onyxsdk-base…/DeviceFeatureUtil.java:20, 54-76`) [code]; this device's digitizer is
   `onyx_emp_Wacom I2C Digitizer` [doc: `docs/planning/handwriting-prototype.md:145`]. So the mask is
   2: **SF render only**, no `AppTouchRender` [inferred]. Every pen-session call maps to exactly one
   set of SF transactions, without stock's and Tangleaf's doubled `AppTouchRender` side effects.
2. **The host view is a `SurfaceView`** (`nb/editor/canvas/DrawCanvas.kt:24-30`) drawn with
   `holder.lockCanvas(dirty)` / `unlockCanvasAndPost` [code]. On this firmware
   `SurfaceHolder.lockCanvas(rect)` calls `Surface.addEpdc(rect, SurfaceView.getDefaultUpdateMode())`
   before locking (`fwv/SurfaceView.java:110-124`) [code]. Notable sets that default mode to
   `HAND_WRITING_REPAINT_MODE` (524290 = `0x80002`) on every surface setup
   (`nb/editor/utils/einkHelper.kt:119-124`) [code]. So each canvas post carries a **dirty-rect**
   update entry with exactly the mode SF lets through while in handwriting schema 7 (07 §4.5)
   [inferred for the SF commit].
3. **But the first erase resets that mode to 5.** `drawBitmapToSurfaceSync`, used by every erase
   commit, calls `EpdController.resetViewUpdateMode(drawCanvas)` → `View.resetUpdateMode()` =
   `defaultUpdateMode = 5` (`nb/editor/canvas/CanvasRefreshManager.kt:131`,
   `dev/device/SDMDevice.java:1954-1956`, `fwv/View.java:13718-13720`) [code]. Nothing sets 524290
   again until the next `updateActiveSurface` (surface created or resized, toolbar toggled). From the
   first erase on, canvas posts carry mode 5 (AUTO), which schema 7 drops [inferred from 07 §4.5].
   Below, "SV mode H" means 524290 and "SV mode A" means 5.
4. **Render stays on while the pen writes and erases.** Pen tip, eraser tool, eraser end and lasso
   all run with SF pen state 2. The eraser end draws through SF's eraser reader with style 8 (or 5),
   enabled by `setEraserRawDrawingEnabled(true, 8)` after every `setRawDrawingEnabled(true)`
   (`nb/editor/utils/einkHelper.kt:236-259`) [code]. No pause at pen-down in any tool [code].
5. **Erase commit is stock-shaped.** After an erase: in-process input off, lock the dirty rect,
   `ENABLE_POST(0)`, `ENABLE_POST(1)` (leaves schema 7), draw and post the erased bitmap, then
   `setRawDrawingEnabled(false)` (pen state 3), wait 150 ms (pen/marker eraser) or 500 ms (lasso
   eraser, scribble-to-erase), `setRawDrawingEnabled(true)` (pen state 2) with a full re-push
   (`nb/editor/canvas/CanvasRefreshManager.kt:82-136`) [code]. Stock: `ENABLE_POST(1)`, render-only
   pause, whole-view invalidate, RS(300) (00 §3.3) [doc].
6. **Every other canvas refresh pauses render for 500 ms.** `refreshUi` / `refreshUiSuspend` post the
   bitmap and call `resetScreenFreeze`: render off (`ENABLE_POST(1)`, pen state 3), 500 ms on a colour
   device, render on (pen state 2); input stays on; overlapping calls coalesce into one resume
   (`nb/editor/utils/einkHelper.kt:320-346`, `:452-467`) [code]. This runs after tool switch, undo/redo,
   lasso without a hit, each scroll/zoom step, page change, menu-pen change, `onPause`. It does **not**
   run after an ordinary pen stroke.
7. **Transient A2 is Notable's own and ends with a GC.** `EpdRefreshArbiter` applies
   `applyTransientUpdate(ANIMATION_X)` = `UI_X_A2_MODE` `0x1000004` (fast) when the first holder
   acquires it and `clearTransientUpdate(true)` when the last releases
   (`nb/editor/utils/EpdRefreshArbiter.kt:62-89`) [code]. Holders: finger scroll/pan/hold-select
   gestures (released 500 ms after lift), a shown selection (released on deselect), library and
   page-list scrolling. In SF the clear with flag 1 queues a one-shot GC (mode 98) plus the full-screen
   mode-5 update (07 §5.3) [doc]: **a full GC16 flash after every finger scroll/zoom and every
   deselect**, by design.
8. **Session lifetime.** The session is closed and reopened only by `setupSurface` (surface created,
   resized, toolbar toggled). Leaving the note or backgrounding pauses (pen state 3) but never sends
   pen state 0: the only `closeRawDrawing` on teardown is guarded by
   `referencedSurfaceView == DrawCanvas.hashCode()`, while `referencedSurfaceView` holds
   `OnyxInputHandler.hashCode()` (`nb/editor/canvas/OnyxInputHandler.kt:66`,
   `nb/editor/canvas/DrawCanvas.kt:21, 141-143`), so it never runs [code].
9. **Regions.** Limit = the view minus a 40 dp toolbar strip, exclude = that strip, pushed only at
   surface setup (`nb/editor/utils/einkHelper.kt:139-177`,
   `nb/editor/canvas/OnyxInputHandler.kt:208-227`) [code]. With the toolbar hidden the exclude is a
   zero-height rect, still sent because the list is non-empty (`pen/RawInputReader.java:418-425`)
   [code]. Region mode is never set [code: grep].
10. **Never called:** `setEpdTurbo`, `setDisplayScheme`, `applyGCOnce`, `byPass`, `ENABLE_COLOR_CU`,
    `handwritingRepaint`, `setPenUpRefreshTimeMs`, `setPostInputEvent`, `enableSideBtnErase`,
    `setAppCTPDisableRegion` [code: grep over `nb/`]. `prepareForPartialUpdate` (display scheme,
    `enableA2ForSpecificView`, turbo 100) and `restoreDefaults` exist but have no caller
    (`nb/editor/utils/einkHelper.kt:282-289, 310-312`) [code].

## 2. Central classes and entry points

| class / file                                 | role                                                           | key entry points                                                                                                                                                                             |
| -------------------------------------------- | -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `nb/editor/canvas/DrawCanvas.kt`             | `SurfaceView` host; stylus MotionEvent sink; surface callbacks | `dispatchTouchEvent` :41-67, `init` :102-148 (`surfaceCreated` :109-118, `surfaceChanged` :120-132, `surfaceDestroyed` :134-145)                                                             |
| `nb/editor/canvas/OnyxInputHandler.kt`       | `TouchHelper` owner, raw callbacks, stroke style per tool      | `touchHelper` :63-74, callbacks :77-127, `updatePenAndStroke` :129-152, `updateIsDrawing` :186-206, `updateActiveSurface` :208-227, `onRawDrawingList` :228-354, `onRawErasingList` :356-388 |
| `nb/editor/canvas/CanvasRefreshManager.kt`   | posts the page bitmap to the surface; erase commit             | `refreshUi` :33-49, `refreshUiSuspend` :51-71, `commitErase` :82-106, `drawBitmapToSurfaceSync` :110-136, `drawCanvasToView` :138-186, `restoreCanvas` :189-208                              |
| `nb/editor/utils/einkHelper.kt`              | all `EpdController` wrappers, session setup, freeze/resume     | `onSurfaceInit` :119-124, `setupSurface` :139-177, `enableNativeEraser` :236-259, `refreshScreen` :306-308, `resetScreenFreeze` :333-346, `delayBeforeResumingDrawing` :452-467              |
| `nb/editor/utils/EpdRefreshArbiter.kt`       | reference-counted transient A2                                 | `acquire` :62-69, `setAnimationMode` :81-89                                                                                                                                                  |
| `nb/editor/canvas/CanvasObserverRegistry.kt` | flows → refresh and pen-state actions                          | `observeRefreshUiImmediately` :97-120, `observeForceUpdate` :122-140, `observeFocusChange` :151-164, `observeIsDrawingSnapshot` :261-277, `observeToolbar` :279-292, `observeMode` :294-306  |
| `nb/editor/EditorViewModel.kt`               | `isDrawing` / `isDrawingAllowed`                               | `isDrawingAllowed` :106-109, `updateDrawingState` :520-530, `setSelectionActive` :757-768                                                                                                    |
| `nb/gestures/EditorGestureReceiver.kt`       | finger gestures (Compose), transient A2 for gestures           | stylus drop :106, hold-select :228, `applyGestureMode` :439-463                                                                                                                              |
| `nb/editor/state/SelectionState.kt`          | selection; holds an arbiter handle                             | `holdRefresh` :74-77, `releaseRefresh` :80-83, `reset` :85-99                                                                                                                                |
| `nb/MainActivity.kt`                         | lifecycle → canvas bus                                         | `onRestart` :237-243, `onPause` :245-251, `onWindowFocusChanged` :253-262, `onResume` :264-270                                                                                               |

The editor composable stacks, bottom to top: `EditorGestureReceiver` (finger gestures), the
`DrawCanvas` `AndroidView`, the selected-bitmap overlay, scroll indicators, the toolbar
(`nb/editor/EditorView.kt:234-255`) [code]. The `SurfaceView` surface sits behind the window; all
Compose UI (toolbar, selection overlay, popups) is drawn in the main window or in popup windows
[inferred: no `setZOrderOnTop`, grep].

## 3. The primitives, traced to SurfaceFlinger

SF transaction names and codes as in 07 §2.2 [doc]. Mask 2 means one `SFTouchRender` per call.

| Notable call (site)                                                                                                                                                               | SDK chain                                                                                                                                                                                                                           | SF transactions, in order                                                                       | SF-visible effect                                                                                                                |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `TouchHelper.create(drawCanvas, cb)` (`OnyxInputHandler.kt:67`, forced at `DrawCanvas` construction by `CanvasRefreshManager(..., inputHandler.touchHelper)`, `DrawCanvas.kt:93`) | ctor `pen/TouchHelper.java:358-375`; `SFTouchRender.bindHostView` → `RawInputManager.setHostView` → `setLimitRect(view.getLocalVisibleRect())` (`pen/RawInputManager.java:141-147`)                                                 | `SET_SCREEN_HANDWRITING_REGION_LIMIT` = local visible rect of a view not yet laid out           | limit replaced; value probably empty at construction [inferred]                                                                  |
| `setRawDrawingEnabled(true)` (`einkHelper.kt:169`, `OnyxInputHandler.kt:190`, `CanvasRefreshManager.kt:98`)                                                                       | no-op unless opened; render(true) if cached off, input(true) if cached off, then `resetPenDefaultRawDrawing` (`pen/TouchHelper.java:170-178, 348-351`)                                                                              | `PEN_STATE(2)` (if render was off); `SET_BRUSH_RAW(1)`; `SET_ERASER_RAW(0, 5)`                  | reader running; tip → reader 2, eraser end → no-op reader 3 until the next `enableNativeEraser` [doc 07 §4.2]                    |
| `setRawDrawingEnabled(false)` (`einkHelper.kt:135, 145`, `OnyxInputHandler.kt:204`, `CanvasRefreshManager.kt:92`)                                                                 | render(false) if cached on: `leaveScribbleMode` + `pauseDrawing` (`pen/touch/SFTouchRender.java:195-198`); input(false) if cached on: in-process `nativePausePen`; reset defaults                                                   | `ENABLE_POST(1)`; `PEN_STATE(3)` (if render was on); `SET_BRUSH_RAW(1)`; `SET_ERASER_RAW(0, 5)` | reader paused, schema 7 left (`Post enabled!`, `EUS s 2` or `s 4`)                                                               |
| `isRawDrawingRenderEnabled = false / true` (`resetScreenFreeze`, `einkHelper.kt:342-344`)                                                                                         | guarded render switch only                                                                                                                                                                                                          | `ENABLE_POST(1)`, `PEN_STATE(3)` / `PEN_STATE(2)`                                               | pause/resume; brush/eraser flags and regions untouched                                                                           |
| `setRawInputReaderEnable(false/true)` (`CanvasRefreshManager.kt:85, 101`)                                                                                                         | in-process reader only (`pen/RawInputReader.java:328-339`)                                                                                                                                                                          | none                                                                                            | callbacks stop; SF unaffected [doc 07 §2.2]                                                                                      |
| `closeRawDrawing()` (`einkHelper.kt:146`; `DrawCanvas.kt:142` unreachable)                                                                                                        | style 0, `leaveScribbleMode`, reader pause, `pauseDrawing`, reader quit, `quitDrawing` (`pen/touch/SFTouchRender.java:271-275, 182-192`); sends even when the helper was never opened (`pen/TouchHelper.java:162-168` has no guard) | `SET_STROKE_STYLE(0)`, `ENABLE_POST(1)`, `PEN_STATE(3)`, `PEN_STATE(0)`                         | STOP [doc 07 §4.1]. Cached render/input flags are **not** reset by close, only by the next open [code]                           |
| `openRawDrawing()` (`einkHelper.kt:167`)                                                                                                                                          | reset cached flags; style 0; reader start; `startDrawing` (`pen/TouchHelper.java:152-160`)                                                                                                                                          | `SET_STROKE_STYLE(0)`, `PEN_STATE(1)`                                                           | START; SF forwards no samples until 2 [doc 07 §3]                                                                                |
| `setLimitRect(listOf(limit))` (`einkHelper.kt:166`)                                                                                                                               | `RawInputReader.setLimitRect(List)` → native + `EpdController.setScreenHandWritingRegionLimit(view, Rect[])` (`pen/RawInputReader.java:538-543`)                                                                                    | `SET_SCREEN_HANDWRITING_REGION_LIMIT`, view rect offset to screen                               | limit = view minus toolbar strip                                                                                                 |
| `setExcludeRect(listOf(strip))` (`einkHelper.kt:166`)                                                                                                                             | returns early only if the list is empty (`pen/RawInputReader.java:418-425`)                                                                                                                                                         | `SET_SCREEN_HANDWRITING_REGION_EXCLUDE`, n = 1 (zero-height rect when the toolbar is hidden)    | exclude table = toolbar strip or one degenerate rect; stock also always keeps ≥ 1 rect (00 §5 item 17) [doc]                     |
| `setStrokeStyle / Width / Color` (`OnyxInputHandler.kt:137-150, 164-183`)                                                                                                         | `EpdPenManager.setStrokeStyle`; `RawInputReader.setStrokeWidth` → native + `EpdController.setStrokeWidth`; `EpdController.setStrokeColor` (`pen/touch/SFTouchRender.java:233-247`)                                                  | `SET_STROKE_STYLE`, `SET_STROKE_WIDTH`, `SET_STROKE_COLOR`, once each                           | SF painter parameters (global, one slot shared with the eraser reader) [doc 07 §4.2]                                             |
| `Device.setStrokeParameters(8, [30, 0.5, 1.0])` / `(5, [5, 9, 9, 0])` / `(7, [..., w≤10, 45])` (`einkHelper.kt:242-251, 270-280`, `OnyxInputHandler.kt:176-181`)                  | `SDMDevice.setStrokeParameters`                                                                                                                                                                                                     | `SET_STROKE_PARAMETERS`                                                                         | style 8 (soft eraser) width 30; style 5 dash; style 7 nib angle. Stock sends style 8 `[width, 0.5, 0.1]` [doc 07 §4.4]           |
| `setEraserRawDrawingEnabled(true, 8 or 5)` (`einkHelper.kt:244, 252`)                                                                                                             | `Device.setEraserRawDrawingEnabled`                                                                                                                                                                                                 | `SET_ERASER_RAW(1, 8/5)`                                                                        | eraser end → reader 4 draws with style 8/5 [doc 07 §4.2]                                                                         |
| `EpdController.enablePost(1)` (`einkHelper.kt:123`), `enablePost(view, 1)` (`:127`), `enablePost(0)`+`enablePost(1)` (`CanvasRefreshManager.kt:122-123`)                          | static `ViewUpdateHelper.enablePost(int)` (`dev/device/SDMDevice.java:3312-3320, 1769-1777`, `fw/ViewUpdateHelper.java:602-608`)                                                                                                    | `ENABLE_POST(-1, v, pid)`                                                                       | `v=1`: leaves schema 7 if active; `v=0`: never has an effect [doc 07 §4.4]                                                       |
| `EpdController.setViewDefaultUpdateMode(drawCanvas, HAND_WRITING_REPAINT_MODE)` (`einkHelper.kt:102-124`); fallback `REGAL`                                                       | reflected `View.setDefaultUpdateMode(524290)` (`dev/device/SDMDevice.java:1959-1961`; `fw/ViewUpdateHelper.java:77`)                                                                                                                | none at call time                                                                               | every later `lockCanvas(rect)` adds an entry `(rect, 524290)` (`fwv/SurfaceView.java:110-124`)                                   |
| `EpdController.resetViewUpdateMode(drawCanvas)` (`CanvasRefreshManager.kt:131`)                                                                                                   | `View.resetUpdateMode()` → 5 (`fwv/View.java:13718-13720`)                                                                                                                                                                          | none                                                                                            | later surface posts carry mode 5                                                                                                 |
| `EpdController.applyTransientUpdate(ANIMATION_X)` (`EpdRefreshArbiter.kt:83`)                                                                                                     | `SDMDevice.m86a(ANIMATION_X)` → `UI_X_A2_MODE` = `0x1000004` (`dev/device/SDMDevice.java:2968-2970, 3573-3620`; `fw/ViewUpdateHelper.java:100`)                                                                                     | `APPLY_TRANSIENT_UPDATE(0x1000004)`                                                             | transient slot fast → `EUS s 4` unless scheme is 3, where it is stored and taken at the next exit from handwriting [doc 07 §5.2] |
| `EpdController.clearTransientUpdate(true)` (`EpdRefreshArbiter.kt:86`)                                                                                                            | `ViewUpdateHelper.clearTransientUpdate(true)` → int 1 (`fw/ViewUpdateHelper.java:494-496`)                                                                                                                                          | `CLEAR_TRANSIENT_UPDATE(1)`                                                                     | if the slot is fast: `EUS s 2` after 100 ms, one-shot GC 98, full-screen mode-5 update; if not fast: nothing [doc 07 §5.3]       |
| `EpdController.repaintEveryThing(REGAL_PLUS)` (`einkHelper.kt:306-308`)                                                                                                           | `ViewUpdateHelper.repaintEverything(9)` (`dev/device/SDMDevice.java:3358-3360`, `fw/ViewUpdateHelper.java:1061-1065`)                                                                                                               | `REPAINT_EVERY_THING_WITH_MODE(9)`                                                              | one-shot entry 9, recomposite; nibble 9 is outside 1–6, so the HW-Tcon path logs `waveform_mode wrong!` [doc 07 §2.2, 00 §2.2]   |
| `EpdController.refreshScreenRegion(view, l, t, w, h, ANIMATION_MONO)` (`einkHelper.kt:291-304`, only from `restoreCanvas`)                                                        | reflected `View.refreshScreen(l,t,w,h, UI_MONO_A2_MODE 0x2000004)` → screen offset → `REFRESH_SCREEN` (`fwv/View.java:13404-13408`, `fw/ViewUpdateHelper.java:1036-1044`)                                                           | `REFRESH_SCREEN(x, y, w, h, 0x2000004)`                                                         | bounded one-shot refresh in A2 mono [doc: 08-surfaceflinger.md `handleRefreshScreen`]                                            |
| `EpdController.setAppScopeRefreshMode(NORMAL)` (`einkHelper.kt:39-42`, Welcome screen button only)                                                                                | `EInkHelper.setAppScopeRefreshMode(int)` → OEC service (`dev/device/SDMDevice.java:2725-2732`, `fw/optimization/EInkHelper.java:1469-1481`)                                                                                         | not SF (EinkWise/OEC store)                                                                     | persists the app's EAC refresh option [doc 05-panel-refresh-levers §3.3]                                                         |

Pen-state constants: 0 stop, 1 start, 2 drawing, 3 pause (`pen/EpdPenManager.java:9-13`) [code].
Notable never sends pen state 4 [code: grep].

## 4. Session state

### 4.1 App-side variables

| variable                                   | where                                    | meaning                                                                                 |
| ------------------------------------------ | ---------------------------------------- | --------------------------------------------------------------------------------------- |
| `TouchHelper` cached open / render / input | `pen/TouchHelper.java:34-40`             | guards every render/input switch                                                        |
| `toolbarState.isDrawing`                   | `nb/editor/EditorViewModel.kt:98`        | drives `updateIsDrawing` → `setRawDrawingEnabled` (`CanvasObserverRegistry.kt:261-277`) |
| `isDrawingAllowed`                         | `EditorViewModel.kt:106-109`             | no selection, no menu / stroke menu / background modal, no QuickNav                     |
| `screenFreezeResetJob`                     | `einkHelper.kt:323-346`                  | one pending 500 ms render resume                                                        |
| `EpdRefreshArbiter.liveHandles`            | `EpdRefreshArbiter.kt:35`                | transient A2 on while > 0                                                               |
| SurfaceView default update mode            | `View.defaultUpdateMode` of `DrawCanvas` | H (524290) after surface setup, A (5) after the first erase commit                      |
| `OnyxInputHandler.isErasing`               | `OnyxInputHandler.kt:56`                 | set at eraser-end begin; unused for routing on Onyx                                     |

### 4.2 States, in stock's notation (00 §2.1)

| state        | pen state | render | input | brush / eraser flags | when Notable is in it                                                                                                         |
| ------------ | --------- | ------ | ----- | -------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| D            | 2         | on     | on    | 1 / (1, 8 or 5)      | writing, erasing (all tools), lasso waiting — the normal state                                                                |
| F ("freeze") | 3         | off    | on    | unchanged            | the 500 ms after any `refreshUi` (`resetScreenFreeze`)                                                                        |
| C ("commit") | 3         | off    | off   | 1 / (0, 5)           | 150 ms or 500 ms after an erase                                                                                               |
| P            | 3         | off    | off   | 1 / (0, 5)           | `isDrawing` false: menu, popup, dialog, focus loss, QuickNav, shown selection, finger hold-select; also after surface destroy |
| O            | 1         | –      | –     | –                    | only transiently inside `setupSurface`                                                                                        |

Stock's R state (render off, input on) matches Notable's F in shape, but stock uses it for ~300 ms
after an erase; Notable uses it after every non-stroke refresh. Notable has no state with pen state
0 while the process lives, except transiently inside `setupSurface` [code].

## 5. The mechanism, action by action

"SF:" lists transactions in emission order. Log lines are the ones 07 and 10 document [doc].

### 5.1 Open a note

1. `EditorSurface` creates `DrawCanvas` (`nb/editor/ui/EditorSurface.kt:26-40`). Its constructor
   builds `OnyxInputHandler` and forces `touchHelper` (§3 row 1): SF limit = local visible rect of an
   unattached view [code; value inferred empty]. `init()` registers the surface callback;
   `registerObservers()` starts the flows [code].
2. `LaunchedEffect(Unit)` → `updateDrawingState()`: `isDrawing` is already `true` (default), so no
   pen action (`nb/editor/EditorView.kt:119-122`, `EditorViewModel.kt:520-530`) [code].
3. `surfaceCreated` → `updateActiveSurface()` (coroutine): `onSurfaceInit` → SV mode H,
   `ENABLE_POST(1)`; `setupSurface(toolbarHeight = 40 dp or 0)` (`einkHelper.kt:139-177`):
   - `debugLog(false)`; `setRawDrawingEnabled(false)` (no-op, not opened);
   - `closeRawDrawing()` → SF `STYLE(0)`, `ENABLE_POST(1)`, pen **3**, pen **0**;
   - limit = view minus strip, exclude = strip (n = 1);
   - `openRawDrawing()` → SF `STYLE(0)`, pen **1**;
   - `setRawDrawingEnabled(true)` → SF pen **2**, `BRUSH(1)`, `ERASER(0,5)`;
   - `enableNativeEraser(PEN)` → SF `STROKE_PARAMETERS(8, [30, 0.5, 1])`, `ERASER(1, 8)`;
   - then `updatePenAndStroke()` → SF style, width (× zoom), colour of the active pen
     (`OnyxInputHandler.kt:129-152`) [code].
4. `post { refreshUi(null) }` (`DrawCanvas.kt:117`): full-canvas lock (entry mode H), bitmap posted,
   `resetScreenFreeze` → SF `ENABLE_POST(1)`, pen **3**, 500 ms, pen **2** [code]. Page init also
   emits `refreshUiImmediately` and, once strokes load, `forceUpdate(null)` → `refreshUiSuspend`
   (`nb/editor/PageView.kt:198-201, 297-302`); each posts a full frame and re-arms the same 500 ms
   resume (coalesced) [code].
5. No GC, no transient mode, no update-scheme call on open [code].

Result: D with limit = view minus toolbar, exclude = strip, eraser end live, SV mode H [code].
Stock by comparison: open, park (3), RS(600) → 2 (00 §3.1) [doc].

### 5.2 Write (pen tool, tip)

1. Down: SF reader 2 inks with the pen's style/width/colour; first sample with `penTriggered == 0`
   → `HandlePenTrigger…`, schema 7, `EUS s 3` [doc 07 §4.2]. App: `onBeginRawDrawing` is empty
   (`OnyxInputHandler.kt:82-83`). No transaction [code].
2. Moves: nothing on the app side [code].
3. Up: the point list is posted to the main thread (`pen/touch/SFTouchRender.java:34-137`) →
   `onRawDrawingList` → under `drawingInProgress`: scribble-to-erase check, else `handleDraw` into the
   page bitmap only (`OnyxInputHandler.kt:296-351`, `nb/editor/utils/draw.kt:14-43`) [code]. No
   transaction, no surface post [code]. The SDK pen-up refresh timer (500 ms default,
   `pen/RawInputReader.java:131, 134`) fires `onPenUpRefresh`, which Notable ignores
   (`OnyxInputHandler.kt:120-122`) [code].
4. `commitHistorySignal` debounced 500 ms → `commitToHistory()` → `drawCanvasToView(null)`: one
   full-canvas lock + post (`CanvasObserverRegistry.kt:308-323`, `DrawCanvas.kt:77-88`) [code]. With
   SV mode H the entry is `(canvas, 524290)`, which schema 7 commits; with SV mode A it is
   `(canvas, 5)`, which schema 7 drops [inferred from 07 §4.5].
5. SF stays in schema 7 and scheme 3 after the stroke; nothing leaves it until the next pause or
   `ENABLE_POST(1)` [inferred, as for stock 00 §3.2].

Per-stroke refresh: one whole-canvas handwriting-repaint entry ~500 ms after the last stroke of a
burst (SV mode H only). Stock sends none on this device; Tangleaf sends a whole-WebView one per
stroke [doc].

### 5.3 Erase with the stylus eraser end (pen tool selected)

1. Down: SF eraser raw drawing is on with style 8 (from the last `enableNativeEraser`), so the rubber
   selects reader 4, which loads style 8 into SF's single style slot and inks the track; the first
   sample triggers schema 7 / `EUS s 3` [doc 07 §4.2, §4.4]. Pen state stays **2** [code].
2. `onBeginRawErasing` (posted to main, so after the first samples): `enableNativeEraser` again (SF
   `STROKE_PARAMETERS(8, …)`, `ERASER(1, 8)`) and `setStrokeColor(BLACK)` (SF `SET_STROKE_COLOR`),
   because the eraser reader paints with the global colour (`OnyxInputHandler.kt:96-108`) [code; the
   colour claim is the authors']. This also turns the pen colour black until restored [inferred].
3. Moves: no app work (`onRawErasingTouchPointMoveReceived` empty) [code].
4. Up: point list → `onRawErasingList` (`OnyxInputHandler.kt:356-388`): delete whole strokes hit by
   a 30 px swath (`nb/editor/utils/eraser.kt:180-216`), redraw the page bitmap, dirty = track bbox +
   10 px ∪ erased bounds; `commitErase(dirty, areaErase = false)`. `onEndRawErasing` →
   `updatePenAndStroke()` (style/width/colour restored) [code].
5. `commitErase` (`CanvasRefreshManager.kt:82-106`) [code]:
   - `setRawInputReaderEnable(false)` (in-process only);
   - main-thread coroutine: `lockCanvas(dirty)` (entry `(dirty, H or A)`), SF `ENABLE_POST(0)` (no
     effect), SF `ENABLE_POST(1)` → schema 7 left: `Post enabled!`, 150 ms sleep inside SF, `EUS s 2`
     [doc 07 §4.3, §5.1] (the Binder call is synchronous, so the main thread waits ~150 ms
     [inferred]); bitmap drawn, `unlockCanvasAndPost`; `resetViewUpdateMode` → **SV mode A from here
     on**;
   - `setRawDrawingEnabled(false)` → SF `ENABLE_POST(1)` (no-op), pen **3**
     (`Control state: toPause -> paused`), `BRUSH(1)`, `ERASER(0, 5)`;
   - 150 ms;
   - if `isDrawing`: `setRawDrawingEnabled(true)` → pen **2**, `BRUSH(1)`, `ERASER(0,5)`;
     `enableNativeEraser` → `STROKE_PARAMETERS`, `ERASER(1, 8)`; style/width/colour; input on.
6. The posted frame is the only repaint of the track: it covers track ∪ erased strokes and is
   committed normally because schema 7 was left before the post [inferred]. First erase: GU with the
   handwrite flag; later erases: mode 5 (AUTO) on the dirty rect, waveform chosen by the kernel
   [unknown]. The commit runs even when nothing was erased [code].

### 5.4 Erase with the eraser tool (tip), each eraser type

Types: `Eraser.PEN` (whole strokes under a 30 px swath) and `Eraser.SELECT` (whole strokes with any
point inside the closed lasso) (`nb/editor/utils/eraser.kt:17-19, 180-216`) [code]. There is no
partial or pixel eraser.

- **Tool entry** (`ChangeMode(Erase)`): `updateDrawingState` (no change); `observeMode` →
  `updatePenAndStroke` → `applyEraserIndicatorStyle(GRAY)`: `PEN` → SF style 2 (`MARKER`), width 30,
  gray; `SELECT` → style 5 (`DASH`), width 3, black, `STROKE_PARAMETERS(5, [5, 9, 9, 0])`; then
  `refreshUiSuspend` → full frame + F (pen 3, 500 ms, pen 2)
  (`CanvasObserverRegistry.kt:294-306`, `OnyxInputHandler.kt:147, 164-184`) [code].
- **Per gesture:** the tip is a brush touch → SF reader 2 inks a marker (or dash) stroke with the
  current colour; schema 7 / `EUS s 3`. Pen state **2** throughout [code + doc 07 §4.2]. Up:
  `onRawDrawingList` → `Mode.Erase` → `onRawErasingList` → `commitErase(dirty, areaErase = type ==
SELECT)` — as §5.3 step 5 with **150 ms** (PEN) or **500 ms** (SELECT)
  (`OnyxInputHandler.kt:234-235, 385-387`) [code]. The SDK arms its pen-up timer for this drawing
  gesture; Notable ignores it [code].
- Unlike stock, the tip trace is a brush style (marker 2 / dash 5), not SF's eraser style 8 [code].

**Eraser end while the eraser tool is active:** as §5.3 (style 8 or 5 per eraser type) [code].

**Scribble-to-erase (pen tool):** a scribble over strokes is detected after pen-up;
`commitErase(track ∪ erased, areaErase = true)` → 500 ms pause; the scribble itself is SF pen ink,
removed by the posted frame (`OnyxInputHandler.kt:305-345`) [code].

### 5.5 Lasso (Select mode)

1. Tool entry: SF style 0 (`PENCIL`), width 3, gray (`OnyxInputHandler.kt:149-150`), then
   `refreshUiSuspend` → full frame + F [code].
2. Stroke: SF reader 2 inks a **solid** thin gray line (not a dash); schema 7 / `EUS s 3`. The app
   gets the points only at pen-up [code].
3. Up: `thread { handleSelect(...) ; refreshUi(bbox + 10) }` (`OnyxInputHandler.kt:236-256`) [code].
   - **Shapes hit** (`nb/editor/utils/Select.kt:244-263, 73-147`): bitmap of the selection, page
     bitmap redrawn without it; `holdRefresh()` → SF `APPLY_TRANSIENT_UPDATE(0x1000004)` while still
     in scheme 3 → slot stored, no switch [doc 07 §5.2]; `setDrawingStateFromCanvas(false)` →
     `updateIsDrawing(false)`: cancel pending resume, wait for the draw lock, post a full frame,
     `setRawDrawingEnabled(false)` → SF `ENABLE_POST(1)` → `Post enabled!`, `clearHandwritingFBSchema`
     sees the fast slot → **`EUS s 4`**; pen **3**; `BRUSH(1)`, `ERASER(0,5)` [code + doc]. Then
     `refreshUi` frames (full and dirty) post under scheme 4 (A2) [inferred]. The gray trace is
     overwritten by those full-canvas posts [inferred]. State P; selection overlay drawn by Compose
     in the main window.
   - **Nothing hit:** `handleSelect` returns; `refreshUi(bbox)` → dirty-rect post + F (pen 3, 500 ms,
     pen 2) [code]. The post and the pause race (post is queued to main, the pause runs on
     `Dispatchers.Default`). If the post lands while SF is still in schema 7 with SV mode A, it is
     dropped and nothing repaints the trace afterwards [inferred].

### 5.6 Drag, resize, delete, deselect a selection

- State P (pen 3, render and input off, brush 1, eraser off), transient `0x1000004` held (scheme 4)
  [code + doc].
- **Drag (stylus or finger):** the Compose overlay's `detectDragGestures` moves the bitmap; frames
  come from the main window (Compose), not the surface; no pen-state or SDK call per step
  (`nb/editor/ui/SelectorBitmap.kt:84-97`) [code].
- **Resize / duplicate / delete buttons:** `refreshUi` → surface post only (`isDrawing` false, so
  no freeze) (`EditorControlTower.kt:222-246`) [code]. Delete also sets `isDrawing` true.
- **Tap outside (deselect):** `applySelectionDisplace()` (surface post), `selectionState.reset()` →
  `releaseRefresh()` → last handle → SF `CLEAR_TRANSIENT_UPDATE(1)`: `EUS s 2` (after 100 ms),
  one-shot GC 98, full-screen mode 5 → **one full GC flash**; `setIsDrawing(true)` →
  `setRawDrawingEnabled(true)` → pen **2**, flags reset, `ERASER(1, 8)`, lasso style re-sent
  (`SelectorBitmap.kt:66-69`, `SelectionState.kt:85-99`, `EpdRefreshArbiter.kt:81-89`) [code + doc
  07 §5.3].
- **Leaving the editor with a selection shown:** `EditorViewModel.onDispose` applies the displacement
  but does not `reset()` the selection, so its handle is never released
  (`EditorViewModel.kt:269-284`, `SelectionState.kt:234`) [code]; the transient A2 would then stay
  on after leaving, until the process ends or a later holder count reaches zero by accident
  [inferred].

**Finger hold (rectangle select):** after 300 ms of a still finger the gesture enters Selection mode:
arbiter acquire (SF `APPLY_TRANSIENT_UPDATE(0x1000004)`), `setIsDrawing(false)` (pen 3); on lift the
rectangle is selected (selection handle taken over), `setIsDrawing(true)` → pen 2 while the
selection sets `isDrawing` false again; gesture handle released 500 ms later
(`EditorGestureReceiver.kt:217-236, 277-289, 439-463`, `EditorControlTower.kt:315-325`) [code]. The
order of the two `isDrawing` writes is racy [inferred].

### 5.7 Menus, popups, dialogs, overlays

- **Toolbar menu, page menu, stroke menu, eraser submenu:** Compose `Popup(focusable = true)`
  windows (`nb/editor/ui/toolbar/ToolbarMenu.kt:48-58`, `StrokeMenu.kt:61-65`,
  `ToolbarElementView.kt:235-239`, `PageMenu.kt:38-41`) [code]. Two paths to P:
  `LaunchedEffect(isMenuOpen, isBackgroundSelectorModalOpen)` → `updateDrawingState()`
  (`nb/editor/ui/toolbar/Toolbar.kt:53-55`) and window focus loss → `isDrawing.emit(false)`
  (`CanvasObserverRegistry.kt:151-164`) [code]. Either → `updateIsDrawing(false)`: full surface post,
  SF `ENABLE_POST(1)`, pen **3**, `BRUSH(1)`, `ERASER(0,5)` [code]. The stroke menu and eraser
  submenu are not keys of that effect and rely on focus loss only [code].
- **Close:** `isMenuOpen` false → `updateDrawingState()` sets `isDrawing = true` synchronously and
  emits the bus value after 500 ms (`EditorViewModel.kt:520-530`). The state observer reacts to the
  synchronous write, so the resume is **immediate**: `closeMenusSignal`, SF pen **2**, flags, style
  8 eraser, pen style [inferred from the flow wiring; the 500 ms emit only repeats the value]. Focus
  regain adds `updatePenAndStroke` and a full surface post [code]. No GC and no Onyx repaint on
  close; the popup area is repainted by normal composition [code: grep].
- **Background selector dialog** (`nb/ui/dialogs/BackgroundSelector.kt:204`, a Compose `Dialog`):
  state key plus focus → P until closed [code].
- **Toolbar hide/show:** `ToggleToolbar` → `updateDrawingState`; `observeToolbar` →
  `updateActiveSurface` (full close/open cycle, new limit/exclude, SV mode H again) +
  `updatePenAndStroke` + `refreshUi(null)` (post + F) (`CanvasObserverRegistry.kt:279-292`) [code].
- **Overlays inside the canvas** (scroll indicators, selection overlay, hint snacks): no exclude
  rects; only the toolbar strip is excluded [code]. A pen stroke over the selection overlay is P
  anyway; over other overlays SF would ink [inferred].

### 5.8 Finger scroll and zoom

- Fingers pass `DrawCanvas.dispatchTouchEvent` (only stylus/eraser pointers are consumed,
  `DrawCanvas.kt:41-67`) to the Compose gesture receiver below; a first-down of stylus type is
  ignored there (`EditorGestureReceiver.kt:106`) [code].
- **Scroll** (one finger, vertical travel past the threshold, `smoothScroll` default on): enter
  Scroll mode → arbiter acquire → SF `APPLY_TRANSIENT_UPDATE(0x1000004)` (`EUS s 4` now, or deferred
  if SF is still in scheme 3 from writing). Each drained delta → `refreshUiImmediately` →
  `refreshUi(full)`: full surface post + F (first step: `ENABLE_POST(1)`, pen **3**, leaving schema 7
  → `EUS s 4` if deferred) (`EditorControlTower.kt:84-114`, `CanvasObserverRegistry.kt:97-120`)
  [code + doc]. Lift → handle released 500 ms later → SF `CLEAR_TRANSIENT_UPDATE(1)` → `EUS s 2`, GC
  98 one-shot, full-screen mode 5 → **full GC flash ~0.6 s after lift**; render back (pen **2**)
  500 ms after the last scroll refresh [code + doc].
- **Two-finger pan / pinch:** Transform mode (same acquire); with `continuousZoom` (default off) a
  pinch is a discrete snap-zoom at gesture end; zoom change → `updatePenAndStroke` (width × zoom) and
  a forced full refresh (`nb/gestures/GestureClassifier.kt:128-139`,
  `EditorGestureReceiver.kt:294-301`, `CanvasObserverRegistry.kt:166-174`)
  [code]. Same flash on release.
- Limit/exclude are not re-pushed during scroll (they do not depend on scroll) [code].
- No hover-based finger suppression (`setPostInputEvent` never called, default false →
  no `PenActiveEvent`, `pen/RawInputReader.java:101, 695-699`) [code].

### 5.9 Page switch

- **Swipe / buttons:** `viewModel.changePage` → `page.changePage` → `refreshUiImmediately` (full
  post + F) → load → `forceUpdate(null)` (full post, F re-armed); selection reset (release → GC flash
  if a selection was held) (`EditorViewModel.kt:683-747`, `PageView.kt:226-257`) [code]. Session
  kept; no GC of its own [code].
- **QuickNav:** opening sets `isQuickNavOpen` → P; page change via `CanvasEventBus.changePage` →
  `refreshScreen()` = SF `REPAINT_EVERY_THING_WITH_MODE(9)`; closing → `updateDrawingState` and
  `refreshScreen()` again (`EditorControlTower.kt:55-69`, `nb/navigation/NotableNavigator.kt:69-80`)
  [code]. Mode 9 on the HW-Tcon path: `waveform_mode wrong!` [inferred from doc]. Preview frames
  during scrubbing use `restoreCanvas` → surface post + `REFRESH_SCREEN(…, 0x2000004)` (A2 mono)
  [code].

### 5.10 Undo / redo

`history.undo()` → commit pending strokes (full post) → model change → `refreshUi` →
`refreshUiSuspend` → full post + F (pen 3, 500 ms, pen 2) (`nb/editor/state/History.kt:55-75`)
[code]. No close, no GC. Stock: PR, dirty-rect repaint, RS(500) (00 §3.10) [doc].

### 5.11 Tool switch, pen change

`ChangeMode` / `ChangePen` / `ChangeEraser` → `updateDrawingState` (no pen-state change when
`isDrawing` stays true); observers → `updatePenAndStroke` (style/width/colour, one each) and
`refreshUiSuspend` (full post + F) for mode and eraser changes; a pen preset change emits
`refreshUi` too (`EditorViewModel.kt:302-306, 362-383`, `CanvasObserverRegistry.kt:223-259`)
[code]. The eraser channel is not re-sent on a tool switch, only on the next
`setRawDrawingEnabled(true)` path [code]; it stays whatever the last `enableNativeEraser` set.

### 5.12 Leave the note

The editor composable is disposed → `AndroidView` removed → `surfaceDestroyed`: the guarded
`closeRawDrawing` does not run (§1 item 8); `onSurfaceDestroy` → `setRawDrawingEnabled(false)` →
SF `ENABLE_POST(1)`, pen **3** (if render was on), `BRUSH(1)`, `ERASER(0,5)`
(`DrawCanvas.kt:134-145`, `einkHelper.kt:132-136`) [code]. No pen state 0, no GC; limit/exclude and
the pen owner pid stay in SF [code + doc 07 §4.1]. The `TouchHelper` is dropped without `quit`, so
the in-process reader is paused, not closed [inferred]. Stock: close (3, 0), `resetEpdPost`, no GC
(00 §3.11) [doc].

### 5.13 Background / foreground, window focus

- **Home / recents:** `onPause` → `refreshUi` → `refreshUiSuspend` (if still `isDrawing`: full post
  - F) (`MainActivity.kt:245-251`); focus loss → P (`MainActivity.kt:253-262`); `onStop` destroys the
    surface → §5.12 [code]. Race: if the `onPause` path arms `resetScreenFreeze` after focus loss has
    already cancelled it, the 500 ms job turns render back on (pen **2**, input off) while the app is in
    the background, with the old limit rect in SF [inferred; ordering of `onPause` vs focus change and
    of the two coroutines is not fixed].
- **Return:** `onRestart` → `reinitSignal` → `drawCanvas.init()` (new callback) + a post that fails
  until the surface exists; `surfaceCreated` → full `setupSurface` (3, 0, 1, 2, re-push) + post + F;
  `onResume` / focus → `updatePenAndStroke`, post, `updateDrawingState` (`MainActivity.kt:237-243,
264-270`, `CanvasObserverRegistry.kt:151-164, 204-212`) [code]. `setupSurface` enables drawing even
  if `isDrawing` is false at that moment [code].
- **Dialog-like windows over the app** (system dialogs, notification shade): focus loss → P; regain
  → resume [code].

### 5.14 Keyboard and text input

The editor has no text field and no IME handling (grep for `ime` / `windowSoftInputMode` finds
nothing in the editor) [code]. Text input exists only inside dialogs (for example the background
selector's page field, `BackgroundSelector.kt:699`), which hold window focus, so the pen is already
in P when the IME appears [inferred].

## 6. Tables

### 6.1 SF-visible effect per action

| action               | Notable                                                                                                    | stock (00 §4)                                                                           |
| -------------------- | ---------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| open                 | 3, 0 (close), limit/exclude, 1, 2, brush 1 / eraser (1, 8), style; full post (SV H); 3 → 2 after 500 ms    | open (1,1), park 3, RS(600) → 2                                                         |
| write                | no transaction; full-canvas post in mode 524290 500 ms after a stroke burst (mode 5 after the first erase) | no transaction, mode-5 invalidates                                                      |
| eraser end, pen tool | 2 throughout, SF style 8 trace; after lift `ENABLE_POST(1)`, dirty post, 3, 150 ms, 2 + full re-push       | 2 throughout, style 8/5; `ENABLE_POST(1)` if erased, render-only 3, invalidate, RS(300) |
| eraser tool          | 2 throughout, SF brush trace (marker 30 gray or dash 3); same commit, 150 / 500 ms                         | 2 throughout, main style 8/5; same as eraser end                                        |
| lasso drawing        | 2, solid pencil trace width 3                                                                              | 2, dash trace                                                                           |
| lasso hit            | `APPLY_TRANSIENT(0x1000004)` (deferred), full post, `ENABLE_POST(1)` → `EUS s 4`, 3                        | PR, second pause, FM on (2308), reader-only resume, steady 3                            |
| lasso no hit         | dirty post, 3, 500 ms, 2                                                                                   | PR, repaint, RS(500)                                                                    |
| drag selection       | steady 3, A2 already on, Compose frames                                                                    | steady 3, fast mode on, overlay frames                                                  |
| deselect             | `CLEAR_TRANSIENT(1)` → GC + full mode 5; 2 + re-push                                                       | named pause/resume, RS(500); clear 5 s later with flag 0                                |
| menu open / close    | 3 (+ flags reset) / 2 immediately + re-push; no repaint of its own                                         | PR / REGAL_PLUS invalidate, RS(500)                                                     |
| finger scroll        | `APPLY_TRANSIENT(0x1000004)`, per-step full post + 3; 2 after 500 ms; clear with GC 500 ms after lift      | FM on (2308), PR + RS(500); clear (flag 0) 5 s later                                    |
| page switch          | full post, 3, 500 ms, 2; no GC                                                                             | PR, display, RS(≥500), GC every 10th tick                                               |
| undo / redo          | full post, 3, 500 ms, 2                                                                                    | PR, dirty repaint, RS(500)                                                              |
| leave                | 3, flags reset; no 0, no GC; regions left                                                                  | 3, 0, `resetEpdPost`; regions left                                                      |
| background           | 3 (focus), surface destroyed; possible stray 2 (race)                                                      | INV (3), session kept                                                                   |
| foreground           | full re-setup (3, 0, 1, 2) + 3 → 2                                                                         | INV → RS(500)                                                                           |

### 6.2 Panel refreshes Notable issues

| refresh                                        | trigger                                                                                             | SF effect                                                  |
| ---------------------------------------------- | --------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| surface post, mode 524290 (dirty rect or full) | every `lockCanvas` while SV mode H                                                                  | committed through schema 7 [inferred]                      |
| surface post, mode 5                           | every `lockCanvas` after the first erase commit                                                     | normal AUTO update; dropped while in schema 7 [inferred]   |
| `CLEAR_TRANSIENT_UPDATE(1)`                    | last arbiter handle released: 500 ms after a finger gesture, deselect, library/page-list scroll end | `EUS s 2`, GC 98 one-shot, full-screen mode-5 update [doc] |
| `REPAINT_EVERY_THING_WITH_MODE(9)`             | QuickNav page change and close                                                                      | recomposite with a rejected waveform nibble [inferred]     |
| `REFRESH_SCREEN(rect, 0x2000004)`              | QuickNav preview, "same page" restore                                                               | A2 mono region refresh [inferred]                          |

No counted GC, no GC on close, no GC on menu close [code].

## 7. Compared with stock

### 7.1 What Notable gets

- **Live eraser trace by the firmware** for the eraser end: SF eraser reader, style 8 (pen eraser) or
  5 (lasso eraser), session running [code]. Same mechanism as stock (00 §3.4) [doc]. For the eraser
  tool Notable uses a brush trace (marker/dash), which is also live [code].
- **No paused session under erase input.** Render stays on at pen-down for every tool; the only
  pauses are timed after the gesture [code]. This avoids the precondition of the observed flash
  (pen input while SF is paused) in the common case [inferred].
- **Stock-like erase hand-back:** `ENABLE_POST(1)` before the repaint, repaint, pause, timed
  resume with a full re-push; 150 ms (stock's `WaitForUpdateFinished` stroke value per the authors'
  comment) instead of stock's 300 ms for the render-only resume [code; stock value doc].
- **Colour brushes and styles:** pen colour is sent to SF (`setStrokeColor`), styles 0–7 including
  charcoal v2 and square pen with a nib angle (`configureCalligraphyLiveAngle`) [code]. Whether SF
  inks in colour on the Kaleido panel is [unknown] (same open point as stock, 00 §3.2).
- **Pressure:** SF styles are pressure-driven; the app keeps per-point pressure and time
  (`nb/editor/utils/handleInput.kt:16-28`, `GeometryExtensions.kt:81`) [code].
- **Fast mode for drags:** a shown selection holds transient A2 like stock's TRANSFORM state
  [code + doc].
- **Session kept for the document's life,** as stock; only surface re-creation closes it [code].

### 7.2 What Notable loses or does differently

- **Soft eraser on the tip:** the eraser tool traces with a marker brush, not style 8; no partial
  (pixel) erase at all, only whole-stroke erasers [code].
- **Per-stroke refresh:** Notable posts a whole-canvas 524290 entry after each stroke burst while SV
  mode H; stock issues none on this device [code; doc]. After the first erase these posts become
  mode 5 and are dropped inside schema 7, so the panel shows SF ink until the next hand-back
  [inferred].
- **Flash policy:** every finger scroll, pan, zoom, hold-select and deselect ends with
  `clearTransientUpdate(true)` → GC one-shot + full-screen update ~0.5 s after release. Stock clears
  5 s later with flag 0 (still one full-screen mode-5 update, no GC) [code + doc 00 §2.1].
- **Many more 500 ms pauses:** tool switch, undo, page change, lasso miss, every scroll step and
  `onPause` put SF in pen state 3 for 500 ms with input still on. Pen input in that window lands on
  a paused session [code]. Stock pauses for fewer events (00 §2.3) [doc].
- **Lasso in the firmware layer:** Notable's trace is a solid pencil line, not stock's dash; on a
  hit the fast mode is applied while still in scheme 3 and taken at the pause (`EUS s 4` right after
  `Post enabled!`), as 07 §6 describes for the deferred variant [code + doc]. No rectangle lasso;
  finger hold gives a rectangle selection instead [code].
- **Palm / finger handling:** no hover gate, no CTP disable region, no named pause registry; a palm
  that starts a finger gesture during writing runs scroll/hold logic (transient A2, freeze)
  [code + inferred]. Stock suppresses finger gestures while the pen hovers (00 §3.8) [doc].
- **EinkWise:** no per-app EinkWise profile is written by the editor; the Welcome screen offers
  `setAppScopeRefreshMode(NORMAL)` and checks for NORMAL/REGAL (`nb/ui/views/WelcomeView.kt:84-91`,
  `einkHelper.kt:39-51`) [code]. The package is not `com.onyx`, so the framework scroll-A2 manager
  (07 §6 Entry A) is not exempted for it [code: `isOnyxApp` test].
- **Full-refresh policy:** no counted GC; the QuickNav `REGAL_PLUS` repaint likely has no visible
  effect on this panel [inferred]; the only effective full refreshes are the transient-clear GCs
  [inferred].
- **Colour CU, turbo, dither:** never touched [code].
- **Teardown:** no pen state 0 on leave; possible stray resume in the background (§5.13) [inferred].

### 7.3 Predicted driver-log behaviour

Log strings as in 07 and 10 [doc]. "Entry A" is the framework `OnyxScrollRefreshManager` path
(07 §6). It needs `topScrollBounds` on Notable's window. Notable is a single-activity Compose app;
the bounds are set by a scroll-capture search and then kept for the life of the `ViewRootImpl`
(07 §6), so a scrollable library grid seen before opening a note could have set them for the editor
too [inferred]; whether they are set is [unknown].

**Erase with the eraser end (pen tool), pen down in state D:**

```
SurfaceFlinger: HandlePenTrigger, sid: -1 value is: 0 pid <notable>      (first erase after a hand-back)
HWEpdcManager: ### EUS, s 3 c 0
   … SF eraser ink (style 8) …
SurfaceFlinger: Post enabled!                                             (ENABLE_POST(1) in commitErase)
HWEpdcManager: ### EUS, s 2 c 0                                           (+150 ms)
   driver: one partial update of the dirty rect (GU + handwrite flag first time, AUTO later)
SurfaceFlinger: Pid <notable> State change to: toPause / Control state: toPause -> paused
   … 150 ms (500 ms for the lasso eraser) …
SurfaceFlinger: Pid <notable> State change to: running
```

No `EUS s 4`, no full-screen update: the session is not paused during the input and Notable applies
no transient mode for erasing [inferred]. **Exception:** an eraser-end or tip touch that starts
inside a C or F window (≤ 500 ms after an erase commit, a tool switch, undo, page change, lasso miss
or scroll). Then SF does not ink (paused); in C the app gets no points either, in F it does (input
on) and erases without a live trace [code]. With Entry A live, a vertical move over 8 px there gives
`APPLY_TRANSIENT_UPDATE(2308)` → `### EUS, s 4 c 0`, A2 (`waveform[4]`) frames, and ~1 s after lift
`### EUS, s 2 c 0` plus the full-screen update — the Tangleaf flash signature [inferred from 07 §6].

**Lasso with a hit:**

```
HandlePenTrigger … / ### EUS, s 3 c 0                                     (lasso trace)
   APPLY_TRANSIENT_UPDATE(0x1000004) — stored, no EUS (scheme 3 guard)
SurfaceFlinger: Post enabled!                                             (setRawDrawingEnabled(false))
HWEpdcManager: ### EUS, s 4 c 0                                           (+150 ms, fast slot)
State change to: toPause
   driver: A2 (waveform[4]) updates for the surface posts and the Compose overlay
   … selection shown, dragged …
   on deselect: ### EUS, s 2 c 0 (+100 ms), GC one-shot → driver waveform[2] update_mode[1] full screen,
   plus the full-screen mode-5 entry; State change to: running
```

The scheme-4 period and the final GC flash are deliberate here, not the paused-session artefact. A
stylus or finger drag of the selection with Entry A live adds a second transient (2308) that Entry
A clears ~1 s after lift, which ends A2 and flashes while the selection is still shown; Notable's
own clear at deselect then finds no fast slot and does nothing [inferred].

**Lasso without a hit:** `EUS s 3` during the trace; after lift `Post enabled!`, `EUS s 2`,
`toPause`, 500 ms, `running`; no `EUS s 4` [inferred].

## 8. Compared with Tangleaf

Observations only (Tangleaf side from 08 [doc]).

Where Notable is closer to stock than Tangleaf:

- Erasing keeps pen state 2 and lets SF draw the eraser-end trace (style 8/5, eraser raw drawing on).
  Tangleaf pauses at eraser-end down and keeps render off for the eraser tool, which is the
  flash precondition [doc 08 §5.4-5.5].
- The erase hand-back is ordered like stock: leave schema 7, post the repaired region, pause, timed
  resume with a re-push. Tangleaf waits for its reconcile frame and resumes without a re-push.
- The session is not closed on undo, page switch or leave; Tangleaf closes on all three.
- The exclude list is never empty (strip or degenerate rect), as stock's; Tangleaf clears it.
- Only `SFTouchRender` is used, so no doubled `ENABLE_POST` / pen-state transactions from
  `AppTouchRender`.

Where Notable is further from stock or weaker:

- Every finger scroll/zoom and deselect ends with a GC (flag 1); Tangleaf applies transient A2 only
  for a selection drag and clears it without a GC.
- It posts 500 ms render pauses after most non-stroke refreshes, with input left on.
- The per-stroke whole-canvas repaint silently changes from 524290 to mode 5 after the first erase.
- No IME, text or overlay suppression; the canvas owns the window and uses focus loss for everything.
  Tangleaf has named pause reasons, IME insets and overlay rects.
- Only whole-stroke erasers; Tangleaf has stroke, pixel and lasso eraser modes (08 §5.5).
- No pen state 0 on leave or background, and a possible stray resume while backgrounded.

Techniques worth studying:

- **`SurfaceView` + `lockCanvas(dirty)` as the update-entry carrier.** The firmware's `SurfaceView`
  hook adds `(dirty rect, view default mode)` per lock (`fwv/SurfaceView.java:110-124`), so a
  dirty-rect handwriting-repaint entry needs no `View.invalidate` of a whole WebView.
- **Re-asserting the eraser channel** (`STROKE_PARAMETERS(8, …)` + `ERASER(1, 8)`) after every
  `setRawDrawingEnabled(true)`, because that call's `resetPenDefaultRawDrawing` turns it off
  (`pen/TouchHelper.java:170-178, 348-351`).
- **`ENABLE_POST(1)` immediately before posting the repaired frame** so the frame is not dropped by
  schema 7; the `ENABLE_POST(0)` before it has no SF effect (07 §4.4).
- **A reference-counted owner for the transient slot** (`EpdRefreshArbiter`), since SF has one
  transient slot shared by every caller, including the framework's own scroll manager.
- **Style-7 nib angle** through `getStrokeParameters(7)` / `setStrokeParameters(7)` for live/dry
  calligraphy parity.

## 9. Open questions as hardware observations

1. Is `topScrollBounds` set on Notable's window in the editor (log
   `OnyxScrollRefreshManager: checkHasScrollCallback` / `hadChildInWhiteList`)? Decides whether
   Entry A can add 2308 periods.
2. Do SV-mode-A posts get dropped in schema 7 (compare driver log for a stroke burst before and after
   the first erase of a session)?
3. What the dirty-rect erase commit looks like in the driver log in mode 524290 vs mode 5.
4. Does `REPAINT_EVERY_THING_WITH_MODE(9)` produce any driver update on this panel?
5. Does SF ink the pen colour on the Kaleido panel (`SET_STROKE_COLOR` non-black)?
6. Does the background race (§5.13) ever leave pen state 2 after Home (`dumpsys`/SF debug
   `pen control state`)?

## 10. Verification of the older `REPORT.md`

`~/tmp_zfs/reference_notes_apps/REPORT.md` (notable rows) re-checked against this checkout:

- "`TouchHelper.create(drawCanvas, inputCallback)` … (2-arg = `AppTouchRender`)" — **wrong**. The
  two-argument constructor picks mask `hasStylus ? 2 : 1` (`pen/TouchHelper.java:45-47`); on this
  device that is 2 = `SFTouchRender` [code + doc]. The call site line (`OnyxInputHandler.kt:67`) is
  correct.
- Limit = view minus 40 dp strip, exclude = the strip, recomputed from `surfaceCreated`,
  `surfaceChanged` and the toolbar toggle — correct (now `einkHelper.kt:139-177`,
  `OnyxInputHandler.kt:214-215`, `DrawCanvas.kt:112, 130`, `CanvasObserverRegistry.kt:287`). Missing:
  with the toolbar hidden a zero-height exclude rect is still sent.
- Modals gate `isDrawingAllowed` → `setRawDrawingEnabled`; window focus as backstop — correct
  (`EditorViewModel.kt:106-109`, `OnyxInputHandler.kt:190, 204`, `MainActivity.kt:253-262`). The
  menu path also goes through `Toolbar.kt:53-55`.
- Finger split in `DrawCanvas.dispatchTouchEvent` — correct (now `DrawCanvas.kt:41-67`).
- IME: none — correct.
- Not covered there: SF eraser channel, erase commit, `resetScreenFreeze`, SurfaceView update-mode
  hook and its reset, transient A2 arbiter with GC, the unreachable `closeRawDrawing`.
