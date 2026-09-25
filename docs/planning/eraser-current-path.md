# Tangleaf's eraser today, end to end

Written 2026-09-25 from static reading of `main` at `0f4dbee` (clean). No device was touched and
nothing was run. This is input for a plan that brings the eraser in line with stock Notes: render
on while erasing, SurfaceFlinger (SF) drawing the trace, stock's post-lift hand-back, and a
stock-like eraser menu. It proposes no code. Section 6 lists the places a change would have to
touch, and section 7 lists the risks.

Tags: **[code]** read in the cited source; **[inferred]** reasoned from code that was read;
**[doc]** taken from an earlier document and not re-verified here; **[unknown]** not settled
statically.

| prefix | location                                                                                               |
| ------ | ------------------------------------------------------------------------------------------------------ |
| `kt/`  | `plugins/tauri-plugin-mobile-system/android/src/main/java/dev/okhsunrog/mobile_system/`                |
| `kts/` | `plugins/tauri-plugin-mobile-system/android/src/test/java/dev/okhsunrog/mobile_system/`                |
| `hw/`  | `src/features/handwriting/`                                                                            |
| `PUB/` | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-pen-1.5.4.3/sources/com/onyx/android/sdk/pen/` (linked SDK) |
| `DEV/` | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-device-1.3.5.2/sources/com/onyx/android/sdk/` (linked SDK)  |
| `NA/`  | `~/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/` (stock Notes 45326, jadx)             |
| `RES/` | `~/tmp_zfs/boox-notes-inspect/decoded/resources/res/`                                                  |
| `SN/`  | `docs/onyx-reversing/stock-notes/`                                                                     |

The linked SDK versions come from `SN/08-tangleaf-current-path.md:27-31` [doc].

---

## 1. Summary

- **The eraser exists only in JS.** Native knows one bit about it: `eraser = tool === "eraser"`
  (`hw/onyx-ink.ts:193` → `kt/OnyxInk.kt:42`). The eraser **mode** (`stroke` / `pixel` / `lasso`) and
  the **radius** never reach native. `OnyxInkArgs` has no such fields (`kt/OnyxInk.kt:31-52`).
  The JS `args` object leaves them out (`hw/onyx-ink.ts:178-204`). `useOnyxInk` does not take
  them (`hw/onyx-ink.ts:46-85`), and `InkCanvas` does not pass them (`hw/ink-canvas.tsx:452-480`). The
  reconfigure effect does not depend on them (`hw/onyx-ink.ts:319-331`) [code].
- **SF is never told to draw an eraser.** The only eraser-related SF calls we make are
  `setEraserRawDrawingEnabled(false, 0)` once at helper creation (`kt/OnyxInk.kt:375`) and
  `resetPenDefaultRawDrawing()` = brush raw drawing **on**, eraser raw drawing **(false, 5)** on
  every resume (`kt/OnyxInk.kt:128` → `PUB/TouchHelper.java:348-351`) [code]. We never send
  `SET_STROKE_STYLE(8)`, `SET_STROKE_PARAMETERS(8, …)`, `ENABLE_COLOR_CU`, or a stand-alone
  `ENABLE_POST(1)`. `SET_STROKE_PARAMETERS(5, [5])` goes out only for the freehand lasso tool
  (`kt/OnyxInk.kt:381`) [code, grep of `kt/` for these calls]. With the eraser flag at 0, SF sends an
  eraser-end touch to `BaseEraseReader`, which paints nothing (`SN/03-erasing.md:138-148`) [doc].
- **Render is off for every erase gesture.** Eraser end: `InkEraserRenderGate.begin(true)` turns
  render off at pen-down (`kt/OnyxInk.kt:802`, `kt/InkEraserRenderGate.kt:8-15`). Eraser tool:
  `toolRendering()` is false, so render stays off the whole time the tool is selected
  (`kt/OnyxInk.kt:638-639`) [code]. Recorded on the tablet: pen state 3 at eraser-end down (T-A4,
  `SN/11-observations.md:75`) [doc].
- **JS draws the eraser.** Every rAF-coalesced preview (≤ one per 32 ms from native) runs
  `eraseGesture` on the preview strokes and republishes the whole canvas. The canvas shows a 2-unit
  black circle of radius `eraserRadius`, or a dashed polygon for the lasso eraser
  (`hw/ink-canvas.tsx:270-290, 389-397`; `kt/OnyxInk.kt:866-875`) [code].
- **Render comes back when the reconcile frame lands, not on a timer.** The gate comes back after
  `canvasReady` (commit) → `SOON` (120 ms) → `refreshFrame()` (whole-WebView invalidate in
  `HAND_WRITING_REPAINT_MODE`) → frame commit → `eraserRenderGate.framePresented()` →
  `restoreToolRendering()` (`kt/OnyxInk.kt:487-491, 521-567, 547, 641-643`) [code]. The hand-back is a
  render-on call alone. There is no re-push of rects, style, width or eraser flags, and for the eraser
  tool it is a no-op (render stays off) [code].
- **Current UI**: the eraser is one toolbar icon. A second tap opens a popover with
  `Stroke | Pixel | Lasso`, three sizes `6 / 12 / 24` (radius in sheet units, sheet = 1000 units
  wide; hidden for Lasso) and "Clear sheet". The defaults are stroke mode and radius 12
  (`hw/handwriting-note-view.tsx:184-185, 642-713`) [code].

---

## 2. The eraser UI today

### 2.1 Toolbar and popover (`hw/handwriting-note-view.tsx`)

| element           | behaviour                                                                                                                                                                                        | lines                   |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------- |
| tools             | `pen` (PenLine), `eraser` (lucide `Eraser`), `lasso` (Lasso)                                                                                                                                     | 86-90                   |
| state             | `tool` "pen"; `width` 3; `eraserMode` "stroke"; `eraserRadius` 12; `lassoMode` "free"; plain `useState`, not persisted                                                                           | 182-186                 |
| tool button       | first tap switches (`setTool`, and `setSelected([])` unless lasso); a tap on the **active** tool opens its options (`aria-label` becomes "`Eraser options`")                                     | 531-555                 |
| options container | one shared `Popover` anchored to the tapped button (`anchor={optionsAnchor}`, `align="center"`, `data-ink-tool-options`); closes on any `tool` change and when editing ends                      | 489-491, 611-617        |
| eraser mode       | three text buttons `Stroke`, `Pixel`, `Lasso` (`aria-label` "`Stroke eraser`" etc.), `variant` secondary when selected, each closes the popover                                                  | 642-664                 |
| eraser size       | group "Eraser size", only when mode ≠ lasso: `[6 Small, 12 Medium, 24 Large]`; the icon is a bordered circle `value` CSS px across (so the Medium icon is 12 px wide for a _radius_ of 12 units) | 665-697                 |
| clear             | "Clear sheet" (`Trash2`): `change({...draft, strokes: []})`, clears the selection and the limit flag; disabled when there are no strokes                                                         | 698-711                 |
| canvas cursor     | `cell` for the eraser tool                                                                                                                                                                       | `hw/ink-canvas.tsx:643` |

