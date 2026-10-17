# Saber — how an open-source Flutter note app drives the BOOX pen and panel

Written 2026-09-25 from static reading of Saber at commit `07774e4` (2026-09-04), its Onyx plugin
`packages/onyxsdk_pen`, the Onyx SDK AARs it resolves, the Flutter engine and framework sources in
`/opt/flutter` (checkout of 2026-08-19), and the Onyx framework and SurfaceFlinger (SF) analysis in
`../stock-notes/`. No device was touched and nothing was run. The purpose is comparison, action by
action, with the stock Notes app (`../stock-notes/00-overview.md`) and with Tangleaf
(`../stock-notes/08-tangleaf-current-path.md`). No fixes for Tangleaf are proposed.

Device facts used here (verified on the tablet by the coordinator, 2026-09-25): firmware 4.2,
`ro.build.display.id=2026-04-28_17-50_4.2-rel_04282_555977efe`, system partition Android 13
(`ro.build.version.sdk=33`), vendor partition Android 11 (`ro.vendor.build.version.sdk=30`),
`vendor.onyx.tablet=true`. The device's `/system/bin/surfaceflinger` and
`/system/framework/framework.jar` are byte-identical to the copies analysed in `../stock-notes/`, so
every SF and framework statement cited from there applies to this device.

## 0. Notation and sources

Tags: **[code]** read in the cited source; **[inferred]** reasoned from code that was read;
**[unknown]** not determinable statically. "07 §4.5" etc. point into `../stock-notes/`.

| shorthand | root                                                                                                                                          |
| --------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `plug/`   | `~/tmp_zfs/reference_notes_apps/saber/packages/onyxsdk_pen/`                                                                                  |
| `kt/`     | `plug/android/src/main/kotlin/com/example/onyxsdk_pen/`                                                                                       |
| `lib/`    | `~/tmp_zfs/reference_notes_apps/saber/lib/`                                                                                                   |
| `pen/`    | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-pen-1.5.4.3/sources/com/onyx/android/sdk/pen/`                                                     |
| `dev/`    | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-device-1.3.5.2/sources/com/onyx/android/sdk/`                                                      |
| `fw/`     | `~/tmp_zfs/onyx_framework/out/sources/android/` (framework.jar)                                                                               |
| `fl/`     | `/opt/flutter/` (engine Java under `engine/src/flutter/shell/platform/android/io/flutter/`, framework Dart under `packages/flutter/lib/src/`) |

### 0.1 SDK version actually linked

- The plugin declares `com.onyx.android.sdk:onyxsdk-device:1.3.4` and
  `com.onyx.android.sdk:onyxsdk-pen:1.5.4` (`plug/android/build.gradle.kts:54-57`) [code]. The Saber
  app itself declares no Onyx dependency, only the BOOX Maven repository
  (`android/build.gradle.kts:6`) [code].
- POMs fetched from `repo.boox.com`: `onyxsdk-pen:1.5.4` → `onyxsdk-base:1.8.5` → `onyxsdk-device:1.3.4`
  [code]. So the resolved set is pen 1.5.4 + device 1.3.4, older than Tangleaf's pen 1.5.4.3 + device
  1.3.5.2 [code].
- Both AARs were decompiled with the same jadx (1.5.6) as the reference trees and diffed after
  re-decompiling the 1.5.4.3 / 1.3.5.2 jars with identical settings [code]:
  - pen: `TouchHelper` differs only by one `Debug.i` line in `printTouchInfo`; `AppTouchInputReader`
    (not used here) differs; 1.5.4.3 adds `TouchPointProcessor`, `TouchPointFilterStats`,
    `data/PenRect`. `SFTouchRender`, `RawInputReader`, `RawInputManager`, `EpdPenManager` are
    identical. `libonyx_pen_touch_reader.so` (arm64) has the same sha256 in both [code].
  - device: 1.3.5.2 adds `setScreenHandWritingRegionConfig`, `drawScreenHandWritingBitmap`,
    `enableDither`, `applySaturation` and a few others; the methods Saber reaches
    (`invalidate(View,UpdateMode)`, `enablePost`, `setScreenHandWritingPenState`,
    `setScreenHandWritingRegionLimit/Exclude`, `setStrokeStyle/Color/Width`,
    `setBrush/EraserRawDrawingEnabled`) have the same bodies up to obfuscated names [code].
  - Therefore every SDK line citation below uses the 1.5.4.3 / 1.3.5.2 trees under `pen/` and `dev/`,
    which the stock-notes documents also cite.
- `Device.currentDevice()` is `SDMDevice` on `lito` (08 §0) [code there].
- Flutter: Saber requires `flutter >=3.47.0` (`pubspec.yaml:22`) [code]. The Flutter behaviour cited
  below (platform-view display mode, lifecycle mapping, `RenderOpacity`) is read from `fl/`; that
  Saber's release build uses a Flutter with the same behaviour is [inferred] (the SurfaceView →
  Virtual Display fallback has existed since Flutter 3.0).

## 1. Summary

1. **One hidden `SurfaceView` per rendered page, hosted in a Flutter Virtual Display.** Every page
   that is in the viewport builds an `OnyxSdkPenArea` widget (`lib/components/canvas/canvas.dart:112`,
   `lib/components/canvas/canvas_gesture_detector.dart:636-642`) [code], which puts an `AndroidView`
   under `Opacity(opacity: 0)` beneath the page content (`plug/lib/src/onyxsdk_pen_area.dart:114-131`)
   [code]. The native view is a bare `SurfaceView` (`kt/OnyxsdkPenArea.kt:133`) [code]. Because the
   view is a `SurfaceView`, Flutter refuses Texture Layer Hybrid Composition and, for `AndroidView`
   (`initAndroidView`, no `hybridFallback`), falls back to **Virtual Display** mode
   (`fl/…/plugin/platform/PlatformViewsController.java:68, 224-246, 565-610`;
   `fl/packages/flutter/lib/src/services/platform_views.dart:134-152, 1262-1284`) [code]. The view
   therefore lives in a `TYPE_PRIVATE_PRESENTATION` window on a virtual display
   (`SingleViewPresentation.java:116`) [code], not in Saber's activity window. `Opacity(0)` makes
   Flutter skip painting it entirely (`fl/…/rendering/proxy_box.dart:949`) [code].
