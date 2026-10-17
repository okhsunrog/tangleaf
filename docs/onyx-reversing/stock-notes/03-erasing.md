# Stock Notes: erasing in every mode

How the stock Onyx Notes app (`com.onyx.android.note`, "knote2") erases on a BOOX Note Air 4C, traced from
the app down through its bundled Onyx SDK, the patched framework, and SurfaceFlinger. Static analysis only;
nothing here was observed on hardware unless it is quoted from `docs/planning/pen-research-handoff.md`.

Claim tags: **[code]** read directly in source or disassembly; **[inferred]** reasoned from code that was
read, not directly shown; **[unknown]** not settled statically.

Path prefixes used below:

| prefix    | location                                                                                                                                               |
| --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `NA/`     | `~/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/` (stock Notes, jadx; `note/…` is app code, `sdk/…` is the SDK copy bundled in the APK) |
| `RAW/`    | `~/tmp_zfs/boox-notes-inspect/decoded/resources/res/raw/`                                                                                              |
| `FW/`     | `~/tmp_zfs/onyx_framework/out/sources/` (framework.jar)                                                                                                |
| `SF@0x…`  | virtual address in `~/tmp_zfs/onyx_framework/native/surfaceflinger`, names from its `.gnu_debugdata` symbols                                           |
| `PTR@0x…` | address in `~/tmp_zfs/onyx_framework/native/libonyx_pen_touch_reader.so` (the in-process pen reader)                                                   |
| `PUB/`    | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-pen-1.5.4.3/sources/com/onyx/android/sdk/` (public SDK, for comparison)                                     |

Jadx renames private methods to single letters. Where the Kotlin `@Metadata` `d2` table gives the original
name, it is written after the jadx name, e.g. `ScribbleHandler.a()` (`beginDrawErasing`). Those name pairings
are **[inferred]** from the metadata list; the bodies are **[code]**.

Version note: the tablet runs firmware 4.2 (build 8548), system Android 13, vendor Android 11, and its
SurfaceFlinger is byte-identical to the analysed binary, so every `SF@` claim applies to the device (verified
2026-09-25, see [00-overview §0](00-overview.md)). The "Android 11 / firmware 4.1" in the original research
brief was wrong.

---

## 1. Summary

- **Stock erases with the pen session running, not paused.** For all three stroke-level eraser types the
  app keeps raw-drawing render **on** (pen state 2) for the whole gesture, whether the eraser is the eraser
  tool (pen tip) or the stylus's eraser end. `EraseHandler.canPenRawRender()` returns true for eraser types
  0, 1 and 2 [code], `EraseBeginAction` pauses render only for eraser types that are _not_ raw-rendered, and no
  such type exists in the menu [code]. The only render pause in the erase cycle happens **after pen-up**, in
  `EraseFinishAction` → `EraseFinishEvent` → `InvalidateViewWithPenControlAction` [code].
  This holds for the pen tool (`EpdShapeHandler`), the geometric-shape tool (`NormalShapeHandler`) and the eraser
  tool. It does **not** hold for every tool that erases: the fill-colour tool (`FillColorHandler`,
  `canPenRawRender()=false`) erases with the eraser end while render stays disabled, and the universal-shape tool
  (`UniversalShapeHandler`) turns render on only at eraser pen-down (§3.7) [code for the app side; the SF state
  during those gestures is inferred]. Tools whose `useDrawErase()` is false (selection, HWR, zoom selection, text
  input, span, layer lock) do not erase at all (§3.7) [code].
- **The firmware draws the live eraser trace, not the app.** Before any erase is possible, stock's
  `ResumeRawDrawingRequest` sends `setStrokeParameters(8, [eraserWidth, 0.5, 0.1])`,
  `setStrokeParameters(5, [5.0])` and `setEraserRawDrawingEnabled(true, 8 or 5)` to SurfaceFlinger, and with
  the eraser tool also `setStrokeStyle(8)` (or 5 for the area eraser) [code]. SurfaceFlinger's pen reader then
  routes an eraser-end touch to its `EraseReader`, which switches its painter to style 8
  (`PainterSoftEraser`) or 5 (`PainterDash`) and paints the gesture through the same handwriting path as ink
  [code]. The app itself draws nothing on screen while the pen is down for these types [code].
- **The app does the real erase in the background** on 20 ms (stroke eraser) / 30 ms (partial and area
  eraser) batches of points on the Note Air 4C (`RAW/note_noteair4c.json`) [code], updating the model only.
- **At pen-up** the app renders the changed region into its bitmap, posts `ENABLE_POST(-1,1,pid)`
  (`InkRenderManager.invalidate` with `enablePost=true, invalidate=false`; only when at least one shape changed
  and intersects a visible page, see §3.4), waits
  `eraserAreaDisplayDelayMillSecond` (0 ms on this model), then pauses **render only** (the input reader stays
  on), invalidates the view, and 300 ms later resumes render with a full `ResumeRawDrawingRequest`
  reconfiguration [code]. That reconfiguration is also what puts the pen's own stroke style back into
  SurfaceFlinger's single painter-style slot after the eraser overwrote it [code for the slot, inferred for its
  role].
- **Why stock does not flash after erasing:** according to the handoff's hardware finding, the flash happens
  when the pen writes while the app's SF session is paused. With the pen, shape or eraser tool active, stock
  does not have the session paused while the eraser is on the glass. Its only pause starts after lift and lasts
  about 300 ms plus scheduling [code]. The exceptions are the fill-colour and universal-shape tools (§3.7),
  where erasing starts with render disabled [code for the app state; whether they flash is E11/E12]. Stock does
  have a window in which a quick new stroke would land in a paused session. Whether that flashes is an open
  experiment (§9, E4).
- **The earlier attempt to copy only the style-8 setup.** In stock, style 8 as the _main_ stroke style is
  set only while the eraser tool is the active handler. In that state stock also reinterprets pen-tip
  "drawing" callbacks as erasing (`usePenTipErase()`) [code]. Every erase ends with a pause, an ENABLE_POST and a
  resume that re-sends the pen's stroke style and eraser setup [code]. SurfaceFlinger keeps a single current
  stroke-style slot, and the eraser reader writes into it without restoring it [code]. So copying style 8 without the
  surrounding lifecycle can leave SF drawing with the soft eraser (a wide, faded trace). It can also leave SF
  in the handwriting schema, so the app's re-render never reaches the panel [inferred]. §5 has the details.

---

## 2. Central classes and entry points

| layer              | class / symbol                                                                                                                                                                                                     | role in erasing                                                                                                                                                                           |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| app                | `NA/note/note/handler/scribble/EraseHandler.java`                                                                                                                                                                  | eraser tool handler (`Provider.SCRIBBLE_ERASE_PROVIDER`)                                                                                                                                  |
| app                | `NA/note/note/handler/scribble/ScribbleHandler.java`                                                                                                                                                               | base of every drawing handler; contains the whole erase gesture pipeline                                                                                                                  |
| app                | `NA/note/note/handler/common/EpdShapeHandler.java`                                                                                                                                                                 | pen tool handler (`SCRIBBLE_EPD_PROVIDER`); `useDrawErase()=true`, so the stylus eraser end erases while it is active                                                                     |
| app                | `NA/note/note/handler/common/NormalShapeHandler.java`, `.../UniversalShapeHandler.java`, `NA/note/note/handler/scribble/FillColorHandler.java`                                                                     | the other handlers with `useDrawErase()=true` (registered in `NA/note/note/handler/NoteEditBundles.java:116-124`); the eraser end erases in them too, with different render gating (§3.7) |
| app                | `NA/note/note/eventhandler/TouchEventHandler.java`                                                                                                                                                                 | turns SDK raw callbacks into handler calls; converts pen-tip drawing into erasing when `usePenTipErase()`                                                                                 |
| app                | `NA/note/note/eventhandler/PenEventHandler.java`                                                                                                                                                                   | the pause/resume arbiter; builds `ResumePenBean` and submits `ResumeRawDrawingRequest`                                                                                                    |
| app                | `NA/note/note/request/pen/ResumeRawDrawingRequest.java`                                                                                                                                                            | the configuration + enable step, including the soft-eraser setup                                                                                                                          |
| app                | `NA/note/note/action/erase/{EraseBeginAction,ErasingShapeAction,EraseFinishAction,AreaEraseFinishAction,EraseByTypeAction,FloatEraseByTypeAction,EraseLayerAction}.java`                                           | gesture begin / during / end, eraser type selection, layer erase                                                                                                                          |
| app                | `NA/note/note/action/render/InvalidateViewWithPenControlAction.java`, `NA/note/note/action/pen/{PauseRawPenAction,ResumeRawPenAction,PauseResumeRawPenAction}.java`                                                | the post-erase pause → invalidate → resume                                                                                                                                                |
| app                | `NA/note/note/menu/data/EraseType.java`, `NA/sdk/scribble/utils/EraseUtils.java`, `NA/sdk/data/EraseConstant.java`                                                                                                 | eraser type constants                                                                                                                                                                     |
| bundled SDK        | `NA/sdk/notecore/editor/NoteManager.java`                                                                                                                                                                          | thin wrapper over `TouchHelper`; posts `BeginRawErasingEvent` etc.                                                                                                                        |
| bundled SDK        | `NA/sdk/pen/TouchHelper.java`, `NA/sdk/pen/touch/SFTouchRender.java`, `NA/sdk/pen/touch/AppTouchRender.java`, `NA/sdk/pen/EpdPenManager.java`, `NA/sdk/pen/RawInputManager.java`, `NA/sdk/pen/RawInputReader.java` | raw input and render control                                                                                                                                                              |
| bundled SDK        | `NA/sdk/device/SDMDevice.java`, `NA/sdk/api/device/epd/EpdController.java`                                                                                                                                         | reflection into `android.onyx.ViewUpdateHelper`                                                                                                                                           |
| framework          | `FW/android/onyx/ViewUpdateHelper.java`                                                                                                                                                                            | Binder transactions to SurfaceFlinger                                                                                                                                                     |
| SF                 | `SurfaceFlingerHelper::onTransact` jump table A (`SF@0x341094`, base `SF@0x55f2e0`)                                                                                                                                | inline handlers for the eraser/brush/side-button flags                                                                                                                                    |
| SF                 | `android::PenManager`, `BtnReader`, `DrawReader`, `BaseDrawReader`, `EraseReader`, `BaseEraseReader` (`SF@0x52df8c…0x52ed60`)                                                                                      | picks draw vs erase reader per touch, paints the live trace                                                                                                                               |
| SF                 | `MemoryPainter` (`SF@0x524c44…`), `PainterSoftEraser` (`SF@0x52b1c0…`), `PainterDash`                                                                                                                              | the painters for styles 8 and 5                                                                                                                                                           |
| native, in-process | `libonyx_pen_touch_reader.so` `android::PenManager::{isRubberDown,isSideTouchDown,isShortcutErasing}`                                                                                                              | tells the Java reader whether a touch is an erase                                                                                                                                         |

### Eraser type constants

| value | `EraseType` enum (`NA/note/note/menu/data/EraseType.java`) | UI name                                                                    | `isEraseTrackByRawRender` | `isMoveStrokeErase` | SF eraser style | default width                     |
| ----- | ---------------------------------------------------------- | -------------------------------------------------------------------------- | ------------------------- | ------------------- | --------------- | --------------------------------- |
| 0     | `STROKE_ERASER`                                            | "stroke eraser": erases whole strokes                                      | true                      | true                | 8               | `DEFAULT_ERASER_STROKE_WIDTH` 5.9 |
| 1     | `MOVE_ERASER`                                              | "move eraser": partial (splits strokes)                                    | true                      | true                | 8               | `DEFAULT_ERASER_MOVE_WIDTH` 59    |
| 2     | `AREA_ERASER`                                              | lasso / area eraser                                                        | true                      | false               | 5 (dash)        | 5.9 (not used for hit testing)    |
| −1    | `CURRENT_LAYER_ERASER`, `ALL_LAYER_ERASER`                 | "erase current layer" / "erase all layers" (one-shot commands, not a tool) | n/a                       | n/a                 | n/a             | n/a                               |

Sources: `EraseUtils.isEraseTrackByRawRender` = `type == 1 || type == 2 || type == 0`, `isMoveStrokeErase` =
`type == 1 || type == 0` (`NA/sdk/scribble/utils/EraseUtils.java:23-29`) [code];
`EraseConstant.getDefaultEraserWidth` and the width map (`NA/sdk/data/EraseConstant.java`) [code]; enum values
(`EraseType.java`) [code]. The UI names are **[inferred]** from the enum and resource names
(`R.string.stroke_eraser`, `move_eraser`, `area_eraser`). Widths are in view pixels at scale 1.
`EraseConstant` also carries `DEFAULT_ERASER_MOVE_WIDTH_MM = 5.0` and `..._STROKE_WIDTH_MM = 0.5`.

Because the style for types 0 and 1 is 8, both the stroke eraser and the partial eraser show the soft-eraser
trace in SF. Only the area eraser uses the dash painter [code, `ResumeRawDrawingRequest.k()` line 64].

---

## 3. The mechanism, step by step

### 3.1 How an erase is detected (stylus eraser end, side button, tip)

Two independent readers see every pen event, and both classify it.

**SurfaceFlinger's own reader** (the one that paints):

- `PenManager::onEvent` (`SF@0x52e33c`) records key state in bytes at `PenManager+0x2055..0x2058`: key
  `0x14a` BTN_TOUCH → `+0`, `0x14b` BTN_STYLUS → `+1`, `0x142` BTN_TOOL_BRUSH → `+2`, `0x141`
  BTN_TOOL_RUBBER → `+3` (jump table at `SF@0x32081d`, stores at `SF@0x52e488…0x52e4ec`) [code].
- `isRubberDown()` = RUBBER && TOUCH (`SF@0x52e520`); `isSideTouchDown()` = side-button-erase flag
  (global byte `SF@0xa5e700`) && STYLUS && TOUCH (`SF@0x52e54c`); `isBrushTouchDown()` = TOUCH && !RUBBER
  (`SF@0x52e584`) [code].
- `BtnReader::onSynEvent` (`SF@0x52e028`, the idle reader) first returns without switching when the byte at
  `PenManager+0x2054` is non-zero (`SF@0x52e038…0x52e048`); what sets that byte was not traced [code for the
  check, meaning unknown]. Otherwise it chooses the next reader, testing rubber, then side-touch, then brush
  [code]:
  - rubber or side-touch down → reader **4** if the eraser-raw-drawing flag (`SF@0xac9ad8`) is set, else
    reader **3**;
  - brush touch down → reader **2** if the brush-raw-drawing flag (`SF@0xa5e704`) is set, else reader **1**.
- The five readers are built in `PenManager::PenManager` (`SF@0x52e0c8`). Their vtables (`SF@0xa33780`,
  `…8d0`, `…850`, `…910`, `…890`) resolve to: 0 `BtnReader`, 1 `BaseDrawReader` (key handling only, draws
  nothing), 2 `DrawReader` (draws), 3 `BaseEraseReader` (key handling only, draws nothing), 4 `EraseReader`
  (draws) [code].
- `EraseReader::onActivate` (`SF@0x52ec64`) calls `MemoryPainter::setStrokeStyle(eraserStyle)` with the
  style stored at `SF@0xa5e708`, then enters `BaseReader::onEvent` (`SF@0x52dfa8`). That is the path that
  calls `PenManager::moveTo` / `quadTo`, which is the same drawing path ink uses [code].
  `DrawReader::onActivate` (`SF@0x52ec18`) does **not** set a style [code].

**The in-process native reader** (`libonyx_pen_touch_reader.so`, loaded by `RawInputReader`) keeps its own
copy: `PenManager::onKeyEvent` (`PTR@0xa358`) stores TOUCH at `+0x206d`, STYLUS at `+0x206e`, BRUSH at
`+0x206f`, RUBBER at `+0x2070`. `isRubberDown` = RUBBER && TOUCH (`PTR@0xaf68`), `isSideTouchDown` =
`enableSideBtnErase` flag (`+0x206c`) && STYLUS && TOUCH (`PTR@0xaf84`), `isShortcutErasing` = active
reader == 3 (`PTR@0xa508`) [code]. This reader has `EraseReader` and `SideEraseReader` classes (symbol
names) [code]. The Java side receives the result as the `erasing` flag of
`RawInputReader.onTouchPointReceived(…, z2=erasing, z3, z4, action, ts)` (`NA/sdk/pen/RawInputReader.java:538-571`).
When `erasing` is set it dispatches `onBeginRawErasing` / `onRawErasingTouchPointMoveReceived` /
`onRawErasingTouchPointListReceived` / `onEndRawErasing` instead of the drawing callbacks (`:434-482`)
[code]. The exact mapping of the JNI boolean arguments (`z3`, `z4` = shortcut drawing / shortcut erasing) is
**[inferred]** from their use.

**Consequences for this device**

- The user's stylus has an eraser end and no side button. The eraser end reports BTN_TOOL_RUBBER. That this
  specific pen does so is **[inferred]** from the code paths; a hardware check with `getevent` is E1.
- Stock never calls `enableSideBtnErase` or `setEnablePenSideButton`. Only the SDK defines them
  (`SFTouchRender.enableSideBtnErase`, `NA/sdk/pen/touch/SFTouchRender.java:329-333`) [code, by
  whole-APK grep]. SurfaceFlinger's side-button-erase flag defaults to **1** (initialised `.data` byte at
  `SF@0xa5e700`), and `handleAppDie` sets it back to 1 (`SF@0x565488`) [code]. With no side button this is
  irrelevant here.
- SF's defaults: brush raw drawing **1** (`SF@0xa5e704`), eraser style **5** (`SF@0xa5e708`), eraser raw
  drawing **0** (`SF@0xac9ad8`, in `.bss`) [code]. These are process-global bytes in SurfaceFlinger. No pid or
  window keys them, so the last app that set them wins until another app sets them [code for the storage,
  inferred for the cross-app consequence].
- `TouchHelper.setRawDrawingEnabled(bool)` always ends with `resetPenDefaultRawDrawing()` = brush raw drawing
  true, eraser raw drawing (false, 5) (`NA/sdk/pen/TouchHelper.java:182-185, 276-284`; identical in
  `PUB/pen/TouchHelper.java:170-178, 348-351`) [code]. Anything that pauses via `setRawDrawingEnabled(false)`
  therefore also turns SF's eraser trace off until the next `ResumeRawDrawingRequest`.

**Tip used as an eraser (eraser tool).** The pen tip reports a normal brush touch. SF uses the
`DrawReader`, and the app gets `onBeginRawDrawing` etc. `TouchEventHandler` then reroutes to the erase
methods whenever the active handler's `usePenTipErase()` is true. Only `EraseHandler` returns true
(`EraseHandler.java:127-130`, `TouchEventHandler.beginRawDraw` `:136-157`, `endRawDrawing`,
`rawDrawingPointsMoveReceived`, `rawDrawingPointsReceived` `:315`) [code].

### 3.2 Selecting the eraser tool

Entry points: the eraser menu → `EraseByTypeAction` / `FloatEraseByTypeAction`
(`NA/note/note/action/erase/EraseByTypeAction.java:103-116`, `FloatEraseByTypeAction.java`). They set
`NotePenManager.activeEraserArgs.type` and width, then call `openEraseHandler()` =
`handlerManager.activeProvider(SCRIBBLE_ERASE_PROVIDER)` if `DeviceConfig.canPenTipErase` (default `true`,
`NA/sdk/note/ui/config/DeviceConfig.java`; not overridden for this model) [code]. After that
`EraseByTypeAction.t()` (`:68-85`) runs an `InvalidateScreenAction` for the stroke-level types [code]. A system
gesture or key "toggle pen type" broadcast (`action.toggle.pen.type` →
`KNoteCommonReceiver.p()` → `TogglePenTypeEvent` → `TogglePenTypeAction.z()`, `TogglePenTypeAction.java:60-67`)
switches between the pen and eraser providers the same way and then runs `InvalidateScreenAction` [code]. The
firmware side of that broadcast (`FW/android/onyx/BroadcastHelper.java:1321`,
`out-kcb/.../GestureActionType.TOGGLE_PEN_TYPE`) is **[inferred]** to be the sender; the two constant names
differ (`TOGGLE_PEN_TYPE_REQUEST_ACTION` vs the app's `ONYX_TOGGLE_PEN_TYPE_ACTION`) and their values were not
compared.

`HandlerManager.activeProvider` (`NA/sdk/notecore/editor/handler/HandlerManager.java:80-107`) calls the old
handler's `onDeactivate()` and the new handler's `onActivate()` [code]. For `EraseHandler.onActivate`
(`EraseHandler.java:61-66`) this runs:

1. `ScribbleHandler.onActivate` (`ScribbleHandler.java:908-919`): `enableColorCU()` because
   `EraseHandler.isEnableColorCU()` is true → `EpdController.enableColorCU()` → `ViewUpdateHelper.enableColorCU(true)`
   → SF `ENABLE_COLOR_CU` (1048693) → `EpdcManager::setEnableColorCU` (`SF@0x559dcc`), which logs
   `E EpdcManager: CU 1` and stores the flag at `EPDC+0x254`, read by `NormalSchema::output` /
   `HWNormalSchema::output` (`SF@0x55b4e0`, `SF@0x566e5c`) [code]. What "CU" does to the output is **[unknown]**.
   After that: register for events, `ActiveRenderModeAction(SCRIBBLE)`.
2. `PauseResumeRawPenAction` with resume delay **150 ms** [code]:
   - `PauseRawPenAction(pauseArgs)`: pause render and input both → `NoteManager.setRawDrawingEnabled(false)`
     (`PauseRawPenAction.java:47-58`) → `TouchHelper.setRawDrawingEnabled(false)`: SF render off
     (`SFTouchRender.h()`: `leaveScribbleMode` = `ENABLE_POST(-1,1,pid)` + pen state **3**; `AppTouchRender`:
     another `ENABLE_POST(-1,1,pid)`), input reader paused, then `resetPenDefaultRawDrawing()` (brush 1,
     eraser 0/5) [code].
   - `ResumeRawPenAction(resumeArgs, 150)` → posts `PenEvent(render=true, input=true, 150)`
     (`ResumeRawPenAction.java:44-48`) → `PenEventHandler.onPenEvent` → `s()` → `q()` (`PenEventHandler.java:155-185`)
     builds a `ResumePenBean` and submits `ResumeRawDrawingRequest` on the pen scheduler [code].
3. `ResumeRawDrawingRequest` configures SF for the eraser tool (full list in §5) and ends with pen state
   **2** [code].

Selecting the eraser tool from the eraser menu therefore produces **two overlapping** pause/resume cycles,
not one [code for each cycle; their interleaving is inferred]:

- `EraseHandler.onActivate`: pause render + input (`setRawDrawingEnabled(false)`), resume requested with a
  150 ms delay;
- `EraseByTypeAction.t()` → `InvalidateScreenAction` (`NA/note/note/action/render/InvalidateScreenAction.java`):
  `RawPenArgs(pause both, resume both, DELAY_ENABLE_RAW_DRAWING_MILLS = 500 on colour)` →
  `InvalidateViewWithPenControlEvent` → `ScribbleHandler.onInvalidateViewWithPenControl` (`:1014-1017`) →
  `InvalidateViewWithPenControlAction` (pause both, `View.invalidate()`, resume after 500 ms).

Both resumes go through `ResumeRawDrawingRequest`, which does nothing unless render or input is disabled at
that moment (§5 point 4). Whether the 150 ms resume lands before or after the second pause decides whether
SF sees PAUSE, RESUME, PAUSE, RESUME or PAUSE, PAUSE, RESUME. No pen is on the glass in either case.

If the eraser tool is **already** the active provider (only the eraser type changes),
`HandlerManager.activeProvider` returns at `this.a != provider` (`HandlerManager.java:82`), so `onActivate`
does not run and only the `InvalidateScreenAction` cycle happens. That cycle's resume is what switches SF
between style 8 and style 5 when moving between the stroke/partial and area erasers [code].

Other entry points: `PenTipEraseToolMenuModel` → `StartEraseAction` (`NA/note/note/action/erase/StartEraseAction.java`)
activates the eraser provider with no `InvalidateScreenAction` (only the `onActivate` 150 ms cycle) [code].
`TogglePenTypeAction` activates it and then runs `InvalidateScreenAction` (above) [code].

### 3.3 The gesture, begin / during / end (types 0, 1, 2)

The same code runs for the eraser tool (pen tip, via `usePenTipErase`) and for the eraser end while the pen
tool is active (`EpdShapeHandler.useDrawErase()=true`, `EpdShapeHandler.java:498-500`). The one gate is
`ScribbleHandler.canDrawErase()` = `useDrawErase() && !penEventHandler.isFloatButtonActived`
(`ScribbleHandler.java:764-773`) [code]. With the floating button active nothing is erased: `a()` returns
early and `l0()` does nothing [code]. When the pen tool is active, the eraser type used by the eraser end is
whatever is currently selected in the eraser menu (`getCurEraserType()` reads
`NotePenManager.activeEraserArgs.type`, `ScribbleHandler.java:826-828`) [code].

**Begin** — SDK `onBeginRawErasing` → `NoteManager` posts `BeginRawErasingEvent`
(`NoteManager.java:458-462`) → `TouchEventHandler.beginRawErasing` (`:159-170`) →
`EraseHandler.onBeginRawErasing` (`EraseHandler.java:69-78`: `RawPenDownEvent`, `updatePenRawTouch(true)`,
haptic update) → `ScribbleHandler.onBeginRawErasing` (`:931-936`) → `a()` / `beginDrawErasing`
(`:239-266`) [code]:

- `m0()` disposes any previous buffer;
- `EraseBeginAction` (`EraseBeginAction.java:73-85`): `beforeRenderObservable` (on the main thread,
  `InkRenderManager.cancelPreRender()`, `EditorBundlesKt.java:109-124`), then
  `flatMapIf(!isEraseTrackByRawRender(type)) → PauseRawPenRenderAction`. For types 0/1/2 the predicate is
  false, so **no pause** [code];
- `enableColorCU()` (SF logs `CU 1`) [code]. `GlobalNoteBundle.enableColorCU()` → `EpdController.enableColorCU()`
  → `SDMDevice.enableColorCU(true)` (reflection) → `ViewUpdateHelper.enableColorCU(true)`, with no device gate
  on the app side [code]. Note the asymmetry in the eraser tool: `onActivate` turns CU on, the first gesture's
  end (`l0()`) turns it off, and it stays off between later gestures until the next begin or re-activation
  (`ScribbleHandler.java:241-266, 463-469, 908-919`) [code]. With the pen tool (`EpdShapeHandler`,
  `isEnableColorCU()` false from `BaseHandler.java:91-93`) CU is on only during each erase gesture [code];
- posts `SelectionStartEvent` (only the audio menu subscribes, `AudioMenuModel.java:436`) [code];
- a fresh `EraseContext` with the current render matrix;
- an Rx buffer of `getEraseBufferTime()` ms (`s()`, `:629-631`: `strokeEraserBufferTimeMs` for type 0,
  `overlayEraserBufferTimeMs` otherwise; **20 / 30 ms** on this model from `RAW/note_noteair4c.json`) [code].
- `EpdEventHandler.onRawPenDownEvent` cancels its fast-mode wake-lock release timer
  (`NA/note/note/eventhandler/EpdEventHandler.java:279-282`) [code].

**During** — `onRawErasingTouchPointMoveReceived` → `RawErasingPointMoveEvent` →
`ScribbleHandler.onRawErasingPointMove` → `k0()` (`:445-451`) pushes the point into the buffer; for types 0/1
it also posts `UpdateEraseViewPositionEvent` (`i0()`). Each buffer tick → `b()` → `p()` (`:568-578`) → `l()`
(`:454-461`) → `m()` (`erasingObservable`, `:471-524`) → `ErasingShapeAction` [code].
`ErasingShapeAction.x()` (`:191-205`) runs `StrokesErasingRequest` (type 0), `OverlayErasingRequest` (type 1),
or `AreaErasingRequest` (type 2). All of them only mutate the model and the render context's erase args
[code]. The display step `ErasingShapeAction.s()` (`:101-110`) returns immediately when
`isEraseTrackByRawRender(type)`, so **nothing is drawn by the app during the gesture** [code].
`ScribbleHandler.h0()` (`:389-393`) would post a `ShowEraseViewEvent`, the app's own eraser cursor overlay
(gated on the `ERASE_DISPLAY_TRACK` MMKV setting, default false, and on non-raw-rendered types), but `h0()` has
**no caller** in either decompilation, so the overlay is never shown [code, by grep of `ScribbleHandler.java` in
both trees]. The per-point `UpdateEraseViewPositionEvent` (types 0/1) only calls `setX/setY` on the
`eraseCircle` `ImageView` (`EraseViewEventHandler.onUpdateEraseViewPositionEvent`), which is declared
`android:visibility="invisible"` (`res/layout/fragment_scribble_normal_mode.xml:33-37`) and is never made
visible. That it produces no app frame is **[inferred]** from Android skipping invalidation of invisible views
without a display list. None of the erasing requests posts a pen
pause: `AreaErasingRequest` explicitly calls `setPauseRawDraw(false)` (`NA/note/note/request/AreaErasingRequest.java:18`),
and `StrokesErasingRequest` / `OverlayErasingRequest` extend `RxBaseWorkRequest`, which has no pause logic
(`NA/sdk/notecore/editor/request/RxBaseWorkRequest.java`) [code]. The default `BaseNoteRequest` does pause
render and input before it runs (`BaseNoteRequest.java:17-19, 34-47`: `h=true, i=true` →
`RawPenPauseEvent`) [code]. The erase path avoids that base class or overrides the default.

What the panel shows during the gesture: SF's `EraseReader` (eraser end) or `DrawReader` (tip in the eraser
tool) paints the trace with painter style 8 (types 0/1) or 5 (type 2) through the handwriting path [code].
Pen state stays **2** throughout [code: no pen-state transaction is issued between begin and end].

**End** — the SDK delivers the whole point list first, then the end callback
(`RawInputReader.N`, `:249-265`: `u(list)` then `t(point)`) [code]. The list goes
`RawErasingPointsReceived` → `ScribbleHandler.onRawErasingPointsReceived` (`:1141-1144`) → `l0()`
(`rawErasingPointsReceived`, `:463-469`): dispose the buffer, `disableColorCU()` (SF logs `CU 0`), `h()`
(`:377-387`), which runs the remaining `m()` and then `q()` (`finishErasing`, `:588-604`) in `doFinally` [code].
`q()` marks the `EraseContext` aborted, stores the screen point list, starts
`EraseFinishAction(curEraseType, eraseShape, scribbleMode)`, posts `SelectionEndEvent` and `HideEraseViewEvent`
[code]. The end callback `onEndRawErasing` → `EraseHandler.onEndRawErasing` (`:89-96`) clears
`penRawTouch`, posts `RawPenUpEvent`, restores the default haptic type. No pen-control call is made there
[code]. Eraser-end strokes never start the SDK's pen-up refresh timer: `RawInputReader.N` returns before `I()`
when `erasing` is set (`:261-264`) [code]. Tip strokes in the eraser tool are _drawing_ strokes to the SDK, so
`I()` does run for them. The timer is on by default (`RawInputReader.o = true`, `:66`; stock never calls
`setPenUpRefreshEnabled`, and `TouchHelper.setFilterRepeatMovePoint`, which by an SDK bug also calls
`setPenUpRefreshEnabled`, is not called either). When it fires it only posts `PenUpRefreshEvent`
(`RawInputReader.r()`, `NoteManager.java:476-479`) → `TouchEventHandler.penUpRefreshEvent` →
`EraseHandler.onPenUpRefresh`, which is the empty `BaseHandler.onPenUpRefresh` (`BaseHandler.java:185-187`).
So it has no SF-visible effect [code].

Both `EraseHandler.onBeginRawErasing` and `ScribbleHandler.onBeginRawErasing` post `RawPenDownEvent`, and both
`onEndRawErasing` overrides post `RawPenUpEvent`, so the eraser tool emits each event twice per gesture
(`EraseHandler.java:72, 92`; `ScribbleHandler.java:934, 965`) [code]. The subscribers
(`EpdEventHandler.onRawPenDownEvent` / `onRawPenUpEvent`) are idempotent here: the up handler only acts when the
fast (transient-update) mode is on, which is only possible for handlers with `renderInFastMode()` true
(selection, universal shape), not for the pen or eraser handler (`EpdEventHandler.k()`,
`BaseHandler.renderInFastMode()=false`) [code].

Edge case [inferred]: if the SDK delivers no point list for an eraser stroke (`RawInputReader.N` calls `u()`
only when its list is non-empty), `onRawErasingPointsReceived` never fires, so `l0()` does not run: colour CU
stays on, and no `EraseFinishAction` (and therefore no post-erase pause/resume) is started. `h()` likewise
returns before `q()` when the list is empty (`ScribbleHandler.java:377-387`) [code for both guards].

### 3.4 After lift: result render, pause, resume

`EraseFinishAction.create()` (`EraseFinishAction.java:512-…`) chains [code]:

1. `G()` / `erase` (`:178-…`): per type, `StrokesEraseFinishedRequest` (type 0), `OverlayEraseFinishedRequest`
   (type 1), `AreaEraseFinishAction` (type 2, which does the lasso hit test and `AreaEraseFinishedRequest`).
   The two `BaseNoteRequest` subclasses call `setPauseRawDraw(false)`
   (`StrokesEraseFinishedRequest.java:16`, `OverlayEraseFinishedRequest.java:34`) [code].
2. `O()` erase tags, `M()` erase links.
3. On the note scheduler: `p()` (`addCommand`: merge the changed-shape beans, push to undo);
   `r0()` (`postShapeEvent`); `q0()` (`HWREraseStrokeEvent`); `z0()` (`resetViewUpdateMode`:
   `EpdController.resetViewUpdateMode(editorView)` → `View.resetUpdateMode()` by reflection,
   `SDMDevice.java:580, 2034-2036`); `r()` (clear the zoom thumbnail).
4. `x0()`: re-render the fill bitmap only in the fill-colour provider.
5. `s0()` (`refreshScreen`) = `render()` then `E()` (`delayDisplay`):
   - `render()` (`:343-…`): returns without any display call when `changedShapeList` is empty or no changed page
     is visible, so an erase stroke that hits nothing sends **no** `ENABLE_POST` here (the pause/resume below
     still runs) [code]. Otherwise `forceRenderVisibleDirtyScreenRectToDisplay(changedShapes,
     DisplayArgs{invalidate=false, enablePost=true})`, which renders the dirty slices and then calls
     `InkRenderManager.invalidate(displayArgs)` (shape-list overload `ForceRenderSliceListActionsKt.java:267-287`,
     which delegates to the box-list overload `:541-566`; the invalidate is its `doOnNext` `p()`, `:428-432`) →
     `enablePost()` → `EpdController.enablePost(view, 1)` → `ViewUpdateHelper.enablePost(1)` →
     **`ENABLE_POST(-1, 1, pid)`**. No `View.invalidate()`, because `invalidate=false`
     (`InkRenderManager.java:79-95`) [code]. Per `08-surfaceflinger.md`, `handleEnablePost` calls
     `EpdcManager::stopHandwriting` when `androidDrawing != 1`, which restores the normal schema and
     recomposites [code in that document, not re-verified here].
   - `E()` (`:156-…`): delay `DeviceConfig.eraserAreaDisplayDelayMillSecond` (default `0`,
     `DeviceConfig.java:89`, not set in `note_noteair4c.json`), then emit
     `RawPenArgs{pauseRender=true, pauseInput=false, resumeRender=true, resumeInput=true, resumeDelay=300}`
     [code].
6. `u()` posts `EraseFinishEvent(rawPenArgs)` (`:416-…`) [code].

`ScribbleHandler.onEraseFinishEvent` → `onEraseFinished`. In `EraseHandler` (`:98-103`) this is `y0()` first:
if `NoteMMKVHelper.isAutoDeselectEraserEnabled()` (MMKV `AUTO_DESELECT_ERASER_KEY`, default **false**,
`NoteMMKVHelper.java:1071-1073`), `switchToPreviousProvider()`, typically back to the pen provider, unless the
previous provider is `NONE` or the eraser itself (`ScribbleHandler.java:1277-1283`). The switch is an
`ActiveProviderAction` posted to the main thread; it runs `EraseHandler.onDeactivate` (CU off) and the new
handler's `onActivate` (`EpdShapeHandler.onActivate` issues no pen control) [code]. Then
`InvalidateViewWithPenControlAction(editorBundle, rawPenArgs, DisplayArgs())`. In other handlers (the pen tool
with the eraser end) it is only the `InvalidateViewWithPenControlAction` (`ScribbleHandler.java:980-983`)
[code]. `InvalidateViewWithPenControlAction.create()` (`:62-…`) [code]:

1. `pauseRawPen(args)` → `PauseRawPenAction.n()` (`:47-58`). Because only `pauseRender` is set, this is
   `NoteManager.setRawDrawingRenderEnabled(false)`, **not** `setRawDrawingEnabled(false)`, so
   `resetPenDefaultRawDrawing` does not run and SF's eraser flags stay as configured [code] →
   `TouchHelper.setRawDrawingRenderEnabled(false)` (deduplicated on the cached flag,
   `TouchHelper.java:286-295`) → `SFTouchRender.h()` = `EpdController.leaveScribbleMode(view)`
   (**`ENABLE_POST(-1,1,pid)`**) + `EpdPenManager.pauseDrawing()` (**pen state 3**), and
   `AppTouchRender.setDrawingRenderEnabled(false)` = another **`ENABLE_POST(-1,1,pid)`**
   (`AppTouchRender.java:268-274`) [code]. The input reader stays enabled.
2. On the main thread: `InkRenderManager.invalidate(DisplayArgs())` → `enablePost=false, invalidate=true` →
   `view.invalidate()`, an ordinary app frame posted while the session is paused [code].
3. `resumeRawPen(args)` → `ResumeRawPenAction` posts `PenEvent(true, true, 300)` → `PenEventHandler.s/q`.
   The render predicate `A()` and the input predicate `z()` are evaluated **now**, against the **current**
   active handler [code]. That this is already the pen handler when auto-deselect ran is **[inferred]**: the
   `ActiveProviderAction` is queued on the main thread before the pause/invalidate/resume chain reaches its own
   main-thread steps, but nothing orders them explicitly. →
   `ResumeRawDrawingRequest`: `ThreadUtils.mySleep(300)`, full reconfiguration, `setRawDrawingRenderEnabled(true)`
   → `EpdPenManager.resumeDrawing()` = **pen state 2** [code]. Input is already enabled, so
   `setRawInputReaderEnable(true)` is skipped (`g()` false) [code].

Net SF-visible sequence for one erase gesture, pen tool or eraser tool (types 0/1/2):

```
pen down (eraser end or tip)      no transactions; SF EraseReader/DrawReader paints style 8 or 5; pen state 2
                                  EpdcManager "CU 1" (enableColorCU)
