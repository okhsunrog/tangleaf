# Mokke Ink Notes: pen and panel path, in the stock documents' terms

Written 2026-09-25 from a static reading of `~/tmp_zfs/reference_notes_apps/mokke-ink-notes`
(commit `37ce43a`, 2026-05-05; vendored `third_party/inksdk` at upstream `33c71c9`,
`third_party/inksdk/UPSTREAM.md`), the decompiled Onyx SDKs and the Onyx framework. No device was
touched and nothing was run. The purpose is comparison, action by action, with the stock Notes
documents in `../stock-notes/` (mainly [00](../stock-notes/00-overview.md) and
[07](../stock-notes/07-firmware-state-machine.md)) and with Tangleaf
([08](../stock-notes/08-tangleaf-current-path.md)). No fixes for Tangleaf are proposed.

## 0. Notation and sources

Tags: **[code]** read in the cited source; **[inferred]** reasoned from code that was read;
**[doc]** taken from a stock-notes document and not re-derived here; **[unknown]** not settled
statically.

| shorthand | root                                                                                      |
| --------- | ----------------------------------------------------------------------------------------- |
| `mk/`     | `mokke-ink-notes/app/src/main/java/com/writer/`                                           |
| `isdk/`   | `mokke-ink-notes/third_party/inksdk/inkcontroller/src/main/java/com/inksdk/ink/`          |
| `idoc/`   | `mokke-ink-notes/third_party/inksdk/docs/`                                                |
| `pen/`    | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-pen-1.5.4.3/sources/com/onyx/android/sdk/pen/` |
| `dev/`    | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-device-1.3.5.2/sources/com/onyx/android/sdk/`  |
| `base/`   | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-base-1.8.5.4/sources/com/onyx/android/sdk/`    |
| `fw/`     | `~/tmp_zfs/onyx_framework/out/sources/android/onyx/`                                      |

### 0.1 SDK version

- The app gets the SDK only through the vendored `inksdk` module (`app/build.gradle.kts:208-212`),
  which declares `com.onyx.android.sdk:onyxsdk-pen:1.5.2` and `onyxsdk-device:1.3.3` as `api`
  dependencies, plus `hiddenapibypass:4.3`
  (`third_party/inksdk/inkcontroller/build.gradle.kts:71-72`) [code]. The project docs say the
  same (`docs/design/engineering.md:67, 862`) [code].
- Neither 1.5.2 nor 1.3.3 is in the local Gradle cache or in `onyx_sdk_decompiled`, and the Onyx
  Maven repository was unreachable from the sandbox. Their POMs, and therefore which
  `onyxsdk-base` / `onyxsdk-device` Gradle finally resolves (1.5.2's transitive base may pull a newer
  device than 1.3.3), are **[unknown]**. Stock uses its own bundled SDK; Tangleaf uses
  `onyxsdk-pen 1.5.4.3` / `device 1.3.5.2` (08 §0).
- Everything below reads the call chain in **pen 1.5.4.3 / device 1.3.5.2**, the nearest decompiled
  versions. Every method mokke calls exists there with the semantics described in 07 §3 and 08 §3.
  That 1.5.2 behaves the same for these calls is **[inferred]**, not verified.
- Mokke also bundles its own AIDL stubs for Onyx's handwriting-recognition service
  (`mk/../com/onyx/android/sdk/hwr/service/*`, service package `com.onyx.android.ksync`,
  `mk/recognition/OnyxHwrTextRecognizer.kt:43-44`). That path is recognition only and makes no
  panel or pen-session call [code].

### 0.2 Package and window facts that decide framework paths

- `applicationId = "com.writer"` (`.dev` suffix in debug) (`app/build.gradle.kts:18, 33`) [code].
  It does not contain `"com.onyx"`, so the framework's `OnyxScrollRefreshManager` (Entry A in
  07 §6) is **not** skipped for mokke, as for Tangleaf and unlike stock [code for the name; gate
  per 07 §6].
- The canvas is a `SurfaceView` (`mk/view/HandwritingCanvasView.kt:49-53`), not a `WebView`, so
  the `WebView.onTouchEvent → applyFastMode` entry (Entry B in 07 §6) cannot fire [code].
- The writing window has no `ScrollView`/`RecyclerView`/`WebView` in its main layout
  (`res/layout/activity_writing.xml`); the only `ScrollView` in that window is inside the
  version-history overlay, `GONE` until history mode (`res/layout/view_version_history_overlay.xml:89`,
  `activity_writing.xml` `historyOverlay`) [code]. Whether `ViewRootImpl` ever sets
  `topScrollBounds` for this window (the precondition of Entry A) is **[unknown]**; with no
  scroll-capture candidate in the normal writing layout it is **[inferred]** to stay null.

### 0.3 Device build

The target is the Note Air 4C on firmware 4.2 (`ro.build.display.id =
2026-04-28_17-50_4.2-rel_04282_555977efe`): system partition Android 13 (SDK 33), vendor
partition Android 11 (SDK 30), `vendor.onyx.tablet=true`. The device's `surfaceflinger` and
`framework.jar` are byte-identical to the extracted copies these documents read, so the SF and
framework behaviour cited from 07 applies to this device (verified on the tablet by the
coordinator, 2026-09-25). Mokke has no branch on firmware or Android version beyond `minSdk 29`
(`app/build.gradle.kts:19`) [code].

## 1. Summary

1. **One TouchHelper per canvas, SF render only.** `OnyxInkController.attach` calls the
   two-argument `TouchHelper.create(view, cb)` on the `HandwritingCanvasView` itself
   (`isdk/OnyxInkController.kt:70`). In the SDK that is feature mask `hasStylus ? 2 : 1`, touch
   listener on (`pen/TouchHelper.java:45-47`); `hasStylus` matches input-device names containing
   `onyx_emp`, `Wacom I2C Digitizer` or `hanvon_tp` (`base/utils/DeviceFeatureUtil.java:20, 54-76`)
   [code]. On the Note Air 4C that gives mask 2, **`SFTouchRender` only** [inferred: device name
   not read]. Stock and Tangleaf both use mask 3 (SF + App render), so mokke never emits the
   `AppTouchRender` extras: no `ENABLE_POST(0)`, no duplicate pen state 1/0 (07 §3) [code].
2. **One lever: `setRawDrawingEnabled(b)`.** Every pause and resume in the app is
   `HandwritingCanvasView.pauseRawDrawing/resumeRawDrawing` → `OnyxInkController.setEnabled(b)` →
   `TouchHelper.setRawDrawingEnabled(b)` (`mk/view/HandwritingCanvasView.kt:2289-2296`,
   `isdk/OnyxInkController.kt:107-111`) [code]. That call switches render **and** input and always
   ends with `resetPenDefaultRawDrawing()` (`pen/TouchHelper.java:170-178, 348-351`) [code]. SF sees:
   - pause: `ENABLE_POST(1)`, pen state **3**, `SET_BRUSH_RAW_DRAWING_ENABLED(1)`,
     `SET_ERASER_RAW_DRAWING_ENABLED(0, 5)`;
   - resume: pen state **2**, brush 1, eraser (0, 5)
     (`pen/touch/SFTouchRender.java:195-203, 278-284`, `pen/EpdPenManager.java:39-45`) [code].
     There is no delay and no re-push of limit, width or style on resume [code].
3. **Pause and resume are nearly always back to back.** Most call sites run
   `pause(); mutate model; drawToSurface(); resume()` synchronously on the main thread, and
   `drawToSurface()` only schedules a Choreographer callback
   (`mk/view/HandwritingCanvasView.kt:1721-1728`). The SurfaceView frame is therefore drawn **after**
   the resume, while SF is at pen state 2 but out of schema 7 [code for the order; SF effect
   inferred from 07 §4.2-§4.5]. The pause's real job is the hand-back: pen state 3 → `stopHandwriting`
   → `Post enabled!`, `EUS s 2` (07 §4.3). The inksdk authors describe exactly this as a black box:
   "while TouchHelper has raw drawing enabled, nothing else is allowed to refresh the panel … pause
   first, refresh, re-enable" (`idoc/eink-pen-architectures.md:241-256`) [code].
4. **No eraser, no lasso, no firmware erase trace.** There is no eraser tool, no eraser mode and no
   selection or lasso. Erasing is by gestures recognised **after pen-up** from ordinary ink strokes:
   scratch-out (`mk/view/HandwritingCanvasView.kt:1410-1447`), strikethrough, and diagram scribble-delete
   (`mk/ui/writing/GestureHandler.kt:198-256`). The gesture itself is inked by SF as a normal black
   pencil stroke in a running session; the pause comes only after the lift [code]. The stylus eraser end
   does nothing: SF selects the no-op reader 3 (eraser raw drawing is always reset to off), the app's
   erasing callbacks are empty (`isdk/OnyxInkController.kt:247-250`) and the eraser-tool MotionEvents are
   swallowed (`mk/view/HandwritingCanvasView.kt:740`) [code; SF reader choice per 07 §4.2].
5. **"Forced refresh" is a second pause/resume, not a refresh request.** After a model mutation
   (erase gesture, shape snap, diagram change) `commitMutationImmediate` coalesces to one frame
   callback that rebuilds the bitmap region and posts a whole-surface SurfaceView frame, then one
   frame later `syncOverlay(force=true)` → `setRawDrawingEnabled(false)` + `(true)`
   (`mk/view/HandwritingCanvasView.kt:1458-1462, 1488-1504, 1586-1607, 1620-1643`;
   `isdk/OnyxInkController.kt:113-123`) [code]. No update mode, GC, `refreshScreen`, transient mode
   or scheme call is made anywhere in the writing path (grep of `mk/`, `isdk/`) [code]. The only
   explicit update mode in the whole app is `EpdController.invalidate(view, DU)` (mode 1,
   `UI_DU_MODE`) on the recognised-name `TextView` of the rename activity
   (`mk/view/HandwritingNameInput.kt:230-245`; `dev/device/SDMDevice.java:908, 1026, 1365-1370`)
   [code].
6. **Per stroke: nothing reaches SF.** At pen-up the app paints the stroke into its bitmap and posts a
   whole-surface SurfaceView frame straight away (`mk/view/HandwritingCanvasView.kt:597-601,
1822-1856`). SF is in schema 7 at that moment, so that frame is dropped (07 §4.5) and the panel
   keeps SF's ink until the next pause [inferred]. No `HAND_WRITING_REPAINT_MODE`, no pen-up refresh
   consumer (`RawInputCallback.onPenUpRefresh` not overridden) [code].
7. **Session lifetime is short.** The session is closed (`closeRawDrawing`) and re-created on every
   `surfaceDestroyed` (background, child activity), on `onResume`, 500 ms after any surface size
   change, before the rename activity, and on every hover-based handover between the two columns in
   landscape (`mk/view/HandwritingCanvasView.kt:626-659, 2267-2286`;
   `mk/ui/writing/WritingActivity.kt:1082, 1928-1953, 2115-2123`) [code].
8. **Regions.** Limit only, never an exclude, never a region mode. `attach` pushes the limit three
   times; the last write wins and comes from `EpdController.setScreenHandWritingRegionLimit(view)`,
   i.e. `(0, 0, view.getRight(), view.getBottom())` in **parent** coordinates, shifted by the view's
   screen origin (`isdk/OnyxInkController.kt:74, 79-80`; `dev/device/SDMDevice.java:1837-1842`;
   `fw/ViewUpdateHelper.java:1224-1238`) [code]. `setLimitRect(limit, emptyList())` never writes the
   exclude table because the SDK returns early on an empty list (`pen/RawInputReader.java:418-421`)
   [code], so **SF keeps whatever exclude table the previous pen app left** (stock never clears its
   own, 00 §2.1) [inferred].
9. **No transient A2, no fast mode, no EinkWise, no colour CU, no GC.** Mokke never calls
   `applyTransientUpdate`, `setEpdTurbo`, `clearTransientUpdate`, `applyGCOnce`,
   `setViewDefaultUpdateMode`, `ENABLE_COLOR_CU`, or any EAC/EinkWise API (grep) [code]. Scheme 4 can
   only come from the framework's Entry A (07 §6), and only if `topScrollBounds` is set (§0.2).

## 2. Central classes and entry points

| class / file                          | role                                                                        | key entry points                                                                                                                                                                                                                                                                                                                                                                      |
| ------------------------------------- | --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `isdk/OnyxInkController.kt`           | the only SDK user on the canvas                                             | `attach` :67-93, `setEnabled` :107-111, `syncOverlay` :113-123, `detach` :125-135, raw callbacks :191-250                                                                                                                                                                                                                                                                             |
| `isdk/InkControllerFactory.kt`        | Bigme if manufacturer/brand is Bigme, else Onyx                             | `create` :21-23                                                                                                                                                                                                                                                                                                                                                                       |
| `mk/view/HandwritingCanvasView.kt`    | SurfaceView canvas, bitmap cache, finger scroll, stroke post-processing     | surface callbacks :611-659, `tryInitOnyx` :674-685, `onHoverEvent` :689-707, `onTouchEvent` :711-787, `handleFingerTouch` :793-942, `finishStroke` :1011-1112, `flushAndRedraw` :1151-1165, scratch-out :1410-1447, refresh plumbing :1452-1643, `drawToSurface` :1721, `nudgeEpdRefresh` :1743, `composeSurface` :1836-1856, session wrappers :2267-2296, `removeStrokes` :2299-2322 |
| `mk/ui/writing/WritingActivity.kt`    | activity: menus, dialogs, lifecycle, column handover                        | see §5                                                                                                                                                                                                                                                                                                                                                                                |
| `mk/ui/writing/WritingCoordinator.kt` | model, undo/redo, scratch-out routing, text-block replacement               | `pauseAndRedraw` :120-125, `onRecognitionComplete` :135-138, `onStrokeCompleted` :239-258, `undo`/`redo` :575-600, text-block paths :755-759, :801-805                                                                                                                                                                                                                                |
| `mk/ui/writing/DisplayManager.kt`     | tap-to-scroll animation, deferred text refresh                              | `scrollToLine` :94-107, `animateScroll` :109-139, `scheduleTextRefresh` :143-151                                                                                                                                                                                                                                                                                                      |
| `mk/ui/writing/GestureHandler.kt`     | strikethrough, scribble delete, underline                                   | `tryHandle` :70, `refreshCanvas` :262-267                                                                                                                                                                                                                                                                                                                                             |
| `mk/view/TouchFilter.kt`              | finger palm filter                                                          | `evaluateDown` :55-86, `evaluateMove`                                                                                                                                                                                                                                                                                                                                                 |
| `mk/view/HandwritingNameInput.kt`     | second, independent TouchHelper in `SaveAsActivity` (rename by handwriting) | `tryInitOnyx` :182-207, `scheduleRefresh` :221-246, `closeRawDrawing` :255-264                                                                                                                                                                                                                                                                                                        |

## 3. The primitives, traced to SurfaceFlinger

Transaction names as in 07 §3 and 08 §3; SF meaning per 07 §4-§5 [doc].

| mokke call (site)                                                                                          | SDK chain                                                                                                                                                                                                                                        | SF transactions, in order                                                                                            | SF-visible effect                                                                                                                                                                                                                                                                                                                                                                                                               |
| ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `TouchHelper.create(view, cb)` (`isdk/OnyxInkController.kt:70`)                                            | ctor mask 2 → `SFTouchRender`; `bindHostView` → `RawInputManager.setHostView` → `setLimitRect(view.getLocalVisibleRect())` (`pen/TouchHelper.java:358-375`, `pen/touch/SFTouchRender.java:216-222, 396-399`, `pen/RawInputManager.java:141-147`) | `SET_SCREEN_HANDWRITING_REGION_LIMIT` = local visible rect + screen origin                                           | limit = canvas [code]                                                                                                                                                                                                                                                                                                                                                                                                           |
| `setStrokeWidth(5f)` (:71)                                                                                 | `RawInputReader.setStrokeWidth` → native + `EpdController.setStrokeWidth` (`pen/RawInputReader.java:359-364`)                                                                                                                                    | `SET_STROKE_WIDTH`(5)                                                                                                | 5 px; the app never calls `setStrokeStyle(width, color)` on the main canvas, so it stays 5 px (`isdk/InkDefaults.kt:10`, grep) [code]                                                                                                                                                                                                                                                                                           |
| `setStrokeStyle(STROKE_STYLE_PENCIL = 0)` (:72)                                                            | `EpdPenManager.setStrokeStyle`                                                                                                                                                                                                                   | `SET_STROKE_STYLE`(0)                                                                                                | pencil [code]                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `setStrokeColor(BLACK)` (:73)                                                                              | `EpdController.setStrokeColor` + reader colour (`pen/touch/SFTouchRender.java:239-242`)                                                                                                                                                          | `SET_STROKE_COLOR`                                                                                                   | black [code]                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `setLimitRect(limit, emptyList())` (:74)                                                                   | `RawInputManager.setLimitRect` → reader limit (native + SF) then `setExcludeRect(empty)` → early return (`pen/RawInputManager.java:149-153`, `pen/RawInputReader.java:409-424`)                                                                  | `SET_SCREEN_HANDWRITING_REGION_LIMIT` = same rect                                                                    | exclude table untouched [code]                                                                                                                                                                                                                                                                                                                                                                                                  |
| `openRawDrawing()` (:75)                                                                                   | cached flags cleared; `SFTouchRender.openDrawing`: style 0, reader start (in-process), `startDrawing` (`pen/TouchHelper.java:152-160`, `SFTouchRender.java:265-268, 176-179`)                                                                    | `SET_STROKE_STYLE`(0), pen state **1**                                                                               | reader started, no inking in state 1 (07 §3) [doc]                                                                                                                                                                                                                                                                                                                                                                              |
| `setRawDrawingEnabled(true)` (:76, and every resume)                                                       | render on (if cached false) → `resumeDrawing`; input on (in-process); `resetPenDefaultRawDrawing` (`pen/TouchHelper.java:170-198, 348-351`)                                                                                                      | pen state **2** (only if render was off), `SET_BRUSH_RAW_DRAWING_ENABLED`(1), `SET_ERASER_RAW_DRAWING_ENABLED`(0, 5) | running; tip → reader 2 (draws), eraser end → reader 3 (no-op) [code + 07 §4.2]                                                                                                                                                                                                                                                                                                                                                 |
| `EpdController.setScreenHandWritingRegionLimit(view)` (:79-80)                                             | `SDMDevice`: `(0, 0, view.getRight(), view.getBottom())` → framework adds `getLocationOnScreen` (`dev/device/SDMDevice.java:1837-1842, 3333, 3391-3395`; `fw/ViewUpdateHelper.java:1224-1238`)                                                   | `SET_SCREEN_HANDWRITING_REGION_LIMIT` = `(sx, sy, sx + right, sy + bottom)`                                          | replaces the local-visible-rect limit. `right`/`bottom` are parent-relative, so the SF limit extends past the canvas by the canvas's own `left`/`top` offset in its parent (the recognised-text view sits above it, `activity_writing.xml`); since the canvas reaches the screen bottom, the excess is off-screen [code for the arithmetic; visible effect inferred]. The in-process reader keeps the local visible rect [code] |
| `setRawDrawingEnabled(false)` (every pause; `detach` :128)                                                 | render off → `leaveScribbleMode` + `pauseDrawing`; input off (in-process `nativePausePen`); reset (`SFTouchRender.java:195-198`)                                                                                                                 | `ENABLE_POST(1)`, pen state **3**, brush 1, eraser (0, 5)                                                            | reader paused; leaves schema 7 if active (`Post enabled!`, `EUS s 2` after a 150 ms sleep on SF's binder thread, 07 §4.3, §5.1). If render was already off: only brush 1, eraser (0, 5) [code]                                                                                                                                                                                                                                  |
| `setRawInputReaderEnable(false)` (`detach` :129)                                                           | guarded, already false after the line above → no-op (`pen/TouchHelper.java:213-223`)                                                                                                                                                             | none                                                                                                                 | [code]                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `closeRawDrawing()` (`detach` :130)                                                                        | style 0; `leaveScribbleMode`, reader pause, `pauseDrawing`; reader quit, `quitDrawing` (`SFTouchRender.java:182-192, 271-275`)                                                                                                                   | `SET_STROKE_STYLE`(0), `ENABLE_POST(1)`, pen state **3**, pen state **0**                                            | STOP; limit left in SF [code]                                                                                                                                                                                                                                                                                                                                                                                                   |
| `syncOverlay(bmp, region, force=false)`                                                                    | early return (`isdk/OnyxInkController.kt:118`)                                                                                                                                                                                                   | none                                                                                                                 | [code]                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `syncOverlay(…, force=true)`                                                                               | `setRawDrawingEnabled(false)` then `(true)` (:119-121)                                                                                                                                                                                           | `ENABLE_POST(1)`, pen **3**, brush 1, eraser (0,5), pen **2**, brush 1, eraser (0,5)                                 | a hand-back with no frame of its own [code]                                                                                                                                                                                                                                                                                                                                                                                     |
| `holder.lockCanvas()/unlockCanvasAndPost` (`composeSurface`, `mk/view/HandwritingCanvasView.kt:1836-1856`) | plain Android Surface post; no Onyx call                                                                                                                                                                                                         | none                                                                                                                 | a SurfaceView buffer; which update mode SF assigns that layer is **[unknown]** (no `setViewDefaultUpdateMode`, not a `View.invalidate` path). Committed only outside schema 7 (07 §4.5) [inferred]                                                                                                                                                                                                                              |
| `EpdController.invalidate(nameDisplay, DU)` (rename activity only)                                         | `View.invalidate(1)` (`dev/device/SDMDevice.java:1365-1370`)                                                                                                                                                                                     | via the next draw of that TextView                                                                                   | mode 1 (`UI_DU_MODE`) update entry for that view [code]                                                                                                                                                                                                                                                                                                                                                                         |

Pen-state 4, `ENABLE_POST(0)`, region mode, exclude, `ENABLE_COLOR_CU`, stroke parameters, side-button
erase, transient/app-scope updates and every scheme or GC call are never sent (grep) [code].

## 4. Session state

App-side state is just `OnyxInkController.isActive` plus the SDK's cached render/input flags, which
`setRawDrawingEnabled` keeps equal. In the stock-notes state letters (00 §2.1) mokke is only ever in:

| state | pen state | render | input | when                                                                                                                    |
| ----- | --------- | ------ | ----- | ----------------------------------------------------------------------------------------------------------------------- |
| U     | 0 / none  | –      | –     | detached: before first attach, after `surfaceDestroyed`, rename, column handover, between detach and attach of a reinit |
| D     | 2         | on     | on    | attached and not paused: the normal writing state                                                                       |
| P     | 3         | off    | off   | any pause (menus, dialogs, finger scroll, space-insert mode, tutorial, history mode, animated scroll)                   |

There is no R (input without render) and no O (START without resume): `attach` goes 1 → 2
immediately [code]. SF-side flags are constant for the life of the app: brush raw drawing 1, eraser
raw drawing (0, 5), style 0, width 5, black [code].

## 5. The mechanism, action by action

Sequences are in emission order. "SF:" lists transactions. "PR" = pause
(`ENABLE_POST(1)`, pen 3, brush 1, eraser (0,5)); "RS" = resume (pen 2, brush 1, eraser (0,5)); "PR+RS" =
both back to back on the main thread; "FRAME" = whole-surface SurfaceView post on the next vsync
(`rebuildComposeCallback`, `mk/view/HandwritingCanvasView.kt:181-187`); "ATTACH" and "DETACH" are the
§3 sequences:

- ATTACH: limit (local rect), width 5, style 0, colour, limit, style 0, pen **1**, pen **2**, brush 1,
  eraser (0,5), limit (view overload).
- DETACH: `ENABLE_POST(1)`, pen **3**, brush 1, eraser (0,5), style 0, `ENABLE_POST(1)`, pen **3**,
  pen **0**.

### 5.1 Open a note (cold start of `WritingActivity`)

1. `onCreate`: layout, full-screen system-UI flags (`WindowInsetsController` and
   `DeviceUtils.setFullScreenOnResume`, a status-bar intent, no panel call)
   (`mk/ui/writing/WritingActivity.kt:297-313`; `base/utils/DeviceUtils.java:255-267`) [code].
2. `onResume` → `reinitializeRawDrawing()`: `detach` is a no-op (inactive), `surfaceReady` is false,
   nothing sent (`WritingActivity.kt:2115-2123`, `HandwritingCanvasView.kt:2280-2286`) [code].
3. `surfaceCreated` → `drawToSurface()` (scheduled) → `tryInitOnyx()` → ATTACH → state D
   (`HandwritingCanvasView.kt:611-615, 674-685`) [code]. `refreshOverlay(force=false)` is a no-op.
4. `surfaceChanged` (first call: size differs from 0×0) → `drawToSurface()` and, because the
   controller is active, `reinitializeRawDrawing` 500 ms later (`:626-638`) [code] → DETACH + ATTACH
   - FRAME. The activity then sets the text/canvas heights (`WritingActivity.kt:572-581`), which
     produces another size change and another debounced reinit [inferred ordering].
5. First launch only: the tutorial pauses (PR) (`WritingActivity.kt:588-590`) [code].
6. No GC, no update mode. The first FRAMEs are committed normally (no pen sample yet, so no
   schema 7) [inferred].

Switching documents from the Open dialog: the dialog's dismiss resumes (RS), then `loadStrokes` →
FRAME (`WritingActivity.kt:900-906, 778-835`; `HandwritingCanvasView.kt:2366-2375`) [code]. No
session close.

### 5.2 Write (pen tip)

1. Down: SF reader 2 inks; first sample since the last pause → `HandlePenTrigger`, schema 7,
   `EUS s 3 c 0` (07 §4.2) [doc]. In-process: the SDK reader posts `onBeginRawDrawing` to the main
   thread (`pen/touch/SFTouchRender.java:40-47`) → `beginStroke` (`HandwritingCanvasView.kt:510-520,
553-556`): `touchFilter.penActive = true`, dwell timer (500 ms). No transaction [code].
2. Moves: model points only (`:558-560`). If the pen dwells still for 500 ms, `drawToSurface()`
   paints a dwell dot → FRAME in schema 7 → dropped [code for the call; drop inferred from 07 §4.5].
3. Up: `onEndRawDrawing` → `endStroke` → `finishStroke` → classify
   (`:562-606, 1011-1112`). In the common case: `appendLastStrokeToBitmap()` + `composeSurface()`
   **synchronously** → whole-surface post while SF is in schema 7 → dropped [code for the post;
   drop inferred]. SF: no transaction.
4. Model observers: `WritingCoordinator.onStrokeCompleted` → `saveSnapshot` →
   `updateUndoRedoButtons()`. Only when the undo/redo alpha actually changes (e.g. the first stroke
   after load or after the undo stack emptied): PR+RS, then `nudgeEpdRefresh()` → compose-only
   FRAME on the next vsync, committed outside schema 7 (`WritingActivity.kt:757-772`,
   `HandwritingCanvasView.kt:1743-1749`) [code].
5. 500 ms after the last point the SDK's pen-up timer fires; `onPenUpRefresh` is the SDK's empty
   default (`pen/RawInputReader.java:278-283`, `pen/RawInputCallback.java:28`) [code].
6. Recognition of a line whose text is shown (a line scrolled above the canvas) ends in
   `scheduleTextRefresh()`: 500 ms later PR, `drawToSurface()`, RS → FRAME
   (`WritingCoordinator.kt:135-138`, `DisplayManager.kt:143-151`) [code]. A new stroke cancels it
   (`WritingCoordinator.kt:241`) [code]. Eager recognition of the line being written does not
   trigger it (`LineRecognitionManager.kt:54-66`) [code].
7. Shape snap or auto-diagram (`drawingScore ≥ 0.5`): `flushAndRedraw()` = PR, `drawToSurface()`,
   RS (`HandwritingCanvasView.kt:1072-1110, 1151-1165`), plus `commitMutationImmediate` from
   `onStrokeEnd` (`:570-596`) → next vsync region rebuild + FRAME, next vsync `syncOverlay(force)` =
   PR+RS [code].

Resulting state: pen state 2 throughout; SF stays in schema 7 and scheme 3 across strokes until the
next pause; SF's ink is the only thing on the panel for those strokes [inferred]. Pressure: SF
pencil style 0 with a fixed width; the app paints its bitmap with a constant-width `Paint`
(`HandwritingCanvasView.kt:2260-2264`, `CanvasTheme.kt:17-22`, `ScreenMetrics.kt:47, 153`, 2.6 dp)
[code]; whether SF's pencil uses pressure is **[unknown]**.

### 5.3 Erase

There is **no eraser tool and no eraser mode** (no stroke/partial/area eraser, no soft eraser)
[code: grep of `mk/`]. Erasing is a post-hoc gesture:

| gesture                                        | detector                                 | where                                                                             |
| ---------------------------------------------- | ---------------------------------------- | --------------------------------------------------------------------------------- |
| scratch-out (zig-zag over ink or a text block) | `ScratchOutDetection`                    | `HandwritingCanvasView.kt:1410-1447` → `WritingCoordinator.onScratchOut` :321-403 |
| strikethrough (flat line through words)        | `GestureHandler.isStrikethroughGesture`  | `GestureHandler.kt:90, 232-256`                                                   |
| diagram scribble-delete                        | `GestureHandler.isScribbleDeleteGesture` | `GestureHandler.kt:173-228`                                                       |

Per scratch-out gesture:

1. Down/moves/up exactly as §5.2: SF inks the zig-zag as a black pencil stroke in state D, schema 7,
   `EUS s 3` if first since a pause [code + doc].
2. Pen-up, main thread: `checkPostStrokeScratchOut()` → PR → SF `ENABLE_POST(1)` (leaves schema 7 →
   `Post enabled!`, 150 ms sleep, `EUS s 2 c 0`), pen 3 (`State change to: toPause`), brush 1,
   eraser (0,5) → `onScratchOut` → `removeStrokes(ids)` → `commitMutationImmediate(bbox of removed
strokes + 4 dp)` (only scheduled) → RS → pen 2 (`running`), brush 1, eraser (0,5)
   (`HandwritingCanvasView.kt:1438-1440, 2299-2322`) [code; log lines per 07 §4].
3. Next vsync: `doCommitMutation` → region rebuild of the bitmap → `composeSurface()` → whole-surface
   post (no live stroke, so `lockCanvas()` without a dirty rect, `:1836-1840, 1862-1869`) → committed
   (no pen sample since the pause) [code; commit inferred].
4. Same callback: `scheduleOverlayRefreshOrDefer` → `refreshOverlay(force=false)` (no-op) and a
   forced-refresh frame callback (`:1586-1607`) [code]. If a new stroke started meanwhile, the sync
   waits for its `onStrokeEnd` (`daemonStrokeInProgress`, `:548-550, 603-606, 1612-1618`) [code].
5. Next vsync: `syncOverlay(force=true)` = PR+RS again. SF is no longer in schema 7, so the
   `ENABLE_POST(1)` finds `androidDrawing == 1` and the pen-3 pause runs `stopHandwriting` as a no-op
   (07 §4.3-§4.4); no `EUS` line [inferred].
6. The zig-zag ink itself disappears only because the committed whole-surface frame covers it
   [inferred]. A full-bitmap rebuild (text block hit, `region == null`) runs on a worker thread and
   posts the same way when it swaps in (`:1528-1580`) [code].

Strikethrough and scribble-delete use `refreshCanvas { removeStrokes }` = PR, `removeStrokes`
(schedules the mutation), `drawToSurface()`, RS; then the same next-vsync FRAME and forced PR+RS
(`GestureHandler.kt:262-267`) [code].

**Stylus eraser end** (any time): SF: eraser raw drawing is 0 → reader 3, no ink, no trigger
(07 §4.2) [doc]. In-process: the reader reports erasing, `onBeginRawErasing` etc. are empty
(`isdk/OnyxInkController.kt:247-250`) [code]. MotionEvents with `TOOL_TYPE_ERASER` reach
`onTouchEvent`, are not finger/stylus, and are swallowed by `consumesMotionEvents`
(`HandwritingCanvasView.kt:711-740`) [code]. Nothing is erased, nothing is sent, pen state stays 2.
Side-button erase is never configured; its SF flag is whatever another app left **[unknown]**.

### 5.4 Lasso / selection and dragging it

Not present: no selection model, no lasso tool, no transform (grep for lasso/selection in `mk/`
finds none) [code]. The two stylus drags that exist:

- **Space-insert mode** (gutter button): entering it = PR and FRAME; every stylus or finger
  move = FRAME in state P; leaving = FRAME + RS (`HandwritingCanvasView.kt:380-393, 949-1000`)
  [code]. This is the only mode in which the stylus drags on the canvas with the session paused
  [code].
- **Split divider drag** (text/canvas border): PR at drag start, RS at end
  (`WritingActivity.kt:684-686`); the resize then triggers the 500 ms reinit (DETACH + ATTACH)
  [code + inferred].

### 5.5 Menus, popups, dialogs, overlays

| UI                                                             | on show                                              | on dismiss                                                       | site                                                                        |
| -------------------------------------------------------------- | ---------------------------------------------------- | ---------------------------------------------------------------- | --------------------------------------------------------------------------- |
| main menu (`PopupWindow`, focusable)                           | PR                                                   | RS (unless it launches the tutorial, SAF picker or rename)       | `WritingActivity.kt:1400-1458`                                              |
| Open Document (`AlertDialog`, with an `EditText` search field) | PR                                                   | RS                                                               | `:884-906, 919-928`                                                         |
| version history (in-window overlay)                            | PR (after the menu's RS)                             | RS                                                               | `:1460-1512`                                                                |
| cue/main peek popups                                           | PR                                                   | RS                                                               | `:2001-2075` (pause :2065, resume :2067)                                    |
| tutorial overlay steps                                         | PR+RS around demo redraws; whole tutorial paused     | reinit at end                                                    | `:2330-2332, 2389`; `TutorialManager.kt:103-105, 236-250, 346-348, 402-404` |
| rename (`SaveAsActivity`)                                      | DETACH before launch                                 | ATTACH via `reopenRawDrawing` on result                          | `:1080-1087, 204`                                                           |
| settings activity                                              | menu RS, then `onStop` → `surfaceDestroyed` → DETACH | `onResume`/`surfaceCreated` ATTACH; `reopenRawDrawing` on result | `:1446-1449, 234-257`                                                       |
| SAF folder picker                                              | stays paused, then DETACH at `onStop`                | RS in the result callback                                        | `:207-231`                                                                  |
| undo/redo alpha change                                         | PR+RS, compose-only FRAME                            | –                                                                | `:757-772`                                                                  |

No exclude rects are ever set for any of these [code]. The floating gutter (menu, undo, redo, space,
mic, toggle, rotate) sits `top|end` over the whole window (`activity_writing.xml:103-191`); whether
its lower buttons overlap the canvas and thus the SF limit (with no exclude) depends on the
recognised-text height on this screen **[unknown]**. Every popup, dialog and soft keyboard appears
over a **paused** session (pen state 3), so SF does not ink over them [code for the pause;
consequence per 07 §4.2]. Resumes are immediate, with no 500 ms delay [code].

### 5.6 Finger scroll and zoom

There is no zoom: a second pointer is rejected by `TouchFilter` (`TouchFilter.kt:77, 117`) [code].

Finger on the canvas (`HandwritingCanvasView.kt:718-721, 793-942`) [code]:

1. Down: `TouchFilter.evaluateDown` rejects while the pen hovers (`onHoverEvent` sets
   `penHovering`, `:689-707`), while the pen is down, within 500 ms of a pen-up, for a contact over
   120 dp, or for multi-touch (`TouchFilter.kt:55-86`). Accepted → **PR at finger-down**, before any
   movement (`:812`).
2. Moves past slop: `scrollOffsetY` changes, `drawToSurface()` → one FRAME per vsync in state P
   (committed; no schema 7) [code + inferred].
3. Up: snap to line, FRAME, RS, `refreshOverlay(force=false)` (no-op) (`:925-937`). A tap without
   movement is the same PR … RS pair (and may start audio playback on a text block) [code].

Scrolling by finger in the recognised-text view above the canvas moves the canvas too, **without any
pause** (`WritingActivity.kt:518-543`) [code]: the canvas FRAMEs go out at pen state 2. If the pen
wrote since the last pause, SF is still in schema 7 and those frames (and the text view's own) are
dropped until something pauses [inferred from 07 §4.5].

Stylus tap on a text line → `scrollToLine` → `animateScroll(…, 500 ms)`: PR, FRAMEs at ~30 fps,
RS after the animation (`DisplayManager.kt:94-139`) [code].

Transient A2: none from mokke. Entry A (07 §6) would add `applyTransientUpdate(2308)` → `EUS s 4`
during a finger scroll (finger is never exempt) and `clearTransientUpdate` → `EUS s 2` + a
full-screen mode-5 update about 1.1 s after lift, **only if** `topScrollBounds` is set for this
window, which §0.2 infers it is not [inferred / unknown].

### 5.7 Page switch

Mokke has no pages; a document is one vertically scrolling column (plus a cue column). The nearest
equivalents are the document switch (§5.1) and, in landscape, the **hover handover** between the
notes canvas and the cue canvas: `ACTION_HOVER_ENTER` over the canvas without a session →
`transferOnyxSession` → donor DETACH + FRAME, recipient ATTACH + FRAME, with a 300 ms cooldown
(`HandwritingCanvasView.kt:700-704, 730-733`; `WritingActivity.kt:1901-1953`) [code]. Portrait
fold/unfold between notes and cues does the same with `closeRawDrawing` + `reinitializeRawDrawing`
(`:1835-1854, 1877-1884`) [code]. Each handover is a full session close and open, and each ATTACH
rewrites the global SF limit to the new canvas [code].

### 5.8 Leave the note, background, foreground

- Back / Home / child activity: `onStop` saves (`WritingActivity.kt:2125-2146`); the SurfaceView's
  `surfaceDestroyed` → `detach()` → DETACH (`HandwritingCanvasView.kt:640-659`) [code;
  surface destruction at stop is standard SurfaceView behaviour, inferred]. `onDestroy` →
  `closeDualCanvasOnyx()` (cue DETACH if active) (`:2440-2455`) [code].
- There is no `onPause` and no `onWindowFocusChanged` handling anywhere in the writing path (grep)
  [code]. Focus loss to a system dialog, the notification shade or a non-activity window leaves
  the session in state D [code]; whether SF then inks over that window is **[unknown]** (07 §4.2
  says SF inks within the limit regardless of window order).
- Foreground: `onResume` → `reinitializeRawDrawing` (DETACH is a no-op if already detached; ATTACH
  if the surface is ready) and `surfaceCreated` → ATTACH (whichever comes first wins; the other is a
  no-op through `isActive`) [code]. A new surface may get SF's new-layer update entry
  (`getNewLayerUpdateEntry`, 07 §5.4) **[unknown]**.
- No GC or full refresh on leave or return [code].

### 5.9 Keyboard / text input

`windowSoftInputMode="stateHidden"` on all three activities (`AndroidManifest.xml:37, 42, 48`)
[code]. The canvas has no text entry; renaming is handwriting + OCR in `SaveAsActivity`, entered after
a DETACH (`WritingActivity.kt:1080-1087`) [code]. One `EditText` exists in the writing window's
reach: the search field of the Open Document dialog, shown while the session is paused (§5.5)
[code]. There is no IME inset listener and no keyboard-specific pause; the dialog's pause covers it
[code].

In `SaveAsActivity` the name canvas runs its own `TouchHelper` (same attach pattern, mask 2) and,
300 ms after each recognition or erase, does PR, a canvas post, `EpdController.invalidate(nameDisplay,
DU)` and RS (`HandwritingNameInput.kt:182-246`) [code]. Its `lockCanvas` happens between the PR and
the RS, i.e. with the session paused [code].

### 5.10 Undo / redo

Gutter button click (finger or stylus; `GutterTouchGuard` rejects palms and long holds,
`WritingActivity.kt:347-376`) → `undo()`: PR, `applySnapshot` (`loadStrokes` → `drawToSurface`),
RS; then `updateUndoRedoButtons` (a second PR+RS if the alpha changed) → FRAME on the next vsync
(`WritingCoordinator.kt:575-600`) [code]. No session close, no GC.

## 6. Tables

### 6.1 SF-visible effect per action

| action                            | pen-state traffic                                                           | frames reaching the panel                    | update mode / scheme                                         |
| --------------------------------- | --------------------------------------------------------------------------- | -------------------------------------------- | ------------------------------------------------------------ |
| open (cold)                       | ATTACH (1, 2), then DETACH (3, 3, 0) + ATTACH ~500 ms later, possibly twice | FRAMEs (committed)                           | none requested; scheme stays 2 [inferred]                    |
| write                             | none                                                                        | none per stroke (dropped in schema 7)        | `EUS s 3` on first sample after a pause                      |
| scratch-out / strikethrough       | PR+RS after lift, then PR+RS again one vsync later                          | one whole-surface FRAME                      | `Post enabled!`, `EUS s 2`; no scheme 4                      |
| eraser end                        | none                                                                        | none                                         | none                                                         |
| lasso / selection                 | n/a                                                                         | n/a                                          | n/a                                                          |
| space-insert drag (stylus)        | PR at mode entry, RS at exit                                                | FRAME per move                               | none from mokke; Entry A only if `topScrollBounds` [unknown] |
| menu / dialog                     | PR on show, RS on dismiss                                                   | whatever the popup draws                     | none                                                         |
| finger scroll / tap               | PR at finger-down, RS at lift                                               | FRAME per move                               | none from mokke; Entry A as above                            |
| text-view scroll                  | none                                                                        | canvas FRAMEs, dropped if schema 7 is active | none                                                         |
| undo / redo                       | PR+RS (×2 if the button alpha changes)                                      | FRAME                                        | none                                                         |
| column handover / resize / rename | DETACH + ATTACH                                                             | FRAME                                        | none                                                         |
| background                        | DETACH (via `surfaceDestroyed`)                                             | –                                            | none                                                         |
| foreground                        | ATTACH                                                                      | FRAME                                        | new-layer entry [unknown]                                    |

### 6.2 Refreshes mokke issues, complete list

1. SurfaceView buffer posts (`composeSurface`) after every mutation, scroll step, dwell dot and
   pen-up [code]. No update mode attached [code].
2. PR+RS hand-backs, as listed in §5 [code].
3. `EpdController.invalidate(nameDisplay, DU)` in the rename activity only [code].

There is no GC, no full refresh, no counted refresh and no Refresh button anywhere (grep for `GC`,
`refreshScreen`, `applyGCOnce`, `repaintEverything` in `mk/` and `isdk/`) [code].

## 7. Device-specific branches

- `InkControllerFactory`: Bigme by manufacturer/brand, otherwise Onyx; attach failure falls back to
  MotionEvent + Canvas drawing (`isdk/InkControllerFactory.kt:20-30`,
  `isdk/OnyxInkController.kt:87-92`) [code].
- `hasStylus` picks mask 2 vs 1 inside the SDK (§1 item 1) [code].
- `ScreenMetrics`: large screen (≥ 900 sw-dp) → canvas 70 % of the height, else 82 %; line
  spacing 50 dp / 41 dp (`mk/view/ScreenMetrics.kt:25-45`) [code]. Nothing depends on colour type,
  `vendor.onyx.*` properties or the firmware version [code].

## 8. Compared with stock

Stock references are 00 §3 and §5 [doc].

**What mokke gets**

- **No flash after erase** [inferred]. The erase gesture is written with the session running
  (pen state 2) and the pause follows the lift. The app requests no transient mode, so SF has no
  fixed-slot fast mode to switch to. The framework's Entry A exempts a stylus when the pen state is
  2 at `ACTION_DOWN` and probably has no `topScrollBounds` in this window anyway. Entry B cannot
  apply to a `SurfaceView`.
- **No per-stroke refresh**, like stock on this device (stock's per-stroke reconcile is gated off by
  `vendor.onyx.tablet`, 00 §3.2). App frames posted at pen-up are dropped inside schema 7 and SF's
  ink stays until the next hand-back [inferred].
- **Immediate resumes.** Stock waits 500 ms (colour) before every resume, 150-300 ms after erase or
  tool change, and 600 ms on open. Mokke resumes synchronously, so the pen works right after a menu
  closes [code].
- **Pen never lands in a paused session during writing or erasing.** Stock has documented paused
  windows with pen input (post-erase R window, fill colour, selection, rectangle lasso, the 500 ms
  after a menu; 00 §2.3). Mokke's only ones are space-insert mode, the 500 ms tap-to-scroll
  animation and finger scroll [code].

**What mokke loses**

- **Firmware eraser trace, soft eraser (style 8), dash eraser (style 5), erase modes.** Mokke has no
  eraser at all. The stylus eraser end is inert, and the scratch gesture shows as black ink until
  the post-lift frame replaces it [code].
- **No flash after an erase, but also no in-place erase feedback.** Stock repaints only the changed
  slices with a mode-5 invalidate after `ENABLE_POST(1)` and a render-only pause. Mokke reposts the
  whole SurfaceView and relies on SF's default treatment of that layer [code + inferred].
- **Lasso in the firmware layer, selection, transforms, fast mode for drags.** None exist [code].
- **Fast mode / transient A2 / turbo / dither.** Stock applies 2308 with turbo 5 and dither 160 for
  scroll, pinch and drags. Mokke's finger scroll repaints full SurfaceView frames in the default
  mode at pen state 3 [code]. How ghosty that looks is **[unknown]**.
- **Full-refresh policy.** Stock has a counted GC every 10th page action or menu click, a REGAL
  repaint on popup dismiss and a Refresh button. Mokke never requests a GC, a REGAL frame or any
  explicit mode. Ghosting is cleared only by whatever SF does with plain buffer posts [code for the
  absence; panel result unknown].
- **EinkWise / EAC.** Stock writes the dither threshold on every fast-mode change. Mokke never
  touches EinkWise; its per-app config is whatever the system default is [code / unknown].
- **Colour and pressure.** SF ink is black pencil at 5 px, and the app bitmap is constant-width
  black. Stock supports colour pens (`ENABLE_COLOR_CU`), several brush styles and per-pen stroke
  parameters [code].
- **Palm and finger handling.** Stock uses hover (`PenActiveEvent`, `setPostInputEvent(true)`) inside
  its own gesture engine. Mokke uses `onHoverEvent` plus a size, cooldown and multi-touch filter in
  software. It has no firmware exclude for overlays and no CTP cut-out. A finger tap or scroll on the
  canvas pauses the session at finger-down [code].
- **Session continuity.** Stock keeps one session per open document and only pauses it. Mokke closes
  and reopens on background, resume, every resize, rename and column handover, and twice on a cold
  open [code].
- **Region hygiene.** Stock re-pushes limit and exclude on every resume, with a non-empty exclude
  list. Mokke never writes the exclude table and never sets region mode. It inherits both from the
  previous pen app, stock's included, because stock never clears them on close (00 §2.1)
  [code + inferred].

**Predicted driver-log behaviour** (for the observation protocol, 10) [inferred]:

- _Scratch-out erase:_ during the gesture `HandlePenTrigger…`, `### EUS, s 3 c 0` if it is the first
  sample since a pause. Within a few ms of lift: `Pid N State change to: toPause`, `Control state:
toPause -> paused`, `Post enabled!`, `### EUS, s 2 c 0`, then `State change to: toResume` /
  `running`. One partial update of the canvas area follows. One vsync later a second
  `toPause`/`running` pair with no `EUS` line. **No `EUS s 4`, no A2 waveform, no delayed
  full-screen GC16.**
- _Eraser end:_ nothing at all.
- _Lasso:_ not applicable. For the nearest analogue, a space-insert or finger drag, the session is
  paused before the drag, so there is pen or finger input into a paused session. The scheme-4 → A2 →
  full-update sequence of 07 §6 would appear only through Entry A. That needs `topScrollBounds`,
  which §0.2 infers is unset. Expected: `toPause` at entry, plain partial updates per move,
  `running` at exit, no `EUS` line. If E1 in 10 shows `checkHasScrollCallback topScrollBounds` for
  mokke's window, expect `beforeScroll`, `EUS s 4`, and about 1.1 s after lift `EUS s 2` with one
  full-screen update.

## 9. Compared with Tangleaf

Observations only. Tangleaf facts are from 08 and 00 §4 [doc].

**Where mokke's path avoids a Tangleaf trigger**

- It never pauses during pen input to erase. Tangleaf pauses at eraser-end down and keeps render off
  for the whole eraser tool, then lets JS draw. That is the paused-session-with-pen precondition of
  the flash (08 §1 item 3). Mokke erases by gesture after the lift, so the precondition never forms
  [code].
- Its canvas is a `SurfaceView` in a window with no scroll-capture candidate. Tangleaf's `WebView` is
  exposed to Entry B unconditionally and is a likely Entry A candidate (07 §6) [inferred].
- One pause per finger gesture versus Tangleaf's pause/resume cycle per scroll event. No exclude
  reset, no `cycleScribbleSession`, no pen-state traffic at app start [code].
- Resume is immediate. Tangleaf waits 500 ms after overlays and waits for a presented reconcile frame
  after an erase (08 §5.4, §5.10) [code / doc].

**Where mokke does less than Tangleaf**

- No eraser tool and no eraser end. Tangleaf has stroke, pixel and lasso erasers with live feedback
  [code].
- No lasso, selection or drag. Tangleaf has all three, with transient A2 during a pen drag of a
  selection [code / doc].
- No full refresh at all. Tangleaf issues a GC 300 ms after close, IME hide and overlay close [doc].
- Its session is closed as often as Tangleaf's (background, resize, handover) and additionally 500 ms
  after the first layout [code].
- It inherits a stale SF exclude table and region mode. Tangleaf clears excludes at start, on resume
  and on close [code / doc].
- No handling of window focus. A system dialog over a running mokke session leaves SF inking
  **[unknown]**. Tangleaf pauses on focus loss [code / doc].

**Techniques worth studying** (no proposal implied)

1. _Pause+resume as a pure hand-back._ A back-to-back `setRawDrawingEnabled(false)`/`(true)` right
   before the app posts a frame gets that frame through: SF leaves schema 7, and the next frame
   commits because no pen sample has re-triggered it. The pen stays live with no resume delay.
   Stock's `ENABLE_POST(1)` after an erase does the same hand-back without the pen-state round trip
   (00 §3.3) [inferred].
2. _Erase as a post-lift decision._ All pen input goes through the running session and SF's own
   ink, and the app mutates the model only after the lift. The flash precondition cannot form
   [code + inferred].
3. _Deferring the overlay sync while a stroke is live_ (`daemonStrokeInProgress`, deferred region
   union, flush at `onStrokeEnd`, `HandwritingCanvasView.kt:548-550, 1586-1618`) [code].
4. _Vsync-coalesced mutation commit._ All mutations within one frame merge into one region rebuild
   and one post, with full rebuilds moved off the main thread (`:1477-1580`) [code].
5. _Hover-driven session ownership between two canvases._ The session moves at `HOVER_ENTER`
   before the pen touches, with a 300 ms cooldown (`WritingActivity.kt:1928-1953`) [code].
6. _Software palm filter keyed on hover, pen-down and a 500 ms post-lift cooldown_, used instead of
   any firmware call (`TouchFilter.kt:55-86`) [code].
7. The inksdk notes record two device claims worth checking against our model. Neither is verified
   here. First, `EpdController.invalidate(view, DU)` "hangs" while raw drawing is active. Second,
   `holder.lockCanvas()` "wedges" because TouchHelper holds the surface
   (`idoc/eink-pen-architectures.md:173-194, 247-252`). Mokke itself calls `lockCanvas` with the
   session running, at pen-up (`HandwritingCanvasView.kt:597-601`), so the wedge claim does not hold
   for mokke's mask-2 path. It may have come from a mask-1 `AppTouchRender` setup **[unknown]**.

## 10. Checks against the earlier `REPORT.md`

`~/tmp_zfs/reference_notes_apps/REPORT.md` (mokke rows) re-checked:

| REPORT claim                                                                                                                  | result                                                                                                                                                     |
| ----------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `TouchHelper.create(view, …)` on the SurfaceView, then `setScreenHandWritingRegionLimit(view)` (`OnyxInkController.kt:70-80`) | confirmed [code]                                                                                                                                           |
| limit = `getLocalVisibleRect` of the SurfaceView                                                                              | incomplete: that value is overwritten in SF by the single-argument overload, `(0,0,right,bottom)` in parent coordinates + screen origin (§3) [code]        |
| exclude "always empty"                                                                                                        | wrong in effect: mokke never writes the exclude table (the SDK drops an empty list), so SF keeps the previous app's excludes (§1 item 8) [code + inferred] |
| 500 ms-debounced detach+reattach on size change (`HandwritingCanvasView.kt:626-639`)                                          | confirmed (:626-638) [code]                                                                                                                                |
| every dialog brackets pause/resume (`WritingActivity.kt:891/905, 1401/1420, 2065/2067`)                                       | confirmed; the menu's resume is at :1422 [code]                                                                                                            |
| finger → scroll with pause at `:815`, stylus swallowed at `:740`                                                              | pause is at :812, at finger-down, and also on a plain tap; :740 confirmed [code]                                                                           |
| palm guard on buttons (`WritingActivity.kt:343-376`)                                                                          | confirmed (:347-376) [code]                                                                                                                                |
| child activity → `closeRawDrawing()`                                                                                          | explicit only for rename (:1082); settings and SAF close through `surfaceDestroyed` at `onStop` [code + inferred]                                          |
| keyboard "structurally impossible"                                                                                            | too strong: the Open Document dialog has an `EditText` (`WritingActivity.kt:919-928`). The session is paused while it is shown [code]                      |
| SDK decompiled as `onyxsdk-pen-1.4.1.aar`                                                                                     | mokke declares 1.5.2 / device 1.3.3 (§0.1) [code]                                                                                                          |

## 11. Open questions

1. What the pen 1.5.2 / device 1.3.3 POMs resolve to, and whether 1.5.2's `TouchHelper` matches
   1.5.4.3 for the calls in §3 (§0.1).
2. Whether `topScrollBounds` is ever set for mokke's writing window (E1 in 10). This decides whether
   finger scroll and space-insert drags produce scheme 4 plus a full update.
3. Which update mode and waveform SF uses for mokke's SurfaceView buffer posts, and whether a new
   surface after resume gets a new-layer GC or fill (07 §5.4).
4. Whether the gutter's lower buttons overlap the canvas (and so the SF limit) on the Note Air 4C.
5. What SF does with a system window over mokke's running session (no focus handling).