2. **The SDK is anchored to that dummy view, the content is elsewhere.** All ink the user sees from
   the app is drawn by Flutter into its own `FlutterSurfaceView` (default `RenderMode.surface`,
   `fl/…/embedding/android/FlutterActivity.java:1248-1249`) [code]. SF sees the app as a buffer layer;
   no Onyx per-view update mode ever describes it [inferred]. Every `EpdController.invalidate(view,
   GC)` Saber issues targets the invisible virtual-display `SurfaceView`, so none of them is a GC of
   the visible content [inferred from the framework path, `fw/view/ViewGroup.java:4762-4768`,
   `fw/onyx/ViewUpdateHelper.java:1477-1485`].
3. **The SF limit rect is at the screen origin and covers roughly the whole screen.** The limit is
   `getLocalVisibleRect` of the view plus `getLocationOnScreen` (`pen/RawInputManager.java:141-147`,
   `fw/onyx/ViewUpdateHelper.java:1225-1239`) [code]. Inside the virtual display both are relative
   to that display: origin (0,0), size = the page's logical size × density (page default 1000 × 1400
   logical, `lib/data/editor/page.dart:20-22`) [code for the sizes; that the SF rect is
   `(0,0)–(1000·d, 1400·d)` on the physical panel, larger than the 1860 × 2480 panel at d ≈ 2, is
   inferred]. SF inks wherever the pen tip is, over the toolbar, dialogs, bottom sheets and the soft
   keyboard [inferred]. No exclude rect is ever sent (`kt/OnyxsdkPenArea.kt:245-247`, empty list →
   `pen/RawInputReader.java:418-421` returns early) [code], so SF's global exclude table keeps
   whatever another app left in it [inferred].
4. **Writing: SF inks live, Flutter redraws are dropped until a debounced hand-back.** With a pen tool
   the session sits in pen state 2 with brush raw drawing on. Flutter redraws the stroke itself, but
   while SF is in handwriting schema 7 those frames carry no `0x80002` entry and are dropped
   (07 §4.5) [inferred]. One second after the **last** pen-up (every pen-down cancels the timer),
   `forceRefresh()` runs on a `java.util.Timer` thread: render off (`ENABLE_POST(1)`, pen 3), a GC
   invalidate of the dummy view, render on (pen 2) (`kt/OnyxsdkPenArea.kt:163-171, 179-183,
   235-239`) [code]. That pause is the only thing that takes SF out of schema 7; after it Flutter's
   frames reach the panel in whatever mode SF picks for the layer [inferred].
5. **Erasing is always app-drawn with the session paused.** The eraser tool, the shape pen, text
   editing and any unmapped tool map to `OnyxStrokeStyle.disabled` (`lib/components/canvas/canvas.dart:47-69`)
   [code], which runs `setRawDrawingEnabled(false)` (`kt/OnyxsdkPenArea.kt:126`) [code] → pen state 3
   for the whole time the tool is selected. The eraser raw-drawing flag is never set to 1 (the SDK's
   `resetPenDefaultRawDrawing` sends `(false, 5)` on every `setRawDrawingEnabled`, `pen/TouchHelper.java:348-351`)
   [code]. There is one eraser mode: whole-stroke, radius 10 (`lib/data/tools/eraser.dart:12-45`) [code].
6. **Lasso, selection drag and laser pointer ink through SF as a solid pen line.** `select` and
   `laserPointer` map to `OnyxStrokeStyle.pen` (`canvas.dart:62-65`) → `STROKE_STYLE_PENCIL` (0),
   black, width 3 (`canvas.dart:71-90`, `kt/OnyxsdkPenArea.kt:213`) [code]. No dash style, no
   stroke parameters, no fast mode [code].
7. **A burst of redundant transactions on every Flutter rebuild.** `didUpdateWidget` invokes
   `updateStroke` unconditionally (`onyxsdk_pen_area.dart:84-92`) [code], and the page widgets are
   rebuilt on every `setState` of the editor and on **every frame of a pan or zoom**
   (`lib/components/canvas/interactive_canvas.dart:1036-1040, 1094-1106`) [code]. Each `updateStroke`
   sends `SET_STROKE_STYLE`, `SET_STROKE_WIDTH`, `SET_STROKE_COLOR`, `SET_BRUSH_RAW_DRAWING_ENABLED(1)`,
   `SET_ERASER_RAW_DRAWING_ENABLED(0,5)` (plus a render/pen-state change if the tool's enabled flag
   changed) and a mode-98 invalidate of the dummy view (`kt/OnyxsdkPenArea.kt:110-127`) [code].
8. **One shared method channel, many native instances.** Every native `OnyxsdkPenArea` installs
   itself as the handler of the same channel name `"onyxsdk_pen_area"` (`kt/OnyxsdkPenArea.kt:38, 242`)
   and never removes itself (`dispose`, `:276-278`) [code]. Every Dart-side call from every page goes
   to the most recently created native instance, even after it was disposed [code]. The Dart state
   registers a lifecycle observer and never unregisters it (`onyxsdk_pen_area.dart:94-109`, no
   `dispose` override) [code].
9. **Global SF state shared by several helpers.** Two visible pages mean two `TouchHelper`s
   driving the same global pen state, limit and flags. A page leaving the viewport disposes its view
   and sends `closeRawDrawing` → pen state 0 (STOP) globally, while the other page's helper still
   believes render is on and will not send pen state 2 again until its cached flag changes
   (`pen/TouchHelper.java:188-197`) [code for the guard; the resulting SF state is inferred].
10. **No refresh policy of its own.** No `applyGCOnce`, `repaintEverything`, `applyTransientUpdate`,
    `setEpdTurbo`, `setViewDefaultUpdateMode`, `handwritingRepaint`, EinkWise call or colour-CU call
    exists in the plugin (grep of `kt/`) [code]. The panel's waveform choices for Saber are whatever
    SF and the EinkWise app config give a non-Onyx package [inferred].
11. **Session lifetime follows the Flutter lifecycle.** `AppLifecycleState.paused` (Android `onStop`)
    → `closeRawDrawing`; `resumed` (Android `onResume` **with window focus**) → `openRawDrawing` +
    render on, regardless of the current tool (`onyxsdk_pen_area.dart:100-109`,
    `kt/OnyxsdkPenArea.kt:221-229`; mapping `fl/…/systemchannels/LifecycleChannel.java:69-110`) [code].
    Inactive (focus loss, `onPause`) sends nothing [code].

## 2. Structure