... gesture ...                   none
pen up                            EpdcManager "CU 0" (disableColorCU)
+ model update time               ENABLE_POST(-1,1,pid)          (InkRenderManager.enablePost, finish render)
+ 0 ms (eraserAreaDisplayDelay)   ENABLE_POST(-1,1,pid)          (SFTouchRender leaveScribbleMode)
                                  SET_PEN_STATE(3,pid)           (EpdPenManager.pauseDrawing)
                                  ENABLE_POST(-1,1,pid)          (AppTouchRender)
                                  app frame (View.invalidate)
+ ~300 ms                         SET_HANDWRITING_REGION_LIMIT / _EXCLUDE, REGION_MODE(1),
                                  SET_STROKE_COLOR, [pen: SET_STROKE_STYLE(pen), SET_STROKE_WIDTH, pen params]
                                  [eraser tool: SET_STROKE_STYLE(8 or 5)],
                                  SET_STROKE_PARAMETERS(5,[5.0]), SET_STROKE_PARAMETERS(8,[w,0.5,0.1]),
                                  SET_ERASER_RAW_DRAWING_ENABLED(1, 8 or 5), SET_BRUSH_RAW_DRAWING_ENABLED(1),
                                  SET_PEN_STATE(2,pid)
```

The transaction order and content are **[code]**. The timings are **[inferred]**; they are the code's
delays plus unknown scheduling latency. Three refinements [code]: the first `ENABLE_POST` (finish render) is
absent when the stroke erased nothing (§3.4 step 5); because `TouchHelper` holds both `SFTouchRender` and
`AppTouchRender`, the limit/exclude regions, stroke style, stroke colour and stroke width are each sent **twice**
(`SFTouchRender` via `RawInputReader`/`EpdPenManager`, `AppTouchRender` via `EpdController` /
`AppTouchInputReader.setLimitRectList/setExcludeRectList`, `AppTouchRender.java:329-342`,
`AppTouchInputReader.java:239-253`, `RawInputReader.java:604-683`); and `SET_PEN_STATE(3)` comes from the
`SFTouchRender` only. The resume's 300 ms `mySleep` runs on the pen scheduler, so it also delays any other pen
request queued behind it [inferred from both requests using `EditorBundlesKt.penScheduler`]. "SET_STROKE_WIDTH" for the pen provider comes from
`NoteManager.applyPenArgs` (`NoteManager.java:205-219`) [code].

### 3.5 Erase current layer / erase all layers ("clear page")

`EraseByTypeAction.t()` → `EraseLayerAction` (`EraseLayerAction.java`): a confirmation dialog, then
`EraseCurrentLayerAction` or `EraseAllLayerShapesAction` [code]. Their requests (`EraseCurrentLayerRequest`,
`ClearAllFreeShapesRequest`, `NA/sdk/notecore/editor/request/`) are plain `BaseNoteRequest`s that do **not**
override the default pause, so before they run they post `RawPenPauseEvent(pauseRender, pauseInput)` →
`PenEventHandler.onRawPenPauseEvent` (`:449-452`) → `PauseRawPenAction` → `setRawDrawingEnabled(false)`
(pen state 3, ENABLE_POST, input reader paused, SF eraser/brush flags reset to defaults) [code]. The removed
shapes are then re-rendered with `forceRenderNoteDirtyRectToDisplay` (default `DisplayArgs`, i.e. a
`View.invalidate()`), and the action ends with `resumePen()` = `PenEvent.resumeRawDrawing()` → resume after
`DELAY_ENABLE_RAW_DRAWING_MILLS` = **500 ms** on a colour device (`RawPenArgs.java:69`) [code]. The dialog
itself also takes window focus. `PenEventHandler.onActivityFocusChangedEvent` (`:335-…`) records `hasFocus`
and runs an `InvalidateScreenAction` (pause both, resume both after 500 ms, and the resume is refused while
focus is lost) [code; that a dialog produces `ActivityFocusChangedEvent` is inferred]. No pen is on the glass
during any of this.

### 3.6 Other erase-like paths (not examined in depth)

- HWR scratch-out gesture (`NA/note/note/action/hwr/HWRHandleEraseGestureAction.java`,
  `CheckAndActiveEraseGestureAction.java`) erases recognised text strokes in the HWR scenes. It was not traced
  **[unknown]** beyond the setting keys (`KEY_RECOGNIZE_ERASE_GESTURE`, `saveAutoActiveEraseGesture`).
- `KeyAction.CHANGE_TO_ERASE_MODE` (keycode 28) exists in the bundled SDK's reader key bindings
  (`NA/sdk/data/KeyBinding.java:11`) but nothing in `note/` handles it [code, by grep].
- Undo of an erase goes through `CommandManager` (populated in `EraseFinishAction.p()`) and is outside this
  document.
- The text-box handwriting input (`NA/note/note/handler/text/HandWritingHandler.java`) owns a **second**
  `TouchHelper` (`TouchHelper.create(view, callback)`, `:155`) and calls `setRawDrawingEnabled(false/true)` on it
  (`pauseRawDrawing` `:472-476`, `resumeRawDrawing` `:510-515`). Each call ends in `resetPenDefaultRawDrawing()`,
  which resets SF's process-global eraser flag to `(0, 5)` and brush flag to 1, whatever the note editor had
  configured [code]. When this happens relative to erasing in the note editor was not traced [unknown].

### 3.7 Erasing in the other drawing tools

`useDrawErase()` is true in exactly five handlers: `EpdShapeHandler` (pen), `EraseHandler`, `NormalShapeHandler`
(`SCRIBBLE_NORMAL_SHAPE_PROVIDER`, geometric shapes), `UniversalShapeHandler` (`SCRIBBLE_UNIVERSAL_SHAPE_PROVIDER`)
and `FillColorHandler` (`SCRIBBLE_FILL_COLOR_PROVIDER`) [code, grep of `boolean useDrawErase` over the app]. It is
false in `ScribbleHandler` (default), `SelectionHandler` (inherits), `ZoomSelectionHandler`, `HWRHandler`,
`TextHWRHandler`, `DiagramHWRHandler`, `MathHWRHandler`; `TextInputHandler`, `SpanHandler` and
`LayerLockHandler` inherit false [code]. None of the non-erasing handlers overrides any `*RawErasing*` method
[code, grep], so for them the eraser end runs `ScribbleHandler.onBeginRawErasing` → `a()`, which stops at
`canDrawErase()`: no model change, no `EraseFinishAction`, and therefore **no post-erase pause, `ENABLE_POST` or
resume** [code]. Whether SF paints an eraser trace in those tools depends on the render state and the eraser
flag left by the last `ResumeRawDrawingRequest` (which enables the eraser trace for every provider, since `k()`
is true for all three eraser types). If it does, nothing in the app removes the trace or restores SF's style
slot afterwards [inferred]. This resolves the "selection tool, eraser end" row that the first version of this
document left open.

The three other erasing handlers differ in render gating [code]:

| handler                 | `canPenRawRender()`                       | `canBrushRawRender()` | pen control around erasing                                                                                                                                                                                                                                                                                                                                                | render during the gesture                                                                                                                                                                                                                             |
| ----------------------- | ----------------------------------------- | --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `NormalShapeHandler`    | true (`:159-161`)                         | false (`BaseHandler`) | `onActivate` posts `PenEvent.pauseDrawingRender()` = `PenEvent(false)`, which `PenEventHandler.onPenEvent` → `q()` turns into "resume nothing" (`A(false)` and `z() && false` are both false), i.e. a **no-op** [code]                                                                                                                                                    | same as the pen tool: on, pen state 2 [inferred from the gates]                                                                                                                                                                                       |
| `FillColorHandler`      | **false** (`:31-33`)                      | false                 | activation goes through `StartFillColorAction` → `InvalidateScreenAction` (pause both, then resume: `A()` refuses render, `z()` allows input). Post-erase `InvalidateViewWithPenControlAction` pauses render (already off, deduplicated) and its resume is refused again                                                                                                  | **off**: input reader on, SF render paused (pen state 3). The app still erases in the model; nothing live is drawn by either side (`ErasingShapeAction.s()` skips display for types 0/1/2) [code for the app; SF behaviour inferred]                  |
| `UniversalShapeHandler` | the field `o`, default false (`:541-546`) | false                 | `onBeginRawErasing` first waits for `n1()` (commits a pending universal shape), then sets `o = true` and posts `PenEvent.resumeRawDrawingImmediately()` (`PenEvent(true, true, 0)`) before calling the normal begin (`b1()`, `:298-307`). `onEndRawErasing` posts the no-op `pauseDrawingRender()` and sets `o = false` (`:626-633`), so the post-erase resume is refused | starts **off** [inferred: every earlier resume was refused while `o` was false], turned **on** by a 0 ms `ResumeRawDrawingRequest` while the eraser is already on the glass [code for the request; the SF reaction to a resume mid-stroke is unknown] |

`UniversalShapeHandler.onEraseFinished` (`:636-641`) also calls `C0()` first, which runs
`switchToPreviousProvider()` when the flag `n` is set (set by `onHWRUniversalShapeEditEvent`, cleared in
`onDeactivate`) (`:198-202`) [code]. After such a switch the post-erase resume is evaluated against the
previous handler, not against `UniversalShapeHandler` [inferred, same ordering caveat as auto-deselect].

On the fill-colour tool the stock app does exactly what the handoff identifies as the flash trigger for our
app: pen input into a session whose render is paused. E11 checks whether stock flashes there.

---

## 4. Tables

### 4.1 Per mode

| mode                                                              | who draws live                            | SF reader / painter style                                          | render flag during gesture                                                 | pen state during gesture | app display during gesture    | SDK callbacks used                                                                                                 | pause                              | resume delay                               | notes                                                                                                                          |
| ----------------------------------------------------------------- | ----------------------------------------- | ------------------------------------------------------------------ | -------------------------------------------------------------------------- | ------------------------ | ----------------------------- | ------------------------------------------------------------------------------------------------------------------ | ---------------------------------- | ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------ |
| eraser tool, stroke eraser (0), tip                               | SF                                        | `DrawReader`, main style **8**, params `[w=5.9 default, 0.5, 0.1]` | on                                                                         | 2                        | none                          | draw callbacks rerouted to erase (`usePenTipErase`)                                                                | after lift, render only            | 300 ms                                     | [code]                                                                                                                         |
| eraser tool, partial eraser (1), tip                              | SF                                        | `DrawReader`, style **8**, `w=59` default                          | on                                                                         | 2                        | none                          | same                                                                                                               | after lift                         | 300 ms                                     | [code]                                                                                                                         |
| eraser tool, area eraser (2), tip                                 | SF                                        | `DrawReader`, style **5** (dash), params `[5.0]`                   | on                                                                         | 2                        | none                          | same                                                                                                               | after lift                         | 300 ms                                     | `showEraseLine=true` in `EraseArgs` (`ScribbleHandler.d()`, `:318-323`) [code]                                                 |
| pen tool, eraser end, type 0/1                                    | SF                                        | `EraseReader`, eraser style **8**                                  | on                                                                         | 2                        | none                          | `onBeginRawErasing`, `onRawErasingTouchPointMoveReceived`, `onRawErasingTouchPointListReceived`, `onEndRawErasing` | after lift                         | 300 ms                                     | uses the currently selected eraser type [code]                                                                                 |
| pen tool, eraser end, type 2                                      | SF                                        | `EraseReader`, eraser style **5**                                  | on                                                                         | 2                        | none                          | same                                                                                                               | after lift                         | 300 ms                                     | [code]                                                                                                                         |
| eraser tool, eraser end                                           | SF                                        | `EraseReader`, style 8/5                                           | on                                                                         | 2                        | none                          | erasing callbacks                                                                                                  | after lift                         | 300 ms                                     | same pipeline [code]                                                                                                           |
| pen tool with floating button active                              | SF still paints if render is on           | `EraseReader`                                                      | per `PenEventHandler.A()`, which requires `!floatButtonActived` for resume | [unknown]                | none                          | callbacks arrive but `canDrawErase()` is false                                                                     | none                               | —                                          | no erase is performed [code]                                                                                                   |
| selection (lasso) tool, eraser end                                | SF if render is on                        | `EraseReader`, style 8/5 (eraser flag enabled by every resume)     | per `SelectionHandler.canPenRawRender()`                                   | [unknown]                | none                          | callbacks arrive, `canDrawErase()` false (`useDrawErase()` not overridden)                                         | none                               | —                                          | **no erase**, no post-erase pause/ENABLE_POST/resume; any SF trace is left in place [code for the app, inferred for SF] (§3.7) |
| HWR / zoom-selection / text / span / layer-lock tools, eraser end | as above                                  | as above                                                           | per handler                                                                | [unknown]                | none                          | `canDrawErase()` false                                                                                             | none                               | —                                          | no erase (§3.7) [code]                                                                                                         |
| geometric-shape tool (`NormalShapeHandler`), eraser end           | SF                                        | `EraseReader`, style 8/5                                           | on                                                                         | 2 [inferred]             | none                          | erasing callbacks                                                                                                  | after lift                         | 300 ms                                     | its `onActivate` "pause" is a no-op `PenEvent(false)` (§3.7) [code]                                                            |
| fill-colour tool (`FillColorHandler`), eraser end                 | nobody live [inferred]                    | SF render paused                                                   | **off**                                                                    | 3 [inferred]             | none                          | erasing callbacks (input reader on)                                                                                | pause is a no-op (already off)     | resume refused (`canPenRawRender()=false`) | stock erases in a paused session here (§3.7, E11) [code for the app]                                                           |
| universal-shape tool, eraser end                                  | SF after the mid-stroke resume [inferred] | `EraseReader` once render is on                                    | off → on at pen-down (0 ms resume)                                         | 3 → 2 [inferred]         | none                          | erasing callbacks, begin deferred behind `n1()`                                                                    | after lift                         | resume refused (`o=false`)                 | (§3.7, E12) [code for the app]                                                                                                 |
| erase current layer / all layers                                  | nobody (no gesture)                       | —                                                                  | off during the request                                                     | 3                        | re-render + `View.invalidate` | —                                                                                                                  | before the request, render + input | 500 ms                                     | [code]                                                                                                                         |
| side button                                                       | not applicable on this pen                | SF would use reader 4/3 (flag default 1)                           | —                                                                          | —                        | —                             | —                                                                                                                  | —                                  | —                                          | stock never enables side-button erase in the in-process reader [code]                                                          |

### 4.2 SF-visible calls issued by the erase path

| call (app → SDK)                                                                                                      | framework transaction (code)                                                                 | SF handler                                                                                                      | SF effect                                                                                                                                                                                                                       |
| --------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `TouchHelper.setEraserRawDrawingEnabled(b, style)` → `Device.setEraserRawDrawingEnabled` (`SDMDevice.java:2145-2152`) | `SET_ERASER_RAW_DRAWING_ENABLED` 1048833 (`FW/android/onyx/ViewUpdateHelper.java:1167-1172`) | inline at `SF@0x55f8e0`                                                                                         | `SF@0xac9ad8 = b`, `SF@0xa5e708 = style`; selects reader 4 vs 3 on rubber-down [code]                                                                                                                                           |
| `TouchHelper.setBrushRawDrawingEnabled(b)` (`SDMDevice.java:2086-2088`)                                               | `SET_BRUSH_RAW_DRAWING_ENABLED` 1048834 (`:1110-1114`)                                       | inline at `SF@0x55f4f4`                                                                                         | `SF@0xa5e704 = b`; reader 2 vs 1 on brush-down [code]                                                                                                                                                                           |
| `Device.setStrokeParameters(style, float[])` (`SDMDevice.java:2281-2283`)                                             | `SET_STROKE_PARAMETERS` 1049089 (`:1252-1264`)                                               | `SurfaceFlingerHelper::handleSetStrokeParameters` (`SF@0x561c14`) → `MemoryPainter::setStrokeParameters`        | per-painter parameters (style 8 = `PainterSoftEraser::setStrokeParameters` `SF@0x52b8a0`) [code for the chain; parameter meaning inferred from the app's constant names `ERASE_PEN_OPACITY=0.5`, `ERASE_PEN_BLACK_OPACITY=0.1`] |
| `TouchHelper.setStrokeStyle(s)` → `EpdController.setStrokeStyle`                                                      | `SET_STROKE_STYLE` 16711688 (`:1266-1270`)                                                   | `handleSetStrokeStyle` (`SF@0x5607ac`) → lambda `SF@0x5646d4` → `MemoryPainter::setStrokeStyle` (`SF@0x52548c`) | writes the single current style slot `MemoryPainter+0x98` (bounded by the painter count) [code]                                                                                                                                 |
| (none, SF-internal) `EraseReader::onActivate`                                                                         | —                                                                                            | `SF@0x52ec64`                                                                                                   | writes the same slot `+0x98` with the eraser style; `DrawReader::onActivate` does not restore it [code]                                                                                                                         |
| `setRawDrawingRenderEnabled(false)`                                                                                   | `ENABLE_POST(-1,1,pid)` ×2 (SF + App render), `SET_SCREEN_HANDWRITING_PEN_STATE(3,pid)`      | `handleEnablePost` / `HWEpdcManager::setHandWritingPenState` (`SF@0x5666bc`)                                    | PAUSE: `TouchReader::pause` + `stopHandwriting` (per `08-surfaceflinger.md`)                                                                                                                                                    |
| `setRawDrawingRenderEnabled(true)`                                                                                    | `SET_SCREEN_HANDWRITING_PEN_STATE(2,pid)`                                                    | same                                                                                                            | RESUME: `TouchReader::ensureStart` + `resume`                                                                                                                                                                                   |
| `InkRenderManager.enablePost()`                                                                                       | `ENABLE_POST(-1,1,pid)`                                                                      | `handleEnablePost`                                                                                              | `stopHandwriting` if `androidDrawing != 1` (per 08)                                                                                                                                                                             |
| `EpdController.enableColorCU()/disableColorCU()`                                                                      | `ENABLE_COLOR_CU` 1048693 (`:565-569`)                                                       | inline `SF@0x55f7c0` → `EpdcManager::setEnableColorCU` (`SF@0x559dcc`)                                          | logs `EpdcManager: CU %d`, sets `EPDC+0x254`, consumed by `(HW)NormalSchema::output` [code]; visual effect [unknown]                                                                                                            |

**Pen state 4 (`EpdPenManager.PEN_ERASING`).** The constant exists (`NA/sdk/pen/EpdPenManager.java`), but
nothing in the APK sends `setScreenHandWritingPenState(…, 4)`. Every call site passes 0, 1, 2 or 3 [code, by
grep]. In SF, `HWEpdcManager::setHandWritingPenState` handles only 0–3; `cmp w1,#3; b.hi` skips anything
larger (`SF@0x5666cc`) [code]. The separate `RawInputReader.resume()/quit()` → `nativeSetPenState(4)`
(`RawInputReader.java:583-602`) sets the in-process reader's own state field (`PTR@0xa898` stores it at
`PenManager+0x30`) and never reaches SF [code]. So SF never sees an "erasing" pen state. Erasing looks to SF
exactly like writing, except for the reader and painter it selects.

