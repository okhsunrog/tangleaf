# notate — how an open-source note app drives the BOOX pen and panel

Written 2026-09-25. Static reading only: the notate sources, the Onyx SDK AARs it resolves, and the
SurfaceFlinger (SF) / framework analysis in `../stock-notes/`. No adb, no tablet. The goal is to
compare notate, action by action, with stock Onyx Notes (`../stock-notes/00-overview.md`) and with
Tangleaf (`../stock-notes/08-tangleaf-current-path.md`). It proposes no change to Tangleaf.

## 0. Sources, tags, build

Tags as in the stock documents: **[code]** read in the cited source, **[inferred]** reasoned from code,
**[doc]** taken from `../stock-notes/*` without re-verifying, **[unknown]** not settled statically.

| shorthand | root                                                                                                                                         |
| --------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `nt/`     | `~/tmp_zfs/reference_notes_apps/notate/app/src/main/java/com/alexdremov/notate/` (git `522db15`, 2026-08-29, `github.com/alexdremov/notate`) |
| `pen/`    | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-pen-1.5.4.3/sources/com/onyx/android/sdk/pen/`                                                    |
| `dev/`    | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-device-1.3.5.2/sources/com/onyx/android/sdk/`                                                     |
| `fw/`     | `~/tmp_zfs/onyx_framework/out/sources/android/onyx/`                                                                                         |
| `fwv/`    | `~/tmp_zfs/onyx_framework/out/sources/android/view/`                                                                                         |

**Device build.** Firmware 4.2, `ro.build.display.id=2026-04-28_17-50_4.2-rel_04282_555977efe`,
system partition Android 13 (SDK 33), vendor partition Android 11 (SDK 30), `vendor.onyx.tablet=true`.
The device's `surfaceflinger` and `framework.jar` are byte-identical to the copies in
`~/tmp_zfs/onyx_framework/`, so the SF and framework analysis in `07` applies to this device
[verified on the tablet by the coordinator, 2026-09-25].

**SDK version.** `app/build.gradle.kts:167-168` declares `onyxsdk-pen:1.5.4` and `onyxsdk-base:1.8.5`
(exact versions, not the `1.5.4.3` Tangleaf links). The Onyx Maven metadata lists both. The
`pen-1.5.4` POM pulls `base 1.8.5`, `penbrush 1.1.1` and `geometry 1.1.3`. The `base-1.8.5` POM pulls
`onyxsdk-device 1.3.4` (both POMs read from `repo.boox.com/repository/maven-public`) [code]. So the
APK carries **pen 1.5.4 + device 1.3.4**. Both AARs were downloaded and decompiled with the same jadx
as `1.5.4.3`/`1.3.5.2` for a diff:

- pen 1.5.4 vs 1.5.4.3: `SFTouchRender`, `AppTouchRender`, `RawInputReader`, `RawInputManager` and
  `EpdPenManager` are identical. `TouchHelper` differs by a single `Debug.i` log line in
  `setRawDrawingEnabled` [code, diff]. Every `pen/` citation below therefore also describes notate's
  copy.
- device 1.3.4 vs 1.3.5.2: `EpdController` 1.3.5.2 adds `setScreenHandWritingRegionConfig`,
  `drawScreenHandWritingBitmap`, `setUpRegalPlus`, dither/saturation helpers and log lines. The
  methods notate uses (`invalidate`, `applyTransientUpdate`, `clearTransientUpdate(boolean)`,
  `setEpdTurbo(int)`, `setScreenHandWritingPenState`, `enter/leaveScribbleMode`, `setDisplayScheme`,
  `setViewDefaultUpdateMode`) and `EpdDeviceManager.enter/exitAnimationUpdate` are the same, and so
  is the `UpdateMode` → mode-int table in `SDMDevice.a(UpdateMode)` [code, diff]. `dev/` citations use
  the 1.3.5.2 line numbers.

The device class is `SDMDevice` (`lito`) as for stock and Tangleaf (`../stock-notes/00` §6) [doc].

## 1. Summary

1. **One SF-only `TouchHelper` on a `SurfaceView`.** `OnyxCanvasView` is a `SurfaceView`
   (`nt/ui/OnyxCanvasView.kt:46-53`) and creates `TouchHelper.create(this, true, penInputHandler)`
   (`:876`), i.e. feature 2 = `SFTouchRender` only, no `AppTouchRender` (`pen/TouchHelper.java:377-379,
   358-375`) [code]. Callbacks reach the app on the main thread through `SFTouchRender`'s `post`
   wrapper (`pen/touch/SFTouchRender.java:44-134`) [code].
2. **The render flag follows the tool.** Pen tool and the two lasso tools (lasso select, lasso
   eraser): render on (pen state 2). Stroke eraser, pixel ("standard") eraser, rectangle select, text
   tool, and pens wider than 10 mm on screen: render off (pen state 3)
   (`nt/ui/input/PenToolConfigurator.kt:31-83`, `nt/config/CanvasConfig.kt:66`) [code]. So the
   eraser, the rectangle select and wide pens all take pen input into a **paused** SF session, as
   Tangleaf's eraser does.
3. **Every pen gesture is bracketed by a transient A2 request.** `onBeginRawDrawing` calls
   `EpdDeviceManager.enterAnimationUpdate(true)` → `applyTransientUpdate(ANIMATION_QUALITY)` = 2308, and
   `onEndRawDrawing` calls `exitAnimationUpdate(true)` → `clearTransientUpdate(true)`
   (`nt/ui/input/PenInputHandler.kt:350-351, 419-420`; `dev/api/device/EpdDeviceManager.java:73-86`)
   [code]. In a paused session this puts SF in scheme 4 at pen-down and, at lift, back in scheme 2
   with a one-shot full-screen GC plus a full-screen mode-5 update (`../stock-notes/07` §5.2–§5.3)
   [inferred from 07]. In a running session SF is already in scheme 3 when the request arrives, so
   the request is stored without a scheme switch and the clear at lift produces no GC [inferred from
   07 §5.2 guard].
4. **Erasing flashes by design, right after lift.** For the stroke and pixel eraser (tool or stylus
   eraser end), render is off, SF draws nothing, the app draws its own eraser preview into the
   SurfaceView under scheme 4, and the `clearTransientUpdate(true)` at lift queues a full-screen GC16.
   A partial GC of the eraser path follows (`PenInputHandler.kt:141-181, 544`) [code for calls; panel
   result inferred].
5. **The stylus eraser end is detected at hover.** `onGenericMotionEvent` switches to the eraser tool
   when a hover event has `TOOL_TYPE_ERASER` or the stylus button (`OnyxCanvasView.kt:497-516`). That
   reconfigures the helper, so the session is already paused before the eraser end touches the glass
   (`PenInputHandler.kt:234-243`) [code].
6. **No per-stroke refresh.** After a pen stroke notate sends nothing that hands the panel back, apart
   from the transient clear. The app's redraw of the SurfaceView is then dropped while SF stays in the
   handwriting schema [inferred from 07 §4.5]. Hand-back comes from the next pause. For plain writing
   that is `performRefresh`, 2 s after the pen leaves hover range: render off/on and a partial GC
   (mode 98) over the **last** stroke's rectangle (`PenInputHandler.kt:139-201`) [code].
7. **Firmware excludes for its own chrome.** The limit is the SurfaceView's local visible rect. The
   floating toolbar and the open sidebar are passed as SF exclude rects on every global layout
   (`nt/CanvasActivity.kt:992-1003`, `nt/ui/ToolbarCoordinator.kt:53-69, 246-250`,
   `OnyxCanvasView.kt:755-763`) [code]. Nothing else (popups, dialogs, IME) is excluded.
8. **Session lifetime.** The helper lives as long as the view. "Re-enable" is `setupTouchHelper()`,
   which calls `openRawDrawing()` again **without a close** (`OnyxCanvasView.kt:871-903`). It runs on
   surface create and change and on each `setDrawingEnabled(true)`. `closeRawDrawing` happens only in
   `surfaceDestroyed` (`:541-544`) [code].
9. **Fast mode for finger work.** Pan, pinch, finger selection drags, the sidebar, the pen-settings
   popup and slider/list drags use `EpdFastModeController`. That is reflection on
   `applyTransientUpdate(UpdateMode.ANIMATION)` (mode 4) and `clearTransientUpdate(true)`, so every exit
   requests an immediate GC (`nt/util/EpdFastModeController.kt:47-75`) [code]. Its `setEpdTurbo(boolean)`
   lookup fails, because the SDK only has `setEpdTurbo(int)` (`dev/api/device/epd/EpdController.java:488`),
   so turbo is never set [code].