**Units** [code]. `eraserRadius` is a **radius in sheet units**. The sheet is 1000 × 1400 units drawn
with `setTransform(width/1000, 0, 0, height/1400, …)` on a `5/7` canvas, so a unit is the same size
on both axes (`hw/ink-canvas.tsx:177, 642`). `eraseGesture` erases where the distance to the
eraser path is below `radius + strokeWidth·(0.25 + 1.5·pressure)/2` (`hw/ink-editing.ts:338-343`).
How many millimetres a unit is depends on the sheet's on-screen width, which static reading cannot
settle [unknown].

**The popover as an overlay** [code]. `Popover` registers `useOverlayInkSuppression(open)`
(`src/components/ui/popover.tsx:8-21`). While it is open, the plugin's pause registry holds a reason
(`src/app/ink-suppression.ts:98-114` → `kt/MobileSystemPlugin.kt:181-195`) and `pauseStateChanged()`
runs a full `pause()`: render off, input off, `cycleScribbleSession()` = `leaveScribbleMode` +
`setScreenHandWritingPenState(3)` (`kt/OnyxInk.kt:295-300, 615-635, 675-680`). On close, the reason is
dropped 300 ms later (`ink-suppression.ts:18, 108`) and `resume()` follows after `RESUME_DELAY_MS`
= 500 ms on colour (`kt/OnyxInk.kt:229-230, 299`). A close that base-ui reports through
`onOpenChange(false)` also schedules one full GC 300 ms later (`src/app/eink-refresh.ts:37-60` →
`kt/MobileSystemPlugin.kt:341-356`, `EpdController.invalidate(webView, GC)`). The option buttons
close the popover with `setOptionsOpen(false)` directly (`hw/handwriting-note-view.tsx:659, 686, 707`).
Whether base-ui fires `onOpenChange` for that controlled close, and therefore whether the GC runs,
is **[inferred] no**.

### 2.2 Stock's eraser menu, for the plan (pointers only)

The user's screenshot (task brief): popover under the eraser button, title "Pixel Eraser 5.00mm",
three type icons (pixel, area, stroke), "Eraser Diameter" with `<` `5.00mm` `>` and a tapered
slider, toggles "Do Not Erase Shapes" and "Auto-deselect Eraser" (with info icon), buttons "Erase
Current Layer" and "Erase All Layers".

- Code: `NA/note/note/menu/popup/ErasePopup.java` (545 lines) binds `LayoutShapeBinding` =
  `RES/layout/layout_shape.xml` (per-size variants in `layout-sw*`). The ids appear in this order:
  `moveEraser`, `areaEraser`, `strokeEraser` (`:23, 49, 74`); `stroke_width_title` /
  `decrease_stroke_width` / `stroke_width_value` / `increase_stroke_width` / `seek_bar` (`:106-150`);
  `disable_erase_graph` (`:177-186`), `auto_deselect_eraser` (+ `_hint`, `_checkbox`) (`:209-228`);
  `eraser_current_layer`, `eraser_all_layer` (`:249-291`) [code].
- Strings: `move_eraser` = "Pixel Eraser", `area_eraser` = "**Lasso** Eraser", `stroke_eraser` =
  "Stroke Eraser", `eraser_stroke_width` = "Eraser Diameter", `disable_erase_graph`,
  `auto_deselect_eraser`, `erase_cur_layer`, `erase_all_layer`
  (`RES/values/strings.xml:1700, 191, 2997, 963, 731, 221, 957, 955`) [code]. Stock's three types map
  one to one onto ours: stroke ↔ type 0, pixel ↔ type 1 (`MOVE_ERASER`), lasso ↔ type 2 (`AREA_ERASER`)
  (`SN/03-erasing.md:103-110`) [doc].
- Width: a `WaterDropSeekBar` over a discrete list `eraseWidthValues`. `<`/`>` step the seek bar by one
  index (`ErasePopup.java:319-323, 375-409`). The mapping is `NoteUnits.widthPercentage(width, type)`
  (`:480-482`) [code]. The list's values and the percent→mm mapping were not read [unknown].
- Toggles: MMKV `isDisableGraphicalShapeErase` and `isAutoDeselectEraserEnabled`
  (`ErasePopup.java:100-103, 150-155`) [code]. Auto-deselect switches back to the previous provider
  after each erase (`SN/03-erasing.md:374-380`) [doc]. We have no shapes and no layers.
- mm → px in stock: `DimensKt.mmToPx(mm) = mm · densityDpi / 25.4` (`NA/sdk/extension/DimensKt.java:11-13`)
  [code]. The recorded `80.314964` equals `5.00 · 408 / 25.4` exactly, and the area eraser's
  `5.905512` equals `0.5 · 300 / 25.4` exactly [inferred, arithmetic]. Which DPI stock actually used,
  and whether 5.00 mm is the physical size on the panel, is [unknown].

---

## 3. What native gets and what reaches SF

### 3.1 Arguments (`configure_onyx_ink`, JS → `OnyxInkArgs` directly, no Rust layer)

JS builds `{session, enabled, left, top, width, height, clipTop, clipBottom, viewportWidth,
strokeWidth, eraser, interaction, fastLasso, hasSelection, selection*, overlayRects}`
(`hw/onyx-ink.ts:178-204`). `kt/MobileSystemPlugin.kt:143-178` parses it with
`invoke.parseArgs(OnyxInkArgs)` (`plugins/tauri-plugin-mobile-system/permissions/default.toml:5`
allows it) [code].

- `strokeWidth` is the **pen** width (`hw/onyx-ink.ts:192`). Native uses it for the SF pen width
  (`kt/OnyxInk.kt:388`), for the `width` of every emitted event (`:770`), and for the damage margin
  of every native point, erase points included (`markNativePoint`, `:708`) [code].