### 4.3 What `TouchHelper` is in stock

`NoteManager.bindPenHostView` creates it with `TouchHelper.create(view, 3, callback, false)`
(`NoteManager.java:221-257`, line 232) [code]. Feature 3 = `SFTouchRender` **and** `AppTouchRender`
(`TouchHelper.java:349-365`). The touch listener is disabled, and `setPostInputEvent(true)` is set. Every render
toggle therefore goes to both renders, which is why a render pause emits two `ENABLE_POST(-1,1,pid)`
transactions [code].

---

## 5. `ResumeRawDrawingRequest` in full, and what it assumes

`NA/note/note/request/pen/ResumeRawDrawingRequest.java` (method names from the Kotlin metadata; the second
decompilation `~/tmp_zfs/reversed_onyx_notes_app/src/.../ResumeRawDrawingRequest.java` is identical):

```
execute():
  if noteManager.touchHelper == null: return                                   // :123
  if bean.resumeRender || bean.resumeInput:
    if f() || g():                     // f = resumeRender && !isRawDrawingRenderEnabled   (canResumeRawDrawingRender)
                                       // g = resumeInput  && !isRawDrawingInputEnabled    (canResumeRawInputReader)
      ThreadUtils.mySleep(bean.delayResumePenTimeMs)                           // :128
      o():                                                                      // updateScribbleRect
        p(): setDrawLimitRect(bean.limitRects); setDrawExcludeRect(bean.excludeRects)
        setPenUpRefreshTimeMs(OnyxSystemConfig.getPenUpRefreshTimeMs())
        setSingleRegionMode()
        applyDrawingArgs(drawingArgs)            // = setStrokeColor(drawingArgs.strokeColor)
        n():                                                                    // updatePenArgs
          if provider == SCRIBBLE_EPD_PROVIDER:  applyPenArgs(penArgs)  // style, width, colour
                                                  m(penArgs)             // per-pen params (brush 5, 22, square 60/61)
          elif provider == SCRIBBLE_NORMAL_SHAPE_PROVIDER: setStrokeColor(currentGraphicsShapeColor)
          if provider == SCRIBBLE_SELECTION_PROVIDER: j()   // setStrokeStyle(5)            (setDashStrokeStyle)
          if provider == SCRIBBLE_ERASE_PROVIDER:
             eraser type 0 or 1: l()                        // setStrokeStyle(8)            (setPathEraseStyle)
             eraser type 2:      j()                        // setStrokeStyle(5)
          i():  Device.setStrokeParameters(5, [5.0])                            (setDashParameters)
                Device.setStrokeParameters(8, [eraserArgs.width, 0.5, 0.1])
          k():  setErasePathDrawing(isEraseTrackByRawRender(type) || provider == SELECTION,
                                    isMoveStrokeErase(type) ? 8 : 5)             (setErasePathDrawing)
                → TouchHelper.setEraserRawDrawingEnabled(...)
          h():  setBrushRawDrawing(bean.canBrushRawRender)                       (setBrushRawDrawing)
                → TouchHelper.setBrushRawDrawingEnabled(...)
      if g(): setRawInputReaderEnable(true)      // RawInputReader.resume(): nativeSetPenState(4) in-process, reportData=true
      if f(): setRawDrawingRenderEnabled(true)   // SFTouchRender: pen state 2
```