10. **Three dead calls:** `setDisplayScheme(SCHEME_SCRIBBLE)` (SF has no handler for `0xff0015`,
    `../stock-notes/05` §4.2 [doc]), `setEpdTurbo(boolean)` (method absent), and
    `View.setEnablePenSideButton` by reflection (absent on the framework's `View`; the method exists
    only on `ViewUpdateHelper`, grep of `fwv/View.java` and `fw/`) (`nt/util/OnyxSystemHelper.kt:18-30`)
    [code].
11. **No IME, focus or dialog pause.** No `onWindowFocusChanged` and no insets or IME handling
    anywhere in `nt/` [code, grep]. Only the toolbar touch, the pen-settings popup, the sidebar,
    pan/zoom and the refresh helpers pause the session.

## 2. The primitives notate uses, traced to SF

Transaction names as in `../stock-notes/07` §2.2. "PS n" = `SET_SCREEN_HANDWRITING_PEN_STATE(n)`.
There is no `AppTouchRender`, so each `TouchHelper` call produces a single transaction, where
stock and Tangleaf get two.

| notate call (site)                                                                                                                  | SDK path                                                                                                                                                                                                                                                                                                                             | SF transactions                                                                                                                                                                                                  |
| ----------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `TouchHelper.create(view, true, cb)` (`OnyxCanvasView.kt:876`)                                                                      | `SFTouchRender` only; `bindHostView` → `RawInputManager.setHostView` → reader `setLimitRect(localVisibleRect)` (`pen/RawInputManager.java:141-147`)                                                                                                                                                                                  | `REGION_LIMIT` = whole view                                                                                                                                                                                      |
| `setLimitRect(limit, exclusionRects)` (`:885, :761`)                                                                                | `RawInputManager.setLimitRect(rect, list)` → reader `setLimitRect` + `setExcludeRect` (`pen/RawInputManager.java:149-153`); `setExcludeRect` **returns early on an empty list** (`pen/RawInputReader.java:419-425`)                                                                                                                  | `REGION_LIMIT`; `REGION_EXCLUDE` only when the list is non-empty [code]                                                                                                                                          |
| `openRawDrawing()` (`:887`)                                                                                                         | resets cached flags; `SFTouchRender.openDrawing` = style 0, reader start, `startDrawing` (`pen/TouchHelper.java:152-160`, `pen/touch/SFTouchRender.java:265-268`)                                                                                                                                                                    | `SET_STROKE_STYLE(0)`, PS 1                                                                                                                                                                                      |
| `setRawDrawingEnabled(b)`                                                                                                           | render(b) + input(b) + `resetPenDefaultRawDrawing` (`pen/TouchHelper.java:170-178, 348-351`)                                                                                                                                                                                                                                         | `true`: PS 2 (if render changed), then `BRUSH_RAW(1)`, `ERASER_RAW(0,5)`; `false`: `ENABLE_POST(1)`, PS 3 (if render changed), then `BRUSH_RAW(1)`, `ERASER_RAW(0,5)`                                            |
| `setRawDrawingRenderEnabled(b)` (configurator)                                                                                      | guarded by the cached flag (`pen/TouchHelper.java:188-198`); off = `leaveScribbleMode` + `pauseDrawing`, on = `resumeDrawing` (`pen/touch/SFTouchRender.java:195-203, 278-284`)                                                                                                                                                      | off: `ENABLE_POST(1)`, PS 3; on: PS 2                                                                                                                                                                            |
| `EpdController.setScreenHandWritingPenState(view, PEN_PAUSE / PEN_DRAWING)` direct (`OnyxCanvasView.kt:89, 95, 743, 861, 866, 896`) | `SDMDevice` reflection, bypasses the guard                                                                                                                                                                                                                                                                                           | PS 3 / PS 2. SF ignores a value equal to the live reader state, but a 3 sent while the reader is already paused (4) runs `pause` + `stopHandwriting` again (`../stock-notes/07` §4.1) [doc]                      |
| `EpdController.enterScribbleMode(view)` (`:895`)                                                                                    | `enablePost(0)` (`dev/device/SDMDevice.java:1759-1766`)                                                                                                                                                                                                                                                                              | `ENABLE_POST(0)`: inert in SF (07 §4.4) [doc]                                                                                                                                                                    |
| `EpdController.leaveScribbleMode(view)` (`:542`)                                                                                    | `enablePost(1)`                                                                                                                                                                                                                                                                                                                      | `ENABLE_POST(1)` → ends schema 7 if active                                                                                                                                                                       |
| `closeRawDrawing()` (`:543`)                                                                                                        | style 0, `leaveScribbleMode`, reader pause, `pauseDrawing`, reader quit, `quitDrawing` (`pen/touch/SFTouchRender.java:271-275`)                                                                                                                                                                                                      | `SET_STROKE_STYLE(0)`, `ENABLE_POST(1)`, PS 3, PS 0                                                                                                                                                              |
| `setSingleRegionMode()` (`:893`)                                                                                                    | reader + `EpdController.setScreenHandWritingRegionMode(view,1)`                                                                                                                                                                                                                                                                      | `REGION_MODE(1)`                                                                                                                                                                                                 |
| `setStrokeColor / Style / Width` (configurator)                                                                                     | one render (`pen/touch/SFTouchRender.java` setters)                                                                                                                                                                                                                                                                                  | `SET_STROKE_COLOR`, `SET_STROKE_STYLE`, `SET_STROKE_WIDTH`, ×1 each                                                                                                                                              |
| `Device.setEraserRawDrawingEnabled(b, DASH)` (configurator, `PenInputHandler.kt:359`)                                               | direct                                                                                                                                                                                                                                                                                                                               | `SET_ERASER_RAW_DRAWING_ENABLED(b, 5)`                                                                                                                                                                           |
| `Device.setStrokeParameters(DASH, [5] or [5,5])` (configurator)                                                                     | direct                                                                                                                                                                                                                                                                                                                               | `SET_STROKE_PARAMETERS(5, …)`                                                                                                                                                                                    |
| `EpdDeviceManager.enterAnimationUpdate(true)`                                                                                       | static `inFastUpdateMode` flag; `EpdController.applyTransientUpdate(ANIMATION_QUALITY)` (`dev/api/device/EpdDeviceManager.java:73-79`)                                                                                                                                                                                               | `APPLY_TRANSIENT_UPDATE(2308)`: scheme 4 unless scheme is 3 (07 §5.2) [doc]                                                                                                                                      |
| `EpdDeviceManager.exitAnimationUpdate(true)`                                                                                        | only if the flag is set: `clearTransientUpdate(true)` (`:81-86`)                                                                                                                                                                                                                                                                     | `CLEAR_TRANSIENT_UPDATE(1)`: if the slot is fast → scheme 2 after 100 ms, clear type 8 → `applyGCOnce` (one-shot 98) **unless scheme is 3**, and always a full-screen mode-5 `addEpdc` + refresh (07 §5.3) [doc] |
| `EpdFastModeController.enterFastMode()`                                                                                             | reflective `EpdController.applyTransientUpdate(UpdateMode.ANIMATION)` → `SDMDevice.a(ANIMATION)` = `EINK_AUTO_MODE_REGIONAL \| WAIT_NOWAIT \| WAVEFORM_MODE_ANIM \| UPDATE_MODE_PARTIAL` = 0\|0\|4\|0 = **4** (`dev/device/SDMDevice.java:1010-1030, 3606-3612`, `fw/ViewUpdateHelper.java:36, 59, 61, 63`); no turbo (lookup fails) | `APPLY_TRANSIENT_UPDATE(4)`; 4 is in SF's fast set (07 §5.2) → scheme 4                                                                                                                                          |
| `EpdFastModeController.exitFastMode()`                                                                                              | reflective `clearTransientUpdate(true)` (no flag check)                                                                                                                                                                                                                                                                              | `CLEAR_TRANSIENT_UPDATE(1)` as above; a no-op in SF when the slot is not fast (07 §5.3 step 1)                                                                                                                   |
| `EpdController.invalidate(view, GC / DU)`                                                                                           | `View.invalidate(98 / 1)` (`SDMDevice.a`: GC → `UI_GC_MODE` 98, DU → `UI_DU_MODE` 1)                                                                                                                                                                                                                                                 | framework update entry for the view rect with that mode                                                                                                                                                          |
| `EpdController.invalidate(view, l, t, r, b, GC / DU)`                                                                               | `View.invalidate(l,t,r,b,mode)` → `invalidateInternal` → parent `invalidateChild(rect, mode)` (`fwv/View.java:10797-10800, 10836-10870`)                                                                                                                                                                                             | partial update entry. On a `SurfaceView` this entry belongs to the window, while the content is in the SurfaceView's own layer; how SF combines the two is **[unknown]**                                         |
| SurfaceView redraw `holder.lockHardwareCanvas()` (`OnyxCanvasView.kt:569`)                                                          | Onyx `SurfaceView` holder: `addEpdc(null)` → `mSurface.addEpdc([0,0,w,h, getDefaultUpdateMode()])` before every lock (`fwv/SurfaceView.java:110-124`)                                                                                                                                                                                | every frame carries a whole-SurfaceView update entry in the view's default mode. notate never sets it, so it is the framework default (5, AUTO) [code for the entry; value inferred]                             |
| `EpdController.setViewDefaultUpdateMode(window.decorView, DU)` (`CanvasActivity.kt:261`)                                            | `View.setDefaultUpdateMode(1)` on the DecorView                                                                                                                                                                                                                                                                                      | whole-window traversals use the root view's mode (`../stock-notes/08` §6.3) [doc] → DU for window-level redraws [inferred]                                                                                       |
| `EpdController.setDisplayScheme(3)` (`:262`)                                                                                        | `SET_APPLICATION_DISPLAY_SCHEME` 0xff0015                                                                                                                                                                                                                                                                                            | none (no SF handler) [doc]                                                                                                                                                                                       |
| `EpdController.getMaxTouchPressure()` (`nt/util/StrokeRenderer.kt:328`, `nt/data/CanvasSerializer.kt:220`)                          | read                                                                                                                                                                                                                                                                                                                                 | none                                                                                                                                                                                                             |

