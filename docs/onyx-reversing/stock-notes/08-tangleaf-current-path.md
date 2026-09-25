# 08 — Tangleaf's current pen path, described in the stock documents' terms

Written 2026-09-25 from static reading of the repository at commit `e078293` (main, clean) and of the
decompiled Onyx SDK and framework. No device was touched. The purpose is comparison, action by
action, with the stock Notes documents in this directory. Nothing here judges the code or proposes
a change.

## 0. Notation and sources

Every claim is tagged:

- **[code]** — read directly in the cited source.
- **[inferred]** — reasoned from code that was read, not read directly.
- **[doc]** — taken from an earlier document in `docs/onyx-reversing/` and not re-verified here.
- **[unknown]** — not determinable statically.

Path shorthands:

| shorthand | root                                                                                         |
| --------- | -------------------------------------------------------------------------------------------- |
| `kt/`     | `plugins/tauri-plugin-mobile-system/android/src/main/java/dev/okhsunrog/mobile_system/`      |
| `hw/`     | `src/features/handwriting/`                                                                  |
| `app/`    | `src/app/`                                                                                   |
| `pen/`    | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-pen-1.5.4.3/sources/com/onyx/android/sdk/pen/`    |
| `dev/`    | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-device-1.3.5.2/sources/com/onyx/android/sdk/`     |
| `fw/`     | `~/tmp_zfs/onyx_framework/out/sources/android/onyx/` (framework.jar, `ViewUpdateHelper`)     |
| `fwv/`    | `~/tmp_zfs/onyx_framework/out/sources/android/view/` (framework.jar, `View`, `ViewRootImpl`) |

SDK version actually linked. The plugin declares only `com.onyx.android.sdk:onyxsdk-pen:1.5.4.3`
(`plugins/tauri-plugin-mobile-system/android/build.gradle.kts:33`) [code]. Its POM pulls
`onyxsdk-base:1.8.5.4`, whose POM pulls `onyxsdk-device:1.3.5.2` (Gradle cache POMs) [code]. So
every `EpdController` / `Device` call below resolves in `onyxsdk-device-1.3.5.2`, and every
`TouchHelper` call in `onyxsdk-pen-1.5.4.3`. This differs from stock Notes, which bundles its own SDK
copy [doc: task brief].

`Device.currentDevice()` is `SDMDevice` on this device: `Device.m33b()` maps board platform `lito` to
`DeviceIndex.SDM`, and `detectDevice()` returns `SDMDevice.createDevice()` for it
(`dev/device/Device.java:92-127`) [code]; the board being `lito` is [doc] (08-surfaceflinger.md
header). `SDMDevice` reaches the framework by reflection on `android.onyx.ViewUpdateHelper`
(`dev/device/SDMDevice.java:1009`) [code], and every `ViewUpdateHelper` setter used below is a
synchronous Binder transaction (`flags 0`) on `ServiceManager.getService("SurfaceFlinger")` with the
`android.ui.ISurfaceComposer` token (`fw/ViewUpdateHelper.surfaceComposerData` :1346-1350,
`transactData(int, Parcel, Parcel)` :1421-1437) [code]. The static setters ignore the `View`
argument the SDK passes as the reflective receiver: `SDMDevice` resolves `enablePost(int)`,
`setScreenHandWritingPenState(int)` and `setScreenHandWritingRegionMode(int)` as one-argument
static methods (`SDMDevice.java:1106, 1110, 1112`) [code], so which view we pass to those three
makes no difference to SurfaceFlinger [code]. Only the limit/exclude setters use their `View`
argument, to add `getLocationOnScreen` (`fw/ViewUpdateHelper.java:1202-1237`) [code].

Firmware version: the tablet runs firmware 4.2, system Android 13, vendor Android 11, with the same
`surfaceflinger` and `framework.jar` that were analysed (verified 2026-09-25, see
[00-overview §0](00-overview.md)). Nothing in our code branches on it.

## 1. Summary

1. Our app drives one `TouchHelper` created on the whole Tauri `WebView` with
   `FEATURE_SF_TOUCH_RENDER | FEATURE_APP_TOUCH_RENDER` and the SDK's touch listener disabled
   (`kt/OnyxInk.kt:339-344`) [code]. The App render therefore never processes a MotionEvent; it only
   contributes its `openDrawing` / `closeDrawing` / `setDrawingRenderEnabled(false)` /
   stroke-parameter side effects, which are extra `ENABLE_POST` and pen-state transactions
   (`pen/touch/AppTouchRender.java:105-113, 186-206`) [code].
2. The render flag (`TouchHelper.setRawDrawingRenderEnabled`) is the only per-gesture lever we pull.
   `false` emits `ENABLE_POST(1)`, pen state 3 (PAUSE), `ENABLE_POST(1)`; `true` emits pen state 2
   (DRAWING) (`pen/TouchHelper.java:188-198`, `pen/touch/SFTouchRender.java:195-203, 278-284`,
   `pen/EpdPenManager.java:39-45`) [code]. The SDK skips the call when the flag already has the
   requested value (`TouchHelper.java:189`) [code].
3. Render is **off (pen state 3)** in these steady states: eraser tool selected; lasso tool in
   rectangle mode; lasso tool in freehand mode while a selection exists; any named pause
   (overlay, text focus, IME); window without focus; app paused
   (`kt/OnyxInk.kt:616-617`, `:579`, `:600`) [code]. Render is **switched off for the duration of a
   gesture** when: the stylus eraser end touches with the pen tool (`InkEraserRenderGate`); a
   lasso gesture starts on an existing selection (drag); a pen-down lands on the floating selection
   menu (`kt/OnyxInk.kt:769-779`) [code]. In all these cases the pen writes while our SF session is
   in pen state 3, which is the proven precondition for the scheme-4/A2 switch and the delayed full
   GC16 "flash" [doc: pen-research-handoff.md, "Established facts"].
4. After an erase gesture with the pen tool, render goes back on only after the reconcile frame has
   been committed by the compositor (`InkEraserRenderGate.framePresented`, called from the
   frame-commit callback, `kt/OnyxInk.kt:525`) [code]. For an erase this is at least ~120 ms after the
   canvas acknowledged the erased frame, because an erase gets no pen-up refresh
   (`pen/RawInputReader.java:613-616`, `kt/InkReconcileGate.kt:44-48`) [code].
5. The reconcile after every stroke is: set the WebView's default update mode to
   `HAND_WRITING_REPAINT_MODE` (524290 = GU | `0x80000`), call `webView.invalidate()` on the whole
   WebView, and reset the mode once that frame is committed (`kt/OnyxInk.kt:499-545`,
   `kt/InkRepaintMode.kt`, `kt/OnyxInk.kt:135-142`) [code]. No `handwritingRepaint`,
   `refreshScreen`, pen-state change or scheme call is made for it [code]. The computed reconcile
   rectangle is used only to decide _whether_ to reconcile and for diagnostics; the invalidate is
   always the whole WebView (`kt/OnyxInk.kt:503-505, 518-523, 544`) [code].
6. Every geometry change reconfigures the session: each `scroll` (capture phase, any element),
   `resize`, `ResizeObserver` hit and tool/selection/overlay change runs the JS `configure()`
   (`hw/onyx-ink.ts:293-299, 319-331`) [code], which sends a new `configure_onyx_ink` only when the
   JSON of its args differs from the last one sent (`:205-207`) [code] — so a scroll of an element
   that does not move the canvas sends nothing, while a scroll of `[data-ink-viewport]` changes
   `top`/`clipTop` and does send [inferred]. Each non-trivial configure
   does render off + input off, then re-pushes region mode, exclude reset, limit rect, brush/eraser
   flags, stroke width/colour/style, and render/input back on (`kt/OnyxInk.kt:316, 355-370`) [code].
   During a scroll this is one pen-state 3 → 2 cycle per scroll event [inferred].
7. **Undo and redo close the whole pen session and open a new one**: `historyBusy` is part of
   `InkCanvas.disabled` (`hw/handwriting-note-view.tsx:798`), `disabled` turns `useOnyxInk`'s
   `enabled` false (`hw/ink-canvas.tsx:454`), and the effect cleanup sends `enabled:false`
   (`hw/onyx-ink.ts:310-314`), which runs `OnyxInk.close()` → `closeRawDrawing` + pen state 0 +
   session cycle + exclude reset + a full GC refresh 300 ms later (`kt/OnyxInk.kt:702-730`) [code].
   The same happens on leaving the page, switching pages and backgrounding [code].
8. Full-panel GC refreshes we issue: 300 ms after any session close; 300 ms after the soft keyboard
   hides; 300 ms after any Dialog / Popover / Select / Menu reports `onOpenChange(false)`
   (`kt/OnyxInk.kt:721-730`, `kt/MobileSystemPlugin.kt:99-120`, `app/eink-refresh.ts:37-60`) [code].
   All three are `EpdController.invalidate(webView, UpdateMode.GC)` → `View.invalidate(98)` with
   `UI_GC_MODE = 98 = GC16|FULL|WAIT` (`dev/device/SDMDevice.java:1365-1370, 1124-1128`,
   `fw/ViewUpdateHelper.java:93`) [code]; that this appears as `update_mode[1]` in the driver log is
   [inferred].
9. We never call any update-scheme API (`setDisplayScheme`, `setEpdTurbo`, `applyGCOnce`,
   `repaintEverything`, `setTrigger`, `byPass`) (grep over `kt/`) [code]. Every scheme switch seen
   in the driver log while our app runs is chosen by SurfaceFlinger itself [inferred].
10. The only transient panel mode we request is `applyTransientUpdate(ANIMATION_QUALITY)` (SF code
    16711782, mode 2308 `UI_A2_QUALITY_MODE`) during a pen-down with the lasso tool inside the
    selection grab box — a pen drag of an existing selection, **and also an eraser-end touch there**
    (the condition has no erasing guard, `kt/OnyxInk.kt:785`) — cleared 5 s after the last such
    gesture (`kt/OnyxInk.kt:146-172, 785, 801-805`, `kt/InkRefreshPolicy.kt`) [code]. The mode value
    is [code]: `SDMDevice.m86a` maps `ANIMATION_QUALITY` (switch case 10,
    `dev/device/SDMDevice.java:940`) to the reflected `UI_A2_QUALITY_MODE` (`:1034`), which the
    framework defines as 2308 (`fw/ViewUpdateHelper.java:85`).
11. Device-specific branches in our code are few: manufacturer `ONYX` (enables everything), Android
    ≥ 10 + hardware acceleration, `Device.getColorType() > 0` which only selects the resume delay
    (500 ms colour / 150 ms mono) (`kt/OnyxInk.kt:203, 226-237, 296-298`) [code], and the JS display
    profile (`auto` resolves from the manufacturer; the user can force `eink`/`standard`,
    `src/app/display-profile.ts:56-57`) which decides whether BASE = GU is set [code]. There are no
    model, tablet/phone or feature-flag checks [code].

## 2. Central classes and entry points