All **[code]** (`ResumeRawDrawingRequest.java:40-138`).

**What the bean contains** (`PenEventHandler.q()`, `:155-185`) [code]: limit rects from
`EditorBundlesKt.getTouchLimitRectList` (read on the main thread in `u()`), a copy of the exclude rects,
`penArgs = activeQuickPen`, `activeProvider`, `currentGraphicsShapeColor`,
`canBrushRawRender = handlerManager.canBrushRawRender()` (`EraseHandler`: `isEraseTrackByRawRender(type)`;
`EpdShapeHandler`: true; `SelectionHandler`: overridden; everything else: false from `BaseHandler`),
`eraserArgs = copy of activeEraserArgs`, `delayResumePenTimeMs`, and the two resume flags. The flags are
`resumeRender = A(requested)` and `resumeInput = z() && requested`.

**Preconditions (the gates in front of it)** [code]:

1. `PenEventHandler.A(render)` (`:105-107`): render is resumed only if requested **and** none of these holds:
   HWR "hold converting", keyboard shown, popup count > 0, status bar shown, toast shown, no-focus system
   dialog, `!hasFocus`, floating button active, screenshot in progress, scene does not support scribble, pen
   host view not bound, `!handlerManager.canPenRawRender()`, current layer disabled, a named pause registered
   (`onPausePenEvent`), document opening, render dialog showing, floating menu being pressed.