Never called by notate [code, grep of `nt/`]: `setPostInputEvent`, `setPenUpRefreshTimeMs`,
`setBrushRawDrawingEnabled`, `enableSideBtnErase`, `setAppCTPDisableRegion`, `applyGCOnce`,
`repaintEveryThing`, `handwritingRepaint`, `HAND_WRITING_REPAINT_MODE`, `enableColorCU`,
`setDitherThreshold`, any EinkWise/EAC (`EInkHelper`, app-scope) call. The reader's
`postInputEvent` defaults to false (`pen/RawInputReader.java:101, 242-244`) [code], so there are no
`PenActiveEvent` posts. The SDK pen-up refresh (500 ms by default) fires, but notate does not override
`onPenUpRefresh` [code].

## 3. Session states notate uses

Same triple as `../stock-notes/00` §2.1 (last PS sent, render flag, input flag):

| tool / situation                              | steady SF state                                          | render | input | who draws live                                                       |
| --------------------------------------------- | -------------------------------------------------------- | ------ | ----- | -------------------------------------------------------------------- |
| pen (≤ 10 mm on screen)                       | PS 2 (running; schema 7 after the first sample)          | on     | on    | SF, the tool's style and RGB colour                                  |
| pen > 10 mm (`isLargeStrokeMode`)             | PS 3                                                     | off    | on    | nobody live; `CursorView` circle only (`PenInputHandler.kt:731-734`) |
| lasso select                                  | PS 2                                                     | on     | on    | SF, `DASH` width 2, parameters `[5,5]`                               |
| rectangle select                              | PS 3                                                     | off    | on    | app (`CursorView.showSelectionRect`)                                 |
| lasso eraser                                  | PS 2                                                     | on     | on    | SF, `DASH` width 5, parameters `[5]`; eraser raw (1, 5)              |
| pixel ("standard") / stroke eraser            | PS 3                                                     | off    | on    | app (SurfaceView preview + `CursorView` circle)                      |
| text tool                                     | PS 3                                                     | off    | on    | nobody                                                               |
| toolbar touched, sidebar open, pen popup open | PS 3                                                     | off    | off   | —                                                                    |
| pan / pinch in progress                       | PS 3 (pinch: PS 2 after the first scale step, see §4.10) | off    | off   | —                                                                    |

The SF brush raw-drawing flag is always 1 (`resetPenDefaultRawDrawing` on every
`setRawDrawingEnabled`) [code]. So the tip always inks through SF's reader 2 when render is on, with
the main stroke style. The lasso tools' dashed trace is therefore drawn by the tip through the
**brush** reader with main style 5, not through the "eraser channel" the source comment claims
(`PenInputHandler.kt:357-358`) [inferred from 07 §4.2].

## 4. Per action: calls in order, SF-visible state, refreshes

"SF:" lists what reaches SurfaceFlinger. Main-thread callbacks for pen events arrive after a hop from
the in-process reader thread through `View.post`, so they always trail SF's own reader
(`pen/touch/SFTouchRender.java:44-134`) [code for the post; latency inferred].

### 4.1 Open a note

`CanvasActivity.onCreate` (`nt/CanvasActivity.kt:212-330`):

1. `setViewDefaultUpdateMode(decorView, DU)` → DecorView mode 1 [code]; `setDisplayScheme(3)` → no-op
   in SF [doc].
2. Layout inflates the full-bleed `OnyxCanvasView`. The toolbar's global-layout listener pushes the
   exclude list as soon as the helper exists (`ToolbarCoordinator.kt:53-69`) [code].
3. `surfaceCreated` → `drawContent()` (first SurfaceView frame, whole-view entry mode 5) and
   `setupTouchHelper()` (`OnyxCanvasView.kt:526-529`). First init (`:871-903`):
   - `TouchHelper.create` → SF limit = whole view.
   - `penInputHandler.setTouchHelper` → configurator. The helper is not opened yet, so render calls
     are no-ops (`pen/TouchHelper.java:188-189`). Colour, style and width go out ×1, plus
     `ERASER_RAW(0,5)` [code].
   - `ignoreSystemSideButton` → `NoSuchMethodException`, nothing [code].
   - `setLimitRect(limit, exclusionRects)` → `REGION_LIMIT`. `REGION_EXCLUDE` goes out only if the
     toolbar rect is already known. Otherwise SF keeps whatever exclude table it held from before
     [inferred].
   - `openRawDrawing` → style 0, **PS 1**.
   - `setRawDrawingEnabled(true)` → **PS 2**, reader resume (in-process), `BRUSH_RAW(1)`,
     `ERASER_RAW(0,5)`.
   - `setSingleRegionMode` → `REGION_MODE(1)`; `enterScribbleMode` → `ENABLE_POST(0)` (inert); direct
     PS 2 (equal, ignored).
   - First init → `performHardRefresh()` (`:857-869`): `setRawDrawingEnabled(false)` →
     `ENABLE_POST(1)`, **PS 3**, reader pause, brush/eraser reset. Then a direct PS 3 (reader already
     paused, so it runs again). `EpdController.invalidate(this, GC)` → `View.invalidate(98)` on the
     SurfaceView. The configurator follows (for the pen, `setRawDrawingRenderEnabled(true)` → **PS 2**).
     Then a direct PS 2 (ignored), and `setRawDrawingEnabled(true)` (render already on; input on;
     reset).
4. `surfaceChanged` → `setupTouchHelper()` again, not a first init: limit/exclude, `openRawDrawing`
   (**PS 1**), `setRawDrawingEnabled(true)` (**PS 2**), region mode, `ENABLE_POST(0)`, PS 2,
   configurator; then `drawContent()` (`:531-539`) [code].
5. The `isDrawingEnabled` collector (StateFlow, initial `true`, `vm/DrawingViewModel.kt:69-70`)
   calls `setDrawingEnabled(true)` → a third `setupTouchHelper()` (`CanvasActivity.kt:561-564`,
   `OnyxCanvasView.kt:736-739`). The order of 3–5 depends on when the surface arrives relative to the
   `STARTED` collector [unknown]. Whichever call comes first creates the helper and runs the
   `performHardRefresh`.
6. `loadMetadata` → `drawContent()` of the loaded page (`OnyxCanvasView.kt:695-706`).

Result: PS 2 for a rendering tool. Note that for a non-rendering tool (eraser, rectangle select,
text) `performHardRefresh` also ends in PS 2, because it runs the configurator **before**
`setRawDrawingEnabled(true)`, which then turns render back on. Render stays on until the next
configurator call, which comes at the first pen-down [code, `:864-868`]. One whole-view GC (mode 98)
per open. The limit is the whole SurfaceView and the excludes are the toolbar. No transient mode.