| piece                                                                            | role                                                                                                                                                                                      | entry points                                                                                                                          |
| -------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| `kt/OnyxsdkPenPlugin.kt`                                                         | Flutter plugin; `isOnyxDevice` = `Build.BRAND == "onyx"`; registers the view factory; calls `RxManager.Builder.initAppContext` and `HiddenApiBypass.addHiddenApiExemptions("")` at attach | `:24-35`, `:39`, `:51`                                                                                                                |
| `kt/OnyxsdkPenAreaFactory.kt`                                                    | creates one `OnyxsdkPenArea` per `AndroidView`                                                                                                                                            | `:11-14`                                                                                                                              |
| `kt/OnyxsdkPenArea.kt`                                                           | one `TouchHelper` on a dummy `SurfaceView`; method channel `updateStroke` / `setDraw`; pen-up refresh timer                                                                               | init `:241-260`, `updateStroke` `:48-128`, `setDraw` `:221-229`, `forceRefresh` `:235-239`, callback `:146-208`, `dispose` `:276-278` |
| `plug/lib/src/onyxsdk_pen_area.dart`                                             | widget: `Stack[Opacity(0, AndroidView), child]`; sends `updateStroke` on every rebuild and `setDraw` on lifecycle                                                                         | `:84-92`, `:100-109`, `:112-132`                                                                                                      |
| `lib/components/canvas/canvas.dart`                                              | maps the Saber tool to an Onyx style, colour, width                                                                                                                                       | `:47-90`, `:112-117`                                                                                                                  |
| `lib/pages/editor/editor.dart`                                                   | Flutter drawing, erasing, selection; `setState` on draw start/end, tool changes                                                                                                           | `:516-556`, `:558-722`, `:745-762`                                                                                                    |
| `lib/components/canvas/canvas_gesture_detector.dart` + `interactive_canvas.dart` | page list, pan/zoom; rebuilds pages per transform frame; builds real canvases only for pages in the viewport                                                                              | `:600-652`; `:1036-1040`, `:1094-1106`                                                                                                |

`TouchHelper.create(view, callback)` (`kt/OnyxsdkPenArea.kt:131`) selects feature
`hasStylus ? 2 : 1` with the touch listener enabled (`pen/TouchHelper.java:45-54`) [code]. With a
stylus digitizer present (true on the Note Air 4C [inferred]) that is **`SFTouchRender` only**: no
`AppTouchRender`, so none of its extra `ENABLE_POST(0)` / pen-state transactions that stock and
Tangleaf both get (07 §3) [code].

The in-process raw reader still runs (`libonyx_pen_touch_reader.so`, reads evdev itself); its
callbacks are posted to the dummy view's handler (`pen/touch/SFTouchRender.java:44-134`) [code].
Saber uses them only as "stroke began" / "stroke ended" signals; the point data is discarded
(`kt/OnyxsdkPenArea.kt:147-161, 231-233`: the preview list is cleared on every update) [code].
Flutter's own stroke comes from the ordinary `MotionEvent`s delivered to `FlutterView`
(`editor.dart:558-722`) [code].

## 3. The primitives, traced to SurfaceFlinger

Transaction names as in 07 §2.2 / 08 §3. All SF calls below are synchronous Binder transactions.