2. `PenEventHandler.z()` (`:271-273`): the same list for input, with `isUsePenInput()` instead of
   `canPenRawRender()`.
3. If neither can be resumed, nothing is submitted (`q()` returns `just(false)`).
4. Inside the request, **nothing happens unless render or input is currently disabled** (`f() || g()`).
   A resume issued while the session is already running is a no-op. It re-sends no style, no eraser flag and
   no pen state. The configuration is only ever re-sent across a real disabled→enabled edge.
5. `TouchHelper.setRawDrawingRenderEnabled` / `setRawInputReaderEnable` deduplicate on cached booleans
   (`TouchHelper.java:286-305`) and do nothing unless `openRawDrawing()` has run (`this.a`). The cached
   booleans are the only record of the SF state on the app side. If they drift from SF's real state, the edge
   is lost.

**The state it assumes** [code for each item, inferred for the combined picture]:

- A `TouchHelper` created with SF+App renders, `openRawDrawing()` done (pen state 1 once, stroke style 0,
  input reader thread started), and a bound host view.
- A preceding pause that disabled render (and usually input). Either `PauseRawPenAction`, which uses
  `setRawDrawingEnabled(false)` when both flags are set and thereby resets SF's eraser flags, or
  `setRawDrawingRenderEnabled(false)` alone. The request re-sends every eraser-related value after either
  kind, so it does not depend on which one it was.