- The frame commit (`commit_onyx_frame`) carries `partial` and the JS damage bounds in sheet units
  (`hw/onyx-ink.ts:343-357`) [code].
- Events back to JS carry `erasing: erasing || args.eraser` (`kt/OnyxInk.kt:771`), so eraser-tool
  tip strokes arrive in JS as erasing [code].

### 3.2 Every SF-visible call on the eraser path (current code)

| where                                                      | call (SDK)                                                                               | SF transaction(s)                                                                                               | source                                                                                                                                                                          |
| ---------------------------------------------------------- | ---------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| helper creation (first configure)                          | `TouchHelper.create(webView, SF\|APP, cb, false)`, `openRawDrawing()`                    | SF render: `setStrokeStyle(0)`, pen state **1**; App render: `setStrokeStyle(0)`, `ENABLE_POST(0)`, pen state 1 | `kt/OnyxInk.kt:361-374`; `PUB/touch/SFTouchRender.java:265-268`; `PUB/touch/AppTouchRender.java:186-190`                                                                        |
| helper creation                                            | `setEraserRawDrawingEnabled(false, 0)`                                                   | `SET_ERASER_RAW_DRAWING_ENABLED(0, 0)`; the App render ignores it                                               | `kt/OnyxInk.kt:375`; `PUB/touch/SFTouchRender.java:292-294`; `PUB/touch/AppTouchRender.java:213-214`                                                                            |
| every configure, non-lasso tool (**eraser tool included**) | `setStrokeWidth(pen·sheetW/1000)`, `setStrokeColor(BLACK)`, `setStrokeStyle(FOUNTAIN=1)` | width, colour and style **1**, each via both renders                                                            | `kt/OnyxInk.kt:386-391`; `PUB/touch/AppTouchRender.java:133-135, 217-220`                                                                                                       |
| every configure, freehand lasso                            | `Device.setStrokeParameters(5, [5])`, width 5, black, style **5**                        | `SET_STROKE_PARAMETERS(5,[5])`, style 5                                                                         | `kt/OnyxInk.kt:377-385`                                                                                                                                                         |
| every `gate.resume`                                        | `setSingleRegionMode`, `setPenUpRefreshTimeMs(500)`, exclude reset, `setLimitRect`       | region mode, exclude `n=0`, limit                                                                               | `kt/OnyxInk.kt:117-127`                                                                                                                                                         |
| every `gate.resume`                                        | `resetPenDefaultRawDrawing()`                                                            | `SET_BRUSH_RAW_DRAWING_ENABLED(1)`, `SET_ERASER_RAW_DRAWING_ENABLED(0, 5)`                                      | `kt/OnyxInk.kt:128`; `PUB/TouchHelper.java:348-351`                                                                                                                             |
| render off (`setRender(false)`)                            | `setRawDrawingRenderEnabled(false)` (deduplicated on the cached flag)                    | SF render: `leaveScribbleMode` = `ENABLE_POST(1)`, pen state **3**; App render: `ENABLE_POST(1)`                | `kt/OnyxInk.kt:645-647`; `PUB/TouchHelper.java:188-197`; `PUB/touch/SFTouchRender.java:195-198`; `DEV/device/SDMDevice.java:1764-1766`; `PUB/touch/AppTouchRender.java:200-206` |
| render on                                                  | `setRawDrawingRenderEnabled(true)`                                                       | SF render: pen state **2**; App render: no transaction                                                          | `PUB/touch/SFTouchRender.java:201-203`; `PUB/EpdPenManager.java:39-41`                                                                                                          |
| reconcile frame                                            | view default mode `HAND_WRITING_REPAINT_MODE` (TRANSIENT layer), `webView.invalidate()`  | `setViewDefaultUpdateMode(…)`, one app frame; mode restored to the SESSION layer (GU) once committed            | `kt/OnyxInk.kt:138-145, 561-566, 553`; `kt/ViewDisplayMode.kt:31-38`                                                                                                            |
| full pause (menus, focus, keyboard)                        | `cycleScribbleSession()`                                                                 | `ENABLE_POST(1)`, pen state 3 (bypasses the SDK cache)                                                          | `kt/OnyxInk.kt:675-680`                                                                                                                                                         |

The render cache starts at `false` after `openRawDrawing()`. `openRawDrawing` does not touch it
(`PUB/TouchHelper.java:152-160, 358-361`), so a session opened with the eraser tool stays in pen
state 1 [code].

Calls we never make on this path [code, grep of `kt/`]: `SET_STROKE_STYLE(8)`,
`SET_STROKE_PARAMETERS(8, …)`, `SET_STROKE_PARAMETERS(1, …)`, `SET_ERASER_RAW_DRAWING_ENABLED(1, …)`,
`ENABLE_COLOR_CU`, `EpdController.enablePost(view, 1)` on its own, and
`EpdController.resetViewUpdateMode` (except where `ViewDisplayMode` restores the raw mode,
`kt/ViewDisplayMode.kt:46`). The linked SDK has all of them: `Device.setStrokeParameters`
(`DEV/device/SDMDevice.java:2842-2844`), `setEraserRawDrawingEnabled(bool,int)`
(`PUB/TouchHelper.java:332-338`), `EpdController.enableColorCU/disableColorCU` (`DEV/api/device/epd/EpdController.java:331-338`),
`enablePost(View,int)` (`:234`), `resetViewUpdateMode` (`:78`) [code].

---

## 4. The three paths, step by step

Pen callbacks come from the SDK reader thread. `SFTouchRender`'s wrapper posts each one to the host
view (`PUB/touch/SFTouchRender.java:40-137`), so every `OnyxInk` callback runs on the main thread,
one hop after the digitizer event [code]. `onPenUpRefresh` is posted again by us (`kt/OnyxInk.kt:887-891`).

### 4.1 (a) Eraser end, pen tool selected

**Tool selection (pen).** `configure()` → `pause(releaseDisplay=false, tearDown=false)` (render off,
input off, region `invalidateAll`, `webView.invalidate()`, `eraserRenderGate.reset()`); pen width,
colour and style 1; `resume()` → `gate.resume(render = toolRendering() = true)`: rects, defaults
(eraser flag **0,5**), input on, render on → pen state 2 (`kt/OnyxInk.kt:338, 386-392, 593-606,
615-635`; `kt/RawDrawingGate.kt:37-42`) [code].

**Pen-down** (native `onBeginRawErasing` → `begin(point, erasing=true)`, `kt/OnyxInk.kt:882, 777-816`) [code]:

1. Guards: config, input enabled, finite point. An unpaired previous gesture runs `end()` first
   (`:778-782`).
2. `movingSelection` is computed; there is no selection with the pen tool, because the tool switch
   cleared it (`hw/handwriting-note-view.tsx:549`). `swallowed = inkOverlayHit(...)`; there are no
   overlays without a selection menu (`:786-798`).
3. `fastPreview = false` (erasing) (`:799`).
4. `eraserRenderGate.begin(true)` → `setRender(false)` → **`ENABLE_POST(1)`, pen state 3,
   `ENABLE_POST(1)`**, `active = true` (`:802`, `kt/InkEraserRenderGate.kt:8-15`).
5. `gesture.begun()`; `reconcile.began(true)` → no pen-up refresh expected (`:804-805`,
   `kt/InkReconcileGate.kt:44-48`); `displayPolicy.begin(false)` (no transient mode) (`:809`).
6. `markNativePoint` (margin `max(3, 2·penWidth)` sheet units), cancel pending refresh/penUpWait and
   any frame submission (`:810-813`); `send("begin", erasing=true)` (`:815`).
7. The same contact reaches the WebView as `MotionEvent` with `TOOL_TYPE_ERASER`.
   `InkStylusRouting` consumes it when the session is active, it lands inside `limit` and it is not on
   an overlay (`kt/OnyxInk.kt:262-277`, `kt/InkStylusRouting.kt:32-46`). Whether this happens before
   or after the posted `begin` is [unknown]; the comment at `:268-270` assumes after.

JS (`hw/onyx-ink.ts:228-257`, `hw/ink-canvas.tsx:425-429, 330-398`) [code]: `gestureOpen = true`,
`traceRef = false`. `nativeInput` → `beginGesture(point, erasing=true)` → action **`erase`** →
`setGestureAction("erase")` → `sampleGesture` → stroke/pixel mode:
`preview = eraseGesture(preview, [point], mode, radius)` → `publish()`. The whole staging buffer is
copied to the visible canvas (`:324-326`), with no selection frame (`:224-229`) and a black circle
of radius `eraserRadius`, line width 2 units (`:283-290`). `onActiveChange(true)` → `busy` →
toolbar disabled (`hw/handwriting-note-view.tsx:234, 539`). `interacting` → any configure is
deferred (`hw/onyx-ink.ts:164-167`).

**Move** (`onRawErasingTouchPointMoveReceived` → `preview(point, true)`, `kt/OnyxInk.kt:884, 866-875`)
[code]: `markNativePoint`; throttled to one event per **32 ms**; `send("preview")`. JS keeps the
latest and handles it once per rAF (`hw/onyx-ink.ts:231-243`). `sampleGesture` → `eraseGesture` over
`[last, point]` → `publish()`. Each publish is a WebView frame in the SESSION mode (GU) while SF is in
pen state 3 [inferred]. No `commit_onyx_frame` happens during the move, because the commit effect
depends on `draft`/`frame`, which previews do not change (`hw/onyx-ink.ts:332-360`) [code].

**Lift** [code]. The SDK delivers the point list, then the end, and arms **no pen-up timer** for an
erase (`PUB/RawInputReader.java:601-616`, return at `:613-615`).

1. `stroke(list, true)` → `markNativePoint` per point, `sequence++`, `send("stroke", erasing=true)`
   (`kt/OnyxInk.kt:840-855`).
2. JS `onStroke = nativeStroke` → `finishGesture(base, event.points)` →
   `eraseGesture(base, sdkPoints, mode, radius)` on the **full SDK list** (the previews are thrown
   away), `onSelectionChange([])`, `endInteraction()`, `publish(strokes)` → `onChange` → writer
   (`hw/ink-canvas.tsx:439-444, 399-424`; `hw/onyx-ink.ts:258-281`); `setFrame`.
3. `end()` → `send("end")`, `cancelFrameSubmission()`, safety net `postDelayed(refresh, 750 ms)`
   (`kt/OnyxInk.kt:818-838`). JS: `gestureOpen=false`, `onActiveChange(false)`, `setFrame`, and any
   deferred configure runs now (`hw/onyx-ink.ts:250-257`).

**Reconcile** [code]. The commit effect runs `commit_onyx_frame(partial, damage)` in the next rAF
(`hw/onyx-ink.ts:343-357`). Damage is the union of changed-ink bounds and the previous/current
overlay, and each erase point is padded by `eraserRadius + 2` (`hw/ink-canvas.tsx:307-322`).
`commit()` adds that damage to `region`, then `frames.request()` and `postVisualStateCallback`
(`kt/OnyxInk.kt:473-494`). On completion `reconcile.canvasReady()` returns `SOON` →
`postDelayed(refresh, 120 ms)` (`:487-491`, `kt/InkReconcileGate.kt:70-75`). `refreshFrame()` then
runs (`kt/OnyxInk.kt:521-567`): `canPresent()`, `region.take(limit)`, `reconciled()`, TRANSIENT
layer = `HAND_WRITING_REPAINT_MODE`, frame-commit callback, `webView.invalidate()` (the whole
WebView).

**Resume** [code]. On the frame-commit callback (posted to main), `frames.present(...)` →
`eraserRenderGate.framePresented()` → `restoreToolRendering()` → `setRender(true)` → **pen state 2**.
Then `repaintMode.release()` → SESSION mode back (`kt/OnyxInk.kt:530-555, 641-643`). Nothing else is
re-sent. If the frame never presents (a pause, lost focus), `pause()` resets the gate _without_
turning render on (`:632`, `kt/InkEraserRenderGate.kt:23-25`), and render returns with the next
`resume()`. A new tip pen-down before that restores render at once (`InkEraserRenderGate.begin(false)`
→ `framePresented()`, `kt/InkEraserRenderGate.kt:9-10`).

### 4.2 (b) Eraser tool, pen tip

**Tool selection** [code]. `setTool("eraser")` → the effect runs `configure()` (`hw/onyx-ink.ts:319-331`)
with `eraser: true`, `interaction: false`. Native (`kt/OnyxInk.kt:302-404`) does
`pause(releaseDisplay=false, tearDown=false)` → render off (`ENABLE_POST(1)`, 3, `ENABLE_POST(1)` if
it was on), input off, `webView.invalidate()`, `send("cancel")` if a gesture was open. It then sends
pen width, colour and **style 1** (the non-lasso branch, `:386-391`). `resume()` →
`gate.resume(false)`: rects, defaults (eraser **0,5**, brush 1), input on, render stays off
(deduplicated). The result is a **steady pen state 3** for as long as the tool is selected, or state
1 if the session was opened with it (§3.2). The tool change also bumps the commit effect (`tool` is a
dependency, `hw/onyx-ink.ts:360`) and republishes (`hw/ink-canvas.tsx:494-512`), which gives one
reconcile frame [code].