| Saber call (site)                                                                          | SDK chain                                                                                                                                                                                                                                                                                                                                                                                                                                                  | SF transactions, in order                                                                                                                   | SF-visible effect                                                                                                                                                                                                                                                                                                                                                                                                                         |
| ------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `TouchHelper.create(view, cb)` (lazy, first touched in init at `kt/OnyxsdkPenArea.kt:256`) | ctor → `SFTouchRender.bindHostView` → `RawInputManager.setHostView` → `getLocalVisibleRect` → `RawInputReader.setLimitRect` (`pen/RawInputManager.java:141-147`, `pen/RawInputReader.java:410-416`)                                                                                                                                                                                                                                                        | `SET_SCREEN_HANDWRITING_REGION_LIMIT` = the view's local visible rect, which is `(0,0,0,0)` because the view is not attached yet [inferred] | limit = empty rect until the first layout [inferred; SF handling of an empty limit unknown]                                                                                                                                                                                                                                                                                                                                               |
| `openRawDrawing()` (`:256`, `setDraw(true)` `:223`)                                        | `SFTouchRender.openDrawing`: `setStrokeStyle(0)`, reader `start`, `EpdPenManager.startDrawing` (`pen/touch/SFTouchRender.java:265-269`, `pen/EpdPenManager.java:35-37`)                                                                                                                                                                                                                                                                                    | `SET_STROKE_STYLE(0)`, pen **1**                                                                                                            | reader START (07 §4.1)                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `setRawDrawingEnabled(true)` when render was off                                           | render → `resumeDrawing`; reader `resume` (in-process); `resetPenDefaultRawDrawing` (`pen/TouchHelper.java:170-178, 348-351`)                                                                                                                                                                                                                                                                                                                              | pen **2**, `SET_BRUSH_RAW(1)`, `SET_ERASER_RAW(0,5)`                                                                                        | reader running; tip → reader 2 (inks); eraser end → reader 3 (no-op)                                                                                                                                                                                                                                                                                                                                                                      |
| `setRawDrawingEnabled(false)` when render was on                                           | render → `leaveScribbleMode` + `pauseDrawing`; reader `pause`; reset                                                                                                                                                                                                                                                                                                                                                                                       | `ENABLE_POST(1)`, pen **3**, `SET_BRUSH_RAW(1)`, `SET_ERASER_RAW(0,5)`                                                                      | leaves schema 7 if active (`Post enabled!`, `EUS s 2`); reader paused                                                                                                                                                                                                                                                                                                                                                                     |
| `setRawDrawingEnabled(x)` with no change                                                   | render and reader guarded out (`pen/TouchHelper.java:188-197, 213-222`)                                                                                                                                                                                                                                                                                                                                                                                    | `SET_BRUSH_RAW(1)`, `SET_ERASER_RAW(0,5)`                                                                                                   | flags rewritten, no state change                                                                                                                                                                                                                                                                                                                                                                                                          |
| `setRawDrawingRenderEnabled(false)` (`:258`)                                               | `leaveScribbleMode` + `pauseDrawing`                                                                                                                                                                                                                                                                                                                                                                                                                       | `ENABLE_POST(1)`, pen **3**                                                                                                                 | pause                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `setLimitRect(limit, [])` (layout listener `:243-247`)                                     | `RawInputReader.setLimitRect` + `setExcludeRect([])` returns early                                                                                                                                                                                                                                                                                                                                                                                         | `SET_SCREEN_HANDWRITING_REGION_LIMIT` = view rect in virtual-display coordinates                                                            | limit at screen origin, page-sized in physical px [inferred, §1 item 3]                                                                                                                                                                                                                                                                                                                                                                   |
| `setStrokeStyle(s)` (`:111`)                                                               | `EpdPenManager.setStrokeStyle` → `EpdController.setStrokeStyle`                                                                                                                                                                                                                                                                                                                                                                                            | `SET_STROKE_STYLE(s)`                                                                                                                       | SF painter style                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `setStrokeWidth(w)` (`:112`)                                                               | `RawInputReader.setStrokeWidth`: native + `EpdController.setStrokeWidth` (`pen/RawInputReader.java:359-364`)                                                                                                                                                                                                                                                                                                                                               | `SET_STROKE_WIDTH(w)`                                                                                                                       | width in SF pixels; Saber passes logical px × zoom (`canvas.dart:79-90`), not × density [code; the visible width mismatch is inferred]                                                                                                                                                                                                                                                                                                    |
| `setStrokeColor(c)` (`:113`)                                                               | `EpdPenManager.setStrokeColor` → `EpdController.setStrokeColor`                                                                                                                                                                                                                                                                                                                                                                                            | `SET_STROKE_COLOR(c)`                                                                                                                       | colour after Saber's HSV clamp: saturation and value forced to 0 or 1 (`kt/OnyxsdkPenArea.kt:69-98`) [code]; how the Kaleido panel shows SF colour ink is [unknown]                                                                                                                                                                                                                                                                       |
| `EpdController.invalidate(view, UpdateMode.GC)` (`:127`, `:237`)                           | `SDMDevice.invalidate` → reflected `View.invalidate(98)` (`dev/device/SDMDevice.java:1124-1128`) → `invalidateInternal` → `ViewGroup.invalidateChild(v, r, 98)`: hardware-accelerated → `ViewRootImpl.addUpdateEntry(view, 98)` + `onDescendantInvalidated`; software → `invalidateChildInParent`, which calls `checkThread()` (`fw/view/View.java:10818-10869`, `fw/view/ViewGroup.java:4762-4768`, `fw/view/ViewRootImpl.java:13133-13158, 12316-12321`) | none directly; an update entry `(view rect in its surface, 98)` on the **virtual display's** `ViewRootImpl`                                 | not a GC of the panel content [inferred]. From the `Timer` thread the software path would throw `CalledFromWrongThreadException`, which `ReflectUtil.invokeMethodSafely` swallows; whether the presentation window is hardware-accelerated is [unknown]. A redraw of the virtual display may make Flutter redraw its last layer tree (`fl/…/shell/common/shell.cc:1318-1346`) and so push one more buffer of the Flutter layer [inferred] |
| `closeRawDrawing()` (`setDraw(false)` `:227`, `dispose` `:277`)                            | `SFTouchRender.closeDrawing`: style 0, `leaveScribbleMode`, reader pause, pen 3, reader quit, pen 0 (`pen/touch/SFTouchRender.java:271-275`); not guarded by the open flag (`pen/TouchHelper.java:162-168`)                                                                                                                                                                                                                                                | `SET_STROKE_STYLE(0)`, `ENABLE_POST(1)`, pen **3**, pen **0**                                                                               | STOP (07 §4.1)                                                                                                                                                                                                                                                                                                                                                                                                                            |

Calls Saber never makes [code, grep of `kt/`]: `setSingleRegionMode` / region mode, `setExcludeRect`
with content, `setPostInputEvent`, `setPenUpRefreshTimeMs`, `enableSideBtnErase`,
`setEraserRawDrawingEnabled(true, …)`, `setStrokeParameters`, `ENABLE_COLOR_CU`, any update-scheme,
transient, turbo, one-shot GC or EinkWise call, `setViewDefaultUpdateMode`, `handwritingRepaint`,
`appResetCTPDisableRegion` / CTP regions, any direct `setScreenHandWritingPenState`.

Style mapping, Saber tool → Onyx constant (`canvas.dart:47-69` → `kt/OnyxsdkPenArea.kt:211-218`)
[code]:

| Saber tool                                     | `OnyxStrokeStyle` | SF style id   | render |
| ---------------------------------------------- | ----------------- | ------------- | ------ |
| fountain pen                                   | brush             | 3 `NEO_BRUSH` | on     |
| ballpoint pen                                  | pen               | 0 `PENCIL`    | on     |
| highlighter                                    | marker            | 2 `MARKER`    | on     |
| pencil                                         | pencil            | 4 `CHARCOAL`  | on     |
| select (lasso)                                 | pen               | 0             | on     |
| laser pointer                                  | pen               | 0             | on     |
| shape pen, eraser, text editing, anything else | disabled          | unchanged     | off    |

## 4. Session states

In the stock notation (00 §2.1: pen state last sent, render flag, input flag):

| state | pen / render / input | when Saber is in it                                                                               |
| ----- | -------------------- | ------------------------------------------------------------------------------------------------- |
| U     | 0 / – / –            | no editor open, after `onStop`, after the last page view is disposed                              |
| R'    | 3 / off / on         | only between the constructor and the first layout of a page view (`kt/OnyxsdkPenArea.kt:256-258`) |
| P     | 3 / off / off        | eraser, shape pen, text editing selected; the few ms inside `forceRefresh`                        |
| D     | 2 / on / on          | pen tools, select, laser pointer; also right after any `resumed`, whatever the tool               |

SF-side (07 §2.2): in D the first tip sample enters schema 7 and scheme 3; nothing leaves it until the
`forceRefresh` pause 1 s after the last pen-up, a tool change to a disabled tool, `onStop`, or a page
view disposal. Brush raw drawing is always 1 and eraser raw drawing always `(0, 5)`, so the eraser end
never inks through SF [code for the flags, 07 §4.2 for the reader selection].

## 5. Per action

"US" = one `updateStroke` (§3; for an enabled tool: `SET_STROKE_STYLE`, `SET_STROKE_WIDTH`,
`SET_STROKE_COLOR`, the `setRawDrawingEnabled` row that applies, then the dummy-view GC invalidate).
"FR" = `forceRefresh` = `setRawDrawingEnabled(false)`, dummy-view GC invalidate,
`setRawDrawingEnabled(true)`, on a `Timer` thread.