### 4.2 Write (pen tool, tip)

1. Contact: SF reader 2 inks with the tool's style/colour/width; the first sample logs
   `HandlePenTrigger`, enters schema 7, `### EUS, s 3 c 0` (07 §4.2) [doc].
2. Main thread, `onBeginRawDrawing` (`PenInputHandler.kt:299-399`):
   - clears the selection, dismisses the selection popup;
   - `updateTouchHelperTool()` → `SET_STROKE_COLOR`, `SET_STROKE_STYLE`, `SET_STROKE_WIDTH`,
     render on (guarded no-op), `ERASER_RAW(0,5)` — sent **while SF is already inking** [code];
   - `enterAnimationUpdate(true)` → `APPLY_TRANSIENT_UPDATE(2308)`. SF is in scheme 3, so the slot is
     stored without a switch [inferred from 07 §5.2]. If this ever overtakes SF's first sample, the
     log shows `EUS s 4` then `EUS s 3` [inferred];
   - `ERASER_RAW(0,5)` again (`:359`); dwell detector armed.
3. Moves: model points only; no transaction [code].
4. Lift, `onEndRawDrawing` (`:405-645`): `exitAnimationUpdate(true)` → `CLEAR_TRANSIENT_UPDATE(1)`.
   The slot was fast, but scheme 3 blocks the switch and the GC. SF still adds the full-screen mode-5
   entry and a refresh (07 §5.3 step 3). Whether that entry reaches the panel while schema 7 drops
   ordinary frames, or waits for the next hand-back, is **[unknown]**. Then the stroke is committed
   on a coroutine → `onStrokeFinished` → `drawContent()` → a SurfaceView frame with a whole-view
   mode-5 entry, dropped by SF while in schema 7 [inferred from 07 §4.5].
5. SF stays in schema 7 / scheme 3 with its own ink on the panel [inferred].
6. Pen leaves hover range → `onHoverExit` → `performRefresh` after 2000 ms, cancelled by any new hover
   or pen-down (`:139-201`). `performRefresh` (`:141-181`) sends `setRawDrawingEnabled(false)` →
   `ENABLE_POST(1)`, **PS 3**. SF: `Post enabled!`, 150 ms, `EUS s 2`. Then brush/eraser reset,
   `View.invalidate(l,t,r,b, 98)` over the last stroke's screen bounds (padded by width × scale + 10),
   `setRawDrawingEnabled(true)` → **PS 2**, and the configurator [code]. `currentStrokeScreenBounds`
   is reset at every pen-down (`:366`), so only the most recent stroke's rectangle gets the GC. The
   earlier strokes of the burst keep SF's ink [code + inferred].

Dwell-to-shape (`:80-135`): `setRawDrawingEnabled(false)` (`ENABLE_POST(1)`, PS 3 with the pen still
down → schema 7 ends. The transient slot is fast at that moment, so `EUS s 4`, 07 §4.3), a DU partial
invalidate around the preview shape, `setRawDrawingEnabled(true)` (PS 2; the next sample re-triggers
`EUS s 3`) [code for calls; SF inferred].

Scribble-to-erase (`:548-557`): an ordinary pen stroke classified as a scribble becomes
`commitEraser(STROKE)`. No extra refresh, so the erased strokes and the scribble's SF ink stay until
the hover-exit refresh, which covers the scribble's rectangle [code + inferred].

### 4.3 Erase with the eraser tool (tip)

Selecting the tool: `setTool` → configurator (`OnyxCanvasView.kt:712-717`, `PenInputHandler.kt:203-210`).
The selection itself goes through the toolbar bracket (§4.8).

**Stroke and pixel ("standard") eraser**, steady state PS 3, eraser raw (0,5):

1. Contact: SF reader paused → no ink, no trigger (07 §4.2) [doc].
2. Main, `onBeginRawDrawing`: configurator (render off no-op, `ERASER_RAW(0,5)`),
   `enterAnimationUpdate(true)` → `APPLY_TRANSIENT_UPDATE(2308)` → **`EUS s 4`** at once, because the
   session is paused (07 §5.2) [inferred]. Then `startBatchSession`.
3. Moves (every ≥ 5 px): pixel eraser → `previewEraser` sets an overlay stroke; stroke eraser → the
   model erases live and dirty tiles are refreshed (`nt/ui/input/EraserGestureHandler.kt:37-75`,
   `nt/ui/controller/CanvasControllerImpl.kt:108-126`). Both end in `drawContent()` →
   SurfaceView frames composed under scheme 4 (A2) [inferred]. `CursorView` (a plain `View`) draws the
   eraser circle (`PenInputHandler.kt:723-735`).
4. Lift: `exitAnimationUpdate(true)` → `CLEAR_TRANSIENT_UPDATE(1)` → SF sleeps 100 ms, `EUS s 2`,
   queues a **one-shot GC (98)** and a **full-screen mode-5 update** [inferred from 07 §5.3]. Commit:
   the overlay is cleared, the model erases, tiles refresh, `drawContent()`.
   `refreshHandler.post { performRefresh(true) }`: `setRawDrawingEnabled(false)` (render already off →
   skipped; input off; reset), `View.invalidate(l,t,r,b, 98)` over the eraser path,
   `setRawDrawingEnabled(true)` → **PS 2**, then the configurator → `ENABLE_POST(1)`, **PS 3**
   [code]. SF therefore sees one extra `running`/`toPause` pair after every erase.

**Lasso eraser** (render on, PS 2, eraser raw (1, 5), style DASH): the tip inks a dashed trace
through SF reader 2 with `EUS s 3` at the first sample. The pen-down transient is stored under the
scheme-3 guard and cleared at lift without a GC. `commitEraser(LASSO)` removes the enclosed strokes,
then `performRefresh(true)`: PS 3 (`Post enabled!`, `EUS s 2`), a partial GC over the lasso bounds
(which should also remove the dashed trace), PS 2 [code for calls; panel inferred].

### 4.4 Erase with the stylus eraser end

With the pen tool active and an eraser tool present in the toolbar (`currentEraser` is the first or
last-used eraser, `vm/DrawingViewModel.kt:62-63, 218, 307`):

1. **Hover** with the eraser end (or the stylus button): `onGenericMotionEvent` → `prepareEraser()`
   → current tool = eraser tool → configurator (`OnyxCanvasView.kt:497-504`,
   `PenInputHandler.kt:234-243`) [code].
   - Stroke or pixel eraser: `ENABLE_POST(1)`, **PS 3** before contact. If SF was still in schema 7
     from writing, `Post enabled!` and `EUS s 2` also come now.
   - Lasso eraser: render stays on, `ERASER_RAW(1, 5)`, style DASH, width 5.
2. Contact: the SDK reader reports `isErasing`, so `onBeginRawErasing` → `onBeginRawDrawing`
   (`PenInputHandler.kt:752-769`). From here it is §4.3: `EUS s 4` at the main-thread pen-down for
   the stroke/pixel eraser; for the lasso eraser, SF reader 4 draws the dashed trace with style 5 and
   triggers scheme 3 [inferred from 07 §4.2].
3. Lift: `onEndRawErasing` → `onEndRawDrawing` (clear with GC, commit, `performRefresh`), then
   `view.post { finishEraser() }` (`:771-777`). That post usually runs while the commit coroutine is
   still suspended in `withContext(Dispatchers.Default)` with `isStrokeInProgress` still true, so it
   returns without restoring the pen [inferred]. The pen tool comes back at the next tip hover
   (`finishEraser` from `onGenericMotionEvent`) → configurator → **PS 2** [code].
4. No hover before contact (hover events not delivered, or a fast flip): `onBeginRawErasing` switches
   tools itself (`:762-767`) → **PS 3 after contact**. SF's eraser end was on the no-op reader 3
   anyway (eraser raw 0), so SF never inks the eraser [inferred].
5. No eraser tool in the toolbar (`eraserTool == null`): the tool is not switched and the gesture is
   committed as a pen stroke unless the side-button flag `b` is set (`:331-342, 495`). What `b`
   carries for the eraser end is **[unknown]** (the reader passes `shortcutErasing`,
   `pen/RawInputReader.java` `m42a`).

### 4.5 Lasso select and rectangle select

**Lasso select** (render on, PS 2, DASH width 2, parameters `[5,5]`, eraser raw (1,5)):