- That the active handler at resume time decides the style: an `ERASE_PROVIDER` resume makes style 8 (or 5)
  the **main** stroke style; an `EPD_PROVIDER` resume puts the pen's own style back via `applyPenArgs`. In the
  eraser tool, pen-tip "draw" callbacks are erase callbacks (`usePenTipErase`). Setting style 8 as the main
  style is correct there and wrong for a writing tool.
- The eraser end relies on SF's reader selection, not on the main style. With the pen tool active, the eraser
  end paints in style 8/5 because `SET_ERASER_RAW_DRAWING_ENABLED(1, 8|5)` was sent. The pen tip keeps the
  pen's style.
- SF has a single style slot, and `EraseReader::onActivate` overwrites it (`SF@0x52ec64`). After the eraser end
  has been used, the main style in SF is the eraser style until the app sends `SET_STROKE_STYLE` again. Stock
  always does: every erase ends with `InvalidateViewWithPenControlAction`, whose resume runs this request and,
  for the pen provider, `applyPenArgs` → `setStrokeStyle(penStyle)` [code]. That the next tip stroke would
  otherwise be painted with the soft eraser is **[inferred]** from the single slot; it is testable with E3.

**Reading the failed copy against this.** The handoff reports "the sheet turned pale, a wide faded trace
appeared, and the mode did not exit" after an attempt that used style 8 with render kept on. From the code
above, three mechanisms could produce those symptoms. Each is **[inferred]**, and each has an experiment in §9:

- _Wide faded trace._ This is simply what `PainterSoftEraser` draws with `[width, 0.5, 0.1]`. Stock shows it
  too, by design, while erasing. With the partial eraser's default width of 59 px it is wide.
- _Mode did not exit._ Stock returns to the pen style only through the post-erase pause/resume edge, which
  re-sends `SET_STROKE_STYLE`. An app that sets style 8 as the main style for the eraser and never re-sends
  the pen style across a real disabled→enabled edge leaves SF painting new pen strokes in style 8. Either the
  resume is deduplicated away (point 4/5 above) or `SET_STROKE_STYLE` is simply not re-sent. The same happens if
  the eraser end is used while style 8 stays set.
- _Pale sheet._ While SF is in the handwriting schema (`androidDrawing == 0`), the app's frames are not
  composited over the region (`08-surfaceflinger.md` §2). Stock always leaves that state after an erase with
  `ENABLE_POST(-1,1,pid)` followed by pen state 3. If neither is sent, the soft-eraser trace stays on the panel
  and the app's re-render never replaces it. Whether "pale" is this or `CU` toggling is **[unknown]**.

---

## 6. Why stock does not flash after erasing

The hardware fact from the handoff: _if the pen writes while this app's SF session is paused (pen state 3), SF
switches the panel to update scheme 4 and drives the input with A2; about 1.5 s after lift it switches back
to scheme 2 with one full GC16 — the flash._ Our app erases with render off, so the session is paused during
the whole erase gesture (handoff).

What stock does instead, all **[code]** unless marked:

1. **The erase tool keeps render eligible.** `EraseHandler.canPenRawRender()` =
   `isEraseTrackByRawRender(type)` = true for 0/1/2 (`EraseHandler.java:40-43`), and the pen tool uses
   `BaseHandler.canPenRawRender()` = true (`BaseHandler.java:55-57`). `PenEventHandler.A()` therefore resumes
   render with either tool active. This is not true of every erasing tool: `FillColorHandler` returns false and
   `UniversalShapeHandler` returns false outside a gesture (§3.7).
2. **Nothing pauses at pen-down.** `EraseBeginAction` pauses only non-raw-rendered types (none in the UI). The
   buffered erasing requests do not use the pausing `BaseNoteRequest` default. The raw-pen-down handlers
   (`EpdEventHandler.onRawPenDownEvent`, `AudioMenuModel.onSelectionStartEvent`) issue no pen control.
3. **The app does not draw while the pen is down.** `ErasingShapeAction.s()` skips display for raw-rendered
   types, and the app's own eraser overlay (`h0()`) is dead code. So the app posts no frame that would need the
   handwriting schema torn down mid-gesture [code; that moving the invisible `eraseCircle` view posts no frame is
   inferred, §3.3].
4. **The firmware is told to draw the eraser input itself** (`SET_ERASER_RAW_DRAWING_ENABLED(1, 8|5)`,
   stroke parameters for 5 and 8, and `SET_BRUSH_RAW_DRAWING_ENABLED(1)` for the tip). SF's `EraseReader` or
   `DrawReader` then takes the input and paints it on the handwriting path, as it does for ink. With the
   eraser flag at its SF default of 0, the eraser end would land in `BaseEraseReader`, which paints nothing.
   Whether SF would then treat the pen input as "not ink" for scheme purposes is **[unknown]** (E2).
5. **The only pause is after lift and brief.** `EraseFinishEvent` pauses render (state 3) with the input
   reader left on, invalidates, and resumes render after `mySleep(300)`. `DELAY_ENABLE_RAW_DRAWING_MILLS` (500
   ms on colour) is not used here; the erase path hard-codes 300 ms (`EraseFinishAction.E()`) [code]. Pausing
   without pen input does not flash (handoff).
6. **SF scheme behaviour around this** (partial, see §9 E5): `afterStartHandwriting` (`SF@0x562c08`) sets
   `androidDrawing = 0`, `penTriggered = 1` and calls `setEbcUpdateScheme(3, 0)` through the provider vtable
   (slot `+0x78` = `HWEpdcManager::setEbcUpdateScheme`, `SF@0x565de0`, which logs `### EUS, s %d c %d` under
   tag `HWEpdcManager`) [code]. `afterStopHandwriting` (`SF@0x562ec0`) calls vtable slot `+0x70` =
   `HWEpdcManager::clearHandwritingFBSchema` (`SF@0x566134`). That function selects scheme **4** when a mode
   it reads through `SF@0x559ab0`/`0x559304` is one of `1, 4, 0x901, 0x904, 0x908, 0x1000001, 0x1000004,
   0x2000004`, otherwise scheme **2** [code]. `HWEpdcManager::syncFBSchema` (`SF@0x565f74`, slot `+0x60`) has
   the same selection [code]. Two things are **[inferred]**: that the mode read there is the current
   app/transient update mode, and that those values are the A2/fast family. On that reading, a correctly
   running stock erase should log `EUS, s 3` at pen-down and `EUS, s 2` after the post-erase pause. What
   drives scheme 4 when the pen writes into a paused session is **[unknown]** and belongs to the refresh/SF
   documents. Who calls `syncFBSchema` was not traced.

In short: with the pen, geometric-shape or eraser tool, stock erases the way it writes. The session is
running, SF paints the trace, and the app redraws only after lift inside one short, pen-up pause. Our app
erases with the session paused, which is the condition the handoff links to the flash. Stock's fill-colour tool
also erases with render paused (§3.7), so it is the stock counterpart to test (E11).

---

## 7. Device-specific branches on the Note Air 4C

| branch                                                 | where                                                                                                                                                                                 | value on Note Air 4C                                                                                                                                                                                                                                          | how determined                                                                                                                      |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| per-model note config                                  | `DeviceConfig.readConfig` → raw resource `"note_" + Build.MODEL.lower().replace('-','_')` (`DeviceConfig.java:27, 110-133`)                                                           | `RAW/note_noteair4c.json`: `overlayEraserBufferTimeMs 30`, `strokeEraserBufferTimeMs 20`, `isTabDevice true`, `showPenUpRefresh false`, `epdTurbo 5`, `loadingShapePointThreshold 20000`, `canDisableCTPTouch true`, `showReadNote true`, `largeObjectSize 0` | [code] for the file; `Build.MODEL == "NoteAir4C"` [inferred] from `ro.product.model = NoteAir4C` in `docs/onyx-reversing/README.md` |
| `eraserAreaDisplayDelayMillSecond`                     | `DeviceConfig` default `0` (`:89`), absent from the json                                                                                                                              | **0 ms**                                                                                                                                                                                                                                                      | [code]                                                                                                                              |
| `canPenTipErase`                                       | `DeviceConfig` default `true`, absent from the json                                                                                                                                   | true, so choosing an eraser type activates the eraser tool                                                                                                                                                                                                    | [code]                                                                                                                              |
| `supportPenHapticType`                                 | `DeviceConfig` default false, absent from the json                                                                                                                                    | false, so `UpdatePenHapticTypeAction` is a no-op                                                                                                                                                                                                              | [code]                                                                                                                              |
| colour device                                          | `DeviceInfoUtil.isColorDevice()` = `Device.getColorType() > 0` → `ViewUpdateHelper.getColorType()` (SF 1048659) (`DeviceInfoUtil.java:136-138`, `FW/…/ViewUpdateHelper.java:648-655`) | true                                                                                                                                                                                                                                                          | [inferred]: Kaleido 3 panel, and `05-panel-refresh-levers.md` states the `getColorType() > 0` gate passes on this device            |
| `DELAY_ENABLE_RAW_DRAWING_MILLS`                       | `RawPenArgs.f` (`RawPenArgs.java:69`), `note/note/event/PenEvent.java`                                                                                                                | **500 ms** (mono: 150); used by eraser type changes, layer erase and `InvalidateScreenAction`                                                                                                                                                                 | [code] + colour [inferred]                                                                                                          |
| `POPUP_RESUME_PEN_TIME_MS`                             | `note/note/event/PenEvent.java:20`                                                                                                                                                    | 500 ms (mono 300); the SDK's copy is 800/400                                                                                                                                                                                                                  | [code] + colour [inferred]                                                                                                          |
| post-erase resume delay                                | `EraseFinishAction.E()`                                                                                                                                                               | **300 ms**, not device dependent                                                                                                                                                                                                                              | [code]                                                                                                                              |
| eraser-tool activation resume delay                    | `EraseHandler.onActivate`                                                                                                                                                             | **150 ms**, not device dependent                                                                                                                                                                                                                              | [code]                                                                                                                              |
| auto-deselect eraser                                   | MMKV `AUTO_DESELECT_ERASER_KEY` (`NoteMMKVHelper.isAutoDeselectEraserEnabled`, default false)                                                                                         | user setting; default keeps the eraser tool after each erase                                                                                                                                                                                                  | [code] for the default; the value on the user's tablet is [unknown]                                                                 |
| stroke-eraser / partial-eraser buffer                  | `DeviceConfig` defaults 10 / 20 ms (`DeviceConfig.java:79-80`), overridden by the json                                                                                                | 20 / 30 ms                                                                                                                                                                                                                                                    | [code]                                                                                                                              |
| `editorViewUpdateModel`                                | `DeviceNoteConfig` (`config_note_<model>.json`) → `InkRenderManager.setViewUpdateConfigModel` (`DisplayBundle.java:81-86`)                                                            | no `config_note_noteair4c.json` exists, so the bean default applies; only used for `enableRegalModeOnce`, which the erase path never sets                                                                                                                     | [code]; the default value is [unknown]                                                                                              |
| SF handwriting eligibility                             | `canScribble()` requires panel type `EPDC+0x24 ∈ {1,2,3}` (08-surfaceflinger.md)                                                                                                      | the colour class, per 08                                                                                                                                                                                                                                      | from 08 [code there]                                                                                                                |
| `ENABLE_COLOR_CU`                                      | toggled by `EraseHandler`/`ScribbleHandler` regardless of device; stored by SF on every device                                                                                        | applied on this device; effect [unknown]                                                                                                                                                                                                                      | [code]                                                                                                                              |
| `GrayscaleRefreshAction` / `HAND_WRITING_REPAINT_MODE` | not referenced anywhere on the erase path                                                                                                                                             | —                                                                                                                                                                                                                                                             | [code, by reading the finish chain]                                                                                                 |
| stock `supportEAC:false`                               | `/onyxconfig/mmkv/onyx_config` (handoff)                                                                                                                                              | not consulted anywhere on the erase path; affects firmware tap-counter refreshes                                                                                                                                                                              | handoff                                                                                                                             |

---

## 8. Interactions with other subsystems

- **Session lifecycle.** The erase path depends on `openRawDrawing` having run (pen state 1, stroke style 0)
  and on `TouchHelper`'s cached render/input flags matching SF (§5, points 4–5). Selecting the eraser tool
  performs one or two pause/resume cycles depending on the entry point (§3.2).
- **Writing.** The eraser end with the pen tool shares the pen tool's session. After every erase, the resume
  re-sends the pen's style, width and colour. SF's style slot is shared between the tip and the eraser end
  (`MemoryPainter+0x98`) [code].
- **Selection / lasso.** `ResumeRawDrawingRequest` enables the SF eraser trace for the selection provider too
  (`provider == SCRIBBLE_SELECTION_PROVIDER`; redundant, because `isEraseTrackByRawRender` is already true for
  every eraser type), and the selection provider's main style is 5 [code]. `SelectionHandler` does not erase:
  `useDrawErase()` is false and it overrides no erasing callback (§3.7) [code].
- **Refresh.** The erase result reaches the panel through `ENABLE_POST` + pen state 3 +
  `View.invalidate()`, not through `HAND_WRITING_REPAINT`. The waveform and update mode of that frame are
  whatever the view's update mode and SF's normal schema produce [unknown]. `EraseFinishAction.z0()` resets
  the view's update mode first.
- **Input routing.** The in-process reader keeps reporting during the post-erase pause (input stays enabled).
  Whether SF's `TouchReader::pause` releases the pen to Android input, so that a stroke in that window reaches
  the app as `MotionEvent`s, is [unknown]. The handoff shows only that pen input during a pause triggers the
  scheme-4 path.
- **Firmware globals.** The SF eraser/brush/side flags are global. The last `setRawDrawingEnabled(bool)` or
  `ResumeRawDrawingRequest` by _any_ app decides them [code for storage; cross-app effect inferred].
- **Colour CU.** `enableColorCU` / `disableColorCU` bracket each erase gesture and follow the eraser tool's
  activation. `ColorModeEventHandler` also toggles it for popups when the active handler is not colour-CU
  enabled (`ColorModeEventHandler.java`) [code].

---

## 9. Open questions as hardware experiments

All experiments use `adb logcat -b all -v time` with the driver lines from the handoff:
`onyx_tcon_send_cmd … waveform[W] update_mode[M] flags[F]`, `HWEpdcManager: ### EUS, s N c M`,
`onyx_tcon_set_upd_scheme`, `SurfaceFlinger: Pid N State change to: …`, plus `E EpdcManager: CU 0|1`
(the colour-CU setter logs at error priority, `SF@0x559dfc`). If `PenLogger` output is enabled
(broadcast `ONYX_NOTE_TOGGLE_DEBUG`, `KNoteCommonReceiver.o()`), stock also prints its pause call stacks.