### 5.1 Open a note

1. The editor builds; pages in the viewport get a real `Canvas` with an `OnyxSdkPenArea`, the others
   a placeholder `SizedBox` with no platform view (`canvas_gesture_detector.dart:636-642`,
   `canvas.dart:108-134`) [code].
2. Per page view, in the native constructor (`kt/OnyxsdkPenArea.kt:241-260`) [code]: channel handler
   taken over; `TouchHelper` created → `LIMIT(0,0,0,0)`; open → `SET_STROKE_STYLE(0)`, pen 1;
   `setRawDrawingEnabled(true)` → pen 2, `BRUSH(1)`, `ERASER(0,5)`; `setRawDrawingRenderEnabled(false)`
   → `ENABLE_POST(1)`, pen 3.
3. First layout of the view in its virtual display → layout listener [code]: `LIMIT(0,0,W,H)` at the
   virtual-display origin; `setRawDrawingEnabled(false)` (render already off: reader pause, `BRUSH(1)`,
   `ERASER(0,5)`); `setRawDrawingEnabled(true)` → pen 2, reader resume, `BRUSH(1)`, `ERASER(0,5)`.
   State D.
4. The creation parameters (style, colour, width) are **not** applied by the constructor; SF keeps
   style 0 and whatever width and colour it last had until the first rebuild triggers a US [code].
   With a disabled tool selected, render stays on until that first US [code + inferred].
5. With two pages visible, steps 2–3 run twice; the last limit wins (both are the same rect) [inferred].
6. No GC, no update mode, no exclude, no region mode [code].

### 5.2 Write (pen tools)

1. Pen down: SF reader 2 inks; first sample → `HandlePenTrigger`, schema 7, `EUS s 3` (07 §4.2).
   In-process reader → `onBeginRawDrawing` posted → cancels a pending FR (`kt/OnyxsdkPenArea.kt:147-152, 174-177`) [code].
2. Flutter's scale gesture starts → `onDrawStart` → `setState` (`editor.dart:558-601`) → rebuild → US:
   style/width/colour unchanged, `BRUSH(1)`, `ERASER(0,5)`, dummy invalidate [code].
3. Moves: SF inks. Flutter adds points and repaints via `page.redrawStrokes()` without `setState`
   (`editor.dart:603-612`) [code]; those frames are dropped by SF's schema 7 (07 §4.5) [inferred].
4. Up: `onDrawEnd` → `setState` → US [code]. `onEndRawDrawing` → FR scheduled at +1000 ms
   (`kt/OnyxsdkPenArea.kt:179-183, 163-171`; `refreshDelay` 1 s from `canvas.dart:113`) [code].
5. +1000 ms after the last pen-up of a burst: FR → `ENABLE_POST(1)` (SF: `Post enabled!`, 150 ms
   sleep, `EUS s 2`), pen 3 (reader pause; `stopHandwriting` no-op), `BRUSH(1)`, `ERASER(0,5)`,
   dummy invalidate, pen 2, `BRUSH(1)`, `ERASER(0,5)` [code for the calls, 07 §4.3 for SF]. After
   this the next Flutter frame reaches the panel; if the dummy invalidate makes Flutter redraw, that is
   the frame that replaces SF's ink with Flutter's [inferred].
6. No per-stroke reconcile, no `0x80002` frame, no GC on the visible layer [code].
7. Pressure: Flutter strokes use stylus pressure (`canvas_gesture_detector.dart:420-445`) [code]. SF's
   width is fixed per US; the width is halved when pressure is enabled (`canvas.dart:79-90`) [code].
   Whether SF styles 3/4 vary with pressure is [unknown].

### 5.3 Erase with the eraser tool

Saber has one eraser: whole-stroke, radius 10 in page coordinates (`eraser.dart:12-45`) [code]. No
partial, area or soft eraser.

1. Tool select (toolbar `setTool`) → `setState` (`editor.dart:1424-1440`) → US(disabled): no style/width/colour,
   `ENABLE_POST(1)`, pen 3, reader pause, `BRUSH(1)`, `ERASER(0,5)`, dummy invalidate [code]. State P.
2. Per gesture: SF paused → no ink, no schema 7. The in-process reader is paused, so no raw
   callbacks and no FR [code: `pen/RawInputReader.java:427-431` gates dispatch on input].
   `onDrawStart` → `setState` → US(disabled) (flags rewritten, invalidate). Each move removes whole
   strokes from the model and calls `page.redrawStrokes()` (`editor.dart:612-620`) [code]; each such
   Flutter frame is composited normally (schema 0) in SF's default mode for the layer [inferred].
3. Lift: `onDrawEnd` → `setState` → US(disabled); with "disable eraser after use" the previous tool is
   restored in the same `setState` → US(enabled) → pen 2 (`editor.dart:668-673`) [code].
4. No firmware eraser trace, no `ENABLE_POST` after the erase, no refresh request [code].

### 5.4 Erase with the stylus eraser end (or the side button)

1. Flutter reports the eraser end as `PointerDeviceKind.invertedStylus`; while hovering,
   `_detectStylusButton` → `onStylusButtonChanged(true)` → tool = `Eraser()` → `setState`
   (`canvas_gesture_detector.dart:456-481`, `editor.dart:745-762`) [code] → US(disabled) → pen 3 before
   the eraser touches the glass [inferred timing: hover precedes contact].
2. SF: the eraser end selects reader 3 (no-op) anyway because the eraser flag is `(0, 5)` [code flags,
   07 §4.2]. Erasing as in 5.3.
3. Lift: `onDrawEnd` restores the last non-eraser tool (`editor.dart:669-673`) → US(enabled) → pen 2
   [code]. Leaving hover with the button released also restores it (`editor.dart:754-759`) [code].
4. If the tool switch does not happen before contact (no hover seen), the pen tool stays active and
   Flutter records an ink stroke from the eraser end, while SF draws nothing (reader 3) [inferred from
   `isHovering` gating at `editor.dart:748`].

### 5.5 Lasso (select tool)

1. Tool select → US: `SET_STROKE_STYLE(0)`, `SET_STROKE_WIDTH(3)`, `SET_STROKE_COLOR(black)`, pen 2 if
   render was off [code].
2. Stroke: SF inks a **solid black pencil line** along the lasso; schema 7. Flutter builds the lasso
   path (`select.dart:52-66`) and repaints; frames dropped [inferred].