1. Contact: SF draws the dashed trace; `EUS s 3`.
2. Main pen-down: clears any selection; configurator (re-sends style, colour, width, `ERASER_RAW(1,5)`,
   `SET_STROKE_PARAMETERS`); `APPLY_TRANSIENT_UPDATE(2308)` stored under scheme 3 [inferred].
3. Moves build the lasso path from raw points (`:695-699`).
4. Lift: `CLEAR_TRANSIENT_UPDATE(1)` under scheme 3 (no GC; full-screen mode-5 entry [unknown fate]);
   a coroutine hit-tests and selects, then `view.post { showActionPopup(); requestEpdRefresh(GC) }`
   (`:425-476`).
5. `showActionPopup` → `SelectionActionPopup.show` (a view in the activity layout) →
   `exitFastMode()` (slot not fast → SF no-op) and a posted `View.invalidate(98)` on the popup root
   (`nt/ui/dialog/SelectionActionPopup.kt:51-66`) [code].
6. `requestEpdRefresh(GC)` → next `drawContent()` draws the selection overlay, then
   `performHardRefresh()`: `ENABLE_POST(1)`, **PS 3** (`Post enabled!`, `EUS s 2`), reset, direct PS 3,
   `View.invalidate(98)` on the whole SurfaceView, configurator (render stays on in the SDK's view),
   direct **PS 2**, `setRawDrawingEnabled(true)` → input on, **`BRUSH_RAW(1)`, `ERASER_RAW(0,5)`**
   (`OnyxCanvasView.kt:557-562, 651-659, 857-869`) [code]. The reset leaves eraser raw drawing off
   until the next configurator call.
7. The SurfaceView frame drawn just before step 6 is posted while SF is still in schema 7. Whether
   the GC invalidate on the SurfaceView's window region brings that buffer to the panel is
   **[unknown]**. The dashed SF trace is expected to disappear with that GC [inferred].

The selection then stays shown with the session **running** (PS 2, DASH). The next pen-down anywhere
clears it (`PenInputHandler.kt:311-313`).

**Rectangle select** (render off, PS 3): pen-down → `EUS s 4` at once. The app draws the rectangle in
`CursorView` under A2. At lift `CLEAR_TRANSIENT_UPDATE(1)` → `EUS s 2` + one-shot GC + full-screen
mode-5, then `performHardRefresh` → another whole-view GC. Because `performHardRefresh` calls the
configurator before `setRawDrawingEnabled(true)`, it ends with **render on (PS 2)** although the
tool does not render. The next pen-down's configurator turns it off after contact, so SF inks the
first samples of that stroke with the last main style [inferred from `:857-869` + 07 §4.2].

### 4.6 Drag, scale, rotate a selection

Stylus `MotionEvent`s are rejected by `onTouchEvent` (`OnyxCanvasView.kt:400-402`), and a pen-down on
the canvas clears the selection, so selection transforms are **finger only** [code].

1. Finger down on a handle or the body: `SelectionInteractor.onDown` → `enterFastMode()` →
   `APPLY_TRANSIENT_UPDATE(4)` → **`EUS s 4`** (the session is running but not in schema 7, unless the
   pen just wrote) → `startMoveSelection`, popup dismissed (`nt/ui/selection/SelectionInteractor.kt:99-139`).
   Long press on an item with the SELECT tool follows the same path (`:142-158`, `OnyxCanvasView.kt:239-258`).
2. Moves: `handleSingleTouchDrag` / multi-touch transform → `drawContent()` per frame → A2 frames
   [inferred]. No pen-state change; SF stays at PS 2 with the DASH style.
3. Up: `exitFastMode()` → `CLEAR_TRANSIENT_UPDATE(1)` → `EUS s 2`, one-shot **GC**, full-screen mode 5.
   Then `showActionPopup` → another `exitFastMode` (no-op) and a GC invalidate of the popup
   (`SelectionInteractor.kt:190-225`) [code]. A body tap without movement commits instead.

### 4.7 Toolbar: tap, tool switch, drag

`DraggableLinearLayout` calls `onDown` on any `ACTION_DOWN` in the toolbar and `onUp` on up/cancel
(`nt/ui/DraggableLinearLayout.kt:72-110, 113-175`). `CanvasActivity.kt:298-322` [code]:

- **Down:** `viewModel.setDrawingEnabled(false)` → collector → `setDrawingEnabled(false)`:
  `setRawDrawingRenderEnabled(false)` (`ENABLE_POST(1)`, **PS 3**), `setRawDrawingEnabled(false)`
  (input off, reset), direct **PS 3** (`OnyxCanvasView.kt:736-745`). Also
  `enterAnimationUpdate(true)` → `APPLY_TRANSIENT_UPDATE(2308)`. The two are ordered by when the
  StateFlow collector runs [inferred]. Either the switch happens at once (`EUS s 4`) or it is stored
  and applied when PS 3 ends schema 7 (`Post enabled!` then `EUS s 4`, 07 §4.3).
- The toolbar is inside the exclude list, so a pen tap there produces no SF ink and no raw
  callback; the stylus `MotionEvent` reaches the toolbar view [code for the exclude; routing
  inferred].
- **Up:** `setDrawingEnabled(true)` → full `setupTouchHelper()` (limit/exclude, **PS 1**, **PS 2**,
  reset, region mode, `ENABLE_POST(0)`, PS 2, configurator). Also `exitAnimationUpdate(true)` →
  `CLEAR_TRANSIENT_UPDATE(1)` → `EUS s 2` + **one-shot GC** + full-screen mode-5 [inferred from 07
  §5.3].
- So **every toolbar touch ends in a full-screen GC16** and a PS 3 → 1 → 2 cycle [inferred].
- Tool switch inside that bracket: `setTool` → configurator for the new tool; a non-rendering tool
  ends up at PS 3 after the reopen's PS 2 (the configurator runs last in `setupTouchHelper`) [code].
- Toolbar drag: margins change → global layout → `setLimitRect(limit, [toolbar])` per layout pass
  (`REGION_LIMIT`, `REGION_EXCLUDE`) [code].
- Toolbar expand → `refreshScreen()` → `performHardRefresh()` (whole-view GC, PS 3 → 2)
  (`CanvasActivity.kt:393-396`).

### 4.8 Menus, popups, dialogs, overlays

| overlay                                                                                                               | open                                                                                                                                                                                                                                                                                          | close                                                                                                                                                | pen session while open                                                                                                                                                                                                                                                                                                                           |
| --------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Pen settings popup** (tap the active tool again; `PopupWindow`, focusable) (`CanvasActivity.kt:1030-1066`)          | `setPenPopupOpen(true)` → `setDrawingEnabled(false)` (PS 3, input off); `enterFastMode` (transient 4 → `EUS s 4`); `show` → `refreshHighQuality(root)` = `exitFastMode` (`CLEAR(1)` → `EUS s 2` + GC) + posted GC invalidate of the popup (`nt/ui/dialog/PenSettingsPopup.kt:80-86, 643-644`) | detach → `onDismiss`: `exitFastMode` (no-op unless a slider drag left it on), `setDrawingEnabled(true)` → `setupTouchHelper` (PS 1, 2, configurator) | Paused, but each setting change calls `setTool` → the configurator. For a pen that sets **render on (PS 2)** while the popup is open, with input off. The popup is not excluded, so SF would ink a pen over the popup [inferred]. Slider and colour-list drags bracket `enterFastMode`/`exitFastMode` (GC on each release) (`:126-137, 205-216`) |
| **Settings sidebar** (`nt/ui/SidebarCoordinator.kt:44-84`)                                                            | `enterFastMode` (transient 4), 300 ms slide; `onStateChanged` → `setDrawingEnabled(false)` (PS 3) and the exclude list gains the sidebar rect                                                                                                                                                 | `enterFastMode`, slide, at animation end `exitFastMode` (`CLEAR(1)` → GC) and `onStateChanged` → `setDrawingEnabled(true)` (reopen)                  | Paused. **The transient mode-4 slot stays set for as long as the sidebar is open** (`open()` has no exit) [code] → the sidebar UI is drawn in A2 [inferred]                                                                                                                                                                                      |
| **Canvas context menu** (finger long press on empty space; `PopupWindow`) (`nt/ui/dialog/CanvasContextMenu.kt:52-89`) | `exitFastMode` (no-op), posted GC invalidate of the menu view                                                                                                                                                                                                                                 | `dismiss()`, no EPD call                                                                                                                             | Not paused; pen tool at PS 2. A pen tap on the menu is inked by SF and its raw pen-down dismisses the popups (`PenInputHandler.kt:312-313`) [inferred]                                                                                                                                                                                           |
| **Selection action popup** (in-layout view)                                                                           | `exitFastMode` + GC invalidate of the popup (`SelectionActionPopup.kt:51-66`)                                                                                                                                                                                                                 | hide                                                                                                                                                 | Running (lasso) or PS 3 (rect); not excluded                                                                                                                                                                                                                                                                                                     |
| **Insert-link dialog** (`Dialog`)                                                                                     | posted `exitFastMode` + `View.invalidate(98)` on the dialog's decor (`nt/ui/dialog/InsertLinkDialog.kt:179-183`)                                                                                                                                                                              | nothing                                                                                                                                              | Not paused, not excluded; no focus handling                                                                                                                                                                                                                                                                                                      |
| **Text edit dialog**                                                                                                  | §4.9                                                                                                                                                                                                                                                                                          | §4.9                                                                                                                                                 | §4.9                                                                                                                                                                                                                                                                                                                                             |

The page-grid dialog would be covered by `updateDrawingEnabledState()` (`isGridOpen`), but
`isGridOpen` is never set to true (`CanvasActivity.kt:61, 988`) [code].

### 4.9 Keyboard and text input

Text tool: configurator → render off (PS 3), input on (`PenToolConfigurator.kt:65-69`). Only finger
taps reach `onSingleTapConfirmed` (the stylus is rejected at `OnyxCanvasView.kt:401-402`). A finger
tap opens `TextEditDialog` (`:296-329, 370-397`) [code]. A pen stroke in the text tool still goes
through the raw pen path: `PenInputHandler` has no TEXT branch, so it would be committed as a stroke
[code: no `TEXT` in `PenInputHandler.kt`; the model behaviour was not traced].

`TextEditDialog` (`nt/ui/dialog/TextEditDialog.kt:80-105`):

1. `setSoftInputMode(SOFT_INPUT_STATE_VISIBLE)` → the IME comes up with the dialog.
2. `onStart` → `enterAnimationUpdate(true)` → `APPLY_TRANSIENT_UPDATE(2308)` → `EUS s 4`, so typing
   and the IME are drawn in A2 [inferred].
3. No pen-state change, no limit change, no exclude for the dialog or the IME [code]. With the text
   tool, SF does not ink (PS 3), but raw input stays on, so a pen on the dialog or keyboard produces
   canvas callbacks and `EUS s 4` [inferred]. When the dialog is opened from the SELECT tool on a text
   item after a lasso (PS 2, DASH), SF inks a pen used over the dialog or IME [inferred].
4. `onStop` → `exitAnimationUpdate(true)` → `CLEAR(1)` → `EUS s 2` + one-shot **GC**. That is the only
   refresh on keyboard hide [inferred from 07].

The manifest sets `adjustResize` on `CanvasActivity`, but the activity is immersive with
`setDecorFitsSystemWindows(false)` (`CanvasActivity.kt:979-985`) and there is no insets listener, so
the limit rect does not follow the IME [code for the absence; resize behaviour inferred].

### 4.10 Finger pan and pinch zoom

`ViewportInteractor` (`nt/ui/interaction/ViewportInteractor.kt:50-150, 225-237`); finger events only
(`OnyxCanvasView.kt:400-437`). Finger events within 150 ms of the last pen stroke end are ignored
(`ViewportInteractor.kt:42, 69-73`) [code].

1. Pan past the touch slop, or pinch begin → `startInteraction()` → `onInteractionStart`:
   `setRawDrawingEnabled(false)` (`ENABLE_POST(1)`, **PS 3**, input off, reset), direct **PS 3**,
   renderer interacting; then `enterFastMode()` → `APPLY_TRANSIENT_UPDATE(4)` → `EUS s 4`
   (`OnyxCanvasView.kt:87-91`) [code; SF inferred].
2. Each move: matrix translate → `drawContent()` → A2 frames [inferred].
3. Each pinch step: `handleScale` → `onScaleChanged` → configurator (`ViewportInteractor.kt:150-176`,
   `OnyxCanvasView.kt:86`). With the pen tool, `setRawDrawingRenderEnabled(true)` sends **PS 2 during
   the pinch** (render was off, the helper is open). Then `SET_STROKE_WIDTH` etc. once per frame.
   Input stays off [code].
4. Up: `onInteractionEnd` → `setRawDrawingEnabled(true)` (PS 2 if render was off; input on; reset),
   direct PS 2 (ignored), configurator; `exitFastMode()` → `CLEAR(1)` → `EUS s 2` + one-shot **GC** +
   full-screen mode 5 (`OnyxCanvasView.kt:92-97`) [code; SF inferred].
5. The limit is not re-pushed on pan: it stays the whole SurfaceView [code].

So every pan or zoom ends in a full-screen GC16 as soon as the fingers lift. Stock waits 5 s and
passes flag 0.

### 4.11 Page switch

There is one infinite or fixed-page canvas and one session. Page buttons live in the toolbar
(`nt/ui/navigation/PageNavigation.kt:136-250`). A button press is the toolbar bracket of §4.7 (PS 3,
transient 2308, reopen and GC on up) plus `jumpToPage` → `viewportController.scrollTo` → matrix
reset, `drawContent()` and the configurator (`CanvasControllerImpl.kt:167-177`,
`OnyxCanvasView.kt:176-199`) [code]. The page-grid dialog opens without a pause (§4.8).

### 4.12 Undo / redo

- From the toolbar: §4.7 bracket plus `undo()`/`redo()` → `refreshAfterEdit` → tile refresh,
  `drawContent()` and `EpdController.invalidate(this, DU)` (`View.invalidate(1)` on the SurfaceView)
  (`OnyxCanvasView.kt:774-796, 905-921`) [code].
- Two-finger double tap (`:439-477`): the same `undo()` without any pause. Straight after writing,
  SF is still in schema 7, so the DU frame is dropped and the undone stroke's SF ink stays on the panel
  until the next hand-back [inferred from 07 §4.5].
- No session close, no transient.

### 4.13 Leave the note

Back → `closeSession`, `finish()` (`CanvasActivity.kt:221-247`). When the window goes away,
`surfaceDestroyed` → `leaveScribbleMode` (`ENABLE_POST(1)`) and `closeRawDrawing()` → style 0,
`ENABLE_POST(1)`, **PS 3**, **PS 0** (`OnyxCanvasView.kt:541-544`) [code]. No GC, no exclude
reset. Limit/exclude and any transient slot left set (for example with the sidebar open) stay in SF
[code for the absence of calls].

### 4.14 Background, foreground, window focus

- **Background** (Home, app switch): `onPause` saves only (`CanvasActivity.kt:902-926`). When the
  activity stops, Android destroys the SurfaceView's surface → `surfaceDestroyed` → close as §4.13
  [inferred: standard `SurfaceView` behaviour].
- **Foreground:** `surfaceCreated` → `drawContent` + `setupTouchHelper` (not a first init, so no GC),
  `surfaceChanged` → again, and the `repeatOnLifecycle(STARTED)` collector re-emits
  `isDrawingEnabled=true` → a third `setupTouchHelper` [code for the three paths; the re-emission is
  StateFlow behaviour, inferred]. SF sees PS 1, 2 once to three times.
- **Focus loss without backgrounding** (notification shade, system dialog, own `Dialog`): no
  handler. The session keeps its state and the screen-space limit rect [code].

## 5. Table: SF-visible effect per action

| action                       | PS sent                                            | render after                      | transient                                 | refresh notate issues                                                                  | SF scheme path (predicted)              |
| ---------------------------- | -------------------------------------------------- | --------------------------------- | ----------------------------------------- | -------------------------------------------------------------------------------------- | --------------------------------------- |
| open note                    | 1,2,3,(3),2,(2) + 1,2 per extra `setupTouchHelper` | on (even for non-rendering tools) | none                                      | whole-view GC (98) once                                                                | 2                                       |
| write (pen)                  | none                                               | on                                | 2308 at down, `CLEAR(1)` at up            | none; 2 s after hover exit: PS 3, partial GC of last stroke rect, PS 2                 | 3 at contact; 2 at the hover-exit pause |
| pixel / stroke eraser (tool) | per gesture: 2 then 3 (from `performRefresh`)      | off                               | 2308 at down, `CLEAR(1)` at up            | one-shot full GC + full-screen mode 5 (SF, from the clear) + partial GC of eraser path | **4 at down → 2 at lift + GC**          |
| lasso eraser                 | 3, 2 after lift                                    | on                                | 2308 / `CLEAR(1)` under scheme 3          | partial GC of lasso bounds                                                             | 3 → 2                                   |
| stylus eraser end (pen tool) | 3 at hover, 2 later at tip hover                   | off during erase                  | as eraser                                 | as eraser                                                                              | 4 → 2 + GC                              |
| lasso select                 | 3, (3), 2, (2) after lift                          | on                                | 2308 / `CLEAR(1)` under scheme 3          | whole-view GC + popup GC                                                               | 3 → 2                                   |
| rectangle select             | 3, (3), 2, (2) after lift                          | **on** (left on)                  | 2308 / `CLEAR(1)`                         | one-shot GC + whole-view GC + popup GC                                                 | 4 → 2 + GC                              |
| finger drag of selection     | none                                               | on                                | transient 4 at down, `CLEAR(1)` at up     | one-shot GC + popup GC                                                                 | 4 → 2 + GC                              |
| toolbar touch                | 3,(3) down; 1,2 up                                 | per tool                          | 2308 down, `CLEAR(1)` up                  | one-shot GC                                                                            | 4 → 2 + GC                              |
| pen popup                    | 3,(3); 1,2 on close                                | per tool (pen: on while open)     | 4 then immediate `CLEAR(1)`               | GC at open + popup GC                                                                  | 4 → 2 + GC                              |
| sidebar                      | 3,(3); 1,2 on close                                | per tool                          | 4 held while open, `CLEAR(1)` at close    | one-shot GC at close                                                                   | 4 while open → 2 + GC                   |
| context menu / link dialog   | none                                               | unchanged                         | `CLEAR(1)` (no-op)                        | GC invalidate of the popup/dialog                                                      | unchanged                               |
| text dialog + IME            | none                                               | off (text tool)                   | 2308 at `onStart`, `CLEAR(1)` at `onStop` | one-shot GC at close                                                                   | 4 while open → 2 + GC                   |
| pan                          | 3,(3); 2,(2)                                       | per tool                          | 4 / `CLEAR(1)`                            | one-shot GC at lift                                                                    | 4 → 2 + GC                              |
| pinch (pen tool)             | 3,(3); 2 at first scale step                       | on during pinch                   | 4 / `CLEAR(1)`                            | one-shot GC at lift                                                                    | 4 → 2 + GC                              |
| page button                  | toolbar bracket                                    | per tool                          | toolbar bracket                           | toolbar GC                                                                             | 4 → 2 + GC                              |
| undo (toolbar / two fingers) | toolbar bracket / none                             | unchanged                         | toolbar / none                            | DU invalidate (+ toolbar GC)                                                           | toolbar / none                          |
| leave, background            | `ENABLE_POST(1)`, 3, 0                             | closed                            | left as is                                | none                                                                                   | 2                                       |
| foreground                   | (1,2) ×1–3                                         | per tool                          | none                                      | none                                                                                   | —                                       |

"(n)" = a direct `setScreenHandWritingPenState` that SF either ignores (equal to the live state) or
re-runs (3 while already paused).

## 6. Refreshes notate issues, complete list

| refresh            | call                                                        | mode                                     | scope                     | when                                                                                                             |
| ------------------ | ----------------------------------------------------------- | ---------------------------------------- | ------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| SurfaceView frame  | `lockHardwareCanvas` → `addEpdc(whole view, default mode)`  | 5 (view default) [inferred]              | whole SurfaceView         | every `drawContent()`                                                                                            |
| hard refresh       | `View.invalidate(98)` on the SurfaceView inside PS 3 … PS 2 | GC                                       | whole view                | first open, after a selection is made, toolbar expand, background-style change, `clear()`                        |
| hover-exit refresh | `View.invalidate(l,t,r,b,98)` inside a render off/on        | GC                                       | last stroke / eraser rect | 2 s after hover exit (pen), right after commit (erase)                                                           |
| dwell preview      | `View.invalidate(l,t,r,b,1)` inside render off/on           | DU                                       | shape preview rect        | dwell-to-shape                                                                                                   |
| undo/redo          | `View.invalidate(1)`                                        | DU                                       | whole view                | every undo/redo                                                                                                  |
| popup/dialog show  | `View.invalidate(98)` of the popup or dialog decor          | GC                                       | popup view                | pen popup, context menu, selection popup, link dialog                                                            |
| transient exits    | `CLEAR_TRANSIENT_UPDATE(1)`                                 | one-shot GC 98 + full-screen mode 5 (SF) | full screen               | every pen-up, toolbar up, fast-mode exit, text-dialog stop; no GC when SF is in scheme 3 or the slot is not fast |

No counted GC, no `HAND_WRITING_REPAINT_MODE`, no `applyGCOnce` of its own [code].

## 7. Compared with stock Notes

### 7.1 What notate gets that stock also has

- **Firmware live ink for writing**, with the session kept running between strokes and no
  per-stroke refresh (same as stock on this device, 00 §3.2). notate's delayed partial GC after hover
  exit has no stock counterpart [code].
- **Firmware lasso trace** (dashed style 5) with the session running, as stock's smart/free lasso
  (00 §3.5). The parameters differ: stock `(5,[5.0])`, notate `(5,[5,5])` and width 2.
- **Firmware excludes for its own floating chrome** (toolbar, sidebar), like stock's typed overlay
  rects, though without stock's always-present degenerate rect and only when the list is non-empty.
- **Transient A2 for finger viewport work and selection drags**, like stock's `ApplyFastModeEvent`.
- **Firmware colour and pressure styles.** notate maps its pen types to SF styles fountain, pencil,
  marker, neo-brush, charcoal and dash (`nt/model/StrokeType.kt:8-27`) and sends an RGB stroke colour
  (mid-luminance colours forced to black, `nt/util/ColorUtils.kt:16-40`). Whether SF inks colour on
  the Kaleido panel is as **[unknown]** as for stock (00 §3.2).

### 7.2 What notate loses relative to stock

1. **Live eraser trace by the firmware.** Stock keeps render on for types 0/1/2, SF's eraser reader
   draws the trace (style 8 soft eraser or 5), and the session stays at PS 2 (00 §3.3–§3.4). notate
   pauses (PS 3) for the stroke and pixel eraser and draws the preview itself. Only its lasso eraser
   gets a firmware trace.
2. **No flash after erase.** Stock ends an erase with `ENABLE_POST(1)`, a render-only pause, a mode-5
   invalidate and a resume after 300 ms, with no GC request. notate's `clearTransientUpdate(true)`
   makes SF queue a full-screen GC after every stroke- or pixel-eraser gesture. A partial GC
   follows.
3. **Soft eraser (style 8).** Not used.
4. **Delayed, flag-0 fast-mode exit.** Stock applies 2308 + turbo 5 + dither 160 and clears it 5 s after
   the gesture with flag 0 (no GC; SF still adds its full-screen mode-5 update). notate uses mode 4
   (`ANIMATION`) or 2308, has no turbo (dead reflection) and no dither change, and clears at once with
   flag 1. Every pan, zoom, drag and toolbar touch ends in a GC.
5. **EinkWise / scroll-A2 exemption.** Stock's `com.onyx` package is exempt from
   `OnyxScrollRefreshManager` (07 §6 Entry A). `com.alexdremov.notate` is not. Entry A fires only if
   the window gets `topScrollBounds`, and whether a non-scrollable `SurfaceView` layout gets them is
   **[unknown]**. Entry B (WebView) does not apply because the canvas is not a WebView. If Entry A does
   fire, it only adds a redundant 2308 request during paused-session stylus moves, and its delayed
   clear finds the slot already cleared by notate [inferred].
6. **Stable session and ordered resumes.** Stock pauses and resumes through one pen thread, with
   gates and a 500 ms delay. notate reopens with `openRawDrawing` (PS 1 → 2) on every
   `setDrawingEnabled(true)` and surface change, with no delay. It also sends direct pen states that
   bypass the SDK guard, and it leaves render on after `performHardRefresh` for non-rendering tools.
7. **Pause on focus, IME, dialogs and popups.** Stock pauses for popups, dialogs, toast, status bar,
   keyboard and focus loss (00 §3.7, §3.12). notate pauses only for the toolbar touch, pen popup,
   sidebar and pan/zoom.
8. **Palm and finger handling.** Stock suppresses finger gestures while the pen hovers
   (`PenActiveEvent`). notate uses a 150 ms post-stroke settle window and routes all non-stylus
   `MotionEvent`s to gestures (`OnyxCanvasView.kt:400-437`). There is no CTP cut-out and no
   post-input event. Its side-button opt-out call is dead (§1 item 10).
9. **Full-refresh policy.** Stock: counted GC every 10th page action, the Refresh button, a
   `REGAL_PLUS` repaint on popup dismiss. notate: a GC after nearly every UI interaction (toolbar
   touch, fast-mode exit, popup show, selection end) and none on popup dismiss or on leaving the note.

### 7.3 Predicted driver log

**Erase, stroke or pixel eraser (tool, or eraser end after a hover):**

```
(hover, eraser end, pen tool)   SurfaceFlinger: Pid N State change to: toPause
                                [if SF was in schema 7] SurfaceFlinger: Post enabled!  HWEpdcManager: ### EUS, s 2 c 0
(contact)                       — no HandlePenTrigger, no EUS s 3 (SF reader paused)
(main-thread pen-down, ~ms)     HWEpdcManager: ### EUS, s 4 c 0
(moves)                         onyx_tcon_send_cmd … waveform[4] …        (app preview frames, A2)
(lift + ~100 ms)                HWEpdcManager: ### EUS, s 2 c 0
                                onyx_tcon_send_cmd … waveform[2] update_mode[1] (full screen, one-shot GC 98)
                                possibly a second full-screen update (SF's mode-5 entry)
(commit, then performRefresh)   State change to: running … State change to: toPause
                                onyx_tcon_send_cmd … update_mode[1] (partial GC over the eraser path)
```

[inferred from 07 §4–§5 and the call sequence in §4.3–§4.4]. So **yes**: the session is paused during
pen input and the panel goes scheme 4 / A2, then back to scheme 2 with a full-screen GC16. The
differences from Tangleaf's observed pattern are (a) the scheme-4 request comes from notate's own
`enterAnimationUpdate`, not from the framework's `ScrollHelper`, so it appears on every erase whatever
the stroke direction, and (b) the GC follows the lift after about 100–200 ms instead of about 1.5 s,
because notate clears at once with flag 1 [inferred]. `OnyxScrollRefreshManager beforeScroll` lines
may also appear if Entry A is live (see §7.2 item 5) [unknown].

**Lasso select (freehand):**

```
(contact)            HandlePenTrigger …   HWEpdcManager: ### EUS, s 3 c 0     (SF dashed trace)
(pen-down callback)  — nothing visible: 2308 stored under scheme 3
(lift)               — clear under scheme 3: no EUS, no GC
(selection made)     State change to: toPause   Post enabled!   ### EUS, s 2 c 0
                     full-view GC16 (update_mode[1]) + popup GC
                     State change to: running
```

No scheme 4 and no A2, because the session stays at PS 2 throughout the gesture [inferred]. The
exception is the race in §4.2 step 2, where `EUS s 4` would precede `EUS s 3`. Rectangle select
behaves like the eraser (scheme 4 during, GC at lift) plus the extra whole-view GC.

**Writing:** `HandlePenTrigger` + `EUS s 3` per burst; nothing at lift; `toPause`, `Post enabled!`,
`EUS s 2`, a partial GC16 and `running` about 2 s after the pen leaves hover range [inferred].

## 8. Compared with Tangleaf (observations only)

Where notate does better, or differently in a way worth studying:

1. **Native `SurfaceView` canvas.** No WebView, so the framework's WebView scroll-A2 entry (07 §6 Entry
   B) cannot fire for it. Every SurfaceView frame carries an explicit whole-view update entry from
   the Onyx `SurfaceView` holder (`fwv/SurfaceView.java:110-124`). A SurfaceView's default update mode
   is therefore a per-frame lever that a WebView-based app does not have in the same form [code].