| Class / file                                                                | Role                                                                                          | Key entry points                                                                                                                                                                                                                       |
| --------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `kt/MobileSystemPlugin.kt`                                                  | Tauri plugin; owns the pause registry and the per-WebView display-mode stack; routes commands | `load` :70-81, `configureOnyxInk` :143-173, `suppressOnyxInk` :180-195, `commitOnyxFrame` :197-210, `setDisplayProfile` :228-246, `requestFullRefresh` :340-357, `onPause`/`onResume`/`onDestroy` :122-141, IME insets listener :76-80 |
| `kt/OnyxInk.kt`                                                             | One pen session on the WebView: `TouchHelper`, render/input gate, reconcile, lifecycle        | `configure` :280-382, `commit` :451-472, `begin`/`end`/`stroke`/`preview` :753-851, `refreshFrame` :499-545, `pause` :593-613, `resume` :571-584, `close` :702-725                                                                     |
| `kt/RawDrawingGate.kt`                                                      | Order of render/input switches                                                                | `pause` :31-34 (render off, input off), `resume` :37-42 (push rects, reset defaults, input on, render per tool)                                                                                                                        |
| `kt/InkEraserRenderGate.kt`                                                 | Render off for an erase gesture until the reconcile frame is committed                        | `begin` :8-15, `framePresented` :17-21, `reset` :23-25                                                                                                                                                                                 |
| `kt/InkReconcileGate.kt`                                                    | Rendezvous between SDK pen-up refresh and canvas acknowledgement                              | `began` :44-48, `canvasReady` :51-54, `penUpRefresh` :57-62                                                                                                                                                                            |
| `kt/InkReconcileRegion.kt`                                                  | Stroke-union / whole-region bookkeeping                                                       | `add` :32-37, `invalidateAll` :40-42, `take` :48-64                                                                                                                                                                                    |
| `kt/InkRepaintMode.kt` + `kt/DisplayModeStack.kt` + `kt/ViewDisplayMode.kt` | WebView default update mode in three layers BASE < SESSION < TRANSIENT                        | `DisplayModeStack.set/clear` :45-60; `ViewDisplayMode` apply → `EpdController.setViewDefaultUpdateMode` :31-38, restore :39-50                                                                                                         |
| `kt/InkRefreshPolicy.kt`                                                    | Transient `ANIMATION_QUALITY` during a selection drag, 5 s quiet period                       | `begin` :11-17, `end` :19-21, `settle` :23-28                                                                                                                                                                                          |
| `kt/InkPauseRegistry.kt`                                                    | Named pause reasons                                                                           | `pause`/`resume` :20-30                                                                                                                                                                                                                |
| `kt/InkOverlayRects.kt`                                                     | Floating DOM controls where a pen-down is swallowed (not firmware excludes)                   | `inkOverlayRects` :28-48, `inkOverlayHit` :51-52                                                                                                                                                                                       |
| `hw/onyx-ink.ts` `useOnyxInk`                                               | JS side of the session: configure, event listener, commit                                     | configure :158-226, listener :228-283, geometry listeners :293-299, cleanup :300-315, commit effect :332-360                                                                                                                           |
| `hw/ink-canvas.tsx` `InkCanvas`                                             | Web canvas that holds the durable ink; gestures; selection menu                               | `nativeInput` :425-438, `nativeStroke` :439-444, `publish` :168-329, `useOnyxInk` call :452-480                                                                                                                                        |
| `hw/handwriting-note-view.tsx`                                              | Page view; decides `disabled`, tool, eraser/lasso mode, menus                                 | `InkCanvas` props :795-812 (`disabled` :798)                                                                                                                                                                                           |
| `app/ink-suppression.ts`                                                    | JS pause reasons: text focus, overlays                                                        | `suppressOnyxInk` :31-39, `installTextFocusSuppression` :63-88, `useOverlayInkSuppression` :98-114                                                                                                                                     |
| `app/eink-refresh.ts`                                                       | Full refresh 300 ms after an overlay closes                                                   | `requestFullRefreshSoon` :37-44, `refreshPanelAfterClose` :53-60                                                                                                                                                                       |

Tauri commands used by the pen path: `configure_onyx_ink`, `commit_onyx_frame`, `suppress_onyx_ink`,
`debug_ink_repaint` (plugin-direct), `request_full_refresh`, `set_display_profile` (through
`src-tauri/src/commands/system.rs:128-165` → plugin) [code].

## 3. The primitives, traced to SurfaceFlinger

Each row is one call our code makes, what the SDK does with it, and the SurfaceFlinger-visible effect.
Transaction codes are from `fw/ViewUpdateHelper.java:111-216` [code]; SF-side meaning is from
08-surfaceflinger.md [doc] unless marked otherwise.