3. Lift: `onDrawEnd` → `select.onDragEnd` + `setState` → US [code]. +1 s FR → Flutter's selection
   rendering reaches the panel; the SF line is replaced only where Flutter's next frame repaints
   [inferred].
4. No dash style, no stroke parameters, no pause, no fast mode, no selection-specific state [code].

### 5.6 Drag a selection

1. Stylus drag inside the selection path (`editor.dart:571-577, 622-633`): SF still in D with the
   select style → SF inks a black line along the drag path [inferred from state D]. Flutter moves the
   strokes each move; frames dropped while in schema 7 [inferred].
2. Lift: `onDrawEnd` records history + `setState` → US; +1 s FR [code].
3. Finger drag: only with finger drawing enabled (`editor.dart:545-551`) [code]; SF is not involved
   and Flutter frames are composited in the default mode unless a pen stroke put SF in schema 7 less
   than 1 s earlier [inferred].
4. No transient A2, no turbo [code].

### 5.7 Open or close a menu, dialog, bottom sheet, colour picker

All are Flutter widgets in the same window (`editor.dart:1692-1716`, `lib/components/toolbar/color_bar.dart:300`)
[code]. The plugin gets no signal from them. The only native traffic is the US from whatever
`setState` the menu action causes [code]. The session stays in D with a pen tool, the limit covers
the screen, so the pen inks over an open dialog or sheet through SF [inferred]. A menu opened less
than 1 s after writing is drawn in frames SF drops until the FR [inferred].

System-level windows (notification shade, permission or share dialogs) take window focus → Flutter
`inactive` → nothing sent; the SF session keeps inking over them [inferred]. Focus back →
`resumed` → `setDraw(true)`: `openRawDrawing` on an already open helper (`SET_STROKE_STYLE(0)`,
pen 1, cached flags reset) then pen 2, `BRUSH(1)`, `ERASER(0,5)`, once per page state registered as an
observer, all routed to the same native instance [code]; SF style 0 until the next US [code].

### 5.8 Finger scroll and pinch zoom

1. Every transform frame → `InteractiveCanvasViewer` `setState` → builder → new page widgets →
   `didUpdateWidget` → US per page per frame [code, §1 item 7]. Width follows the zoom
   (`canvas.dart:81`) [code].
2. No pause, no pen-state change, no fast mode, no limit re-push (the virtual display does not move)
   [code].
3. Pages entering the viewport create a platform view (sequence 5.1 steps 2–3); pages leaving it are
   disposed → `closeRawDrawing` → pen 3, pen 0 globally (§1 item 9) [code + inferred].
4. Flutter frames while scrolling are composited in SF's default mode for the layer; within 1 s after
   a stroke they are dropped until the FR [inferred].
5. Whether the framework's scroll-A2 manager (07 §6, Entry A) fires for Saber depends on the
   activity window getting `topScrollBounds`: Saber is a non-Onyx package, so the package exemption
   does not apply; the bounds need a scroll-capture result or a class in the EinkWise
   `forceScrollRefreshClsList` (`fw/view/ViewRootImpl.java:3385-3396, 4367-4374`,
   `fw/onyx/optimization/EInkHelper.java:1256-1261`). `FlutterView` implements no scroll-capture
   callback and the whitelist contents are [unknown]; the likely outcome is no `topScrollBounds` and
   no Entry A [inferred]. Entry B needs a `WebView` and does not apply [code].

### 5.9 Page switch

Saber has no discrete page turn: pages are a vertical column (`canvas_gesture_detector.dart:616-652`)
[code]. "Go to page" / insert page set the transform (`editor.dart:1674-1684`), which behaves like
5.8 with a single jump. No page-turn GC [code].

### 5.10 Undo / redo

`setState` → US. No pause, no session change, no refresh [code]. The redraw happens in Flutter
frames, dropped if SF is still in schema 7 from a stroke less than 1 s earlier [inferred].

### 5.11 Leave the note

The editor route is popped → page widgets disposed → each `AndroidView` disposed → native `dispose`
→ `closeRawDrawing` per page view: `SET_STROKE_STYLE(0)`, `ENABLE_POST(1)`, pen 3, pen 0 [code]. No
GC; SF limit left at the last value; exclude untouched [code].

Afterwards the channel handler still points at the last (disposed) native instance and the Dart
states stay registered as lifecycle observers [code]. The next background/foreground cycle of the app
(from the file browser) sends `setDraw(false)` then `setDraw(true)` to that instance: the second
reopens a raw-drawing session (pen 1, pen 2, reader restarted) with no editor on screen, so SF would
ink within the stale limit over the browser until the app next stops [inferred].

### 5.12 Background / foreground

- `onStop` → `paused` → `setDraw(false)` from every live observer, all to one native instance →
  `closeRawDrawing` N times (the first stops the session; the rest repeat style 0, `ENABLE_POST(1)`,
  pen 3, pen 0) [code; SF's reaction to pen 3 on a stopped reader follows 07 §4.1: 3 ≠ 0 so
  `pause` + `stopHandwriting` run again, inferred].
- `onPause` alone (a translucent activity or dialog activity on top) → `inactive` → nothing [code].
- Return → `resumed` → `setDraw(true)` N times → open, pen 1, pen 2, flags; render on for any tool
  [code]. Style 0 until the next US; no limit re-push (the view does not re-layout) [code].
- The session is closed while in the background, unlike stock (00 §3.12) [code].

### 5.13 Keyboard and text input

- The note-title `TextFormField` sits in the app bar and autofocuses on a new unnamed note
  (`editor.dart:1653-1661`, `:123`) [code]. Nothing tells the plugin; with the default pen tool the
  session is in D and the limit covers the keyboard, so the pen inks over the IME through SF
  [inferred]. No CTP region is touched, so key taps work [code].
- Quill text on a page uses the text tool → disabled → pen 3 while it is selected (`canvas.dart:66-67`,
  `editor.dart:548`) [code].
- Keyboard hide: nothing [code].

## 6. Every refresh Saber issues

| trigger                                                                     | call                                            | reaches the visible panel content as                                                                                                                     |
| --------------------------------------------------------------------------- | ----------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| every US (every editor `setState`, every pan/zoom frame, every tool change) | `EpdController.invalidate(dummyView, GC)`       | an entry on the virtual display's window, not a GC of Saber's Flutter layer [inferred]; possibly one extra Flutter frame in SF's default mode [inferred] |
| FR, 1 s after the last pen-up                                               | the same invalidate, bracketed by pen 3 / pen 2 | the pause ends schema 7, so later Flutter frames are shown [inferred]                                                                                    |