**Pen-down** (`onBeginRawDrawing` → `begin(point, false)`) [code]: as in (a), with these
differences. `eraserRenderGate.begin(erasing || args.eraser = true)` → `setRender(false)` is a no-op
(already false), and `active = true`. `reconcile.began(true)`. The emitted event has
`erasing: true` (`kt/OnyxInk.kt:771`). The MotionEvent is `TOOL_TYPE_STYLUS` and is consumed by
`InkStylusRouting` on the sheet. JS: `tool !== "pen"` → `beginGesture(..., erasing=true)` → action
`erase` (`hw/ink-canvas.tsx:427-428`).

**Move** [code]: `preview(point, false)` passes the gate because `config.eraser` is true
(`kt/OnyxInk.kt:869`). The JS drawing is the same as in (a).

**Lift** [code]: `stroke(list, false)` → event `erasing: true`; `end()`. To the SDK this was a
**drawing** stroke, so it arms the 500 ms pen-up timer, and `onPenUpRefresh` arrives about 500 ms
later (`PUB/RawInputReader.java:613-616`; `kt/OnyxInk.kt:124, 501-512`). `penUpRefresh()` adds its
rectangle to `region` and marks `penUpRefreshSeen`. If the canvas is ready at that moment the
reconcile runs `NOW`. Otherwise the rectangle waits for the next commit. Which comes first on the
device, the 120 ms `SOON` or the pen-up refresh, is [unknown] (`SN/08-tangleaf-current-path.md:437-444`)
[doc].

**Reconcile / resume** [code]: same reconcile. `framePresented()` → `restoreToolRendering()` →
`setRender(false)` → no-op. **SF gets no transaction per gesture** in the eraser tool.

### 4.3 (c) Lasso eraser (`eraserMode === "lasso"`)

The lasso eraser is a JS mode only. Native runs (a) or (b) unchanged, depending on which end is used
[code]. JS differences (`hw/ink-canvas.tsx`) [code]:

- During the gesture `sampleGesture` skips `eraseGesture` (the `eraserMode !== "lasso"` branch) and
  only republishes (`:389-397`). `publish` draws the closed **dashed polygon**
  `lassoPolygon(points, "free")`, line width 3 units, dash `[7,7]` (`:269-282`;
  `hw/ink-editing.ts:141-144`). No circle is drawn (`:285`).
- Damage still pads every point by `eraserRadius + 2`, although the radius means nothing to this
  mode (`:307-316`).
- At lift `eraseGesture(strokes, points, "lasso")` → `selectLasso(strokes, simplifyGesture(points))`,
  which removes every stroke with a point inside the polygon or a segment crossing it
  (`hw/ink-editing.ts:309-313, 97-115`).
- The size group is hidden for this mode (`hw/handwriting-note-view.tsx:665`).

**Eraser end while the lasso tool is selected** [code]. JS erases here too, because `erasing` forces
action `erase` whatever the tool (`hw/ink-canvas.tsx:339-340`). Stock does not erase in its selection
tool (`SN/03-erasing.md:467-479`) [doc]. Native: with no selection and freehand mode, render is on,
and `begin` turns it off (`kt/OnyxInk.kt:802-803`). With a selection, render is already off. If the
eraser lands in the selection grab box (`movingSelection`, `:786-787`), `displayPolicy.begin(true)`
requests `APPLY_TRANSIENT_UPDATE(ANIMATION_QUALITY)` and arms the 5 s settle, because `:809` has no
erasing guard (`kt/OnyxInk.kt:149-154`, `kt/InkRefreshPolicy.kt:11-17`). `finishGesture` clears the
selection (`hw/ink-canvas.tsx:412`), so a deferred `configure` (pause/resume, re-push) follows the lift.

### 4.4 Non-native pointer path (desktop, mouse, non-BOOX)

`start()` treats a pen/mouse pointer as erasing when `tool === "eraser"`, `button === 5` or
`buttons & 32` (`hw/ink-canvas.tsx:590-612`, `:604`). From there it follows the same
`beginGesture`/`sampleGesture`/`finishGesture` as above [code]. With the BOOX SDK active the pen
never takes this path (`onyx && pointerType === "pen"` returns, `:595`) [code].

---

## 5. Side by side with stock (recorded 2026-09-25)

| step                       | stock (task brief; `SN/11-observations.md:47-58`)                                                                                                                                                          | Tangleaf now                                                                                                 |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| eraser tool selected       | main style **8**, `SET_STROKE_PARAMETERS(8,[80.314964,0.5,0.1])` (pixel, 5.00 mm), or style **5** with `(8,[5.905512,0.5,0.1])` (area); `SET_ERASER_RAW_DRAWING_ENABLED(1,8\|5)`; brush 1; pen state **2** | style **1** + pen width; eraser flag **(0,5)**; brush 1; pen state **3** (or 1) (§4.2)                       |
| eraser end, pen tool, down | render stays on (2); SF paints the trace; `ENABLE_COLOR_CU(1)`                                                                                                                                             | `ENABLE_POST(1)`, pen state 3, `ENABLE_POST(1)` after one main-thread hop; SF paints nothing (flag 0); no CU |
| during                     | no transaction; app draws nothing                                                                                                                                                                          | JS circle / polygon + erased preview, one full canvas publish per rAF (≤ 32 ms input)                        |
| lift                       | `ENABLE_COLOR_CU(0)`; `resetViewUpdateMode`; `ENABLE_POST(-1,1,pid)` while running (`Post enabled!`, scheme 2)                                                                                             | nothing at lift; commit → 120 ms                                                                             |
| +~170 ms                   | render off (pen state 3), one partial update `waveform[2] update_mode[0]` of the erased area                                                                                                               | reconcile frame in `HAND_WRITING_REPAINT_MODE` (whole WebView invalidate)                                    |
| +~340 ms                   | full re-send: limit, exclude, region mode, colour, style, width, stroke parameters 1/5/8, eraser raw drawing (1, 8\|5), brush 1, then pen state 2                                                          | on frame commit: render on (pen state 2), no re-send; eraser tool: render stays off                          |

---