- **E1. Eraser-end detection.** With `adb shell getevent -lt` on the pen device, flip the stylus and touch.
  Expect `BTN_TOOL_RUBBER 1` before `BTN_TOUCH 1`. If the pen instead reports BTN_STYLUS or BTN_STYLUS2, SF
  would use the side-button path (flag default 1) and the in-process reader would not flag the touch as
  erasing unless `enableSideBtnErase` is on.
- **E2. Stock stays running while erasing.** In stock Notes, pen tool active, erase one stroke with the
  eraser end. Confirm: no `State change to: toPause` between pen-down and lift; `CU 1` at down and `CU 0` at up;
  `EUS, s 3` near pen-down (prediction from `afterStartHandwriting`); no `update_mode[1]` update in the
  following 3 s. After lift expect one pause/running pair about 300 ms apart. Repeat with the eraser tool
  selected, for types 0, 1 and 2. If a `toPause` appears before lift, the model in §3.3 is wrong.
- **E3. SF style slot.** In stock, pen tool, erase with the eraser end, then flip the pen and write
  **immediately** (under 200 ms after lift). If the first stroke is painted pale or wide, the single style slot
  is confirmed and stock's post-erase resume is what restores the style. Compare with a stroke written after
  more than 1 s.
- **E4. Stock's own vulnerable window.** Same as E3, but watch the driver log. Does a stroke that starts
  inside the ~300 ms paused window produce the scheme-4 → scheme-2 + `update_mode[1]` sequence (a flash)? This
  tells whether stock is merely lucky with timing or whether something else prevents the flash.
- **E5. Scheme numbers.** During ordinary stock writing, log which `EUS, s N` values appear at pen-down and
  after lift. §6 predicts 3 during handwriting and 2 afterwards (4 only when the current mode is in the A2
  family). If writing shows 4, the reading of `clearHandwritingFBSchema` is wrong.
- **E6. Eraser raw drawing off.** Using the debug tooling in our app: keep render on, send
  `setEraserRawDrawingEnabled(false, 5)`, and erase with the eraser end. SF uses `BaseEraseReader` (no
  painting). Does the driver log show handwriting scheme 3, the paused-session scheme 4, or neither? This
  separates "session paused" from "input not painted by SF" as the flash trigger.
- **E7. Pale sheet reproduction.** In our app, keep render on and send `setStrokeStyle(8)` plus
  `setStrokeParameters(8,[59,0.5,0.1])` while the pen tool is active, without the post-erase pause. Check
  whether (a) tip strokes become pale and wide (style slot), and (b) an app redraw without `ENABLE_POST(1)` /
  pen state 3 reaches the panel (look for `onyx_tcon_send_cmd` lines while the region stays unchanged
  visually). This checks the §5 explanation without copying anything into production.
- **E8. Colour CU.** Toggle `ENABLE_COLOR_CU` on and off from the debug bridge on a static colourful page and
  compare the next normal-schema update's `waveform`/`flags` and the visual result. This settles what "CU" is.
- **E9. Layer erase.** In stock, run "erase all layers" and confirm: pause (render + input) before the
  request, one normal-schema update for the removed shapes, resume about 500 ms later. Note whether a full
  `update_mode[1]` appears (it would come from the dialog and focus changes, not from the erase itself).
- **E11. Stock erasing in a paused session (fill-colour tool).** In stock, select the fill-colour tool, wait
  more than 1 s, then erase a stroke with the eraser end. Expect `State change to: toPause` at tool selection and
  no `running` afterwards (render refused), no live trace, and the result appearing only after lift. Record
  whether the driver log shows the scheme-4 → scheme-2 + `update_mode[1]` sequence. If stock flashes here too,
  the flash is a property of "pen input while paused" and stock merely avoids that state in its main tools; if it
  does not flash, something else differs (for example the input reader state or the eraser flag) and must be
  found.
- **E12. Universal-shape tool.** In stock, select the universal-shape tool and erase with the eraser end. Look for
  a `running` state change _after_ pen-down (the 0 ms resume, §3.7) and note whether the first part of the
  trace is painted and whether a flash follows.
- **E13. Eraser end in the selection tool.** In stock, select the lasso tool and stroke with the eraser end.
  Confirm nothing is erased, and note whether a soft-eraser/dash trace stays on screen and whether the next pen
  stroke (after switching back to the pen) is drawn in the pen's style.
- **E14. Eraser menu entry points.** Select the eraser from the eraser menu (tool change) and, separately, change
  the eraser type while the eraser is already active. Count the `toPause`/`running` pairs: §3.2 predicts two
  overlapping cycles in the first case and one in the second.
- **E10. Build check.** `adb shell getprop ro.build.display.id` and the SF binary hash against the extracted
  one, so the `SF@` addresses in this document can be trusted for the running firmware.

---

## 10. Verification notes

An independent re-check of this document against the primary sources (static only; no device access).

**Checked and confirmed** (opened in `~/tmp_zfs/boox-notes-inspect/decoded/sources`, cross-checked in
`~/tmp_zfs/reversed_onyx_notes_app/src` where noted):

- `EraseUtils.isEraseTrackByRawRender` / `isMoveStrokeErase`; `EraseHandler` overrides (`canPenRawRender`,
  `canBrushRawRender`, `isEnableColorCU`, `usePenTipErase`, `onActivate` with the 150 ms resume,
  `onEraseFinished` with `y0()`).
- `EraseBeginAction` predicate (`!isEraseTrackByRawRender`), so no pause at pen-down for types 0/1/2.
- `ScribbleHandler` gesture pipeline: `a()`, `k0()`, `l()`, `m()`, `l0()`, `h()`, `q()`, `canDrawErase()`,
  `s()` buffer times, `onEraseFinished`.
- `TouchEventHandler` rerouting of draw callbacks when `usePenTipErase()`.
- `ErasingShapeAction.s()` early return and `x()` request dispatch; `AreaErasingRequest` (`setPauseRawDraw(false)`),
  `BaseEraseRequest extends RxBaseWorkRequest`, `BaseNoteRequest` default pause fields.
- `EraseFinishAction.E()` (`pauseResumeArgs`, 300 ms, input not paused), `render()` with
  `invalidate=false, enablePost=true`, `u()` posting `EraseFinishEvent`.
- `InvalidateViewWithPenControlAction` order (pause on the pen scheduler, invalidate on main, resume);
  `PauseRawPenAction.n()` branch (render only → `setRawDrawingRenderEnabled(false)`); `ResumeRawPenAction`.
- `TouchHelper.setRawDrawingEnabled` → `resetPenDefaultRawDrawing()`; dedup in `setRawDrawingRenderEnabled` /
  `setRawInputReaderEnable`; `SFTouchRender.h()` (leaveScribbleMode + pen state 3); `AppTouchRender`
  `ENABLE_POST(1)` on disable; `EpdPenManager` pen-state constants.
- `ResumeRawDrawingRequest` in full (identical to the document's pseudo-code), `PenEventHandler.q()`, `A()`,
  `z()`, `s()`.
- `InkRenderManager.invalidate` / `enablePost`; `ViewUpdateHelper.enableColorCU` → `ENABLE_COLOR_CU`.
- `DeviceConfig` defaults and `RAW/note_noteair4c.json` contents.
- SF disassembly spot checks: `EraseReader::onActivate` (`SF@0x52ec64`) loads the style from `0xa5e708` and
  calls `MemoryPainter::setStrokeStyle` (`SF@0x52548c`, bounded store to `+0x98`); `DrawReader::onActivate`
  (`SF@0x52ec18`) sets no style; the inline eraser handler at `SF@0x55f8e0` stores the flag to `0xac9ad8` and the
  style to `0xa5e708`; `BtnReader::onSynEvent` (`SF@0x52e028`) reader selection 4/3/2/1;
  `setHandWritingPenState` `cmp w1,#3; b.hi` (`SF@0x5666cc`). In-process reader: `isRubberDown` /
  `isSideTouchDown` offsets (`PTR@0xaf68`, `PTR@0xaf84`).

**Changed** (old → new):

- "Stock never has the session paused while the eraser is on the glass" → true for the pen, geometric-shape and
  eraser tools only; the fill-colour tool erases with render off, and the universal-shape tool turns render on
  only at eraser pen-down (new §3.7, table rows, E11/E12).
- "`h0()` … only for non-raw-rendered types, so it never fires for 0/1/2" → `h0()` has no caller at all (dead
  code in both decompilations); the per-point `UpdateEraseViewPositionEvent` moves an invisible `ImageView`
  (no-frame claim marked [inferred]).
- "Selecting the eraser tool produces one PAUSE … followed about 150 ms later by a RESUME" → two overlapping
  cycles from the eraser menu (`onActivate` 150 ms + `InvalidateScreenAction` 500 ms), one cycle when only the
  type changes or when entered through `StartEraseAction`.
- "Erase strokes never start the SDK's pen-up refresh timer" → true for eraser-end strokes; tip strokes in the
  eraser tool do start it, but its only consumer is the empty `BaseHandler.onPenUpRefresh`.
- "At pen-up … posts `ENABLE_POST(-1,1,pid)`" → only when the stroke changed a visible shape; the pause/resume
  runs regardless.
- "…evaluated against the current active handler, which is the pen handler again if auto-deselect ran [code]"
  → ordering downgraded to [inferred]; auto-deselect default (false) added.
- "Selection (lasso) tool, eraser end … not traced [unknown]" → resolved: no erase, no post-erase pen control.
- `ForceRenderSliceListActionsKt.java:541-566` citation → the shape-list overload at `:267-287` that delegates to
  it.

**Added**: §3.7 (all `useDrawErase()` handlers and the non-erasing ones); the `PenEvent.pauseDrawingRender()`
no-op; the `BtnReader` `+0x2054` early return; colour-CU asymmetry in the eraser tool; doubled
`RawPenDownEvent`/`RawPenUpEvent`; the `EraseBeginAction` pre-render cancel; the empty-point-list edge case;
duplicate region/style/colour/width transactions from the two renders; `HandWritingHandler`'s second
`TouchHelper` resetting SF's global eraser flags; `PenTipEraseToolMenuModel` → `StartEraseAction` entry point;
`UniversalShapeHandler.C0()` provider switch; buffer-time defaults; experiments E11–E14.

**Could not confirm statically**:

- What sets `PenManager+0x2054` in SF, and therefore whether a paused SF session selects an eraser reader at all.
- SF's reaction to a `SET_PEN_STATE(2)` that arrives while the pen is already down (universal-shape tool).
- Whether the fill-colour and universal-shape paths flash on hardware (E11, E12).
- The in-process reader's initial `enableSideBtnErase` value (irrelevant without a side button).
- `isShortcutErasing` (`PTR@0xa508`) was not disassembled beyond its prologue; its "active reader == 3" reading is
  carried over from the first version.
- The SF/`08-surfaceflinger.md` claims about `handleEnablePost` / `stopHandwriting` and the scheme selection in
  `clearHandwritingFBSchema` were not re-disassembled here.
- Whether the text-box `HandWritingHandler` is ever active while the note editor erases.