2. **Eraser-end detection at hover.** notate switches to the eraser configuration from
   `ACTION_HOVER_MOVE` with `TOOL_TYPE_ERASER`, so the PS 3 lands before contact. Tangleaf switches
   render off at contact, after the reader-to-main-thread hop (08 §5.4). The hover path decides tool
   state before any sample exists [code].
3. **Firmware excludes for floating chrome.** notate keeps the toolbar and sidebar out of the SF
   region with `setLimitRect(limit, excludes)`. Tangleaf clears excludes and swallows pen-downs on DOM
   overlays (08 §3, §8) [code].
4. **One session for the view's life.** notate never closes the helper on undo or page changes.
   Tangleaf closes and reopens with a GC (08 §5.12–§5.13) [code].
5. **Lasso with render on, selection shown with render on.** notate keeps PS 2 while a selection is
   shown, where Tangleaf goes to steady PS 3 (08 §5.6). The cost is that a pen-down clears the
   selection, so notate's selection is moved by finger only [code].
6. **Partial GC tied to the stroke rect** instead of a whole-WebView GU reconcile (08 §5.3). The hand-back
   (PS 3) happens only on hover exit + 2 s, so SF ink stays the visible layer while writing, as in stock
   [code + inferred].

Where notate does worse than Tangleaf's current path, or has the same weakness:

1. **Same paused-session erase.** notate's stroke/pixel eraser and rectangle select put pen input
   into PS 3, which is the precondition for Tangleaf's flash (08 §1 item 3). notate then requests the
   A2 and the GC itself, so its flash is certain and immediate [inferred].
2. **Transient requests and stroke-parameter pushes on every pen-down** (colour, style, width,
   eraser flag, 2308, clear), several of them while SF is inking. Tangleaf sends nothing per stroke
   (08 §5.3). Whether mid-stroke `SET_STROKE_*` with unchanged values disturbs SF ink is **[unknown]**.
3. **No per-stroke reconcile.** App frames after a stroke are dropped until the next pause. Undo by
   two-finger tap right after writing is not shown. Tangleaf's `HAND_WRITING_REPAINT_MODE` reconcile
   frame is the one entry schema 7 lets through (08 §5.3, 07 §4.5).
4. **No pause for dialogs, IME, focus loss or popups** (the pen popup even turns render back on
   while open). Tangleaf pauses for all of them (08 §5.10, §5.14).
5. **Reopen without close, direct pen states that bypass the SDK guard, render left on for
   non-rendering tools after `performHardRefresh`.** Tangleaf's `cycleScribbleSession` also bypasses
   the guard (08 §9 item 4), but its render state follows the tool.