Nothing else: no one-shot GC, no counted GC, no transient/fast mode and its full-screen exit update,
no `REGAL` frames, no GC on close [code].

## 7. Compared with stock

What Saber gets and loses relative to stock's capabilities (stock column from 00 and 01–07).

| capability                          | stock                                                                                                     | Saber                                                                                                                                    |
| ----------------------------------- | --------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| live SF ink for writing             | yes, style per pen, pen state 2 kept                                                                      | yes, styles 3/0/2/4 for its four pens [code]                                                                                             |
| live eraser trace drawn by firmware | yes, eraser raw drawing on with style 8 (types 0/1) or 5 (type 2); eraser end uses reader 4 (00 §3.3–3.4) | **no**: eraser flag always `(0,5)`, eraser tool pauses the session [code]                                                                |
| soft / partial / area eraser        | three types, soft style 8 with `[width,0.5,0.1]`                                                          | whole-stroke only [code]                                                                                                                 |
| no pause at erase pen-down          | yes (render stays on)                                                                                     | eraser tool: session paused for the whole tool; eraser end: paused from hover on [code + inferred]                                       |
| erase finish                        | `ENABLE_POST(1)`, render-only pause, invalidate, RS(300)                                                  | nothing native; Flutter frames only [code]                                                                                               |
| per-stroke refresh                  | none on this device (reconcile gated off by `vendor.onyx.tablet`)                                         | none; one debounced pause/resume 1 s after writing stops [code]                                                                          |
| hand-back of SF ink to app frames   | at the next pause (menu, erase finish, fast-mode exit)                                                    | at FR, tool change to a disabled tool, `onStop`, page-view disposal [code]                                                               |
| lasso in firmware layer             | dashed style 5 with dash parameters                                                                       | solid black style 0, width 3 [code]                                                                                                      |
| selection shown / drag              | state R (brush 0), fast mode 2308 + turbo 5 + dither 160, no SF ink during drag                           | state D, SF inks a black trail, no fast mode [code + inferred]                                                                           |
| fast mode for scroll, zoom, drags   | FM on/off with 5 s delayed clear                                                                          | none [code]                                                                                                                              |
| EinkWise / scroll-A2 exposure       | package contains `com.onyx` → exempt from Entry A; not a WebView → no Entry B                             | non-Onyx package → Entry A applies **if** `topScrollBounds` is set [unknown, likely not]; no Entry B [code]                              |
| colour brushes                      | colour pens (visible colour of SF ink [unknown])                                                          | colour sent to SF after a saturate-or-black clamp; Flutter draws true colour after the hand-back [code]                                  |
| pressure                            | SF styles with per-pen parameters                                                                         | Flutter strokes pressure-aware; SF fixed width per update [code]                                                                         |
| limit / exclude                     | `EditorView` bounds, typed exclude rects for overlays                                                     | virtual-display-origin rect of page size, no excludes; SF inks over the whole UI [inferred]                                              |
| palm / finger                       | `setPostInputEvent(true)`, hover filter suppresses finger gestures                                        | Flutter only: finger drawing pref, auto-off when a stylus is seen (`canvas_gesture_detector.dart:446-451`) [code]; SF reads only the pen |
| full-refresh policy                 | counted GC every 10th page action / menu click, Refresh button                                            | none reaching the panel [inferred]                                                                                                       |
| session across background           | kept (focus loss → pause)                                                                                 | closed on `onStop`, reopened on `resumed` [code]                                                                                         |
| update-mode ownership               | own view modes (5, REGAL one-shots), transient 2308                                                       | none; SF/EinkWise default for a buffer layer [inferred]                                                                                  |

### 7.1 Predicted driver-log behaviour

From 07 §4–§6. "Entry A live" means the activity window got `topScrollBounds` (§5.8 item 5).

**Writing, then stop** [inferred]:
`HandlePenTrigger …`, `### EUS, s 3 c 0` at the first sample; nothing during the stroke; one second
after the last lift: `Post enabled!`, `### EUS, s 2 c 0`, `Pid N State change to: toPause`
(`Control state: toPause -> paused`), `Pid N State change to: running`. Then Flutter frames in the
default waveform.

**Erase with the eraser tool** [inferred]:

- At tool selection: `Post enabled!` + `### EUS, s 2 c 0` if SF was still in schema 7, then
  `State change to: toPause`.
- During the gesture the SF session is paused while the pen is on the glass. SF itself does nothing
  (no scheme path from pen state, 07 §1 item 1). Flutter frames update the erased strokes in the
  default waveform, one per move batch.
- Entry A not live (expected): no `EUS s 4`, no delayed full update. The erase shows as a series of
  ordinary partial updates.
- Entry A live: the stylus is not exempt (pen state is 3 at ACTION_DOWN), so a mostly vertical erase
  longer than the touch slop gives `beforeScroll` → `APPLY_TRANSIENT_UPDATE(2308)` →
  `### EUS, s 4 c 0` (A2 frames during the erase), and after lift + `scroll_refresh_delay_v2` (≥ 1 s):
  `### EUS, s 2 c 0` and a full-screen mode-5 update, i.e. the same delayed flash Tangleaf shows.
  Horizontal erase strokes would not trigger it (Entry A needs `|dy| > |dx|`).

**Erase with the eraser end** [inferred]: as the eraser tool, with the pause sent at hover. Same
Entry A condition.

**Lasso and selection drag** [inferred]: the session is running (pen state 2), so Entry A exempts the
stylus; no scheme 4. Expected: `HandlePenTrigger`, `EUS s 3` at the first lasso sample; SF draws the
solid line; 1 s after lift the FR lines as in "writing". No transient, no full-screen update. A finger
drag of a selection (finger drawing on) is a finger gesture: Entry A would apply to it if live.

**Finger scroll / zoom**: no pen-state lines except `stop`/`start`/`running` from page views being
disposed and created at the viewport edges [code + inferred]. If Entry A is live, every vertical
finger scroll gives `EUS s 4` and, after lift, `EUS s 2` plus a full-screen update.

## 8. Compared with Tangleaf