| Our call (site)                                                                                                                                                                                                                          | SDK chain (1.5.4.3 / device 1.3.5.2)                                                                                                                                                                                                                                                                                                                                                                           | SF transactions, in order                                                                                                                                                                 | SF-visible effect                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `TouchHelper.create(webView, 3, cb, false)` (`kt/OnyxInk.kt:339`)                                                                                                                                                                        | ctor `pen/TouchHelper.java:358-375`: adds `SFTouchRender` then `AppTouchRender`; `setTouchListenerEnabled(false)` before `bindHostView` so no `OnTouchListener` is installed (`AppTouchRender.java:105-113`); `SFTouchRender.bindHostView` → `RawInputManager.setHostView` → `RawInputReader.setLimitRect(view.getLocalVisibleRect())` (`pen/RawInputManager.java:141-147`, `pen/RawInputReader.java:410-416`) | `SET_SCREEN_HANDWRITING_REGION_LIMIT` (16711694) = whole WebView visible rect, translated to screen                                                                                       | limit region = whole WebView until the next `setLimitRect` [code for the call; effect doc]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `setPenUpRefreshTimeMs(500)` (:121, :348)                                                                                                                                                                                                | SF render → `RawInputReader.f61n` (`RawInputReader.java:370-372`); App render no-op                                                                                                                                                                                                                                                                                                                            | none                                                                                                                                                                                      | in-process timer only [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `setPostInputEvent(false)` (:349)                                                                                                                                                                                                        | `RawInputReader.f69v` = false → no `PenActiveEvent`/EventBus posts (`RawInputReader.java:374-376, 492-501, 695-699`)                                                                                                                                                                                                                                                                                           | none                                                                                                                                                                                      | none [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `setHostViewScrollListenerEnabled(false)` (:350)                                                                                                                                                                                         | no-op in both renders and `RawInputManager` (`RawInputManager.java:196-197`, `AppTouchRender.java:237`)                                                                                                                                                                                                                                                                                                        | none                                                                                                                                                                                      | none [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `setSingleRegionMode()` (:120, :351)                                                                                                                                                                                                     | `RawInputReader.setSingleRegionMode`: `nativeSetRegionMode(1)` + `EpdController.setScreenHandWritingRegionMode(view,1)` (`RawInputReader.java:394-398`); App render no-op                                                                                                                                                                                                                                      | `SET_SCREEN_HANDWRITING_REGION_MODE` (1048620) = 1                                                                                                                                        | `PenManager+0x38` = 1 [doc]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `setLimitRect(limit, NO_EXCLUDES)` (:123, :352, :360, :365)                                                                                                                                                                              | SF render → `RawInputReader.setLimitRect(limit)`: native limit + `EpdController.setScreenHandWritingRegionLimit(view,[limit])`; `setExcludeRect(empty)` returns early (`RawInputReader.java:410-425`). App render → `AppTouchInputReader` lists only (`AppTouchRender.java:270-273`)                                                                                                                           | `SET_SCREEN_HANDWRITING_REGION_LIMIT` = limit                                                                                                                                             | global `PenManager` limit array replaced; exclude array untouched by this call [code + doc]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `clearLatchedExcludes()` = `EpdController.setScreenHandWritingRegionExclude(null, emptyArray())` (:631-634)                                                                                                                              | `SDMDevice.setScreenHandWritingRegionExclude(View,Rect[])` → int[0] (`SDMDevice.java:3439-3453`) → framework `setScreenHandWritingRegionExclude(null, int[0])` writes view flag 0 and an empty array (`fw/ViewUpdateHelper.java:1202-1217`)                                                                                                                                                                    | `SET_SCREEN_HANDWRITING_REGION_EXCLUDE` (16711714), n = 0                                                                                                                                 | `PenManager::setExcludeRegion` n ≤ 0 → exclude table cleared [doc]. The int[] overload in `SDMDevice` failed to decompile; that it forwards unchanged is [inferred]                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `openRawDrawing()` (:352)                                                                                                                                                                                                                | `TouchHelper.m58a()` sets cached render=false, input=false; `SFTouchRender.openDrawing`: `setStrokeStyle(0)`, `RawInputReader.start()`, `EpdPenManager.startDrawing()`; `AppTouchRender.openDrawing`: `setStrokeStyle(0)`, `enablePost(view,0)`, pen state 1 (`TouchHelper.java:152-160`, `SFTouchRender.java:171-179, 265-268`, `AppTouchRender.java:186-190`)                                                | `SET_STROKE_STYLE`(0), pen state **1**, `SET_STROKE_STYLE`(0), `ENABLE_POST(-1,0,pid)`, pen state **1**                                                                                   | pen owner pid recorded, `TouchReader::start` [doc]; `ENABLE_POST(0)` = suppress posting [doc: 03 §1.3, inference there]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `setEraserRawDrawingEnabled(false, 0)` (:353)                                                                                                                                                                                            | SF render → `Device.setEraserRawDrawingEnabled(false,0)` (`SFTouchRender.java:292-294`, `SDMDevice.java:2826-2833`); App no-op                                                                                                                                                                                                                                                                                 | `SET_ERASER_RAW_DRAWING_ENABLED` (1048833) (0, 0)                                                                                                                                         | firmware does not ink the eraser path [inferred from name]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `setStrokeWidth(w)` (:361, :366)                                                                                                                                                                                                         | SF render → `RawInputReader.setStrokeWidth`: native + `EpdController.setStrokeWidth` (`RawInputReader.java:359-364`); App render → reader + `EpdController.setStrokeWidth` (`AppTouchRender.java:217-220`)                                                                                                                                                                                                     | `SET_STROKE_WIDTH` (16711687) ×2                                                                                                                                                          | pen width; also the exclude hit-test inflation [doc]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `setStrokeColor(BLACK)`                                                                                                                                                                                                                  | SF render → `EpdPenManager.setStrokeColor` → `EpdController.setStrokeColor` + reader colour; App → `EpdController.setStrokeColor` (`SFTouchRender.java:239-242`, `AppTouchRender.java:138-140`)                                                                                                                                                                                                                | `SET_STROKE_COLOR` (16711686) ×2                                                                                                                                                          | ink colour black [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `setStrokeStyle(FOUNTAIN=1 / DASH=5)`                                                                                                                                                                                                    | both renders → `EpdController.setStrokeStyle` (`SFTouchRender.java:234-236`, `AppTouchRender.java:133-135`, `EpdPenManager.java:52-54`)                                                                                                                                                                                                                                                                        | `SET_STROKE_STYLE` (16711688) ×2                                                                                                                                                          | brush style [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `Device.setStrokeParameters(5, [5f])` (:359, lasso only)                                                                                                                                                                                 | `SDMDevice.setStrokeParameters` (`SDMDevice.java:2842-2844`) → `fw/ViewUpdateHelper.java:1252-1264`                                                                                                                                                                                                                                                                                                            | `SET_STROKE_PARAMETERS` (1049089)                                                                                                                                                         | dash pattern for style 5 [inferred from name]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `resetPenDefaultRawDrawing()` (gate `resetDefaults`, :125)                                                                                                                                                                               | `Device.setBrushRawDrawingEnabled(true)`, `Device.setEraserRawDrawingEnabled(false, 5)` (`TouchHelper.java:348-351`)                                                                                                                                                                                                                                                                                           | `SET_BRUSH_RAW_DRAWING_ENABLED` (1048834) = 1; `SET_ERASER_RAW_DRAWING_ENABLED` = (0, 5)                                                                                                  | [code]; SF semantics [unknown]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `setRawInputReaderEnable(true)` (gate `input`, :113)                                                                                                                                                                                     | guarded by cached flag (`TouchHelper.java:213-223`); SF render → `RawInputReader.resume()`: `nativeSetPenState(4)`, report flag `f51d = true` (`RawInputReader.java:328-333`); App → flag only                                                                                                                                                                                                                 | none (in-process native reader)                                                                                                                                                           | none on SF [code]: `libonyx_pen_touch_reader.so` (arm64, from the 1.5.4.3 AAR) links only `liblog`, `libm`, `libc++_shared`, `libdl`, `libc` (`readelf -d`), has no Binder/`libgui` dependency and opens `/dev/input/event%d`; its `PenManager`/`TouchReader::setPenState` are its own in-process copies [code: ELF]. What native pen state 4 does inside that reader [unknown]                                                                                                                                                                                                                                   |
| `setRawInputReaderEnable(false)`                                                                                                                                                                                                         | SF render → `RawInputReader.pause()`: `nativePausePen()`, `f51d = false` (`RawInputReader.java:335-339`)                                                                                                                                                                                                                                                                                                       | none                                                                                                                                                                                      | none on SF [code]. While `f51d` is false the reader dispatches **nothing** for states 0-3 (down/move/up): `onTouchPointReceived` gates the whole dispatch on `state == 6 \|\| m30j()` (`RawInputReader.java:431`); the inner `\|\| erasing` terms at `:641, 662, 764, 780` are only reached once that outer gate has passed, i.e. with input enabled or on a state-6 force release [code]. (Earlier version of this row said erase events are still delivered with input off — wrong.) Even if one got through, `OnyxInk.begin` drops it because `isRawDrawingInputEnabled` is false (`kt/OnyxInk.kt:754`) [code] |
| `setRawDrawingRenderEnabled(false)` (`setRender`, :623-625)                                                                                                                                                                              | guarded; SF render `m212g`: `leaveScribbleMode(view)` → `enablePost(1)`, then `pauseDrawing()`; App render: `enablePost(1)` (`SFTouchRender.java:195-198`, `AppTouchRender.java:200-206`, `SDMDevice.java:1764-1778`)                                                                                                                                                                                          | `ENABLE_POST(-1,1,pid)`, pen state **3**, `ENABLE_POST(-1,1,pid)`                                                                                                                         | pen state 3 → `TouchReader::pause` + `stopHandwriting` (schema 7 → restore) [doc]; driver log `SurfaceFlinger: Pid N State change to: toPause` [doc: handoff]                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `setRawDrawingRenderEnabled(true)`                                                                                                                                                                                                       | guarded; SF render `m213j` → `resumeDrawing()`; App render sets a flag only                                                                                                                                                                                                                                                                                                                                    | pen state **2**                                                                                                                                                                           | `TouchReader::ensureStart/resume`, schema not restored [doc]; log `State change to: running` [doc: handoff]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `cycleScribbleSession()` (:653-658)                                                                                                                                                                                                      | `EpdController.leaveScribbleMode(webView)` + `setScreenHandWritingPenState(webView, PEN_PAUSE)` directly, **not** through `TouchHelper`, so no guard                                                                                                                                                                                                                                                           | `ENABLE_POST(-1,1,pid)`, pen state **3**                                                                                                                                                  | as above, emitted even if already paused [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `closeRawDrawing()` (:706)                                                                                                                                                                                                               | `TouchHelper.closeRawDrawing`; SF render: `setStrokeStyle(0)`, `m211f` (`leaveScribbleMode`, reader pause, pen state 3), `m210c` (reader `quit` → `nativeSetPenState(4)`, `nativeRawClose`; pen state 0); App render: `setStrokeStyle(0)`, `enablePost(1)`, pen state 0 (`TouchHelper.java:162-168`, `SFTouchRender.java:182-192, 271-275`, `RawInputReader.java:341-353`, `AppTouchRender.java:193-197`)      | `SET_STROKE_STYLE`(0), `ENABLE_POST(1)`, pen **3**, pen **0**, `SET_STROKE_STYLE`(0), `ENABLE_POST(1)`, pen **0**                                                                         | STOP: `TouchReader::stop`, auto-sync off [doc]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `EpdController.setScreenHandWritingPenState(webView, PEN_PAUSE)` + exclude reset (plugin `load`, `kt/MobileSystemPlugin.kt:90-96`)                                                                                                       | direct                                                                                                                                                                                                                                                                                                                                                                                                         | pen **3**, `SET_SCREEN_HANDWRITING_REGION_EXCLUDE` n=0                                                                                                                                    | clears a stale schema/exclude latch at app start [doc]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `displayMode.set/clear` → `EpdController.setViewDefaultUpdateMode(webView, mode)` (`kt/ViewDisplayMode.kt:31-38`)                                                                                                                        | `SDMDevice.setViewDefaultUpdateMode` → reflected `View.setDefaultUpdateMode(int)` (`SDMDevice.java:1134, 1959-1961`) → field write (`fwv/View.java:14577-14579`)                                                                                                                                                                                                                                               | none at call time                                                                                                                                                                         | subsequent `View.invalidate()` of the WebView carries this mode up to `ViewRootImpl.invalidateChildInParent(.., mode)` → `invalidateRectOnScreen(rect, mode)` (`fwv/View.java:10814-10816, 10836-10870`; `fwv/ViewRootImpl.java:13131-13158`) [code]. How the entry reaches SF (`addEpdc`, `ViewRootImpl.java:3257`) is [inferred]                                                                                                                                                                                                                                                                                |
| `restoreRaw` (last layer cleared)                                                                                                                                                                                                        | reflective `View.setDefaultUpdateMode(previousRaw)`, else `resetViewUpdateMode` → `View.resetUpdateMode()` = 5 (`kt/ViewDisplayMode.kt:39-50`, `fwv/View.java:13718-13719`)                                                                                                                                                                                                                                    | none                                                                                                                                                                                      | [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `EpdController.invalidate(webView, UpdateMode.GC)` (close, IME hide, overlay close, `request_full_refresh`)                                                                                                                              | `SDMDevice.invalidate` → reflected `View.invalidate(int)` with mode 98 (`SDMDevice.java:1124-1128, 1365-1370`) → `View.invalidate(true, 98)` (`fwv/View.java:10788-10790, 10818-10820`)                                                                                                                                                                                                                        | via the next draw                                                                                                                                                                         | `UI_GC_MODE = 98 = GC16 \| FULL \| WAIT` (`fw/ViewUpdateHelper.java:93`) [code]; full-screen GC16 in the driver log [inferred]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `EpdController.applyTransientUpdate(ANIMATION_QUALITY)` (:148)                                                                                                                                                                           | `SDMDevice.applyTransientUpdate` → `fw/ViewUpdateHelper.applyTransientUpdate(int)` (`SDMDevice.java:2968-2970`, `fw/ViewUpdateHelper.java:404-408`)                                                                                                                                                                                                                                                            | `APPLY_TRANSIENT_UPDATE` (16711782), mode 2308 [code: `SDMDevice.m86a` case 10 → `UI_A2_QUALITY_MODE`, `SDMDevice.java:940, 1034`; `fw/ViewUpdateHelper.java:85`]                         | per-app transient A2-quality mode [doc]. The returned "accepted" is not a firmware answer: `invokeMethodSafely` returns a non-null placeholder for a `void` method, so it is true whenever reflection succeeded [code: `ReflectUtil.invokeMethodSafely`, `dev/utils/ReflectUtil.java:570-579`]                                                                                                                                                                                                                                                                                                                    |
| `Device.currentDevice().clearTransientUpdate(false)` (:154, :248)                                                                                                                                                                        | `fw/ViewUpdateHelper.clearTransientUpdate(false)` → int 0 (`fw/ViewUpdateHelper.java:488-496`)                                                                                                                                                                                                                                                                                                                 | `CLEAR_TRANSIENT_UPDATE` (16711783), 0 = `CLEAR_FLAG_NONE` [doc]                                                                                                                          | ends the transient mode without a GC [doc]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `EpdController.appResetCTPDisableRegion(activity)` (:699, init only)                                                                                                                                                                     | reflected `InputManager.appResetCTPDisableRegion()` (`SDMDevice.java:1194, 2175-2177`)                                                                                                                                                                                                                                                                                                                         | not SF (InputManager)                                                                                                                                                                     | clears a capacitive-touch cut-out [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `Device.currentDevice().colorType` (:235)                                                                                                                                                                                                | `SDMDevice.getColorType` → `fw/ViewUpdateHelper.getColorType()` (`SDMDevice.java:2783-2789`, `fw/ViewUpdateHelper.java:648-655`)                                                                                                                                                                                                                                                                               | `GET_COLOR_TYPE` (read), once per process: the result is cached in `colorPanelProbe`; first reached from `status()` at the end of the first `configure`, or from `RESUME_DELAY_MS` [code] | none [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `EpdController.getTouchWidth/getTouchHeight/getMaxTouchPressure` (:332-335, helper creation only)                                                                                                                                        | `Device` getters                                                                                                                                                                                                                                                                                                                                                                                               | reads; which service answers them was not traced [unknown]                                                                                                                                | none expected [inferred]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `EpdController.getViewDefaultUpdateMode(webView)` and reflective `View.getDefaultUpdateMode()` (`kt/ViewDisplayMode.kt:34-35, 63-67`; called on the first layer set, from `status()`, `applyDisplayProfile` and a `Log.d` in `resume()`) | `SDMDevice` reflection on the `View` field (`SDMDevice.java:1135-1136`)                                                                                                                                                                                                                                                                                                                                        | none (in-process field read) [code]                                                                                                                                                       | none [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |

Pen-state constants: `PEN_STOP 0, PEN_START 1, PEN_DRAWING 2, PEN_PAUSE 3, PEN_ERASING 4`
(`pen/EpdPenManager.java:9-13`) [code]. **We never send pen state 4 to SurfaceFlinger** (grep of `kt/`;
the only `4` is `RawInputReader.nativeSetPenState(4)`, which is in-process) [code].

Mode values as bit fields (`fw/ViewUpdateHelper.java:40-101`) [code]: the low nibble is the
waveform field (`EINK_WAVEFORM_MODE_MASK = 15`), and the framework names value 2 there
`EINK_WAVEFORM_MODE_GC16`. `UI_GU_MODE = 2` is that nibble alone; `UI_GC_MODE = 98 = 2 |
EINK_UPDATE_MODE_FULL (32) | EINK_WAIT_MODE_WAIT (64)`; `HAND_WRITING_REPAINT_MODE = 524290 = 2 |
EPDC_FLAG_HANDWRITE_GU (0x80000)`; `UI_DEFAULT_MODE = 5 = EINK_WAVEFORM_MODE_AUTO`. So our "GU" view
mode, our reconcile mode and our GC refresh all carry the same waveform nibble and differ only in
the FULL/WAIT/handwrite flags [code]. Whether SurfaceFlinger's decode (08-surfaceflinger.md: `2→2`)
drives that nibble as a GU or a GC16 waveform on the Kaleido panel is [unknown] statically; the
driver log's `waveform[W]` for a plain GU partial update would settle it.

## 4. Our session state

| Variable                           | Where                         | Values / meaning                                                                                                                                       |
| ---------------------------------- | ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `TouchHelper.f77b` (render cached) | `pen/TouchHelper.java:37`     | reset to false by `openRawDrawing`; guards `setRawDrawingRenderEnabled` [code]                                                                         |
| `TouchHelper.f78c` (input cached)  | `:40`                         | same for input [code]                                                                                                                                  |
| `InkPauseRegistry.held`            | `kt/InkPauseRegistry.kt:12`   | reasons: `ime` (native insets), `text-focus`, `overlay:<id>` (JS) [code]                                                                               |
| `OnyxInk.resumed`                  | `kt/OnyxInk.kt:86`            | Activity resumed [code]                                                                                                                                |
| `webView.hasWindowFocus()`         | `kt/OnyxInk.kt:549, 572`      | window focus [code]                                                                                                                                    |
| `DisplayModeStack` layers          | `kt/DisplayModeStack.kt:28`   | BASE = GU from the display profile; SESSION = GU while a session is resumed; TRANSIENT = `HAND_WRITING_REPAINT_MODE` during one reconcile frame [code] |
| `InkEraserRenderGate.active`       | `kt/InkEraserRenderGate.kt:5` | render held off after an erase gesture [code]                                                                                                          |
| `InkRefreshPolicy.fastRequested`   | `kt/InkRefreshPolicy.kt:5`    | transient `ANIMATION_QUALITY` requested [code]                                                                                                         |
| `InkReconcileRegion.whole`         | `kt/InkReconcileRegion.kt:25` | next reconcile owes the whole limit rect [code]                                                                                                        |
| `InkReconcileGate`                 | `kt/InkReconcileGate.kt`      | `penUpRefreshSeen`, `expecting`, `canvasReady`, `penUpReady` [code]                                                                                    |
| `InkGesturePairing.drawing`        | `kt/InkGesturePairing.kt:12`  | a native gesture is open [code]                                                                                                                        |
| `swallowed`                        | `kt/OnyxInk.kt:85`            | pen-down landed on the selection menu [code]                                                                                                           |

"Tool rendering" (whether render should be on outside a gesture), `kt/OnyxInk.kt:616-617` [code]:

```
toolRendering = !eraser && (!interaction || (fastLasso && !hasSelection))
  eraser      = tool == "eraser"                              (hw/onyx-ink.ts:193)
  interaction = tool == "lasso"                               (hw/onyx-ink.ts:194)
  fastLasso   = tool == "lasso" && lassoMode == "free"        (hw/onyx-ink.ts:195)
  hasSelection = selection bounds non-null                    (hw/onyx-ink.ts:196)
```

| Tool / mode                                  | toolRendering | Steady SF pen state while resumed |
| -------------------------------------------- | ------------- | --------------------------------- |
| pen                                          | true          | 2 [code + inferred]               |
| eraser (stroke / pixel / lasso eraser modes) | false         | 3                                 |
| lasso freehand, no selection                 | true          | 2 (dashed style 5, width 5)       |
| lasso freehand, selection exists             | false         | 3                                 |
| lasso rectangle                              | false         | 3                                 |

## 5. The mechanism, action by action

Sequences below are in emission order. "SF:" lines list SurfaceFlinger transactions; "log:" lines
are what the handoff says the driver log shows for such transitions [doc], stated only where our
code makes the precondition hold.

### 5.1 App start (plugin load)

1. `MobileSystemPlugin.load` → `clearStaleInkState` (`kt/MobileSystemPlugin.kt:70-72, 90-96`) [code]:
   SF: pen state **3**; exclude n=0.
2. IME insets listener installed (`:76-80`) [code].
3. Appearance provider calls `set_display_profile(eink)` once the display resolves
   (`app/appearance.tsx:124-137`) [code]. On e-ink: `DisplayModeStack.set(BASE, GU)` →
   `View.setDefaultUpdateMode(2)` on the WebView (`kt/MobileSystemPlugin.kt:254-270`) [code], then
   `ensureRefreshProfile()` writes the EinkWise config (`refresh_mode_4`, updateMode 0, turbo 0) once,
   guarded by a SharedPreferences marker (`:292-331`) [code]. This requires the EAC service reflection
   hooks to exist (`:293-294`) [code]; the handoff records that our package has `supportEAC: true`
   while stock has `false` [doc].
4. `setEinkRefreshEnabled(true)` arms the JS overlay-close refresh (`app/appearance.tsx:121`) [code].

### 5.2 Open a handwriting page

Preconditions: `HandwritingNoteView` is mounted with `key={page.uuid}`
(`src/features/workspace/workbench.tsx:365-367`); `InkCanvas` renders only after the draft loads, and
native ink is enabled when `capabilities.nativeDeviceEvents && !disabled`
(`hw/handwriting-note-view.tsx:793-812`, `hw/ink-canvas.tsx:454`); `nativeDeviceEvents` is `true` on
every Android build (`src-tauri/src/commands/system.rs:35`) [code].

JS (`hw/onyx-ink.ts:130-299`) [code]:

1. New session id (`crypto.randomUUID`), listener registered, then `configure()` measures the canvas
   and the `[data-ink-viewport]` clip and invokes `configure_onyx_ink` with `enabled`, geometry, tool
   flags, selection, overlay rects.

Native (`kt/MobileSystemPlugin.kt:143-173`, `kt/OnyxInk.kt`) [code]:

2. If no `OnyxInk` exists: constructor → `VendorAccess.ensure()` (hidden-API exemption for
   `Landroid/onyx/` and `Landroid/view/View;`, `kt/VendorAccess.kt:17-26`) →
   `Device.clearTransientUpdate(false)` (SF `CLEAR_TRANSIENT_UPDATE` 0) → `resetPalm()`
   (`InputManager.appResetCTPDisableRegion`) → window-focus listener (`kt/OnyxInk.kt:242-255`).
3. `configure(args)`: session differs from `config?.session` (null) → `close()` (`:299-301`). With no
   helper yet: `pause()` → `releaseDisplayMode()` (SESSION cleared, nothing set yet),
   `gate.pause()` (helper null → no transactions), `cycleScribbleSession()` → SF `ENABLE_POST(1)`,
   pen **3** (`:653-658`); `region.invalidateAll()`, `webView.invalidate()` (`:593-613`). `hadSession`
   false, so no second cycle and no close refresh (`:703-724`).
4. `pause(releaseDisplay=false, tearDown=false)` (`:316`) — no transactions (helper null) besides
   `webView.invalidate()`.
5. Geometry: `sheet` = canvas rect × (`webView.width / viewportWidth`); `limit` = sheet clipped to the
   viewport's top/bottom and to the WebView (`:319-327`).
6. Helper creation (`:330-354`): `ResManager.init`; digitizer range check; `TouchHelper.create(...)`
   → SF limit = whole WebView (row in §3); `setPenUpRefreshTimeMs(500)`; `setPostInputEvent(false)`;
   `setHostViewScrollListenerEnabled(false)`; `setSingleRegionMode()` → SF region mode 1;
   `setLimitRect(limit)` → SF limit = sheet; `openRawDrawing()` → SF style 0, pen **1**, style 0,
   `ENABLE_POST(0)`, pen **1**; `setEraserRawDrawingEnabled(false, 0)` → SF eraser raw drawing (0, 0).
7. Tool parameters (`:355-369`): pen/eraser/rectangle-lasso → `setLimitRect(limit)` (SF limit),
   `setStrokeWidth(strokeWidth × sheetWidth / 1000)` (SF width ×2), colour black (SF ×2), style
   `FOUNTAIN` (SF ×2). Freehand lasso → `setStrokeParameters(DASH, [5])`, limit, width 5, black,
   style `DASH`.
8. `resume()` (`:571-584`): `displayMode.set(SESSION, GU)` → `View.setDefaultUpdateMode(2)`;
   `gate.resume(toolRendering())` → `pushRects`: region mode 1, pen-up 500 ms, exclude n=0, limit;
   `resetDefaults`: brush raw drawing 1, eraser raw drawing (0, 5); input on (in-process
   `nativeSetPenState(4)`); render per tool → pen tool: SF pen **2**; eraser/rectangle lasso: no
   transaction (flag already false after `openRawDrawing`), SF stays in pen state **1** from step 6
   [code]. Then `commit(full)` because SESSION was just acquired.
9. Back in `configure`: geometry changed from the empty previous values → a second `commit(full)`
   that supersedes the first (`:371-374`).
10. `commit` → `postVisualStateCallback`; on `onComplete`, `reconcile.canvasReady()` returns `SOON`
    (no pen-up refresh expected) → `refreshFrame()` after 120 ms (`:451-472`, `:219`).
11. `refreshFrame` (`:499-545`): `region.take` yields the whole limit (`whole` was set); TRANSIENT =
    `HAND_WRITING_REPAINT_MODE` → `View.setDefaultUpdateMode(524290)`; frame-commit callback
    registered; `webView.invalidate()` → the next frame of the whole WebView carries mode 524290. On
    commit: TRANSIENT cleared → mode back to GU.

JS also publishes the canvas in a layout effect (`hw/ink-canvas.tsx:494-512`) and, once `native` is
true, the commit effect sends `commit_onyx_frame(partial:true, damage)` on every
`draft/frame/native/decoration/tool` change (`hw/onyx-ink.ts:332-360`) [code].

Resulting state with the pen tool: pen state 2, render on, input on, limit = visible sheet, exclude
empty, region mode single, WebView mode GU, `ENABLE_POST` last written = 0 by `openRawDrawing`
[code]. Note the eraser and rectangle-lasso case: the last pen state written is **1** (START), not 3,
until the next pause [code]; how SF treats pen state 1 with pen input is [unknown].

### 5.3 Write (pen tool, pen tip)

The firmware draws the ink itself; in-process events arrive on the raw-input thread, pass
`RawInputManager` (`pen/RawInputManager.java:24-97`) and are **posted to the main thread** by
`SFTouchRender`'s wrapper (`pen/touch/SFTouchRender.java:40-137`) [code]; the pen-up refresh is posted
a second time by us (`kt/OnyxInk.kt:863-867`) [code].

1. Pen down → `onBeginRawDrawing` → `begin(point, false)` (`kt/OnyxInk.kt:753-792`) [code]:
   - Not on an overlay → `fastPreview = false`.
   - `eraserRenderGate.begin(false)` → if an erase frame is still pending, `framePresented()` →
     `restoreToolRendering()` → SF pen **2** (only in that case).
   - `interaction` false → no render change.
   - `reconcile.began(false)` → `expecting = penUpRefreshSeen`.
   - `displayPolicy.begin(false)` → no transient mode.
   - `cancelFrameSubmission()` → releases TRANSIENT if a reconcile was in flight (WebView mode back to
     GU before that frame).
   - emits `begin`.
   - SF: **no transaction** in the common case [code].
2. JS `begin`: `nativeInput` ignores it for the pen tool (`hw/ink-canvas.tsx:427-428`);
   `onActiveChange(true)` → `setActive(true)` → `busy` → toolbar buttons get `disabled`
   (`hw/handwriting-note-view.tsx:234-239, 539`) whose class includes `disabled:opacity-50`
   (`src/components/ui/button.tsx:8`) [code]. Whether this toolbar repaint reaches the panel during
   the stroke is [unknown].
3. Moves: `preview()` only extends `region` via `markNativePoint`; nothing is sent for the pen tool
   (`kt/OnyxInk.kt:842-851`) [code].
4. Pen up: `RawInputReader.m35b` delivers the point list, then `onEndRawDrawing`, then arms the
   pen-up timer (500 ms) because the gesture was not erasing (`pen/RawInputReader.java:601-617`,
   `:278-283`) [code].
   - `stroke()` → `sequence++`, emits `stroke` with normalised points (`kt/OnyxInk.kt:816-831`).
   - `end()` → emits `end`; `cancelFrameSubmission()`; `postDelayed(refresh, 750)` as a net
     (`:794-814`).
5. JS: `stroke` → `applyOnyxStroke` → new draft → `onChange` → persisted by the draft writer;
   `setFrame`. `end` → `onActiveChange(false)`, `setFrame`, deferred configure if any
   (`hw/onyx-ink.ts:246-282`). Canvas layout effect draws the stroke; the commit effect sends
   `commit_onyx_frame` (twice: after the draft change and after the frame bump; each supersedes the
   previous) [code].
6. `commit` → `onComplete` → `reconcile.canvasReady()`: after the first ever pen-up refresh,
   `expecting` is true → `WAIT` → `penUpWait` armed at 750 ms. Before the first one → `SOON` (120 ms)
   (`kt/InkReconcileGate.kt:44-75`) [code].
7. ~500 ms after the last point: `onPenUpRefresh(rect)` (stroke bounds + width, unioned with the
   previous stroke, `pen/RawInputReader.java:747-759`, delivered only while input is enabled,
   `:791-797`) → `penUpRefresh` → `region.add`, `reconcile.penUpRefresh()` → `NOW` if the canvas is
   ready → `refreshFrame()` (`kt/OnyxInk.kt:479-490`) [code].
8. `refreshFrame`: WebView mode = 524290, `webView.invalidate()`, reset to GU after the commit (§5.2
   step 11). SF: **no pen-state change**; the next WebView frame's update entry carries mode 524290
   (GU waveform with `EPDC_FLAG_HANDWRITE_GU`) [code for the mode, doc for its meaning].

Resulting state: pen state stays 2 throughout [code]. The handoff notes stock's per-stroke
`GrayscaleRefreshAction` (the only stock user of `HAND_WRITING_REPAINT_MODE`) does not run on this
device, while ours marks every reconcile with it [doc].

### 5.4 Erase with the stylus eraser end (pen tool selected)

1. Eraser end down → native reader reports `isErasing` → `onBeginRawErasing` → posted →
   `begin(point, true)` (`pen/RawInputReader.java:427-436, 762-773`; `kt/OnyxInk.kt:858`) [code].
2. `begin`: `eraserRenderGate.begin(true)` → `setRender(false)` → SF `ENABLE_POST(1)`, pen **3**,
   `ENABLE_POST(1)`; `active = true` (`kt/InkEraserRenderGate.kt:8-15`) [code]. log: `State change to:
toPause` [doc]. `reconcile.began(true)` → `expecting = false`. Emits `begin` with `erasing:true`.
   - Timing: the PAUSE is sent only after the event has crossed from the reader thread to the main
     thread; eraser contact before that happens with pen state 2 [inferred].
3. JS `begin` → `beginGesture(point, erasing=true)` → action `erase`, draws the eraser circle into
   the canvas (`hw/ink-canvas.tsx:330-398, 285-290`); `onActiveChange(true)`; `interacting` true →
   configure deferred (`hw/onyx-ink.ts:164-167`) [code].
4. Moves: `preview()` sends at most every 32 ms because `erasing` is true (`kt/OnyxInk.kt:845-850`);
   JS coalesces to one per animation frame and runs `eraseGesture` → `publish` → canvas redraw
   (`hw/onyx-ink.ts:231-243`, `hw/ink-canvas.tsx:389-396`) [code]. These canvas redraws produce
   WebView frames with the GU (SESSION) mode while SF is in pen state 3 [inferred]. The handoff's
   proven mechanism — pen input while our session is PAUSED → scheme 4 / A2 — applies here [doc +
   inferred that eraser-end motion counts as "pen writes"].
5. Up: `onRawErasingTouchPointListReceived` → `stroke(..., erasing=true)` → JS `finishGesture` →
   `eraseGesture` over all points, `onSelectionChange([])`, `publish`, `onChange` (`hw/ink-canvas.tsx:399-424,
439-444`); `onEndRawErasing` → `end()`. **No pen-up timer** for an erase
   (`pen/RawInputReader.java:613-615`) [code].
6. JS commit → `commit` → `canvasReady()` → `SOON` → `refreshFrame()` after 120 ms (reconcile mode 524290) [code].
7. When that frame is committed: `eraserRenderGate.framePresented()` → `restoreToolRendering()` →
   SF pen **2** (log: `running` [doc]); then TRANSIENT released → mode GU (`kt/OnyxInk.kt:508-533`)
   [code].
8. If the reconcile frame never presents (e.g. a pause), the gate is reset without re-enabling render
   (`InkEraserRenderGate.reset`, called from `pause()`, `kt/OnyxInk.kt:610`), and render comes back
   through the next `resume()` [code]. A new pen-tip down before the frame also restores render
   immediately (`InkEraserRenderGate.begin(false)`) [code].

The ~1.5 s-later scheme 4 → 2 switch with one full GC16 is SurfaceFlinger's own behaviour [doc];
nothing in our code issues it [code].

Eraser end with the **lasso** tool (added in verification) [code, `kt/OnyxInk.kt:753-792`]:

- No selection, freehand mode (render on): as above — `eraserRenderGate.begin(true)` sends
  `ENABLE_POST(1)`, pen **3**, `ENABLE_POST(1)`; `setRender(fastPreview = false)` is then a no-op;
  pen **2** returns after the reconcile frame.
- Selection present (render already off): no pen-state transaction. If the contact point is inside
  the selection grab box (`movingSelection`, :762-763), `displayPolicy.begin(args.interaction &&
movingSelection)` has no erasing guard, so an eraser touch there also sends
  `APPLY_TRANSIENT_UPDATE(2308)` and arms the 5 s settle, exactly as a selection drag does. JS
  treats the gesture as `erase` (`hw/ink-canvas.tsx:339-340`), and `finishGesture` clears the
  selection (`:411-413`), so a `configure` (pause/resume, limit re-push) follows the lift [code].

### 5.5 Erase with the eraser tool (pen tip)

Tool switch first (§5.9): `configure` with `eraser:true` → `pause(tearDown=false)` → render off
(SF `ENABLE_POST(1)`, pen **3**, `ENABLE_POST(1)` if render was on), input off; then `resume()` →
`gate.resume(false)` → rects/defaults re-pushed, input on, render stays off [code]. **Steady pen state
3 for as long as the eraser tool is selected** (unless the session was freshly opened with the eraser
tool: then pen state 1, §5.2) [code].

Per gesture:

1. The tip produces **drawing** events (`onBeginRawDrawing`), not erasing ones; `begin(point, false)`
   treats it as an erase because `args.eraser` is true: `eraserRenderGate.begin(true)` →
   `setRender(false)` is a no-op (flag already false), `active = true`; `reconcile.began(true)`;
   preview events are sent (`config.eraser`); JS receives `erasing: erasing || args.eraser = true`
   (`kt/OnyxInk.kt:747, 778-781, 845`) [code].
2. SF: **no transaction per gesture** [code]. The pen writes while our session is in pen state 3 →
   scheme 4 / A2 then the delayed GC16 [doc + inferred].
3. JS: stroke/pixel eraser as §5.4 steps 3-5; the lasso eraser mode draws a dashed polygon instead of a
   circle and erases on finish (`hw/ink-canvas.tsx:270-282, 389, 411`) [code].
4. Because the native reader saw a drawing gesture, `RawInputReader` **does** arm the pen-up timer
   and delivers `onPenUpRefresh` ~500 ms later (input is enabled) (`pen/RawInputReader.java:601-617`,
   `:791-797`) [code]. `reconcile.began(true)` set `expecting = false`, so the canvas
   acknowledgement alone schedules the reconcile at 120 ms (`SOON`); a pen-up refresh that arrives
   while the canvas is ready triggers it immediately (`NOW`) [code]. Which one wins on the device is
   [unknown]. If `SOON` wins, `reconciled()` clears `canvasReady`, so the later pen-up refresh only
   adds its rectangle to `region` and returns `WAIT`; that rectangle is consumed by the next
   reconcile, whenever one runs (`kt/InkReconcileGate.kt:120-151`, `kt/OnyxInk.kt:479-490`) [code].
5. Reconcile frame presented → `framePresented()` → `restoreToolRendering()` →
   `setRender(false)` → no-op; pen state stays 3 [code].

### 5.6 Lasso

Freehand lasso, no selection (`toolRendering` true, render on, style DASH width 5):

1. `begin(point, false)`: not moving a selection → `fastPreview = true`; `interaction` →
   `setRender(true)` (no-op, already on); `displayPolicy.begin(false)` (`kt/OnyxInk.kt:762-785`)
   [code]. SF: none. The firmware draws the dashed trace [inferred from the recipe comment,
   `:356-358`].
2. No preview events (`fastPreview` suppresses them, `:845`) [code]. JS `beginGesture(...,
nativeFast=true)` records points without drawing (`hw/ink-canvas.tsx:375`); `traceRef` holds the
   commit effect back while the trace is on screen (`hw/onyx-ink.ts:247, 339`) [code].
3. Up: `stroke` → JS `finishGesture` → `onSelectionChange(selectLasso(...))`; `end` → `setFrame` →
   commit. Pen-up refresh is armed and delivered (drawing gesture) [code].
4. The selection changes → `hasSelection` changes → `configure()` → `pause(tearDown=false)` → SF
   `ENABLE_POST(1)`, pen **3**, `ENABLE_POST(1)`; input off; `resume()` → render stays off because
   `toolRendering` is now false (`kt/OnyxInk.kt:316, 370, 616-617`) [code]. The selection frame and
   the floating menu are drawn by the web canvas / DOM (`hw/ink-canvas.tsx:255-268, 651-673`) [code].

Freehand lasso with a selection present, or rectangle lasso: render is already off (pen state 3).
`begin` → `setRender(fastPreview)`: for a new freehand selection `fastPreview` is true → SF pen **2**
for the gesture; the firmware draws the trace; after the new selection is set, configure turns it off
again (step 4) [code]. For rectangle mode `fastPreview` is false → no transaction; preview events drive
the JS polygon (`hw/ink-canvas.tsx:397`); the pen writes in pen state 3 [code + inferred].

Note: after a gesture that turned render on for the trace, nothing turns it off at `end()`; only the
following `configure` does (`kt/OnyxInk.kt:794-814`) [code]. If the lasso selected nothing,
`hasSelection` stays false, no configure follows, and render stays on with the DASH style [inferred].

### 5.7 Drag a selection

Pen drag (lasso tool, pen-down within 12 units of the selection bounds, `kt/OnyxInk.kt:762-763`,
`hw/ink-canvas.tsx:57-78`) [code]:

1. `begin`: `movingSelection` → `fastPreview = false` → `setRender(false)` (no-op if already off,
   which it is with a selection) → pen state stays **3**; `displayPolicy.begin(true)` →
   `EpdController.applyTransientUpdate(ANIMATION_QUALITY)` → SF `APPLY_TRANSIENT_UPDATE` (2308)
   (`kt/OnyxInk.kt:146-151, 785`) [code].
2. Previews every ≥32 ms → JS `moveSelection` preview → `publish` and the menu is moved directly in
   the DOM each animation frame (`hw/ink-canvas.tsx:142-154, 379-388`) [code]. Canvas frames go out
   under the transient A2-quality mode [inferred].
3. Up: `stroke` → JS `finishGesture` (move committed); `end` → `displayPolicy.end` → deadline +5 s;
   `settleDisplay` posted at 5000 ms (`kt/OnyxInk.kt:801-805`) [code]. The normal reconcile
   (`refreshFrame`, mode 524290) runs meanwhile [code].
4. 5 s later, if no gesture is open: `displayPolicy.settle` → `Device.clearTransientUpdate(false)` →
   SF `CLEAR_TRANSIENT_UPDATE(0)`; then `commit(partial, qualityDamage)` → one more reconcile frame
   (`:159-172`) [code]. A new pen-down before that cancels the timer; the transient stays requested
   until the next quiet 5 s (`kt/InkRefreshPolicy.kt:11-28`) [code]. Any full `pause()` (overlay,
   focus loss, close) also clears it early through `releaseDisplayMode` → `displayPolicy.reset()`
   (`kt/OnyxInk.kt:594, 660-668`); a reconfigure pause does so only while a gesture is open [code].
5. After the lift the moved selection changes `selection*` (and the menu's overlay rect), and the
   configure that was deferred during the gesture runs (`hw/onyx-ink.ts:164-167, 254-257, 319-331`)
   → native `pause(tearDown=false)` / `resume()`: render already off (no pen-state transaction),
   input off/on, region mode, exclude n=0, limit re-pushed, width/colour/style ×2, and a
   `commit(full)` if the overlay rects changed (`kt/OnyxInk.kt:316-374`) [code].

Finger drag: `startFinger` handles touch pointer events on the canvas only inside the selection
(`hw/ink-canvas.tsx:572-589`); JS `publish` per move; no native call during the drag (no transient
mode, no pen-state change) [code]. `beginGesture` sets `gestureAction = "move"`, so `interacting`
becomes true and holds any configure back; at the end, `interacting` false plus the moved
selection bounds send one configure with the same effects as step 5 above [code]. (The earlier
version said "no native call until the commit effect" — the post-drag reconfigure was missing.)

### 5.8 Pen tap on the floating selection menu

`begin`: `inkOverlayHit` → `swallowed = true` → `setRender(false)` → SF `ENABLE_POST(1)`, pen **3**,
`ENABLE_POST(1)` if render was on; no event to JS (`kt/OnyxInk.kt:768-774`). The tap reaches the DOM
button through the WebView's ordinary input [inferred from the comment at `:764-767`; the canvas
itself ignores pen pointers while native ink is on, `hw/ink-canvas.tsx:595`]. `end` →
`restoreToolRendering()` → pen **2** if the tool renders (`:796-799`) [code]. With the lasso tool and a
selection (the only case the menu exists) `toolRendering` is false, so render stays off [code]. The
menu's actions change the draft/selection → commit and/or configure follow [code].

### 5.9 Switch tool, change width / eraser / lasso mode

1. `setTool(id)` (and `setSelected([])` unless lasso) (`hw/handwriting-note-view.tsx:540-550`);
   `useEffect([tool, width, lassoMode, ...])` → `configure()` (`hw/onyx-ink.ts:319-331`) [code].
2. Native: args differ in more than overlays → `pause(releaseDisplay=false, tearDown=false)`:
   render off (SF `ENABLE_POST(1)`, pen **3**, `ENABLE_POST(1)` if it was on), input off,
   `region.invalidateAll()`, `webView.invalidate()` (GU); then tool parameters (§5.2 step 7) and
   `resume()` (rects, defaults, input on, render per tool → pen **2** if rendering) [code]. No
   `cycleScribbleSession`; no commit unless the sheet, limit or overlay rects changed
   (`kt/OnyxInk.kt:371`) — switching away from the lasso clears the selection and with it the menu's
   overlay rect, so that switch does commit [code].
3. The width / eraser / lasso options live in a `Popover` (`hw/handwriting-note-view.tsx:610-744`),
   i.e. an overlay (§5.10). Picking an option calls `setOptionsOpen(false)` directly
   (`:628-631, 657-660, 684-687, 727-731`); for this controlled popover the suppression follows the
   `open` prop (`app/ink-suppression.ts:98-110`, `src/components/ui/popover.tsx:8-20`), but
   `onOpenChange` is not called on a programmatic close, so **no full refresh** is scheduled in that
   case; a dismissal by outside press / Escape goes through `onOpenChange(false)` and does schedule
   one [inferred from base-ui semantics: `onOpenChange` fires on user-initiated changes; the wiring
   is code].
4. Eraser mode (stroke/pixel/lasso) and radius are JS-only; they do not reach `configure` (not in
   `args`, `hw/onyx-ink.ts:178-204`) [code].

### 5.10 Open / close a menu, dialog, popover, select; text focus; soft keyboard

Open (any of `Dialog`, `Popover`, `Select`, the note `Menu`, the new-note `Menu`, the note-row
`Menu`; `src/components/ui/{dialog,popover,select}.tsx`, `hw/handwriting-note-view.tsx:111-113`,
`hw/new-note-button.tsx:38`, `src/features/pages/note-row-menu.tsx:38`) [code]:

1. `useOverlayInkSuppression` → `suppress_onyx_ink("overlay:<id>", true)` (`app/ink-suppression.ts:103-106`).
2. Native: `inkPauses.pause` → changed → `pauseStateChanged()` → `pause(releaseDisplay=false)`
   with `tearDown=true` (`kt/OnyxInk.kt:273-278`, `kt/MobileSystemPlugin.kt:180-195`) [code]:
   - `gate.pause()`: render off (SF `ENABLE_POST(1)`, pen **3**, `ENABLE_POST(1)` if on), input off.
   - `cycleScribbleSession()`: SF `ENABLE_POST(1)`, pen **3** — unconditionally.
   - `region.invalidateAll()`, `reconcile.reset()`, `webView.invalidate()` (mode GU, SESSION kept),
     `eraserRenderGate.reset()`, open gesture → `cancel` event.
3. The overlay itself is drawn by the WebView (GU) [inferred].

Close:

4. `refreshPanelAfterClose` → `requestFullRefreshSoon()` → after 300 ms `request_full_refresh` →
   `EpdController.invalidate(webView, GC)` → full GC16 [code; waveform inferred]
   (`app/eink-refresh.ts:37-60`, `kt/MobileSystemPlugin.kt:340-357`). Only when `onOpenChange(false)`
   fires (see §5.9 step 3) [code].
5. Suppression release after 300 ms (`OVERLAY_RESUME_MS`, `app/ink-suppression.ts:108`) →
   `pauseStateChanged()` → `resumeGate` after `RESUME_DELAY_MS` = 500 ms on a colour panel
   (`kt/OnyxInk.kt:226-227, 277`) → `resume()` → rects, defaults, input on, render per tool → pen **2**
   [code]. So on a colour panel: GC at ≈300 ms after close, pen back at ≈800 ms [inferred timing].

Only transitions reach native code: `InkPauseRegistry.pause` returns true only for the first reason
and `resume` only for the last (`kt/InkPauseRegistry.kt:20-30`), and `suppressOnyxInk` calls
`pauseStateChanged()` only then (`kt/MobileSystemPlugin.kt:186-189`). A second overlay opened over a
first one, or text focus inside an open dialog, sends nothing [code]. The JS side also drops
repeats (`app/ink-suppression.ts:31-39`) [code].

Text focus: `focusin` on an input/textarea/contentEditable → reason `text-focus` → same pause;
`focusout` with no editable focus → release (`app/ink-suppression.ts:63-88`) [code].

Soft keyboard: WebView insets listener → `imeChanged(true)` → reason `ime` → pause
(`kt/MobileSystemPlugin.kt:76-80, 112-120`, `kt/OnyxInk.kt:258-265`); on hide: release → resume after
500 ms, and **`fullRefreshSoon`** → GC after 300 ms (`kt/MobileSystemPlugin.kt:99-120`) [code]. The
`ime` reason is recorded only through `onyxInk?.imeChanged` (`:116`): while no `OnyxInk` object
exists yet (before the first handwriting page of the process) the keyboard pauses nothing, but its
hide still schedules the GC [code]. Rename
opens a `Dialog` with an input, so it combines overlay + text-focus + IME [code for the dialog;
keyboard appearance inferred].

### 5.11 Scroll the sheet, resize

1. Any `scroll` event in capture phase anywhere in the document, window `resize`, or the canvas /
   viewport `ResizeObserver` → `configure()` (`hw/onyx-ink.ts:293-299`) [code]. Deduplicated only by
   the JSON of the args (`:205-207`), which changes with every scroll offset [inferred].
2. Deferred while a native gesture is open or `interacting` (`:164-167`) [code].
3. Native per call: `pause(tearDown=false)` → render off (SF `ENABLE_POST(1)`, pen **3**,
   `ENABLE_POST(1)`), input off, `webView.invalidate()`; new limit; SF width/colour/style (×2 each);
   `resume()` → region mode, exclude n=0, limit, brush/eraser flags, input on, render on (pen **2**);
   `commit(full)` because the sheet moved → reconcile frame (mode 524290) of the whole limit once the
   WebView acknowledges (`kt/OnyxInk.kt:316-374`) [code].
4. The SDK's host-view scroll listener is disabled (no-op anyway) and the reader's screen offset
   (`getLocationOnScreen`) is refreshed on `start`/`resume` (`pen/RawInputReader.java:226-233,
319-333`) [code].
5. If the sheet is scrolled completely out of the viewport, `limit` becomes empty (`:325-327`) and
   `resume()` returns at its first line (`limit.isEmpty`, `:572`), so after the reconfigure pause the
   session stays with render and input off (pen state 3 if render had been on) and `canPresent()` is
   false, so no reconcile runs, until a later configure yields a non-empty limit [code]. The
   tool-parameter step still calls `setLimitRect(emptyRect)`, which `RawInputReader.setLimitRect`
   forwards because it only rejects `null` (`pen/RawInputReader.java:410-416`), so SF receives an
   empty (screen-offset) limit [code]; how SF treats an empty limit is [unknown].

### 5.12 Undo / redo

1. `navigateHistory` → `setHistoryBusy(true)` (`hw/handwriting-note-view.tsx:471-486`) →
   `InkCanvas.disabled` true (`:798`) → `useOnyxInk.enabled` false → effect cleanup → queued
   `configure_onyx_ink {enabled:false}` (`hw/onyx-ink.ts:300-315`) [code].
2. Native `configure` with `enabled:false` and matching session → `close()` (`kt/OnyxInk.kt:281-284,
702-725`) [code]:
   - `pause()` (full): SESSION cleared (WebView mode falls back to BASE GU), render off (SF
     `ENABLE_POST(1)`, pen **3**, `ENABLE_POST(1)` if on), input off, `cycleScribbleSession()` (SF
     `ENABLE_POST(1)`, pen **3**).
   - `closeRawDrawing()`: SF style 0, `ENABLE_POST(1)`, pen **3**, pen **0**, style 0,
     `ENABLE_POST(1)`, pen **0** (§3).
   - `cycleScribbleSession()` again (SF `ENABLE_POST(1)`, pen **3**) and exclude n=0.
   - `closeRefresh` posted: `EpdController.invalidate(webView, GC)` 300 ms later → full GC16
     [code; waveform inferred]. It is not cancelled by a reopen (the reopen's `close()` has
     `hadSession = false`) [code].
3. History update applied → `historyBusy` false → `enabled` true → new session id → §5.2 from step 1
   (the `OnyxInk` object is reused; `helper` is null, so a new `TouchHelper` is created) [code].

### 5.13 Switch pages, leave the page

- Page switch: `HandwritingNoteView` is keyed by `page.uuid` (`src/features/workspace/workbench.tsx:365-367`),
  so the old view unmounts → `enabled:false` → `close()` as §5.12 step 2; the new view opens a new
  session as §5.2 [code]. Pane navigation from a handwriting page to a text page leaves only the close.
- Back / leave: `prepareLeave` → `setLeaving(true)` → `disabled` → `close()` (`hw/handwriting-note-view.tsx:417-431, 798`)
  [code].
- Other triggers of the same `close()` (added in verification): `editing` turning false (ownership
  lost, note opened in another pane; `disabled` includes `!editing`, `:798`); `pagehide`
  (`setSuspended(true)`, `:352-357`); and a canvas whose bounding rect has zero width or height,
  because `configure` then sends `enabled:false` for the same session (`hw/onyx-ink.ts:180`,
  `kt/OnyxInk.kt:281-284`) [code].

### 5.14 Background / foreground, window focus

Background (Home, app switch) [code]:

1. JS `visibilitychange` → both `configure()` (now `enabled:false` because `visibilityState ===
"hidden"`, `hw/onyx-ink.ts:180, 299`) and `suspend()` → `suspended` → `disabled`
   (`hw/handwriting-note-view.tsx:326-363`) → `close()` as §5.12 step 2.
2. Native (order vs. step 1 [unknown]): window focus lost → `onWindowFocusChanged(false)` → `pause()`
   (full, with teardown); Activity `onPause` → plugin → `OnyxInk.onPause()` → `resumed=false`,
   `pause()` again (`kt/OnyxInk.kt:561-569`, `kt/MobileSystemPlugin.kt:138-141`). Each `pause()` sends
   `cycleScribbleSession()` (SF `ENABLE_POST(1)`, pen **3**) regardless of state [code].

Foreground [code]:

3. `onResume` → `resumed=true`, `resume()` returns early if the window has no focus yet;
   `onWindowFocusChanged(true)` → `resumeGate` after 500 ms (`kt/OnyxInk.kt:562-569`).
4. JS visible → `resume()` → `openNote()` → `adopt` → `suspended=false` → `enabled` → new session
   (§5.2) [code]. If the session was already closed, `resume()` in step 3 returns early
   (`config == null`) [code].

Window focus loss without backgrounding (notification shade, system dialog, another window):
`pause()` with teardown and SESSION release; regain → `resume()` after 500 ms → SESSION GU set again
→ `commit(full)` → whole-region reconcile (`kt/OnyxInk.kt:563-584`) [code].

### 5.15 Plugin destroy / WebView re-creation

`onDestroy` → `destroy()` → `close()` + focus listener removed + `ime` reason dropped
(`kt/MobileSystemPlugin.kt:122-130`, `kt/OnyxInk.kt:732-738`). A re-created WebView makes the plugin
destroy the old session before creating a new one (`kt/MobileSystemPlugin.kt:152-160`) [code].

### 5.16 Configure failure (added in verification)

Any exception inside `configure` after geometry is computed (digitizer range 0, SDK failure, the
`isRawDrawingCreated` check) runs `close()` and rethrows (`kt/OnyxInk.kt:377-381`) [code]: the full
close sequence of §5.12 step 2, including the GC 300 ms later if a helper existed. The plugin rejects
the invoke (`kt/MobileSystemPlugin.kt:169-171`); JS sets `unsupported = true`, sends
`enabled:false` for the session (ignored, the session is already closed) and reports
`available:false` (`hw/onyx-ink.ts:218-225`). `unsupported` stops every later `configure()` of that
effect run, so native ink stays off until `enabled` toggles or the view remounts [code]. The
`check(...)` for Android ≥ 10 / hardware acceleration (:296) runs before the `try` and throws without
closing [code].

## 6. Tables

### 6.1 SF-visible effect per action

"PS" = pen-state transactions we send, in order. "Render" is the TouchHelper render flag after the
action. "Refresh" lists every refresh request we make (not what SF adds on its own).

| Action                                               | PS sent                                                                                                       | Render after        | Input after                          | Limit / exclude / region mode                                                                          | WebView update mode                       | Transient                                                                                                         | Refresh we issue                                                                              |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- | ------------------- | ------------------------------------ | ------------------------------------------------------------------------------------------------------ | ----------------------------------------- | ----------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| App start                                            | 3                                                                                                             | —                   | —                                    | exclude n=0                                                                                            | BASE GU (profile)                         | cleared on first `OnyxInk`                                                                                        | EAC profile write once (causes a redraw per code comment, `kt/MobileSystemPlugin.kt:288-290`) |
| Open page (pen tool)                                 | 3, 1, 1, 2                                                                                                    | on                  | on                                   | limit=whole WebView, then sheet (×3); exclude n=0; mode 1 (×2)                                         | SESSION GU; one frame 524290              | CLEAR(0) at `OnyxInk` creation                                                                                    | whole-limit reconcile frame (524290)                                                          |
| Open page (eraser / rect lasso)                      | 3, 1, 1                                                                                                       | off                 | on                                   | same                                                                                                   | same                                      | same                                                                                                              | same                                                                                          |
| Write                                                | none                                                                                                          | on                  | on                                   | none                                                                                                   | 524290 for one frame after each stroke    | none                                                                                                              | reconcile frame per stroke (stroke ∪ previous stroke gate; whole WebView invalidated)         |
| Stylus eraser end (pen tool)                         | 3 at down, 2 after reconcile frame                                                                            | on                  | on                                   | none                                                                                                   | 524290 for one frame                      | none                                                                                                              | reconcile frame                                                                               |
| Stylus eraser end (lasso, inside selection grab box) | none (steady 3)                                                                                               | off                 | on                                   | re-pushed by the configure after lift (selection cleared)                                              | 524290 frame                              | `APPLY_TRANSIENT_UPDATE(2308)` at down (no erasing guard); `CLEAR(0)` 5 s after last up or at the next full pause | reconcile frame                                                                               |
| Eraser tool, per gesture                             | none (steady 3)                                                                                               | off                 | on                                   | none                                                                                                   | 524290 for one frame                      | none                                                                                                              | reconcile frame                                                                               |
| Lasso freehand, no selection                         | none; then 3 when the selection is set                                                                        | off after configure | on                                   | re-pushed by configure                                                                                 | 524290 frame                              | none                                                                                                              | reconcile frame                                                                               |
| Lasso freehand with selection                        | 2 at down; 3 at following configure                                                                           | off                 | on                                   | re-pushed                                                                                              | 524290 frame                              | none                                                                                                              | reconcile frame                                                                               |
| Lasso rectangle                                      | none (steady 3)                                                                                               | off                 | on                                   | none / re-pushed if selection changes                                                                  | 524290 frame                              | none                                                                                                              | reconcile frame                                                                               |
| Pen drag of selection                                | none (steady 3)                                                                                               | off                 | on                                   | none during the drag; re-pushed (region mode, exclude n=0, limit) by the deferred configure after lift | 524290 frame                              | `APPLY_TRANSIENT_UPDATE(2308)` at down; `CLEAR(0)` 5 s after last up                                              | reconcile frame; one more after settle                                                        |
| Finger drag of selection                             | none                                                                                                          | unchanged           | off/on once (reconfigure after lift) | re-pushed by the configure after lift                                                                  | 524290 frame via commit                   | none                                                                                                              | reconcile frame                                                                               |
| Pen tap on selection menu                            | 3 (if render was on)                                                                                          | tool default        | on                                   | none                                                                                                   | —                                         | none                                                                                                              | none                                                                                          |
| Tool switch                                          | 3 (if on), 2 (if tool renders)                                                                                | per tool            | on                                   | re-pushed                                                                                              | GU                                        | none                                                                                                              | none                                                                                          |
| Overlay / text focus / IME open                      | 3 (if on), 3                                                                                                  | off                 | off                                  | none                                                                                                   | SESSION GU kept                           | none                                                                                                              | none                                                                                          |
| Overlay close                                        | 2 after 300 + 500 ms (if tool renders)                                                                        | per tool            | on                                   | re-pushed                                                                                              | GU; 524290 frame when next reconcile runs | none                                                                                                              | GC (98) after 300 ms if `onOpenChange(false)` fired                                           |
| IME hide                                             | 2 after 500 ms                                                                                                | per tool            | on                                   | re-pushed                                                                                              | GU                                        | none                                                                                                              | GC (98) after 300 ms                                                                          |
| Scroll (per event)                                   | 3, 2                                                                                                          | on / per tool       | on                                   | limit moved, exclude n=0, mode 1                                                                       | 524290 frame                              | none                                                                                                              | whole-limit reconcile frame                                                                   |
| Undo / redo                                          | 3, 3, 3, 0, 0, 3, then reopen 3, 1, 1, 2                                                                      | on / per tool       | on                                   | exclude n=0 ×2 (close, reopen resume), limit ×4 (whole WebView at `TouchHelper.create`, then sheet ×3) | SESSION dropped and re-set                | none (see note)                                                                                                   | GC (98) 300 ms after close + whole-limit reconcile after reopen                               |
| Page switch / leave                                  | 3, 3, 3, 0, 0, 3                                                                                              | closed              | closed                               | exclude n=0                                                                                            | back to BASE GU                           | none                                                                                                              | GC (98) after 300 ms                                                                          |
| Background                                           | close: 3 (if on), 3, 3, 0, 0, 3; plus 3 from each of focus loss and `onPause` (order vs. the close [unknown]) | closed              | closed                               | exclude n=0                                                                                            | BASE GU                                   | none                                                                                                              | GC (98) after 300 ms                                                                          |
| Foreground                                           | 3, 1, 1, 2 (new session)                                                                                      | per tool            | on                                   | as open                                                                                                | SESSION GU                                | none                                                                                                              | whole-limit reconcile                                                                         |
| Window focus lost / regained                         | 3 (if on), 3 / 2 after 500 ms                                                                                 | per tool            | on                                   | re-pushed                                                                                              | SESSION dropped / re-set                  | none                                                                                                              | whole-limit reconcile after regain                                                            |

(The "Undo/redo" transient cell: `Device.clearTransientUpdate(false)` runs only in the `OnyxInk`
constructor, `kt/OnyxInk.kt:248`, and the plugin keeps the object across `close()`; so no CLEAR is
sent on reopen [code].)

### 6.2 Refreshes we issue, complete list

| Refresh          | Call                                                                                                                                               | Mode                                         | Scope                                                                   | When                                                                                     |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------- | ----------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| Reconcile frame  | `View.setDefaultUpdateMode(524290)` + `webView.invalidate()`, reset on frame commit                                                                | `HAND_WRITING_REPAINT_MODE` (GU + `0x80000`) | whole WebView invalidated; actual damage decided by Chromium [inferred] | after every commit + (pen-up refresh or 120 ms), `kt/OnyxInk.kt:499-545`                 |
| Pause invalidate | `webView.invalidate()`                                                                                                                             | current default (GU)                         | whole WebView                                                           | every `pause()` (`:609`) and every reconfigure                                           |
| Close GC         | `EpdController.invalidate(webView, GC)`                                                                                                            | 98 `GC16\|FULL\|WAIT`                        | full                                                                    | 300 ms after `close()` with a live helper (`:721-730`)                                   |
| IME-hide GC      | same                                                                                                                                               | 98                                           | full                                                                    | 300 ms after keyboard hides (`kt/MobileSystemPlugin.kt:112-120`)                         |
| Overlay-close GC | `request_full_refresh` → same                                                                                                                      | 98                                           | full                                                                    | 300 ms after `onOpenChange(false)` of Dialog/Popover/Select/Menu (`app/eink-refresh.ts`) |
| Debug            | `debug_ink_repaint`: `handwritingRepaint`, `invalidate(GU)`, render/input toggles, exclude resets, session cycle, region mode, `leaveScribbleMode` | various                                      | —                                                                       | manual only (`kt/OnyxInk.kt:397-417`)                                                    |

There is no `handwritingRepaint`, `refreshScreen`, `repaintEverything`, `applyGCOnce` or scheme call
anywhere in the non-debug path [code]. The firmware's own tap-counted full refresh also applies to
our app since `c89bc5f` [doc: handoff].

### 6.3 Update-mode layers on the WebView

| Layer     | Set by                                                          | Value  | Cleared by                                                                                                                 |
| --------- | --------------------------------------------------------------- | ------ | -------------------------------------------------------------------------------------------------------------------------- |
| BASE      | `setDisplayProfile(eink=true)` (`kt/MobileSystemPlugin.kt:265`) | GU (2) | `setDisplayProfile(false)`                                                                                                 |
| SESSION   | `resume()` when not set (`kt/OnyxInk.kt:574-578`)               | GU (2) | `releaseDisplayMode()` in every full `pause()` (not in reconfigure or named-pause pauses unless a gesture is open, `:594`) |
| TRANSIENT | `refreshFrame` via `InkRepaintMode.acquire`                     | 524290 | frame committed, `cancelFrameSubmission`, `releaseDisplayMode`                                                             |

The DecorView's mode is never touched [code]; whole-window traversals in `ViewRootImpl` use
`getViewUpdateMode(this.mView)` of the root view (`fwv/ViewRootImpl.java:3795, 4355-4360`) [code],
so such entries carry the DecorView's default (5, AUTO, `fwv/View.java:2321`) [inferred that the root
view keeps its default].

## 7. Device-specific branches (Note Air 4C)

| Branch                                               | Site                                                                                                       | Condition                                                                                                                                                                                                                                          | Branch taken on Note Air 4C                                    | How determined                                                                                                                                             |
| ---------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Pen path enabled at all                              | `OnyxInk.supported()`, `kt/OnyxInk.kt:203`; plugin commands `:147, :233, :344`; `clearStaleInkState` `:91` | `Build.MANUFACTURER` equals `ONYX` (case-insensitive)                                                                                                                                                                                              | enabled                                                        | [inferred] (BOOX reports ONYX)                                                                                                                             |
| E-ink display kind                                   | `DisplayKind.of`, `kt/DisplayKind.kt:13-14`                                                                | manufacturer `onyx`                                                                                                                                                                                                                                | `eink`                                                         | [inferred]                                                                                                                                                 |
| Colour panel for the frontend                        | `DisplayKind.colorPanel()` `kt/DisplayKind.kt:21`                                                          | always `null`                                                                                                                                                                                                                                      | frontend decides mono vs colour ink                            | [code]                                                                                                                                                     |
| Hidden-API exemption                                 | `VendorAccess.ensure`, `kt/VendorAccess.kt:17-26`                                                          | SDK ≥ P                                                                                                                                                                                                                                            | exemption requested                                            | [code]; whether granted [unknown] (status `available`/`error` would show)                                                                                  |
| Minimum platform                                     | `kt/OnyxInk.kt:296-298`                                                                                    | SDK ≥ Q and WebView hardware-accelerated                                                                                                                                                                                                           | passes                                                         | [inferred]                                                                                                                                                 |
| Resume delay                                         | `RESUME_DELAY_MS`, `kt/OnyxInk.kt:226-237`                                                                 | `Device.getColorType() > 0`; only a thrown exception defaults to colour — a failed reflective call returns 0 from `SDMDevice.getColorType` (`dev/device/SDMDevice.java:2783-2789`, `invokeMethodSafely` swallows errors) and so selects **150 ms** | 500 ms if SF answers `GET_COLOR_TYPE` > 0                      | [inferred]: Kaleido 3 is a colour panel; the SF answer is [unknown]; `status().colorPanel` / `resumeDelayMs` would confirm                                 |
| Device backend                                       | `Device.currentDevice()`                                                                                   | board platform                                                                                                                                                                                                                                     | `SDMDevice` (`lito`)                                           | [code] + [doc]                                                                                                                                             |
| `View.invalidate(int)` vs `invalidateWithUpdateMode` | `SDMDevice.java:1124-1128`                                                                                 | method presence                                                                                                                                                                                                                                    | `View.invalidate(int)` exists (`fwv/View.java:10788`)          | [code]                                                                                                                                                     |
| Eraser raw-drawing signature                         | `SDMDevice.java:1249-1253, 2826-2833`                                                                      | 2-arg method present                                                                                                                                                                                                                               | 2-arg (`fw/ViewUpdateHelper.java:1167`)                        | [code]                                                                                                                                                     |
| EAC profile write                                    | `kt/MobileSystemPlugin.kt:292-331`                                                                         | reflection hooks present, marker unset                                                                                                                                                                                                             | written once                                                   | [inferred]; marker in app SharedPreferences                                                                                                                |
| Display profile                                      | `app/appearance.tsx:124-137`, `src/app/display-profile.ts:56-57`                                           | user setting `auto` → `DisplayKind` says `eink`; or the user forces `eink`/`standard`                                                                                                                                                              | BASE = GU with `auto` or `eink`; no BASE layer with `standard` | [inferred] from `DisplayKind`; the setting in use on the device [unknown]. Before `displayInfo` arrives, `auto` resolves to `standard` for one pass [code] |

There are no checks for model name, tablet vs phone, pen side button, `supportEAC`, `supportRegal`,
or any Onyx feature flag in `kt/`, `hw/` or `app/` [code, by grep]. The SDK paths we use contain no
colour/mono or tablet/phone branch either (read: `TouchHelper`, `SFTouchRender`, `AppTouchRender`,
`RawInputManager`, `RawInputReader`, `EpdPenManager`) [code]. The stylus has no side button
[doc: brief]; `enableSideBtnErase` is never called by us [code].

## 8. Interactions with other subsystems

- **Session lifecycle.** A session is tied to one `useOnyxInk` effect run (session id). Anything that
  toggles `enabled` — `disabled` (not editing, suspended, historyBusy, leaving), unmount, hidden
  document — closes the `TouchHelper` and SF pen session; everything else (named pauses, focus,
  reconfigure) keeps the helper and toggles render/input [code].
- **Writing.** The firmware draws; the web canvas takes over at the reconcile. The JS canvas draws the
  stroke only when the native `stroke` list arrives at pen-up; there is no live web preview for the
  pen tool (`hw/ink-canvas.tsx:427, 440`) [code].
- **Erasing.** Always in JS (`eraseAt` / `eraseGesture`); the firmware never renders erase ink
  (`setEraserRawDrawingEnabled(false, …)`) [code]. Render is off for every erase [code].
- **Selection.** Selection frame, lasso polygon (except the freehand trace), menu, and drag preview
  are web content. The freehand trace is firmware ink with style DASH [code].
- **Refresh.** Per-stroke reconcile via view mode 524290; full GC after closes and overlay closes;
  transient A2-quality during a drag. EinkWise profile `refresh_mode_4` written once [code].
- **Input routing.** Pen: SDK raw reader only (canvas ignores pen pointer events while native ink is on,
  `hw/ink-canvas.tsx:595`). Finger: WebView pointer events; only a selection drag is handled on the
  sheet (`:572-589`); scroll is native WebView scrolling of `[data-ink-viewport]`
  (`hw/handwriting-note-view.tsx:785-788`) [code]. No capacitive cut-out is ever armed
  (`kt/OnyxInk.kt:690-700`) [code]. The firmware's own finger / palm handling while render is on is
  [unknown].
- **Firmware.** Exclude table always empty (reset on start, every resume, every close); limit =
  visible sheet; single-region mode; no pen state 4; `ENABLE_POST(0)` only at `openRawDrawing` [code].

## 9. Facts useful for the comparison (no judgement)

1. `configure` runs on every scroll event and pauses/resumes render each time [code].
2. The reconcile invalidates the whole WebView, not the computed rectangle [code].
3. The App touch render never sees MotionEvents but still emits `ENABLE_POST(0)` at open and
   `ENABLE_POST(1)` at every render-off and at close [code].
4. `cycleScribbleSession` bypasses the TouchHelper guard and sends pen state 3 on every full pause,
   named pause, close and app start, including when no session exists [code].
5. When a session opens with a non-rendering tool, the last pen state sent is 1 until the next pause
   [code].
6. Undo/redo, page switch, leave and background each produce a session close with a GC 300 ms later
   [code].
7. After an erase with the pen tool, pen state returns to 2 only after the reconcile frame commits
   [code].
8. With the eraser tool, erasing is a drawing gesture to the SDK, so the pen-up refresh fires for it
   [code].
9. A freehand lasso that selects nothing leaves render on with the DASH style until the next
   configure [inferred].
10. The programmatic close of the tool-options popover schedules no GC; an outside-press close does
    [inferred].

## 10. Open questions as hardware experiments

Common setup for all: `adb logcat -c && adb logcat -b all -v time` recorded in the background (per
the user's logcat rule), filtered for `onyx_tcon_send_cmd`, `onyx_tcon_set_upd_scheme`,
`HWEpdcManager: ### EUS`, `SurfaceFlinger: Pid .* State change`, and `OnyxInk`. `status()` is readable
through the Tauri MCP bridge as `configure_onyx_ink` with `enabled:false` of a non-matching session
[doc: handoff]. Each experiment is to be run in our app; the same action in stock Notes is the
comparison.

1. **Eraser end, pen tool: when does PAUSE land relative to contact?** Erase one short stroke with the
   stylus back end. Confirm: `State change to: toPause` appears after the first eraser samples
   (timestamps vs. the first `onyx_tcon_send_cmd` of the gesture); then `EUS s 4` / `waveform[4]`
   updates during the stroke; `running` after the reconcile; `EUS s 2` + one
   `waveform[2] update_mode[1]` ≈1.5 s after lift. Refutes the model if no `toPause` appears or if
   the scheme switch precedes it.
2. **Eraser tool: steady PAUSE.** Select the eraser tool, wait, erase. Expect no `State change` lines
   during the gesture, `EUS s 4` during it and the full GC16 after lift. Check whether
   `onPenUpRefresh` fired (`status().penUpRefreshCount` increments) — confirms §5.5 step 4.
3. **Session opened with a non-rendering tool leaves pen state 1.** The tool resets to pen on remount
   (`hw/handwriting-note-view.tsx:182`) but survives undo, which reopens the session (§5.12). Select
   the eraser tool, write one stroke, tap undo, then write again with the tip. Expect after the undo:
   `toPause`, a STOP, then a `start` with no following `running` or `toPause`. Observe whether SF inks
   the tip stroke, whether `EUS s 4` appears, and whether the full GC16 follows — i.e. whether pen
   state 1 behaves like 2 or like 3 for the flash mechanism.
4. **Scroll cost.** Scroll the sheet with a finger for 2 s. Count `State change to: toPause/running`
   pairs and `onyx_tcon_send_cmd` lines with `update_mode[1]`; confirm one pause/resume per scroll event
   and whether any scheme switch or GC16 results.
5. **Reconcile mode on the wire.** Write one stroke. Identify the reconcile update ~500 ms after lift:
   expect a partial update with `flags` containing the handwrite bit (0x80000 path per 08 §Q3) and
   waveform GU; record its rectangle (whole WebView vs. stroke) — settles whether Chromium limits the
   damage.
6. **Undo cost.** Tap undo once. Expect: `toPause`, pen STOP (`State change to: stop`), a full GC16
   ≈300 ms later, then `start`/`running`. Confirms §5.12; measure time from tap to the GC.
7. **Overlay close.** Open and dismiss the note menu (outside tap) vs. choose a pen width in the tool
   popover (programmatic close). Expect a `update_mode[1]` GC16 only in the first case, ≈300 ms after
   close, and `running` ≈800 ms after close in both.
8. **Selection drag transient.** Drag a selection with the pen. Expect an A2-quality waveform during the
   drag with no `State change` lines, and a clear of the transient 5 s after lift (look for any update
   right after). Confirms that the drag happens in pen state 3 and whether it also triggers the
   scheme-4/GC16 sequence.
9. **Freehand lasso that selects nothing.** Draw a lasso loop on empty paper, then another. The render
   flag is not in `status()`; the observable is the log: no `toPause` after the first loop and
   dashed firmware ink on the second confirm §9 item 9; a `toPause` after the first loop refutes it.
10. **Colour type.** Read `status().colorPanel` and `resumeDelayMs` once; confirms the 500 ms branch.
11. **Pen tap on the selection menu.** Tap a menu button with the pen. Look for a `toPause` at the tap
    and any ink dot left at the tap point; confirms §5.8 ordering (render off only after the main-thread
    hop).
12. **Background/foreground.** Press Home and return. Count `toPause` lines (expected ≥3 from close,
    focus loss and onPause) and whether the GC16 after close is emitted while the app is already
    hidden.
13. **Eraser end inside a selection (lasso tool).** Make a selection, then touch the eraser end
    inside its box and erase. Expect `transient animation quality requested` in the `OnyxInk` log,
    A2-quality updates during the stroke, and `transient mode cleared` ≈5 s after lift; compare
    with an eraser-end touch outside the selection (no transient). Confirms §5.4's added lasso
    branch.
14. **Sheet scrolled out of view.** Scroll until the sheet leaves the viewport and back. Expect one
    `toPause` (if render was on) and no `running` while the sheet is off screen, and a `running` once
    it is back; record what the empty-limit transaction does to later pen input outside the app.

## 11. Verification notes

An independent re-check on 2026-09-25, static only (no adb, no device). Every row of §3 was
re-opened in the cited source; §5 and §6 were re-traced from `OnyxInk.kt`, `MobileSystemPlugin.kt`,
the gate classes, `onyx-ink.ts`, `ink-canvas.tsx`, `handwriting-note-view.tsx`,
`ink-suppression.ts`, `eink-refresh.ts` and `popover.tsx`.

Checked and confirmed as written:

- SDK resolution: `build.gradle.kts:33` declares `onyxsdk-pen:1.5.4.3`; the Gradle-cache POMs pull
  `onyxsdk-base:1.8.5.4` → `onyxsdk-device:1.3.5.2`.
- `TouchHelper` guards (`f76a` session flag plus cached render/input flags, `TouchHelper.java:188-223`),
  render-list order SF then App (`:358-375`), `openRawDrawing`/`closeRawDrawing` emission order
  (`SFTouchRender.java:171-203, 265-284`, `AppTouchRender.java:186-206`), pen-state constants
  (`EpdPenManager.java:9-13, 35-50`), `leaveScribbleMode` = `enablePost(1)` (`SDMDevice.java:1764-1777`),
  `ENABLE_POST` payload `(-1, enable, pid)` and pen-state payload `(state, pid)`
  (`fw/ViewUpdateHelper.java:602-608, 1195-1200`).
- `RawInputReader` pen-up timer: armed only for non-erasing gestures (`:613-616`), default 500 ms and
  enabled (`:131, 134`), delivered only with input enabled (`:791-797`), union with the previous
  stroke (`:747-759`); `SFTouchRender` posts every callback, including `onPenUpRefresh`, to the
  host view (`SFTouchRender.java:36-137`), and `OnyxInk` posts it again.
- All pen-state sequences in §6.1 except the corrected cells; `cycleScribbleSession` bypassing the
  guard; the close-refresh not being cancelled by a reopen; `DisplayModeStack` layering;
  `View.invalidate(int)` / `setDefaultUpdateMode` / `resetUpdateMode` (`fwv/View.java:10788-10820,
13718-13719, 14577-14579`); `SDMDevice.invalidate` preferring `View.invalidate(int)`
  (`SDMDevice.java:1124-1128, 1365-1370`); the popover's `onOpenChange` wiring.
- Call-site coverage: a grep of `kt/` for `EpdController.`, `Device.currentDevice()` and `helper`
  found no pen/panel call missing from §3 other than the reads now added (touch range, view-mode
  getters).

Changed:

- §0: the Binder claim was [doc]; now [code] with the `transactData` citation, plus the fact that
  the one-argument static setters ignore the `View` receiver.
- §1 item 6: "each scroll sends a new `configure_onyx_ink`" → a send happens only when the args JSON
  changes.
- §1 item 10 / §3: mode 2308 for `ANIMATION_QUALITY` upgraded from [doc] to [code]; the eraser end
  inside a selection also requests the transient (new).
- §1 item 11 / §7: the display-profile user override added; "probe failure ⇒ colour" narrowed —
  only an exception gives colour, a failed reflective call gives 0 → 150 ms.
- §3 `setRawInputReaderEnable(false)`: the earlier "erase events are still delivered while input
  is off" was wrong (`RawInputReader.java:431` gates everything but state 6).
- §3 `setRawInputReaderEnable(true)`: "in-process" backed by the native library's ELF dependencies.
- §3: mode bit-field note (GU, GC and the reconcile mode share waveform nibble 2).
- §5.7 / §6.1: pen and finger selection drags are followed by a full reconfigure after lift (limit
  and exclude re-push, input off/on); the earlier "no native call" / "limit none" was incomplete.
- §5.9: the tool-switch commit also happens when the overlay rects change.
- §6.1 undo/redo: "exclude n=0 ×3, limit ×3" → ×2 and ×4; background row spelled out.
- Added: §5.4 eraser end with the lasso tool; §5.5 fate of a late pen-up refresh; §5.10 pause
  transitions only, and the IME reason needing an existing `OnyxInk`; §5.11 empty limit; §5.13
  other close triggers; §5.16 configure failure; experiments 13-14.

Not confirmed (left as tagged):

- Board platform `lito` for the Note Air 4C (drives `SDMDevice`; any other value not in
  `Device.m33b` would give a no-op `BaseDevice`) — [doc].
- SF-side semantics of every transaction (pen states, `ENABLE_POST`, empty limit, eraser/brush raw
  drawing flags, stroke parameters) — [doc] from 08-surfaceflinger.md or [unknown].
- How the WebView's hardware-accelerated frames carry the View's default update mode to SF
  (`ViewRootImpl.addEpdc`) — [inferred]; Chromium may produce frames that bypass `invalidateChild`.
- What `getColorType()` returns on this device, the display-profile setting in use, and whether
  the hidden-API exemption was granted — [unknown] until read from `status()`.
- Cross-queue ordering at page switch: the old view's `enabled:false` and the new view's configure
  are on different promise queues; either order ends with one close, but which one runs the close
  (and therefore which session's GC timer is armed) is [unknown].
- `onyxsdk-pennative:1.0.4` is also in the Gradle cache; which copy of `libonyx_pen_touch_reader.so`
  ends up in the APK was not checked (the ELF check used the one in the pen 1.5.4.3 AAR).