6. **GC on every UI interaction** (toolbar, fast-mode exits, popups). Tangleaf issues GCs only after
   closes, keyboard hide and overlay close (08 §6.2).
7. **Dead calls:** `setDisplayScheme`, `setEpdTurbo(boolean)`, `View.setEnablePenSideButton`. Tangleaf
   makes no calls of this kind (08 §1 item 9).

Techniques worth studying, without judging them for Tangleaf:

- hover-time tool switching from `TOOL_TYPE_ERASER`;
- firmware excludes for floating UI;
- SurfaceView frames with an explicit update entry;
- hand-back delayed until the pen leaves hover range;
- `enterAnimationUpdate`/`exitAnimationUpdate(true)` around paused-session pen gestures. This is a
  self-managed version of the same scheme-4 → GC sequence the framework produces for Tangleaf, but
  immediate, and a hardware log could compare the two.

## 9. Open questions (static limits)

1. Is the full-screen mode-5 entry that `CLEAR_TRANSIENT_UPDATE` adds while SF is in scheme 3 dropped,
   shown, or deferred to the next hand-back (07 §5.3 vs §4.5)?
2. How does a `View.invalidate(mode)` on a `SurfaceView` (a window-layer entry) combine with the
   SurfaceView layer's own `addEpdc` entry in SF, for example whether notate's GC invalidates refresh
   the canvas pixels at all?
3. Does the notate window get `topScrollBounds` (Entry A live), and does the framework log
   `OnyxScrollRefreshManager beforeScroll` during paused-session stylus moves?
4. Which reaches SF first at pen-down with render on: SF's first sample (scheme 3) or the
   main-thread `applyTransientUpdate(2308)`?
5. Do the three near-simultaneous `setupTouchHelper` calls at open and foreground (reopen without
   close) leave the in-process reader or SF in a consistent state?
6. The side-button flag `b` delivered with eraser-end events (decides the no-eraser-tool case, §4.4
   item 5).
