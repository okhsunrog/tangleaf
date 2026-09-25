# PngNote: pen and panel path, in the stock documents' terms

This is a static reading of the open-source note app PngNote (`io.github.karino2.pngnote`, repository
at `~/tmp_zfs/reference_notes_apps/PngNote`, single commit `e949ad5`, 2026-07-18, versionName 7),
written 2026-09-25 so it can be compared action by action with the stock Onyx Notes app
([../stock-notes/00-overview.md](../stock-notes/00-overview.md)) and with Tangleaf
([../stock-notes/08-tangleaf-current-path.md](../stock-notes/08-tangleaf-current-path.md)). Nothing
was run and the tablet was not touched. No changes to Tangleaf are proposed.

## 0. Notation and sources

Tags, as in the stock documents:

- **[code]**: read directly in the cited source or decompiled class.
- **[inferred]**: reasoned from code that was read.
- **[doc]**: taken from `stock-notes/` (mostly 07) and not re-derived here.
- **[unknown]**: cannot be settled statically.

| shorthand   | root                                                                                                             |
| ----------- | ---------------------------------------------------------------------------------------------------------------- |
| `png/`      | `~/tmp_zfs/reference_notes_apps/PngNote/app/src/main/java/io/github/karino2/pngnote/`                            |
| `pen141/`   | jadx 1.5.6 output of `app/libs/onyxsdk-pen-1.4.1.aar` → `classes.jar`, `sources/com/onyx/android/sdk/pen/`       |
| `dev129/`   | jadx 1.5.6 output of `app/libs/onyxsdk-device-1.2.9.aar`, `sources/com/onyx/android/sdk/`                        |
| `base1627/` | jadx 1.5.6 output of `app/libs/onyxsdk-base-1.6.27.aar`, `sources/com/onyx/android/sdk/`                         |
| `dev1352/`  | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-device-1.3.5.2/sources/com/onyx/android/sdk/` (Tangleaf's device SDK) |
| `fw/`       | `~/tmp_zfs/onyx_framework/out/sources/android/onyx/` (framework.jar of firmware 4.2)                             |
| `fwv/`      | `~/tmp_zfs/onyx_framework/out/sources/android/view/`                                                             |

The three PngNote AARs had not been decompiled before. The `pen141/`, `dev129/` and `base1627/` line
numbers are from `~/tmp_zfs/boox-notes-inspect/jadx/bin/jadx -d <out> classes.jar` (jadx 1.5.6) and
can be regenerated that way. AAR SHA-256: pen `dfc828b6…cdaad2b`, device `9a7af825…36a289f5`, base
`dd0fd1fb…ce624e1f` (full hashes via `sha256sum app/libs/*.aar`).

Device facts, verified on the tablet by the coordinator on 2026-09-25: firmware 4.2
(`ro.build.display.id=2026-04-28_17-50_4.2-rel_04282_555977efe`), system partition Android 13 (SDK 33),
vendor partition Android 11 (SDK 30), `vendor.onyx.tablet=true`. The device's
`/system/bin/surfaceflinger` and `/system/framework/framework.jar` are byte-identical to the copies
under `~/tmp_zfs/onyx_framework/`. The `fw/`/`fwv/` sources and the SF model in 07 therefore describe
this tablet.

### 0.1 SDK version actually linked

`app/build.gradle:57-58`: the Maven line `onyxsdk-pen:1.2.1` is commented out and the app links
`fileTree(dir: 'libs', include: ['*.aar'])` [code]. `app/libs/` holds exactly
`onyxsdk-pen-1.4.1.aar`, `onyxsdk-device-1.2.9.aar` and `onyxsdk-base-1.6.27.aar` [code]. There is no
Gradle resolution, so no newer transitive device SDK can replace 1.2.9 [inferred]. This is an older
generation than both Tangleaf's (`pen 1.5.4.3` / `device 1.3.5.2`) and the SDK bundled with stock.

## 1. Summary

1. **On the Note Air 4C, none of PngNote's Onyx calls reach SurfaceFlinger.** `device-1.2.9` maps
   the board platform to a device class only for `rk3288`, `rk312x`, `rk3368`, `msm8953`, `sdm660`
   and `bengal` (`dev129/device/Device.java:75-87`). `lito` is not in that list, so
   `Device.currentDevice` falls to `new BaseDevice()` (`:90-124`) [code]. `device-1.3.5.2` adds `lito`
   (`dev1352/device/Device.java:22, 105`) [code], which is why Tangleaf gets `SDMDevice`. The Air 4C's
   board is `lito` [doc: 00 §0, from `vendor/build.prop`]. Every `BaseDevice` method PngNote reaches
   has an empty body: pen state, `enablePost`/`leaveScribbleMode`, stroke width/colour/style, region
   limit/mode/exclude (`dev129/device/BaseDevice.java:392-399, 442-450, 469-491`) [code]. So on this
   tablet PngNote sends **zero** SF pen, region or refresh transactions [inferred from code].
2. A second, independent reason: even `device-1.2.9`'s `SDMDevice` looks the pen methods up as static
   methods on `android.view.View` (`dev129/device/SDMDevice.java:438-473`, e.g. `r0 =
View.setScreenHandWritingPenState(int)`). The 4.2 framework has them on
   `android.onyx.ViewUpdateHelper` (`fw/ViewUpdateHelper.java:602, 1195-1272`) and `fwv/View.java`
   contains no `HandWriting` method at all (grep count 0) [code]. This framework.jar is byte-identical
   to the tablet's (§0). The reflection would resolve to null and do nothing even if the board were
   SDM-mapped [inferred].
3. As a result, on the Air 4C the in-process reader also gets no coordinate mapping.
   `BaseDevice.getRawTouchPointToScreenMatrix()` returns null (`BaseDevice.java:1098-1123`) and
   `mapToView` is empty (`:206-207`), so `RawInputReader.d()` maps every sample to
   `(−viewX, −viewY)` (`pen141/RawInputReader.java:398-416`). `mapToRawTouchPoint(RectF)` returns
   an empty rect (`BaseDevice.java:229-238`), so every `setLimitRect` logs
   `E RawInputReader: Empty region detected when mapping!!!!!` (`RawInputReader.java:473-479`)
   [code]. Predicted on-device behaviour: no firmware ink, and a stroke collapses to one off-canvas
   point, which `BitmapBackend.drawOrErasePointsToBitmap` discards as an empty region
   (`png/ui/BitmapBackend.kt:117-120`). **Nothing is drawn** [inferred]. The README lists only older
   devices (Note2, Note3, Note Air, Nova2) as working (`README.md:12-23`) [code].
4. The rest of this document describes the path **as the code intends it**, as if the calls reached SF.
   That is what happens on a board where they do, and it is the part that matters when comparing
   technique. Every SF effect in §3–§5 is therefore conditional: [code] for the SDK call chain,
   [inferred] for the SF reaction via the 4.2 model in 07. Older boards run older SurfaceFlinger
   builds whose behaviour was not read [unknown].
5. **The intended design, in one line.** One `TouchHelper` in `SFTouchRender`-only mode on a
   `SurfaceView`. SF inks every tip stroke itself, the pen and eraser tools included (the "eraser" is a
   white 30 px pencil). The app never draws live ink. It repaints the whole `SurfaceView` from its
   bitmap only while the pen session is **fully closed**: every repaint is wrapped in
   `withTempNoRawRendering`, a `closeRawDrawing` + `openRawDrawing` cycle (§4). Pen strokes are
   repainted 2 s after the last pen activity. Eraser strokes, stylus eraser-end strokes, undo/redo,
   page and tool changes are repainted immediately.
6. PngNote never sets an update mode, GC, transient/fast mode, turbo, dither, exclude rect, region
   mode, brush/eraser raw-drawing flag, `ENABLE_COLOR_CU` or view default update mode (grep of `png/`:
   the only direct `EpdController` call is `enablePost` at `png/ui/CanvasBoox.kt:123`) [code]. Its
   only panel refreshes are `SurfaceView` buffer posts and ordinary Compose redraws of the toolbar.

## 2. Central classes and entry points

| Class / file                      | Role                                                                      | Key entry points                                                                                                                                                                                                                                                                                                                                                                   |
| --------------------------------- | ------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `png/ui/CanvasBoox.kt`            | `SurfaceView` canvas; owns the `TouchHelper`, callbacks and every repaint | `touchHelper` (lazy) :167; callbacks :97-149; `applyRawDrawingSettings` :183-204; `ensureCloseRawRendering` :169-175; `withTempNoRawRendering` :362-375; `syncBackendToSurface` :423-452; delayed sync :377-418; tool switch :328-351, :454-462; page :465-476; lifecycle hooks `ensureInit` :275, `onRestart` :284, `onTryRawDrawing` :295, `onEnsureClose` :305, `onFinish` :514 |
| `png/BookActivity.kt`             | Compose screen: `TopAppBar` above, `AndroidView(CanvasBoox)` below        | `onResume` :169-172; `onRestart` :174-178; `onWindowFocusChanged` :182-185; `onStop` :187-191; toolbar :299-395; `AndroidView` factory :399-409 and `update` :410-425; `handler.post { initCount = 1 }` :435-437                                                                                                                                                                   |
| `png/ui/BitmapBackend.kt`         | ARGB bitmap, undo list, pixel erase                                       | `drawOrErasePointsToBitmap` :96-144; `eraseByPoints` :146-150; `needEraseUpdate` :30-31                                                                                                                                                                                                                                                                                            |
| `pen141/TouchHelper.java`         | one render, chosen by `hasStylus`                                         | ctor :176-190; `openRawDrawing` :91-96; `closeRawDrawing` :98-101; `setRawDrawingEnabled` :103-110; `setRawDrawingRenderEnabled` :120-127                                                                                                                                                                                                                                          |
| `pen141/touch/SFTouchRender.java` | the only render used                                                      | `openDrawing` :239-242 (`b()` :314-317); `closeDrawing` :245-249 (`g()` :175-179, `c()` :170-173); `setDrawingRenderEnabled` :252-258 (`h()` :181-184, `k()` :186-188)                                                                                                                                                                                                             |
| `pen141/RawInputReader.java`      | in-process digitizer reader (`libonyx_pen_touch_reader.so`)               | `start` :232-239; `resume` :241-246; `pause` :248-252; `quit` :254-267; `setLimitRect` :324-330; `setExcludeRect` :332-339; dispatch :341-373                                                                                                                                                                                                                                      |

The app's lifecycle wiring runs **through Compose state**. Activity callbacks only bump counters
(`restartCount`, `closeCount`, `tryRawDrawing`, `undoCount`, …, `BookActivity.kt:91-101`), and the
`AndroidView` `update` block compares each counter with the canvas's copy and acts
(`BookActivity.kt:410-425`) [code]. SDK work therefore happens at the next recomposition, not inside
the activity callback [code]. The order is fixed by the `update` block: `ensureInit`, `penOrEraser`,
`onPageIdx`, `onRestart`, `onEnsureClose`, `onTryRawDrawing`, `undo`, `redo`, `refreshUI` [code].

## 3. The primitives, traced to SurfaceFlinger (intended path)

`TouchHelper.create(view, cb)` → `hasStylus(ctx)` (`pen141/TouchHelper.java:24-26`), which matches
input-device names containing `onyx_emp`, `Wacom I2C Digitizer` or `hanvon_tp`
(`base1627/utils/DeviceFeatureUtil.java:14, 44-64`) [code]. The Air 4C digitizer is
`onyx_emp_Wacom I2C Digitizer` [doc: `docs/planning/handwriting-prototype.md:145`]. So the render is
**`SFTouchRender` alone** (`TouchHelper.java:181-183`), with no `AppTouchRender` [code]. (The older
REPORT.md said the two-argument `create` selects `AppTouchRender`. That is wrong for this SDK on a
stylus device.) With one render, each call below is sent once, not twice as in stock and Tangleaf.

`pen 1.4.1` has **no** `setBrushRawDrawingEnabled`, `setEraserRawDrawingEnabled`,
`resetPenDefaultRawDrawing` or stroke-parameter API (grep of `pen141/`: only
`NeoCharcoalPen.java:81` reads parameters) [code]. `setRawDrawingEnabled` is just render + input
(`TouchHelper.java:103-110`) [code].

| PngNote call (site)                                                                         | SDK chain (pen 1.4.1)                                                                                                                                                                                                                            | SF transactions, in order (if they reached SF)                                        | SF-visible effect [inferred via 07 unless marked]                                                                                                                                              |
| ------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `TouchHelper.create(this, inputCallback)` (`CanvasBoox.kt:167`, forced by `firstInit` :270) | `SFTouchRender.bindHostView` → `RawInputManager.setHostView` → reader limit = `view.getLocalVisibleRect()` (`RawInputManager.java:134-140`, `RawInputReader.java:324-330`)                                                                       | `SET_SCREEN_HANDWRITING_REGION_LIMIT` = local visible rect of a view not yet laid out | limit = whatever that rect is at construction (likely empty; [unknown])                                                                                                                        |
| `setStrokeWidth(w)`                                                                         | `RawInputReader.setStrokeWidth`: native + `EpdController.setStrokeWidth` (`RawInputReader.java:273-278`)                                                                                                                                         | `SET_STROKE_WIDTH` ×1                                                                 | SF painter width                                                                                                                                                                               |
| `setStrokeColor(c)`                                                                         | `EpdPenManager.setStrokeColor` → `EpdController.setStrokeColor`; reader field only (`SFTouchRender.java:213-216`)                                                                                                                                | `SET_STROKE_COLOR` ×1                                                                 | SF painter colour: black for the pen, **white** for the eraser tool                                                                                                                            |
| `setLimitRect(limit, emptyList())` (:195, :202)                                             | reader `setLimitRect` then `setExcludeRect(empty)`, which returns early (`RawInputManager.java:142-146`, `RawInputReader.java:332-335`)                                                                                                          | `SET_SCREEN_HANDWRITING_REGION_LIMIT` = limit; **no exclude transaction**             | limit replaced; **SF's exclude table is left as the previous app left it** (stock leaves one degenerate rect at its view origin; Tangleaf clears it, 00 §5 items 17-18)                        |
| `setStrokeStyle(STROKE_STYLE_PENCIL = 0)`                                                   | `EpdPenManager.setStrokeStyle` (`EpdPenManager.java:50-52`, `SFTouchRender.java:208-210`)                                                                                                                                                        | `SET_STROKE_STYLE(0)`                                                                 | pencil style                                                                                                                                                                                   |
| `openRawDrawing()`                                                                          | flags cleared; `SFTouchRender.openDrawing`: style 0, `RawInputReader.start` (new reader thread), `EpdPenManager.startDrawing` (`TouchHelper.java:91-96`, `SFTouchRender.java:239-242, 314-317`)                                                  | `SET_STROKE_STYLE(0)`, `PEN_STATE(1)`                                                 | pen owner = PngNote; `TouchReader::start` (waits up to 800 ms for `loop running`), `### EBAS n`; SF forwards nothing in state 1 [doc 07 §3-4.1]. **No `ENABLE_POST(0)`** (no `AppTouchRender`) |
| `setRawDrawingEnabled(true)`                                                                | render: `resumeDrawing`; input: `RawInputReader.resume` (`nativeSetPenState(4)`, in-process)                                                                                                                                                     | `PEN_STATE(2)`                                                                        | reader running; SF inks with whatever brush flag is set globally (never set by PngNote)                                                                                                        |
| `setRawDrawingEnabled(false)`                                                               | render: `SFTouchRender.h()` = `leaveScribbleMode` + `pauseDrawing`; input: reader pause (`SFTouchRender.java:181-184`, `RawInputReader.java:248-252`)                                                                                            | `ENABLE_POST(1)`, `PEN_STATE(3)`                                                      | leaves schema 7 if in it (`Post enabled!`, `### EUS, s 2 c 0`, 150 ms sleep on the binder thread, 07 §5.1), then `TouchReader::pause` (waits up to 100 ms)                                     |
| `closeRawDrawing()`                                                                         | `SFTouchRender.closeDrawing`: style 0; `g()` = `leaveScribbleMode`, reader pause, `pauseDrawing`; `c()` = reader `quit` (native close, executor shut down), `quitDrawing` (`SFTouchRender.java:245-249, 170-179`, `RawInputReader.java:254-267`) | `SET_STROKE_STYLE(0)`, `ENABLE_POST(1)`, `PEN_STATE(3)`, `PEN_STATE(0)`               | STOP: `### EBAS 0`, `TouchReader::stop` (waits up to 800 ms), `stopHandwriting`. A 3 sent while already paused (4) is **not** deduplicated and logs again (07 §4.1)                            |
| `touchHelper.isRawDrawingRenderEnabled = x` (:172, :199)                                    | Kotlin setter → `setRawDrawingRenderEnabled(x)`; the cached flag already equals `x` at both sites (`TouchHelper.java:120-127`)                                                                                                                   | none                                                                                  | none [code]                                                                                                                                                                                    |
| `EpdController.enablePost(this, 1)` (:123, stylus eraser-end begin)                         | `SDMDevice.enablePost` (would be; `BaseDevice` here)                                                                                                                                                                                             | `ENABLE_POST(1)`                                                                      | leaves schema 7 **without pausing**: `androidDrawing = 1`, pen state stays 2 (07 §4.4)                                                                                                         |
| `holder.lockCanvas()` … `unlockCanvasAndPost()` (:256-263, :433-450, :482-498)              | not Onyx; the `SurfaceView`'s own `BufferQueue`                                                                                                                                                                                                  | none (a layer buffer)                                                                 | a whole-surface frame at the next SF composition. No View update mode applies to a `SurfaceView` buffer; which waveform SF uses for it is [unknown]                                            |

Never called: `setSingleRegionMode`/`setMultiRegionMode`, `setPenUpRefreshTimeMs` (the reader keeps
its 500 ms default, `RawInputReader.java:58-59`, and `onPenUpRefresh` stays the empty base method,
`pen141/RawInputCallback.java:28-29`), `setPostInputEvent` (field default false,
`RawInputReader.java:47`, so no `PenActiveEvent`), `enableSideBtnErase`, `enableFingerTouch` [code].

## 4. `withTempNoRawRendering`, precisely

```kotlin
// png/ui/CanvasBoox.kt:362-375
fun withTempNoRawRendering(f: ()->Unit) {
    val isRawRendering = touchHelper.isRawDrawingRenderEnabled
    if (isRawRendering) { touchHelper.setRawDrawingEnabled(false); touchHelper.closeRawDrawing() }
    f()
    if (isRawRendering) { touchHelper.openRawDrawing(); touchHelper.setRawDrawingEnabled(true) }
}
```

Its only caller is `syncBackendToSurface` (`:423-452`), which first returns if `drawingState.isDrawing`
(a pen-down seen within the last 10 s without its point list, `:16-38, :425-426`), and otherwise does
the whole-surface repaint inside it [code]. All steps run synchronously on the main thread [code].

When render was on (the steady state after open), the SF-visible sequence is [code for the
transactions; SF effects inferred from 07]:

| #   | call                                     | SF transaction                        | SF effect (4.2 model)                                                                                                                                                                                                                                           |
| --- | ---------------------------------------- | ------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `setRawDrawingEnabled(false)`            | `ENABLE_POST(1)`                      | if SF is in schema 7 (any stroke since the last exit), `stopHandwriting`: `Post enabled!`, `clearHandwritingFBSchema` → `### EUS, s 2 c 0` (no fast fixed slot is ever set by PngNote), **150 ms `usleep` on the binder thread**, so the main thread blocks too |
| 2   |                                          | `PEN_STATE(3)`                        | `Pid N State change to: toPause`; `TouchReader::pause`, waits up to 100 ms for `paused`; `stopHandwriting` no-op                                                                                                                                                |
| 3   | `closeRawDrawing()`                      | `SET_STROKE_STYLE(0)`                 | painter style 0 (already 0)                                                                                                                                                                                                                                     |
| 4   |                                          | `ENABLE_POST(1)`                      | `androidDrawing` already 1 → nothing                                                                                                                                                                                                                            |
| 5   |                                          | `PEN_STATE(3)`                        | reader is 4 ≠ 3 → runs `pause` + `stopHandwriting` again; logs a second state change                                                                                                                                                                            |
| 6   |                                          | `PEN_STATE(0)`                        | `### EBAS 0`, `TouchReader::stop` (up to 800 ms), `cytp_lo_filter` 0, `stopHandwriting` no-op                                                                                                                                                                   |
| 7   | `f()`: lock, draw, `unlockCanvasAndPost` | none                                  | a full-surface buffer is queued; SF composes it outside schema 7 (normal schema, scheme 2), so it is **not withheld** (07 §4.5)                                                                                                                                 |
| 8   | `openRawDrawing()`                       | `SET_STROKE_STYLE(0)`, `PEN_STATE(1)` | `TouchReader::start` (up to 800 ms), `### EBAS n`; in-process reader restarted on a new thread                                                                                                                                                                  |
| 9   | `setRawDrawingEnabled(true)`             | `PEN_STATE(2)`                        | `ensureStart` + `resume`; schema 7 is **not** re-entered until the next pen sample                                                                                                                                                                              |

What it achieves: the app frame is posted while SF is out of the handwriting schema, so it cannot be
dropped by `HandwritingSchema::output`, and SF's own ink is replaced by the app's rendering of the
same stroke. The code comment "eraser tends to fail for update screen. I don't know the reason"
(`:115-116`) fits that withholding rule [inferred]. PngNote gets the same result as Tangleaf's
`HAND_WRITING_REPAINT_MODE` reconcile frame, but by leaving the schema instead of marking the frame.

Side effects [inferred]:

- **It is a full close, not a pause.** Steps 2–6 and 8–9 cycle the SF reader through
  paused → stop → start → running, and the in-process reader is quit and restarted (a new executor
  thread and `nativeRawReader`). Limit, width and colour are **not re-sent** on reopen. SF keeps them
  globally (07 §2.2) [inferred]. Whether the native in-process reader keeps its limit/width across
  `nativeRawClose` is [unknown].
- **Main-thread blocking.** Each transaction is a synchronous Binder call. Summed worst-case waits
  are about 150 + 100 + 100 + 800 + 800 + 100 ms. Typical cost is the 150 ms schema-exit sleep plus
  thread hand-offs [unknown without a trace].
- **Pen input during the gap is lost**, not inked and not recorded. SF's reader is paused/stopped,
  and the in-process reader is quit, so neither delivers samples. After step 9, a pen-down re-triggers
  schema 7 on its first sample [inferred].
- **Frame against resume race.** If a pen-down arrives after step 9 but before SF has composed the
  step-7 buffer, SF enters schema 7 and the frame is withheld until the next exit [inferred].
  Step 8 blocks until SF's reader loop runs, which makes this window small [inferred].
- **Scheme 4 is never involved.** PngNote puts nothing in a fixed slot, so every exit is `s 2`
  [inferred]. The framework paths that do put A2 there are discussed in §7.2.

When render was already off (session closed by `ensureCloseRawRendering`), `withTempNoRawRendering`
is just `f()`: a buffer post with no SF transaction [code].

## 5. Per action (intended path; on the Air 4C every SF column is "none")

"Cycle" below = the nine steps of §4. "Frame" = one whole-surface `SurfaceView` buffer post.

### 5.1 Open a note

1. `BookActivity.onCreate` → `setContent` → `AndroidView` factory: `CanvasBoox(...)` then `firstInit()`,
   which creates the `TouchHelper` (limit = local visible rect at that moment) and registers a layout
   listener and a `SurfaceHolder.Callback` (`BookActivity.kt:399-409`, `CanvasBoox.kt:269-273`) [code].
2. Several triggers then call `applyRawDrawingSettings()`: `surfaceCreated` (:222-224), every layout
   change (:206-210), `ensureInit(1)` after `handler.post { initCount = 1 }` (`BookActivity.kt:435-437`),
   `onResume` → `onTryRawDrawing` and `onWindowFocusChanged(true)` → `onRestart`
   (`BookActivity.kt:169-185`) [code]. The author notes the ordering is racy
   (`BookActivity.kt:180-181`); which trigger runs first is [unknown].
3. The first trigger with a valid surface and a non-empty limit (`:184-185`) opens the session
   (`:191-200`) [code]:
   - frame (`clearSurfaceViewByBitmap` → `cleanSurfaceView` draws the initial page bitmap scaled, `:254-267, :478-504`);
   - `SET_STROKE_WIDTH(3)`, `SET_STROKE_COLOR(black)`;
   - `SET_SCREEN_HANDWRITING_REGION_LIMIT(getLocalVisibleRect offset by (0, −40))` (`:153-161`);
   - `SET_STROKE_STYLE(0)` ×2, `PEN_STATE(1)`, `PEN_STATE(2)`.
   - Region mode, exclude, brush/eraser flags and `ENABLE_POST(0)` are never sent.
4. Later triggers with the same size send only `SET_SCREEN_HANDWRITING_REGION_LIMIT` (`:201-203`). A
   size change while open runs `ensureCloseRawRendering` (close sequence) and reopens (`:163-164,
:187-189`) [code]. The focus-gain path also runs one cycle + frame first (`onRestart`, `:284-291`).
5. No GC, update mode or fast mode [code].

The −40 px offset is empirical ("I don't know the reason", `:157-158`). It shifts the limit
**upwards**, so the bottom 40 px of the canvas lie outside the SF limit and the top edge extends
40 px into the toolbar [inferred].

### 5.2 Write (pen tool, tip)

1. Down: SF reader 2 inks, if the global brush flag is on (never set by PngNote, so it is whatever the
   previous app left; stock's pause sets it to 1, 00 §2.1). First sample → `HandlePenTrigger`, schema 7,
   `### EUS, s 3 c 0` [inferred]. App: `onBeginRawDrawing` → `drawingState.startDrawing()`,
   `postponeDelaySync()` (`:98-101`). No transaction.
2. Moves: SF inks. `onRawDrawingTouchPointMoveReceived` is empty (`:107-108`): the app draws nothing
   and sends nothing [code].
3. Up: `onRawDrawingTouchPointListReceived` → `endDrawing`, draws the path into the bitmap with a
   fixed 3 px anti-aliased `Paint` (no pressure), pushes undo, notifies the lazy saver
   (`:110-119`, `BitmapBackend.kt:96-144`). `onEndRawDrawing` → `requestDelaySync()` (`:103-105,
:385-388`) [code]. The reader's 500 ms pen-up refresh fires into the empty base
   `onPenUpRefresh` [code].
4. +2000 ms: `maySyncBack` (`:395-418`) re-arms itself while pen activity continues (postponed within
   the last 1.9 s) or a sync ran within 100 ms. Otherwise, if render is on, it runs
   `syncBackendToSurface` → **cycle + frame** [code].
5. So during a writing burst SF stays in schema 7 / scheme 3 and shows only its own ink. About 2 s
   after the last stroke, one exit (`Post enabled!`, `EUS s 2`) and one whole-canvas frame replace SF's
   ink with the app's 3 px rendering [inferred]. SF's live stroke (style 0, width 3, possibly pressure
   shaped [unknown]) and the app's fixed-width path can differ visibly at that moment [inferred].
6. Extra: the first stroke after an undo changes `canRedo`. That bumps `refreshCount`
   (`BookActivity.kt:118-127`), so `refreshUI` runs a cycle + frame at the next recomposition,
   straight after the point list [code].

### 5.3 Erase with the eraser tool (tip)

The only eraser mode is a hard-edged 30 px pixel eraser (`eraserPaint`, white, no anti-aliasing,
`:59-66`) [code]. There is no stroke eraser, soft eraser or area eraser.

- **Tool entry** (toggle in the top bar, `BookActivity.kt:314-318` → `update` → `penOrEraser(false)` →
  `eraser()`, `:340-351`): `SET_STROKE_WIDTH(30)`, `SET_STROKE_STYLE(0)`, `SET_STROKE_COLOR(white)`
  with the session left running, then `syncBackendToSurface` → cycle + frame [code].
- **Per gesture:** the tip is a brush touch, so SF reader 2 inks a **white 30 px pencil line**
  (schema 7, `EUS s 3` on the first sample). This is a live firmware "erase" trace built from the
  normal pen reader [inferred; that SF honours white on the Kaleido panel is [unknown]]. The app's
  point list at lift draws the white path into the bitmap (`:110-114`), then `if (isEraser)
syncBackendToSurface()` (`:117-118`) → **cycle + frame immediately after every erase stroke**
  [code]. `onEndRawDrawing` also arms the 2 s check, which then finds a recent sync and re-arms
  or returns (`:397-405`).
- The white trace also paints over the page template (`background`) until the frame restores it
  [inferred].
- **Tool exit** (`pencil()`, `:328-338`): width 3, style 0, colour black, cycle + frame [code].

### 5.4 Erase with the stylus eraser end

1. SF side: `BtnReader` sees the rubber and selects reader 4 only if the global eraser raw-drawing
   flag is set, else the no-op reader 3 (07 §4.2) [doc]. PngNote cannot set that flag with pen 1.4.1.
   After stock's pause (`(0, 5)`) or Tangleaf's session (`(0, 0)`) it is off, so **SF draws nothing
   and does not enter schema 7 for the eraser end** [inferred]. If the previous app left it on, SF draws
   an eraser trace in that app's style [inferred].
2. App: the in-process reader reports `isErasing`. `onBeginRawErasing` → `EpdController.enablePost(view,
1)` (`:121-126`) → `ENABLE_POST(1)`. If an earlier tip stroke left SF in schema 7 (likely: pen
   strokes wait 2 s), this exits it (`Post enabled!`, `EUS s 2`, 150 ms blocking) **while pen state
   stays 2** [inferred via 07 §4.4].
3. During: erase points are accumulated and applied to the bitmap every 100 points or once 300 ms
   have passed since the last batch (`:132-138`, `BitmapBackend.kt:30-31, 146-150`). **No frame is
   posted**, so the user sees no trace during the gesture [code for no post; inferred for no trace].
4. Up: `onRawErasingTouchPointListReceived` → final batch; `onEndRawErasing` → `syncBackendToSurface`
   → cycle + frame (`:128-130, :140-143`) [code].
5. The side button: `enableSideBtnErase` is never called; the native default is [unknown].

### 5.5 Lasso, selection, dragging

Not implemented. The README lists "pencil, erasure, undo, redo" only (`README.md:5-8`) and there is no
selection code in `png/` [code].

### 5.6 Menus, dialogs, overlays

- The editor has no in-canvas menu, popup, dialog or keyboard. The `TopAppBar` is a sibling above the
  canvas (`BookActivity.kt:296-427`), so there is nothing to exclude [code]. With the −40 px offset the
  SF limit reaches about 40 px into the bar, so SF may ink over the lower part of the bar
  [inferred].
- Pen taps on toolbar buttons arrive as ordinary stylus `MotionEvent`s to Compose [inferred].
- "Not yet undo-able." toast (`:328`): no pause; the toast window takes no focus. SF would ink over it
  if it overlaps the limit [inferred].
- Share (`:194-207`): `closeCount++` → at the next recomposition `onEnsureClose` →
  `ensureCloseRawRendering` (`ENABLE_POST(1)`, 3, style 0, `ENABLE_POST(1)`, 3, 0) + plain frame
  (`:305-312`); then the chooser activity starts [code].
- Window focus loss (notification shade, a dialog activity): **nothing**. Only `hasFocus == true` is
  handled (`BookActivity.kt:182-185`) [code]. The session stays at pen state 2 and SF keeps inking
  under or over the system UI within the limit [inferred].

### 5.7 Finger scroll and pinch zoom

None. `CanvasBoox` has no `onTouchEvent` or gesture handling, and the page is fixed-size [code].
Fingers on the canvas do nothing in the app. SF's and the in-process reader only read the pen
[inferred].

### 5.8 Page switch (prev / next / first / last / add / half-shift / grid return)

1. Button → `ensureSave()` (a synchronous PNG write on the main thread) → `pageIdxValue` or
   `shiftHalf` changes (`BookActivity.kt:160-165, 221-265, 340-343`) [code].
2. Recomposition → `onPageIdx` → load bitmap (disk, main thread) → `setupNewPage` → `syncBackendToSurface`
   → cycle + frame (`CanvasBoox.kt:465-476`) [code]. `setupNewPage` clears undo, which can bump
   `refreshCount` and add a second cycle + frame (`BitmapBackend.kt:177-193`, `BookActivity.kt:118-127`)
   [code].
3. No GC, ever. The page change is one whole-canvas frame in whatever waveform SF picks for the
   `SurfaceView` layer [unknown]. No counted full refresh as in stock [code].
4. Grid (`:267-272`) starts `PageGridActivity`, which goes through §5.10.

### 5.9 Undo / redo

Button → counter → recomposition → `undo()`/`redo()` → bitmap patch → `refreshAfterUndoRedo` →
`syncBackendToSurface` → cycle + frame (`CanvasBoox.kt:73-93`) [code]. A `canRedo` change adds a
second cycle via `refreshUI` [code].

### 5.10 Leave the note, background, foreground

- **Back arrow** (`BookActivity.kt:388-392`): `finishListener()` = `onFinish()` →
  `ensureCloseRawRendering()` directly → `ENABLE_POST(1)`, 3, style 0, `ENABLE_POST(1)`, 3, 0; then
  `ensureSave()`, `finish()` [code]. No GC; limit left in SF [code].
- **System back / Home / recents** → `onStop` → `closeCount++`, `ensureSave()` (`:187-191`) [code]. The
  close itself runs only in the `update` block at the next recomposition [code]. Compose pauses its
  window frame clock on `ON_STOP`, and on API ≥ 29 `ON_STOP` is dispatched before `Activity.onStop`
  runs. So that recomposition probably does not happen until the activity starts again, and **the SF
  session stays open at pen state 2 while PngNote is in the background**, with PngNote as the pen
  owner and its limit active over whatever app is in front [inferred from Compose's
  `WindowRecomposer`/`PausableMonotonicFrameClock` behaviour; not verified against Compose 1.5.0
  sources; unknown until observed]. On a finished activity the pending close may never run [inferred].
- **Foreground** (`onRestart` → `restartCount++`, `onResume` → `tryRawDrawing++`, focus →
  `restartCount++`). The first recomposition runs, in `update` order: `onRestart` (cycle + frame, then
  limit), `onEnsureClose` (the deferred close + plain frame), `onTryRawDrawing` (reopen: frame, width,
  colour, limit, style, 1, 2). A focus recomposition then adds another cycle + frame [inferred from
  the `update` order `BookActivity.kt:419-421`; batching of the counter changes into one
  recomposition is [unknown]].
- Screen off/on: no handling [code].

### 5.11 Keyboard and text input

None in the editor. The only `TextField` is the new-book dialog in `BookListActivity` (a different
activity with no `TouchHelper`) [code; REPORT.md's table agrees].

## 6. Panel refreshes PngNote issues

| trigger                                                             | what reaches the panel                                                                  |
| ------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| every pen stroke                                                    | SF's own ink only (schema 7), no app frame                                              |
| 2 s after the last pen activity                                     | schema exit (`EUS s 2`) + whole-canvas `SurfaceView` frame, inside a full session cycle |
| every eraser-tool stroke, every eraser-end stroke                   | same, immediately after lift                                                            |
| tool switch, page change, undo/redo, `canRedo` change, focus gain   | same                                                                                    |
| session open                                                        | whole-canvas frame before `PEN_STATE(1)`                                                |
| toolbar state changes                                               | ordinary Compose redraw (view mode 5 AUTO) of the bar [inferred]                        |
| full GC / GC16 / `applyGCOnce` / `repaintEverything` / transient A2 | **never** [code]                                                                        |

Whether a `SurfaceView` buffer on this firmware updates in GU, AUTO or GC is [unknown]. PngNote does
nothing to choose it. Ghosting is never cleared by an explicit full refresh [inferred].

## 7. Compared with stock

### 7.1 Capabilities

| stock capability (00)                                                                                   | PngNote (intended path)                                                                                                                                                                                               |
| ------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| one session per document, paused/resumed (`PEN_STATE` 3/2)                                              | session **closed and reopened** (3,3,0,1,2) around every repaint                                                                                                                                                      |
| live eraser trace by firmware (SF eraser reader, style 8 soft / 5 dash)                                 | eraser tool: yes, but as a white 30 px **pencil** through the brush reader; eraser end: none (eraser flag never set)                                                                                                  |
| stroke, partial ("move") and area erasers, soft eraser                                                  | one pixel eraser, hard edge, fixed 30 px                                                                                                                                                                              |
| no flash after erase (`ENABLE_POST(1)`, render-only pause, invalidate, RS(300))                         | no flash either: schema exit + one whole-canvas `SurfaceView` frame. No GC and no A2 [inferred]. Stock repaints only the changed slices                                                                               |
| no per-stroke refresh (reconcile gated off on `vendor.onyx.tablet`)                                     | none per stroke either; one debounced 2 s-idle whole-canvas repaint instead. Stock leaves SF ink until the next hand-back                                                                                             |
| fast mode (transient 2308, turbo 5, dither 160) for drags, scroll, zoom, transforms                     | none; there are no drags, scroll or zoom                                                                                                                                                                              |
| counted GC every 10th page action                                                                       | no GC at all                                                                                                                                                                                                          |
| EinkWise: stock writes dither to the store; `isOnyxApp` exempts it from the framework scroll-A2 manager | PngNote writes nothing to EinkWise. It is a non-Onyx package, so it is **not** exempt from `OnyxScrollRefreshManager` (Entry A, 07 §6) [code for the gate]. Whether Entry A is live for its window is discussed below |
| colour pens, colour CU (`ENABLE_COLOR_CU`)                                                              | black only; no CU                                                                                                                                                                                                     |
| pressure / pen styles from the bundled pen config                                                       | SF style 0 (pencil) at width 3. The app re-renders a fixed-width path, so any pressure shaping SF applies is lost at the repaint [inferred]                                                                           |
| lasso in the firmware layer (dash trace, style 5)                                                       | no lasso                                                                                                                                                                                                              |
| palm/finger: hover-suppressed finger gestures, typed exclude rects for overlays                         | nothing. Fingers are ignored because nothing listens. No excludes; SF's exclude table is inherited from the previous app                                                                                              |
| pause on popups, toasts, status bar, keyboard, focus loss                                               | no pause on any of them; only leaving (back arrow) closes; backgrounding probably defers the close (§5.10)                                                                                                            |
| limit/exclude/flags re-pushed on every resume                                                           | limit pushed on every layout/resume; flags and excludes never                                                                                                                                                         |
| page turn: PR, display, RS(≥500)                                                                        | cycle + frame, no delay; strokes during the cycle are lost                                                                                                                                                            |

### 7.2 Predicted driver-log behaviour for erase (and lasso)

Lasso: not applicable (no lasso).

**Eraser tool, intended path** (session at 2, not in schema 7) [inferred via 07]:

```
pen down      HandlePenTrigger, sid: -1 value is: 0 pid <png>   ; ### EUS, s 3 c 0
(white trace inked by SF; no app frame)
pen up (+ main-thread hop)
              Post enabled!                                      ; ### EUS, s 2 c 0   (150 ms)
              Pid <png> State change to: toPause                 ; Control state: toPause -> paused
              Pid <png> State change to: toPause                 (second 3: reader was 4)
              Pid <png> State change to: stop                    ; ### EBAS 0
              (SurfaceView frame, normal schema, scheme 2)
              Pid <png> State change to: start                   ; ### EBAS n
              Pid <png> State change to: running                 ; Control state: toResume -> running
```

**Stylus eraser end** (eraser flag off): no `HandlePenTrigger` for the rubber, and one
`Post enabled!` / `EUS s 2` at eraser-down, but only if a previous tip stroke was still in schema 7.
Then the same close/reopen block after lift [inferred].

**Is the SF session paused during pen input (scheme 4 / A2 + delayed full GC16)?**

- PngNote never pauses at pen-down. It closes the session only after lift: at the point list for the
  eraser tool, at `onEndRawErasing` for the eraser end, after 2 s idle for the pen. So pen input
  normally lands in a **running** session (`GET_PEN_STATE == 2`). The framework's Entry A stylus
  exemption holds, and SF's scheme-3 guard would block a switch anyway [inferred via 07 §6].
- Pen input can land in the **closed window** if the next stroke starts while the cycle runs. There the
  pen state is 3/0/1, so Entry A would not exempt the stylus. But scheme 4 still needs a transient
  A2 from the framework:
  - Entry B is WebView-only, and PngNote has no WebView [code].
  - Entry A needs `topScrollBounds` for the window. The editor has no scrollable view: a Compose
    `Column` with a fixed `TopAppBar` and a `SurfaceView` in an `AndroidView`
    (`BookActivity.kt:296-427`). The scroll-capture search is therefore unlikely to find one
    [inferred]. Whether Compose 1.5.0 exposes a scroll-capture target is [unknown].
  - The main thread is blocked for the whole cycle, so a stylus `ACTION_DOWN` is delivered to the
    window only after step 9, when the SF state is already 2 again [inferred].
- **Prediction:** no `### EUS, s 4`, no A2 driver lines and no delayed full-screen update from
  PngNote's erase path. Strokes started inside a cycle are lost rather than flashing [inferred].
  Observation would need an SDM board where the calls reach SF (§1).
- **On the Air 4C as shipped:** SF gets no pen state from PngNote. Its reader stays in whatever
  state the last owner left, normally 0 after a clean close, so `GET_PEN_STATE ≠ 2` for every stylus
  down. If Entry A were live for this window, every vertical stroke would produce `EUS s 4` + A2 and a
  full-screen update about 1 s after lift. With no `topScrollBounds` expected, the predicted log is
  **empty** apart from repeated `E RawInputReader: Empty region detected when mapping!!!!!` at each
  `setLimitRect` [inferred; that logcat line is a cheap way to confirm §1 without touching SF].

## 8. Compared with Tangleaf

Observations only.

**Where PngNote's design is better or simpler**

1. **Render stays on for the eraser tool.** Tangleaf holds pen state 3 for the whole time its eraser
   tool is selected and draws the eraser in the WebView (08 §1 item 3). PngNote keeps state 2 and lets
   SF draw a live trace by making the "eraser" a white pencil. No pen-down ever lands in a paused
   session because of the eraser tool.
2. **No pause at pen-down, ever.** Tangleaf pauses at eraser-end down with the pen tool and at
   selection-menu pen-downs. PngNote's pauses/closes all happen after lift.
3. **`ENABLE_POST(1)` as a "leave schema 7 without pausing" lever.** At eraser-end begin PngNote sends
   only `ENABLE_POST(1)` (`CanvasBoox.kt:123`). By 07 §4.4 that ends the handwriting schema, SF
   frames flow again, and pen state stays 2, so the framework's stylus exemption is kept. Tangleaf
   uses `ENABLE_POST(1)` only as part of render-off and session cycles.
4. **Reconcile by leaving the schema, not by marking the frame.** Tangleaf gets its per-stroke
   reconcile through schema 7 with a `HAND_WRITING_REPAINT_MODE` (0x80002) whole-WebView frame
   (08 §1 item 5). PngNote exits schema 7 first (state 3 / `ENABLE_POST(1)`) and posts an ordinary
   frame. Both avoid the drop rule. PngNote's costs a 150 ms SF sleep per exit plus the reader cycle.
5. **Debounced repaint for writing.** Pen strokes are repainted once, 2 s after the last pen
   activity, not after every stroke. SF's ink stays on the panel meanwhile.
6. **No full GC anywhere, no transient A2, no WebView.** There are no flashes from app policy, and
   Entry B cannot fire. Tangleaf issues GCs after session close, keyboard hide and overlay close, and
   its WebView exposes it to Entry B (00 §5 items 12, 15).
7. **`SurfaceView` canvas.** App frames are buffers on their own layer. No View update mode, no
   WebView compositing, and the frame's content is exactly the page bitmap.

**Where it is worse or fragile**

1. **It does not run on the Air 4C at all** with its bundled SDK (§1). Every design point is moot on
   this tablet until the device SDK knows `lito` and `ViewUpdateHelper`.
2. **Full close/reopen per repaint** (3,3,0,1,2 plus an in-process reader restart), synchronous on
   the main thread, with strokes during it lost. Tangleaf also closes the session on undo/redo, page
   switch, leave and background (00 §5 item 1), but not on every stroke's reconcile.
3. **Inherited SF state.** It never sends exclude rects, region mode, brush or eraser raw-drawing
   flags, so it runs with whatever the previous pen owner left. Tangleaf resets the exclude table on
   every resume and sets both flags.
4. **No pause for focus loss, toasts or system overlays**, and the background close is probably
   deferred until the app returns (§5.10). Tangleaf closes on background and pauses on focus loss,
   IME and overlays.
5. **The eraser end has no live feedback** (SF eraser reader off, no app frame until lift). Tangleaf
   draws the eraser circle and erased canvas in JS during the stroke.
6. **The −40 px limit fudge** pushes the SF limit into the toolbar and leaves the canvas bottom
   unlimited [inferred].
7. **The white-pencil eraser** paints over the page template until the repaint. On colour panels the
   SF painter's handling of white is [unknown].

**Techniques worth studying (hardware questions, not proposals)**

- Does `ENABLE_POST(1)` at pen state 2 really give a schema exit with `GET_PEN_STATE` still 2, and
  does the next tip sample re-enter schema 7 cleanly (07 §4.4 predicts yes)?
- Does a tip stroke with `SET_STROKE_COLOR(white)` and a wide pencil give a usable live "erase" look
  on the Kaleido panel, and in which waveform?
- What waveform does SF use for a whole-surface buffer post outside schema 7, for a `SurfaceView`
  layer as opposed to a WebView invalidate?
- How much latency does a full close/open cost on this firmware (the 800 ms `stop`/`start` waits)?
  This bounds how a close-per-repaint design would feel.

## 9. Check of the earlier REPORT.md (PngNote rows)

| REPORT.md claim                                                                                          | verdict                                                                                                                                                         |
| -------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `TouchHelper.create(this, inputCallback)` at `CanvasBoox.kt:167`                                         | correct                                                                                                                                                         |
| limit = `getLocalVisibleRect` + `offset(0,-40)`; set from layout, `surfaceCreated`, resume/restart/focus | correct (`:153-161`, `:206-224`, `BookActivity.kt:169-185`)                                                                                                     |
| exclude always empty                                                                                     | correct, and the empty list is not even sent (`RawInputReader.java:332-335`), so SF's previous exclude table survives                                           |
| closes and reopens only when the size changed                                                            | correct for `applyRawDrawingSettings` (`:163-164, :187-189`). Incomplete overall: every repaint also closes/reopens through `withTempNoRawRendering`            |
| `:182-185` re-arms on focus, `:187-191` closes on `onStop`                                               | partly: `onStop` only bumps `closeCount`; the close runs in the Compose `update` block, probably deferred until restart (§5.10). The back arrow closes directly |
| "every surface repaint is wrapped in `withTempNoRawRendering`"                                           | correct for `syncBackendToSurface`. `clearSurfaceViewByBitmap` / `cleanSurfaceView` at open run with the session already closed                                 |
| SDK notes: two-argument `create` = `AppTouchRender` (said for notable)                                   | wrong for pen 1.4.1 on a stylus device: it is `hasStylus ? SFTouchRender : AppTouchRender` (`TouchHelper.java:176-190`)                                         |
| (not mentioned)                                                                                          | the device SDK 1.2.9 has no `lito` mapping and uses the pre-`ViewUpdateHelper` API, so PngNote is inert on the Air 4C (§1)                                      |

## 10. Open items

1. Confirm §1 on the tablet cheaply: install, open a note, look for
   `Empty region detected when mapping!!!!!` and the absence of any `Pid <png> State change` line.
   This needs no SF debug.
2. Whether Compose 1.5.0 pauses recomposition on `ON_STOP` in this app's setup (the §5.10 background
   prediction).
3. SF's default brush/eraser raw-drawing flags at boot, which PngNote relies on implicitly.
4. The waveform SF uses for a `SurfaceView` buffer post in scheme 2.