Saber is the reference app closest to Tangleaf's architecture: the UI and the durable ink are drawn
by a non-native toolkit (Flutter / a WebView), and the Onyx SDK is attached to one Android view that
the toolkit sits on or next to. The comparison below is observations only.

**Where the two differ structurally**

1. Tangleaf's `TouchHelper` host is the WebView that actually shows the content (08 §1 item 1), so
   `View.setDefaultUpdateMode`, the `HAND_WRITING_REPAINT_MODE` reconcile invalidate and the mode-98
   GC all land on the visible layer (08 §3). Saber's host is an invisible dummy `SurfaceView` inside
   a Flutter virtual display; the visible content is a separate `FlutterSurfaceView` buffer layer with
   no Onyx view modes at all. Every View-level Onyx call Saber makes misses the content [inferred].
   A Flutter app that wanted Tangleaf-style per-frame modes would need the SDK host in the real
   window (hybrid composition, or a plain native view in the activity), which this plugin does not do.
2. Tangleaf computes the limit from the sheet geometry in CSS, converts to screen px and re-pushes it
   on scroll (08 §1 item 6, §5.11). Saber pushes it once per layout, at the virtual-display origin,
   page-sized; SF's pen region is effectively the whole screen and never follows scrolling [inferred].
3. Tangleaf dedups `configure` by argument JSON (08 §1 item 6). Saber repeats five SF transactions and
   an invalidate on every Flutter rebuild, including every pan/zoom frame [code].
4. Tangleaf keeps one session per page and closes it on undo, page switch, leave and background
   (08 §1 item 7). Saber keeps sessions across undo and tool changes, but runs one helper per visible
   page against global SF state, so viewport changes send STOP (pen 0) while another helper still
   believes it is drawing [code + inferred].

**Erase and the flash**

5. Both erase in the app layer with SF's session paused and the SF eraser flag off (08 §5.4–5.5).
   Tangleaf pauses at eraser pen-down after a main-thread hop; Saber pauses at tool selection or at
   eraser-end hover, i.e. before contact [code]. Both therefore have pen input into a paused session,
   the precondition for the scheme-4 → full-update sequence.
6. The difference is which framework entry could fire. Tangleaf has Entry B (WebView, no pen-state
   check) and probably Entry A (a WebView is a scroll-capture candidate) (07 §6). Saber has no Entry B
   and probably no Entry A (FlutterView gives no scroll-capture result) [inferred]. If an observation
   shows Saber erasing vertically with no `EUS s 4`, that supports "the flash comes from the framework
   scroll path, not from pausing"; if Saber does flash, Entry A is live for a plain non-Onyx window
   and the WebView is not needed for it. Saber is a useful control for that question.

**Reconcile after writing**

7. Tangleaf marks a whole-WebView frame with `0x80002` after every stroke (08 §1 item 5), the one
   kind of frame SF commits during schema 7. Saber never produces such a frame: it lets SF drop all
   its frames while SF owns the ink and hands the panel back with one pause/resume one second after
   writing stops (debounced by the next pen-down) [code]. The technique worth studying is that
   debounce: during a burst of strokes there is no per-stroke transaction at all, and SF's ink is the
   display until the pause. Its cost in Saber is that anything else Flutter draws in that second
   (menus, scrolling, undo) is also invisible until the pause [inferred].
8. Saber's hand-back is a full pause (pen 3) followed immediately by resume (pen 2). Tangleaf's
   render toggles look the same in SF terms (08 §3). Neither sends stock's resume re-push
   (limit, exclude, stroke parameters) [code].

**Other observations**

9. Neither app sends `ENABLE_COLOR_CU`, turbo, dither or EinkWise writes; Tangleaf does apply
   transient 2308 for a selection drag (08 §1 item 10), Saber applies nothing [code].
10. Tangleaf swallows pen-downs on DOM overlays and suppresses ink for text focus and the IME (08 §2);
    Saber has no overlay, dialog, IME or focus handling at all, and its SF ink covers the whole UI
    [code + inferred]. The earlier `~/tmp_zfs/reference_notes_apps/REPORT.md` (see §9) put
    Saber's "ink over dialogs" down to the lack of excludes; the virtual-display limit makes it
    whole-screen regardless.
11. Saber's full-refresh policy is empty; Tangleaf issues GC 300 ms after session close, IME hide and
    overlay close (08 §1 item 8). Saber's panel cleanliness therefore depends entirely on SF's and
    EinkWise's defaults for a non-Onyx app [inferred].

## 9. Check of the older `REPORT.md` entries on Saber

`~/tmp_zfs/reference_notes_apps/REPORT.md` (lines 10, 28, 60, 63) against the code:

- `TouchHelper.create(view, callback)` at `OnyxsdkPenArea.kt:131`, limit only from the layout listener
  at `:243-251` with a `false`/`true` re-arm: correct [code]. Missed: the constructor's empty limit
  push, and that the limit is in virtual-display coordinates [code + inferred].
- "Always empty" excludes: correct; the SDK sends nothing for an empty list, so SF's table is left as
  it was, not cleared [code].
- "Text focus handled purely by tool, `canvas.dart:66-67`": correct, via the `default` branch [code].
  The title field (`editor.dart:1650-1663`) changes no tool: correct [code].
- "Finger-drawing pref enforced only in Dart": correct; `SFTouchRender.enableFingerTouch` is empty
  (`pen/touch/SFTouchRender.java:342`) [code].
- "Ink paints over dialogs": consistent with the code, for the reason in §8 item 10 [inferred].
- Not in the report: the Virtual Display hosting, the per-rebuild `updateStroke` storm, the shared
  channel and leaked observers, the per-page helpers sharing global SF state, the 1 s debounced
  hand-back, and that the GC invalidates never reach the visible content.

## 10. Open questions for an observation run

1. Does Saber's window get `topScrollBounds` (`OnyxScrollRefreshManager checkHasScrollCallback` or
   `hadChildInWhiteList` in logcat)? That decides the erase prediction in §7.1.
2. What waveform SF uses for Saber's `FlutterSurfaceView` updates outside schema 7 (default AUTO, or
   the EinkWise app mode), and whether frames during schema 7 are really dropped.
3. Where SF ink lands relative to the pen tip and whether it covers the toolbar and IME (confirms the
   virtual-display limit).
4. Whether the mode-98 invalidate of the dummy view produces any panel update, and whether it throws
   on the `Timer` thread.
5. The SF pen state after a page scrolls out while another page stays visible (expect `stop` with no
   later `running` until an FR or tool change).