## 6. What would have to change to reach stock's behaviour

Grouped by concern. Each item names the current code that owns it. Order does not matter here; the
plan decides it.

**6.1 Carry the eraser setting to native**

1. `OnyxInkArgs`: add the eraser type (stroke/pixel/lasso) and the SF eraser **diameter in view
   px**, or the radius in sheet units for native to convert (`kt/OnyxInk.kt:31-52`). Validate them in
   `configure` next to `strokeWidth` (`:309-313`).
2. `onlyOverlaysChanged` must compare the new fields. Otherwise an overlay and eraser change arriving
   together is taken as overlay-only and nothing is pushed (`kt/OnyxInk.kt:407-416`).
3. JS: add the fields to the `args` object (`hw/onyx-ink.ts:178-204`), to the `useOnyxInk`
   parameters and the `state` ref (`:46-126`), and to the reconfigure deps (`:319-331`). Pass
   `eraserMode`/`eraserRadius` from `InkCanvas` (`hw/ink-canvas.tsx:452-480`).
4. Units: the SF parameter is in panel/view px [inferred from stock's `mmToPx`]. Sheet units → view
   px is `sheet.width()/1000` natively (`kt/OnyxInk.kt:345-346`). mm → px needs a DPI source.
   Stock's formula is `densityDpi/25.4` (§2.2). The UI switch to mm and its step list is a UI
   decision (§6.6).

**6.2 Configure SF the way `ResumeRawDrawingRequest` does**

5. Replace `resetDefaults()` = `resetPenDefaultRawDrawing()` (eraser **0,5**) in the gate
   (`kt/OnyxInk.kt:128`, `kt/RawDrawingGate.kt:11-16, 37-42`) with a push of
   `setStrokeParameters(5,[5.0])`, `setStrokeParameters(8,[diameterPx,0.5,0.1])`,
   `setEraserRawDrawingEnabled(true, lasso ? 5 : 8)` and `setBrushRawDrawingEnabled(true)`.
   `NA/note/note/request/pen/ResumeRawDrawingRequest.java:52-55, 61-65` has the exact values [code].
   The initial `setEraserRawDrawingEnabled(false, 0)` at helper creation (`kt/OnyxInk.kt:375`) goes too.
6. Tool style in `configure` (`kt/OnyxInk.kt:377-391`): the eraser tool needs main style **8**, or
   **5** for the lasso eraser, instead of FOUNTAIN + pen width. The pen tool keeps style 1 + width.
   Stock also sends pen parameters for style 1 (`(1,[0.3,0.6])`, `SN/11-observations.md:37-38`) [doc];
   we send none.
7. This style/width/colour/parameter push exists only inside `configure` today. `gate.resume()` does
   not repeat it (`kt/RawDrawingGate.kt:37-42`). Stock repeats all of it on every resume. It has to
   become a function that the resume path and the post-erase hand-back can call (see 6.4).

**6.3 Render on while erasing**

8. `toolRendering()` (`kt/OnyxInk.kt:638-639`): the eraser tool must render (`config.eraser` must
   stop forcing false).
9. `begin()` (`kt/OnyxInk.kt:802`): stop `eraserRenderGate.begin(erasing || args.eraser)` from
   pausing at pen-down. `InkEraserRenderGate` (`kt/InkEraserRenderGate.kt`) changes meaning, from "paused
   since pen-down until frame" to "hand-back in progress after lift". Its callers are
   `kt/OnyxInk.kt:132-135, 547, 632, 802`.
10. Lasso tool + eraser end: `setRender(fastPreview)` for interaction tools (`kt/OnyxInk.kt:803`)
    forces render off for an erase, because `fastPreview` is false when erasing (`:799`). It needs an
    erasing exception. With a selection shown, render is off by design (`toolRendering`,
    `:638-639`), so decide whether the eraser end should erase at all there (stock: no, §4.3).
11. `ENABLE_COLOR_CU(1)` at erase pen-down and `(0)` at lift (stock, §5):
    `EpdController.enableColorCU()/disableColorCU()` (`DEV/api/device/epd/EpdController.java:331-338`)
    from `begin`/`stroke` or `end` (`kt/OnyxInk.kt:777-855`). What CU does is [unknown]
    (`SN/03-erasing.md:532`).
12. JS must stop drawing during the gesture. Drop the circle (`hw/ink-canvas.tsx:285-290`), the
    erase polygon (`:270-282`) and the per-preview `eraseGesture` + `publish` (`:389-397`) on the
    native path. The pointer path keeps them (§4.4). The existing switch is `Gesture.nativeFast`, which
    makes `sampleGesture` return early (`:375`). It is fed from `event.fastPreview` (`:428`), and that
    is false for erasing (`kt/OnyxInk.kt:799`). Note that `fastPreview` also mutes native previews
    (`:869`) and sets `traceRef`, which holds the commit (`hw/onyx-ink.ts:247, 339`). Reuse it
    deliberately or add a separate flag.

**6.4 Stock's post-lift hand-back**

13. New sequence, native side, after an erase gesture ends. Stock's order:
    `resetViewUpdateMode` → `ENABLE_POST(1)` while running (`EpdController.enablePost(webView, 1)`,
    `DEV/api/device/epd/EpdController.java:234`) → about 170 ms → render off (`setRender(false)`: SF
    `ENABLE_POST(1)` + pen state 3, App `ENABLE_POST(1)`) with input left **on** (unlike
    `gate.pause()`, `kt/RawDrawingGate.kt:31-34`) → our erased frame → **300 ms**
    (`EraseFinishAction.E()`, `SN/03-erasing.md:368-371`) [doc] → full re-send (6.2) → render on. The
    natural hooks are `end()` / `stroke()` (`kt/OnyxInk.kt:818-855`) and `commit()` → `refreshFrame()` →
    `framePresented` (`:473-494, 521-567, 547`).
14. The erased frame's mode. Stock's frame is a plain `View.invalidate()` after `resetViewUpdateMode`,
    and was recorded as `waveform[2] update_mode[0]`. Ours is always marked `HAND_WRITING_REPAINT_MODE`
    (`kt/OnyxInk.kt:138-145, 561`). The handoff lists "skip the handwriting repaint mode for erase
    reconciles" as untested (`docs/planning/pen-research-handoff.md:37-42`) [doc]. Do not call
    `EpdController.resetViewUpdateMode` directly: `DisplayModeStack` owns the view mode
    (`kt/DisplayModeStack.kt:5-26`). Clearing or skipping the TRANSIENT layer is the equivalent.
15. Only send the stand-alone `ENABLE_POST(1)` when the erase changed something? Stock sends it only
    when a visible shape changed (`SN/03-erasing.md:357-360`) [doc]; the pause/resume runs either way.
    Our JS knows whether the draft changed (`hw/ink-canvas.tsx:423`); native does not, today.
16. The re-send must restore the **pen's own style**. SF has a single style slot, and
    `EraseReader::onActivate` writes the eraser style into it without restoring it
    (`SN/03-erasing.md:149-152, 631-635`) [doc]. Without the re-send, the next tip stroke after an
    eraser-end erase paints in style 8. This is the "wide faded trace / mode did not exit" failure of
    the earlier attempt (`docs/planning/pen-research-handoff.md:37-40`, `SN/03-erasing.md:637-651`)
    [doc].

**6.5 Region, reconcile and events**

17. `markNativePoint` margin (`kt/OnyxInk.kt:704-710`) uses `2·penWidth`, at least 3 units, for erase
    points. Once SF paints a trace of `diameterPx`, the reconcile region should cover at least
    `diameterPx/2` around each point (see 7.3).
18. Eraser-tool tip strokes arm the SDK pen-up timer (§4.2). A pen-up refresh about 500 ms after lift
    can trigger a second `refreshFrame()` (`kt/OnyxInk.kt:501-512`) around the time of the 300 ms
    re-send. Stock's consumer for it in the eraser tool is empty (`SN/03-erasing.md:320-327`) [doc].
    `reconcile.began(erasing || args.eraser)` (`:805`) already stops the reconcile from waiting for it.
19. `applyOnyxStroke`'s erasing branch uses `eraseAt` with a fixed radius of 12 in whole-stroke mode
    (`hw/onyx-ink.ts:29-31`, `hw/ink-model.ts:40-49`). `InkCanvas` always supplies `onStroke`, so this
    branch is dead in the app and only reached by tests [code]. Leave it or remove it.

**6.6 UI**

20. Replace the text buttons and the three radii (`hw/handwriting-note-view.tsx:642-713`) with the
    stock layout: a title "<Type> Eraser <d>mm", three type icons (pixel, lasso, stroke, in stock's order
    `layout_shape.xml:23, 49, 74`), a diameter row with `<`/value/`>` and a slider, the two toggles
    (auto-deselect is meaningful for us; "Do Not Erase Shapes" has no shapes to act on), and
    "Clear sheet" in place of the layer buttons. The value becomes a **diameter in mm**, where today it
    is a radius in sheet units, so `eraseGesture`'s `radius` has to be derived from it
    (`hw/ink-editing.ts:302-307`).
21. The popover already opens under the tool button on the second tap (`:540-547`) and already holds
    the pen through the overlay pause (§2.1). Stock's menu close is a `REGAL_PLUS` partial repaint
    (`SN/11-observations.md:60-64`) [doc], where ours is a GC after `onOpenChange(false)` (§2.1). That
    is a separate decision.
22. Persisting the eraser settings (stock keeps them in MMKV) is new; ours are component state
    (`hw/handwriting-note-view.tsx:184-185`).

---

## 7. Risks

**7.1 Frame fence** (`kt/InkFrameFence.kt`). `canSubmit`/`present` refuse while `gesture.drawing`
(`:25-35`). A pen-down cancels a dispatched frame (`kts/InkFrameFenceTest.kt:29`). A post-lift
hand-back that waits on the erased frame inherits this: a new stroke inside the hand-back window
cancels the frame, and nothing then drives render back on unless a timer or the pen-down path does.
Today `InkEraserRenderGate.begin(false)` is that path (`kt/InkEraserRenderGate.kt:9-10`). CLAUDE.md
requires a device retest for any frame-fence or eraser-gate change.

**7.2 Reconcile gate** (`kt/InkReconcileGate.kt`). Erases never wait for a pen-up refresh (`:44-48`,
`kts/InkReconcileGateTest.kt:41-48`). The hand-back's timing then rests on `canvasReady` → 120 ms,
while stock's is about 170 ms to the frame and about 340 ms to the re-send. If the hand-back instead
starts at `end()`, the JS erased frame may not exist yet: the commit comes one rAF after the stroke
event (`hw/onyx-ink.ts:343`). Render-off before the frame is harmless; a re-send and render-on before
the frame is presented would leave the SF trace in place until the next reconcile [inferred]. The
750 ms net in `end()` (`kt/OnyxInk.kt:837`) and the pen-up refresh of eraser-tool tip strokes (6.5
item 18) are extra `refreshFrame()` triggers that can land inside or after the hand-back.

**7.3 Damage region.** `refreshFrame` always invalidates the **whole WebView** (`kt/OnyxInk.kt:566`).
`region` only decides whether a reconcile runs at all (`take` → null → return, `:525-527`) and feeds
diagnostics. So the SF trace is covered as long as some reconcile runs [code for the call; that the
panel update then covers the trace is inferred]. Two things to keep: (1) the erase has to produce a
reconcile even when it erased nothing. Today it does: the JS `frame` bump commits on every end
(`hw/onyx-ink.ts:252, 281`), and `publish` redraws the whole canvas (`hw/ink-canvas.tsx:324-326`).
(2) If the plan narrows the invalidate to the region, the margin must grow to the eraser half-width
(6.5 item 17). Stock's own partial update covers "the erased area" (`SN/11-observations.md:51`)
[doc], and with the stroke eraser that can be much larger than the path
(`docs/planning/eink-panel-upkeep.md:1005-1006`) [doc].

**7.4 SF compositing while the session runs.** While SF is in the handwriting schema, app frames
over the region are not composited, apart from mode-marked ones (`SN/03-erasing.md:648-651`,
`SN/00-overview.md:449-452`) [doc]. JS must therefore not rely on showing anything during the gesture
(6.3 item 12). The erased frame has to follow the `ENABLE_POST(1)` / pen state 3, or the pale-sheet
symptom returns (`SN/03-erasing.md:648-651`) [doc].

**7.5 Stylus routing** (`kt/InkStylusRouting.kt`). It is independent of render state:
`active = helper && resumed && focus && !paused && !limit.isEmpty` (`kt/OnyxInk.kt:271`). Erase
gestures keep being consumed with render on. It must stay. Within the hand-back window the session is
paused, and a stylus MotionEvent reaching the WebView there would set off the framework's scroll-A2 +
full-screen GC path (`SN/11-observations.md:68-82`) [doc]. It is also what fixed the post-erase
flash while render is off (F6-A4, `SN/11-observations.md:84-88`) [doc].

**7.6 Style slot and SF globals.** SF keeps the eraser/brush flags and the style slot as process
globals, not per session (`SN/03-erasing.md:175-178, 631-635`) [doc]. Every pause path of ours that
resumes through `gate.resume()` has to leave SF with the eraser flag and the tool style that match the
current tool: menus (`kt/OnyxInk.kt:295-300`), focus (`:585-591`), `onResume` (`:584`),
configure (`:338-392`). Today configure is the only place that sets the style. So a resume after
another app, or after an eraser-end erase, that does not re-push leaves SF with whatever was last set
[inferred].

**7.7 Lasso trace.** The lasso eraser in SF is style 5 with `[5.0]`, the same painter and value as
our lasso tool (`kt/OnyxInk.kt:179, 381`). The JS dashed polygon for the erase (`hw/ink-canvas.tsx:270-282`)
then becomes a duplicate that SF does not composite [inferred]. For the lasso **tool**, `traceRef`
holds commits while an SF trace is on screen (`hw/onyx-ink.ts:128-129, 247, 334-339`). An SF-drawn
erase trace needs the same protection only if a commit could otherwise run mid-gesture. Today none
can (no draft/frame change during an erase), but the rule is implicit.

**7.8 Selection.** An erase always clears the selection (`hw/ink-canvas.tsx:412`), which in the lasso
tool changes `hasSelection` and triggers a deferred full `configure` (pause/resume) right after the
lift (`hw/onyx-ink.ts:254-257`). That configure would collide with a hand-back in flight: `pause()`
resets the eraser gate (`kt/OnyxInk.kt:632`) and cancels frame submission (`:621`). The eraser in the
grab box starts a transient ANIMATION_QUALITY request (§4.3). Both need an explicit rule.

**7.9 Tool switch and menus during the hand-back.** `configure()` and `pauseStateChanged()` call
`pause()`, which currently cancels everything and resets the gate (`kt/OnyxInk.kt:615-635`). A new
hand-back timer must be removed there too (alongside `refresh`, `penUpWait`, `resumeGate`,
`:617-619`), and it must never turn render on while `pauses.isPaused`. `restoreToolRendering` already
checks this (`:641-643`).

**7.10 Pen state 1 on a fresh session with the eraser tool.** Today the eraser tool can leave SF at
pen state 1 (START) (§3.2). With the eraser tool rendering, the first resume moves it to 2. Session
open behaviour changes with it.

---

## 8. Tests that cover the eraser today

| test                                                      | covers                                                                                                                                                | touched by the change                                              |
| --------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `kts/InkEraserRenderGateTest.kt:7-18`, `:20-31`           | pause once per erase run, resume on frame; pen-down resumes; `reset` never resumes                                                                    | yes; the gate's meaning changes (6.3 item 9)                       |
| `kts/RawDrawingGateTest.kt:14-31`                         | resume order `rects, defaults, input, render`; pause `render, input`                                                                                  | yes, if `resetDefaults` becomes the eraser/brush push (6.2 item 5) |
| `kts/InkReconcileGateTest.kt:41-48`                       | an erase never waits for a pen-up refresh                                                                                                             | maybe (eraser-tool pen-up refresh, 6.5 item 18)                    |
| `kts/InkFrameFenceTest.kt:29`                             | a pen-down cancels a dispatched frame                                                                                                                 | behaviour relied on by the hand-back                               |
| `kts/InkStylusRoutingTest.kt:21-72`                       | stylus consumption on the sheet; tool-agnostic (`stylus` boolean)                                                                                     | no                                                                 |
| `hw/ink-editing.test.ts:25-49, 69-75`                     | `eraseGesture` pixel split, stroke, dots, lasso, broad phase (radius 9 / 12)                                                                          | if the radius becomes a mm-derived value                           |
| `hw/ink-model.test.ts:16-20`, `hw/onyx-ink.test.ts:24-40` | `eraseAt` / `applyOnyxStroke` erasing (the path the app does not use, 6.5 item 19)                                                                    | only if that path is removed                                       |
| `hw/ink-canvas.test.tsx:219-244`                          | pointer eraser (button 5 / buttons 32), pixel mode, radius 9, cancel leaves ink                                                                       | if the props change shape                                          |
| `hw/onyx-ink-session.test.tsx:159-220`                    | native erasing with the pen tool (`erasing:true` begin/preview/stroke/end), `EditorSheet` fixed `eraserMode="pixel"`, `eraserRadius={9}` (`:147-148`) | yes, once previews stop drawing / args grow                        |
| `hw/onyx-ink-session.test.tsx:336-366`                    | erasing starts from ink React has not rendered yet                                                                                                    | no                                                                 |
| `hw/onyx-ink-session.test.tsx:80-85`                      | configure args (`toMatchObject`, so extra fields pass)                                                                                                | no; add assertions for the new fields                              |
| `hw/handwriting-note-view.test.tsx:457-479`               | first tap switches, second tap opens "Eraser options", "Pixel eraser" closes the popover                                                              | yes, with the new menu                                             |

Nothing tests `OnyxInk` itself (no `OnyxInkTest` in `kts/`), the arguments that reach native for the
eraser, or any SF call order on the erase path [code, directory listing]. The pure helpers
(`RawDrawingGate`, `InkEraserRenderGate`, `InkReconcileGate`) are where ordering rules can be unit
tested without a device.

---

## 9. Unknown statically

- The sheet's width in view px on the tablet, and so what our radii 6/12/24 are in mm or in SF px.
- Which DPI stock's `80.314964` came from, and the physical size of "5.00mm" (§2.2).
- The order of the posted raw `begin` and the WebView's MotionEvent down (§4.1 step 7).
- Whether `SOON` (120 ms) or the pen-up refresh (about 500 ms) wins for eraser-tool strokes (§4.2).
- What `ENABLE_COLOR_CU` changes on the panel (`SN/03-erasing.md:532`).
- Whether a pen stroke that starts inside stock's roughly 300 ms paused window flashes
  (`SN/03-erasing.md` E4). Stock's recorded window was 270–620 ms, and flipping the pen took
  0.86–1.4 s (`SN/11-observations.md:52-53`). The eraser **tool** allows a tip stroke within
  milliseconds.
- Stock's `eraseWidthValues` list and `NoteUnits.widthPercentage` (§2.2).
