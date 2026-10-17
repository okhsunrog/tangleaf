# Eraser stock parity: implementation plan

Written 2026-09-25 against `main` at `0f4dbee`. Static analysis only: nothing was run and the tablet
(BOOX Note Air 4C, firmware 4.2, system Android 13, vendor Android 11) was not touched while writing it.

Goal: Tangleaf's eraser looks, feels and behaves like the eraser in stock Onyx Notes (`com.onyx.android.note` 45326) on the tablet, and its eraser menu looks like stock's. The work runs **one step at a time**. After each
step the user installs the debug build, records the driver log and the PenTrace call log, and compares them with
the stock recordings. The next step starts only after the user reports back.

Tags: **[code]** read in source at the cited line; **[log]** read in a recorded trace; **[doc]** taken from an
earlier document and not re-verified here; **[inferred]** reasoned from what was read; **[unknown]** not settled
statically; **[HW]** an assumption the hardware must confirm (collected in §11).

| prefix  | location                                                                                                                                                                                        |
| ------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `kt/`   | `plugins/tauri-plugin-mobile-system/android/src/main/java/dev/okhsunrog/mobile_system/`                                                                                                         |
| `kts/`  | `plugins/tauri-plugin-mobile-system/android/src/test/java/dev/okhsunrog/mobile_system/`                                                                                                         |
| `hw/`   | `src/features/handwriting/`                                                                                                                                                                     |
| `SN/`   | `docs/onyx-reversing/stock-notes/`                                                                                                                                                              |
| `NA/`   | `~/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/` (stock Notes 45326, jadx)                                                                                                      |
| `RES/`  | `~/tmp_zfs/boox-notes-inspect/decoded/resources/res/`                                                                                                                                           |
| `PUB/`  | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-pen-1.5.4.3/sources/com/onyx/android/sdk/pen/` (the SDK we link)                                                                                     |
| `DEV/`  | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-device-1.3.5.2/sources/com/onyx/android/sdk/` (the SDK we link)                                                                                      |
| `MAP`   | `docs/planning/eraser-current-path.md` (Tangleaf's eraser today)                                                                                                                                |
| `FW/`   | `~/tmp_zfs/onyx_framework/out/sources/` (the tablet's framework.jar, jadx)                                                                                                                      |
| `OBS/`  | `~/tmp_zfs/pen-observation/` (recordings of 2026-09-25)                                                                                                                                         |
| `A4`    | `OBS/a4-stock.txt` (stock, pen tool, one eraser-end stroke; line numbers of that file)                                                                                                          |
| `A5`    | `OBS/a5-stock.txt` (stock, eraser end then tip)                                                                                                                                                 |
| `A7`    | `OBS/a7-stock.txt` (stock, eraser tool, menu, type changes)                                                                                                                                     |
| `FA4`   | `OBS/fa4.txt` (Tangleaf, eraser end, before the stylus-routing fix)                                                                                                                             |
| `F6`    | `uv run OBS/penlog.py OBS/run-4.log F6-A4 R` (Tangleaf after `0f4dbee`; line numbers of that output)                                                                                            |
| `SHOT`  | `OBS/shots/stock-eraser-menu.png` (the user's screenshot of the open stock menu)                                                                                                                |
| `PATCH` | the reverted soft-eraser experiment, `~/tmp_zfs/claude_tmp/claude-1000/-home-okhsunrog-code-rust-notes-rs/977a944e-8c88-465b-bebe-fa0927676d77/scratchpad/uncommitted-eraser-experiments.patch` |

---

## 0. Rules for the executor

- **One step per branch** (`eraser-parity-s1`, `-s2`, …), branched from the previous step once the user has
  accepted it. Stop at the end of the step and hand over. Commit only when the user asks. Never title a commit
  "fix" before the user has confirmed the step on the tablet. Commits and PR texts never mention an assistant.
- **Task ids** (`S2.4` etc.) are stable. When the plan is amended, add ids; do not renumber.
- **Validation gates** for every step: `vp check`, `vp test`, and the plugin unit tests
  (`cd src-tauri/gen/android && ./gradlew :tauri-plugin-mobile-system:testDebugUnitTest`; the project name is from
  `src-tauri/gen/android/tauri.settings.gradle`). Then `just build-android-debug` for the device build. No Rust
  command changes are planned, so `src/lib/bindings.ts` is not regenerated.
- **Stop and report** instead of improvising when: a cited line no longer matches (the queued handwriting layout
  and per-page size work may land first and move code), a change would touch a file or function this plan does
  not name, a device-verified mechanism listed in §9 would have to change, or a unit test can only pass by
  weakening an existing assertion.
- The executor does not drive the tablet. The user installs, records and reports.
- Constants that are 1000 today (sheet units across) must come from one place. If the per-page size change has
  landed, use the page width it introduces, never a literal `1000`.

**Recording procedure for the user** (the same as on 2026-09-25, `SN/11-observations.md:7-23`): logcat of all
buffers with the PenTrace module active, a `adb shell log -t PenMark <step>-<action>` marker before each action,
then `uv run penlog.py <log> <from-marker> <to-marker>`. Each step below names its markers. Every step's
recording must also show **no** `EInkHelper.beforeScroll`, **no** `### EUS, s 4` and **no** `update_mode[1]`
around an erase. Those three were the flash, and the stylus routing removed them (`SN/11-observations.md:84-88`)
[doc].

---

## 1. The steps and why in this order

| step | what changes                                                                           | SF calls change? | what the user compares                                              |
| ---- | -------------------------------------------------------------------------------------- | ---------------- | ------------------------------------------------------------------- |
| 1    | eraser settings in mm (stock ranges, defaults, per-type memory) and the stock menu     | no               | the menu against SHOT; eraser size on the glass; `status()` numbers |
| 2    | eraser end with the pen tool: firmware trace, no pause at pen-down, stock hand-back    | yes              | A4 / A5 against the new recording                                   |
| 3    | eraser tool (tip): main style 8, render on, the same hand-back                         | yes              | A7 gestures 1-2 (`A7:2-155`)                                        |
| 4    | lasso eraser type: dash trace (style 5) for the eraser end and the eraser tool         | yes              | `A7:241-330`                                                        |
| 5    | erase geometry where stock differs visibly: lasso cuts vertices out; centreline radius | no (JS only)     | the same drawings erased in stock and in Tangleaf                   |

The menu comes first because every native step needs the eraser width in stock's unit. Stock sends the width as
`mm × densityDpi / 25.4` device pixels (`NA/sdk/extension/DimensKt.java:11-13`) [code]. With the mm model in place, step 2's target line is byte-identical to stock's
(`SF SET_STROKE_PARAMETERS [8, 3, 1114388593(59.05512f), …]` for a 5.00 mm pixel eraser, `A7:445`) [log]. Today
the eraser size is a radius in sheet units, and SF would get whatever that happens to be in pixels.

The eraser end with the pen tool comes before the eraser tool because it touches fewer paths: the pen tool
already renders, so step 2 only removes a pause and adds the hand-back. Step 3 then changes what the eraser tool
renders, and the pen state on session open. The lasso type gets its own step because it uses another SF painter
(style 5) and another JS drawing path (the dashed polygon, `hw/ink-canvas.tsx:270-282`) [code].

---

## 2. Facts the plan rests on

### 2.1 Stock, recorded on the tablet

Eraser end, pen tool (`A4:1-60`, the same in `A5:5-64` and `A5:74-132`) [log]:

```
cb onBeginRawErasing                                   A4:2
EpdController.enableColorCU()  SF ENABLE_COLOR_CU [1]  CU 1          A4:3-5      (no pen-state change)
cb onEndRawErasing                                     A4:7
EpdController.disableColorCU() SF ENABLE_COLOR_CU [0]  CU 0          A4:9-11     (lift +7 ms)
EpdController.resetViewUpdateMode(EditorView)          A4:12-13      (lift +57 ms)
EpdController.enablePost(EditorView, 1)  SF ENABLE_POST [-1, 1, pid]  Post enabled!  ### EUS, s 2 c 0   A4:14-19  (lift +93 ms)
TouchHelper.setRawDrawingRenderEnabled(false)  SF ENABLE_POST, PEN_STATE [3], toPause, ENABLE_POST     A4:20-24  (+170 ms)
PANEL waveform[2] update_mode[0] flags[0x0] (×2)                     A4:25-26
TouchHelper.setLimitRect … setExcludeRect … setPenUpRefreshTimeMs(500) … setSingleRegionMode()         A4:27-35  (+343 ms)
setStrokeColor, setStrokeStyle(1), setStrokeWidth(7.80099), setStrokeColor                            A4:36-47
ViewUpdateHelper.setStrokeParameters(1, [0.3, 0.6]) / (5, [5.0]) / (8, [80.314964, 0.5, 0.1])        A4:48-53
TouchHelper.setEraserRawDrawingEnabled(true, 8)  TouchHelper.setBrushRawDrawingEnabled(true)          A4:54-57
TouchHelper.setRawDrawingRenderEnabled(true)  SF PEN_STATE [2]  running                              A4:58-60
```

The timings repeat. From `enablePost` to render off: 170, 168 and 170 ms (`A4:14,20`; `A5:18,24`; `A7:62,68`). From
render off to the re-send: 343, 323 and 341 ms (`A4:20,27`; `A5:24,31`; `A7:68,76`) [log]. Stock's code has the
300 ms (`EraseFinishAction.E()`, `SN/03-erasing.md` §3.4) [doc]. The 170 ms is not a constant in stock's code.

**The `ENABLE_POST(1)` call itself takes about 150 ms when it takes SF out of schema 7** (review, native):

- The transaction is a synchronous binder call: `ViewUpdateHelper.enablePost(int)` → `transactData(ENABLE_POST, …)`
  → `service.transact(i, parcel, parcel2, 0)`, flags 0 (`FW/android/onyx/ViewUpdateHelper.java:602-607, 1421-1437`)
  [code]. `EpdController.enablePost(view, 1)` reaches it through `SDMDevice.enablePost` (reflection,
  `DEV/device/SDMDevice.java:1769-1776`) [code].
- `SFTouchRender.setDrawingRenderEnabled(false)` issues `leaveScribbleMode` (ENABLE_POST) and `pauseDrawing`
  (PEN_STATE 3) back to back on one thread (`PUB/touch/SFTouchRender.java:195-198, 278-283`) [code]. When SF
  logs `Post enabled!` for that ENABLE_POST, the PEN_STATE 3 follows 150-165 ms later: `A7:8→13` (161 ms, stock),
  `OBS/ta4.txt:6→14` (163 ms), `OBS/r2.txt:300→307`, `:566→573`, `:953→960` (162-164 ms, Tangleaf) [log]. When SF
  was not in schema 7 the two are 1-2 ms apart (`A4:21-22`, `A7:165-166`) [log]. When SF picked scheme 4 instead
  of 2 the gap is 12-13 ms (`OBS/r2.txt:59→63`, `:246→252`) [log].
- `### EUS, s 2 c 0` is logged in the same millisecond as `Post enabled!` (`A4:16-17`, `A7:64-65`, `A7:121-122`)
  [log], so the 150 ms is spent after the scheme switch and before the call returns. `SN/00-overview.md` §2.2
  ("sleeps 150 ms and calls `clearHandwritingFBSchema`") is consistent with that; the earlier reading here, that
  SF switches the scheme only after the sleep, was wrong.
- Stock's 170 ms is therefore that blocking call on its main thread (`A4:14`) plus the hop to the pen scheduler
  [inferred]. Our hand-back calls `enablePost` on the main thread too (S2.3), so the main thread, and with it
  every raw-input callback that `SFTouchRender` posts there (`PUB/touch/SFTouchRender.java:40-137`), waits
  about 150 ms. The settle delay is measured from the start of the call, not from its return (S2.3).

Eraser tool (`A7:2-155`) [log]: on selection, `SET_STROKE_STYLE [8]` becomes the main style, followed by
`(5, [5.0])`, `(8, [80.314964, 0.5, 0.1])`, `[1, 8]`, brush `[1]`, `setRawInputReaderEnable(true)`, and render on
(`A7:19-45`). **No `setStrokeWidth`** is sent for the eraser tool. Tip strokes come as `onBeginRawDrawing`
(`A7:48`), with CU 1/0 around each one (`A7:51, 58`). After each lift the hand-back is the same as above, with
style 8 in the re-send (`A7:62-100`). `onPenUpRefresh` arrives about 500 ms after the lift and nothing reacts
to it (`A7:75`).

Lasso type (`A7:241-257`) [log]: `SET_STROKE_STYLE [5]`, `(5, [5.0])`, `(8, [5.905512, 0.5, 0.1])`,
`SET_ERASER_RAW_DRAWING_ENABLED [1, 5]`.

Menu (`A7:157-257`) [log]:

- Opening it: `ENABLE_COLOR_CU [1]`, `setRawDrawingEnabled(false)` (pen state 3, brush 1, eraser `[0, 5]`),
  then partial updates flagged `0x400000`.
- Closing it: `ENABLE_REGAL [1]`, `setViewDefaultUpdateMode(REGAL_PLUS)`, `SET_APPLICATION_DISPLAY_SCHEME [3]`,
  one `waveform[16] update_mode[0]`, and the resume about 500 ms later (`A7:223-257`).

### 2.2 The width values: the brief's pairing is wrong

`80.314964` is **6.80 mm**, not 5.00 mm. With stock's float arithmetic,
`(hundredths / 100f) * densityDpi / 25.4f` at `densityDpi = 300` gives exactly these values:

- 500 → `59.05512f` (bits `1114388593`)
- 50 → `5.905512f` (`1086126580`)
- 680 → `80.314964f` (`1117823299`)

All three bit patterns match the recorded payloads [log `A7:37, 250, 445`; arithmetic run for this plan].
`fromPercentage` is `percentage / 100.0f` (`NA/sdk/scribble/utils/NoteUnits.java:28-30`) [code]. The 5.00 mm
pixel eraser in SHOT belongs to `59.05512` (`A7:445`, recorded after the user chose it). `SN/12-eraser-menu.md`
§6.4 reaches the same conclusion. `SN/13-eraser-geometry.md` §2.2 explains the difference with a second
densityDpi of 408; that explanation is superseded [inferred: one DPI and a 6.80 mm setting explain all three
values]. **Target for a 5.00 mm pixel eraser: `59.05512`.**

### 2.3 Tangleaf today

- Eraser end, pen tool: render goes off 1-6 ms after `onBeginRawErasing` (`F6:113-119`, `F6:145-150`). A
  `HAND_WRITING_REPAINT_MODE` frame follows about 260 ms after the lift, and render comes back on about 12 ms
  after that (`F6:135-139`). There is no re-send, no CU and no `ENABLE_POST` of its own [log]. Details are in
  `MAP` §4.1 [doc].
- The eraser end often lands in two contacts: begin and end 3-6 ms apart, then a second begin 9-80 ms later
  (`F6:113-126`, `F6:168-177`) [log]. The hand-back in step 2 has to survive this.
  - In 4 of the 5 recorded Tangleaf eraser-end gestures the first end arrived 0-20 ms after our pen-down
    render-off (`F6:115→117`, `F6:171→172`, `OBS/ta4.txt:5→11`, `FA4:127→132`); the fifth had no split
    (`F6:146→153`) [log]. Stock, which keeps render on, shows one begin and one end per gesture (`A4:2,7`;
    `A5:5,11`; `A5:74,79`) [log]. The split is therefore probably caused by the render-off mid-contact
    [inferred], and the trace path of step 2 should remove it [HW H14].
- Sheet size on the recorded session: the limit was `[69,462][1792,2479]` (`FA4:70`) [log], so the sheet is about
  1723 device px wide and one sheet unit is about 1.72 px [inferred: the limit is the sheet clipped vertically
  only]. Today's radii 6 / 12 / 24 are then 1.75 / 3.50 / 7.0 mm in diameter at 300 dpi [inferred arithmetic].
  Stock's defaults are 0.50 mm (stroke) and 5.00 mm (pixel) (`SN/12-eraser-menu.md` §6.1) [doc], so the stroke
  eraser becomes about seven times narrower than today's default.
- **Tangleaf does not run at stock's density.** The project records the tablet's WebView at 661×882 CSS px, DPR
  2.8, "per-app DPI set by the user" (`HANDWRITING_UI_PLAN.md:13`, measured 2026-09-06) [doc]. The concurrent
  layout plan uses DPR 2.8125 (`docs/planning/handwriting-page-layout.md` §1) [doc]. The recorded limit agrees: a
  sheet of 661 − 2·24 (`sm:px-6`) − 2 (border) = 611 CSS px is 1718 device px at 2.8125, against the recorded 1723. At DPR 1.875 the sheet would be capped at 900 CSS px, which is 1684 device px (`FA4:70`;
  `hw/handwriting-note-view.tsx:787, 794`) [inferred]. So Tangleaf's `densityDpi` is presumably 450 = 2.8125 × 160,
  while stock runs at 300 (906 px for its 483 dp popup, `A7:203` via `SN/12` §2.2) [inferred].
  - Stock's formula with our `densityDpi` would make every eraser 1.5 times too large. S1.9 uses the density
    without the per-app override instead.
  - Stock's dp are not our CSS px: 1 stock dp = 1.875 device px = 0.667 CSS px here. `SN/12-eraser-menu.md` §11
    assumed 1 CSS px = 1 dp, and that does not hold at this DPI. S1.3 scales every size.
- SF keeps one stroke-style slot. `EraseReader::onActivate` writes the eraser style into it and nothing restores
  it (`SN/03-erasing.md` §3.1, §5) [doc]. The eraser and brush flags are process-global in SF
  (`SN/03-erasing.md` §3.1) [doc].

---

## 3. The failed attempt, read against this plan

`PATCH` (reverted) did the following [code, read in PATCH]:

- It deleted `InkEraserRenderGate`, so render stayed on for erase gestures.
- It sent `(8, [2 × radius in px, 0.5, 0.1])` and `setEraserRawDrawingEnabled(true, 8)` on every resume.
- For the eraser tool it set style 8 **and** `setStrokeWidth(eraser px)`.
- It marked erase reconciles with no update mode at all (`if (!reconcile.erasing) repaintMode.acquire()`).
- It had **no** `ENABLE_POST(1)`, **no** pause after the lift and **no** re-send of the pen style after an
  erase, and JS kept drawing the circle and the previews during the gesture.

| symptom on the tablet       | mechanism (from PATCH and `SN/03-erasing.md` §5)                                                                                                                                                                                                                                                       | prevented by                                                                                                                                                               |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| the sheet turned pale       | SF stayed in handwriting schema 7 after the erase: render never went off and no `ENABLE_POST(1)` was sent. The erased frame carried no mode, so SF did not composite it, and the half-opaque soft-eraser trace stayed over the ink [inferred]. JS frames during the gesture were not composited either | S2.6 (`ENABLE_POST(1)` after the erased canvas is ready, then render off and a plain frame, the stock order); S2.8 (JS draws nothing during the gesture)                   |
| a wide faded trace appeared | this is `PainterSoftEraser` with `[w, 0.5, 0.1]`, stock's look while the pen is down. It was wide because the width was a sheet-unit radius doubled (24 units ≈ 41 px) and, in the eraser tool, also the stroke width. It stayed because nothing removed it                                            | S1 (width in mm like stock; stroke eraser default 0.50 mm); S3.3 (no `setStrokeWidth` for the eraser tool, `A7:19-45`); S2.6 (the hand-back removes the trace at the lift) |
| the mode did not exit       | (a) After an eraser-end contact the SF style slot held 8, and nothing re-sent style 1 until the next tool change, so tip strokes were drawn as eraser. (b) With render never going off, SF never left schema 7 [inferred]                                                                              | S2.2 (the full pen settings are re-sent on every resume, the stock `ResumeRawDrawingRequest` set); S2.6 (the hand-back ends with that re-send); S2.5 (style-slot rule)     |

---

## 4. Step 1: eraser settings in mm and the stock eraser menu

No SF call changes in this step. Erasing still pauses render and JS still draws the circle, but at the new size.

### 4.1 Settings model (JS)

**S1.1** New `hw/eraser-settings.ts`:

- `EraserMode` stays `"stroke" | "pixel" | "lasso"` (`hw/ink-editing.ts:3`). Stock values: stroke 0, pixel 1,
  lasso 2 (`SN/12-eraser-menu.md` §5.1) [doc].
- Menu order: `["pixel", "lasso", "stroke"]` (`layout-sw623dp/layout_shape.xml:22-97` via `SN/12` §4.3) [doc].
- Names: `Pixel Eraser`, `Lasso Eraser`, `Stroke Eraser` (`strings.xml:1700, 191, 2997` via `SN/12` §5.1) [doc].
- Diameter as an integer in hundredths of a mm:
  - stroke `50..800`, default `50`;
  - pixel `100..800`, default `500`;
  - lasso fixed `50`, never shown;
  - step `5` for the `<`/`>` buttons and for each slider position, so 151 positions for stroke and 141 for pixel
    (`SN/12` §6.1) [doc].
- `eraserTitle(type, d)` returns `"Pixel Eraser 5.00mm"`, `"Stroke Eraser 0.50mm"` or `"Lasso Eraser"`. The value
  is `(d / 100).toFixed(2) + "mm"`, with no space before `mm` (`SN/12` §5.2) [doc].
- `stepDiameter(type, d, ±1)` clamps at the ends. `indexOf(type, d) = (d − min) / 5` and
  `diameterAt(type, i) = min + 5i`.
- Store: `zustand` + `persist`, the pattern of `hw/input-capabilities.tsx:22-40` [code], under the name
  `tangleaf.eraser-settings.v1`. Fields: `type` (default `"stroke"`, stock's fresh-install type, `SN/12` §5.1),
  `diameters: {stroke, pixel, lasso}`, `autoDeselect: false`.
- `normalizeDiameter(type, d)`: clamp to the type's range, then round to a multiple of 5 the way stock does
  (`remainder < 3` rounds down, else up; `NA/sdk/utils/NumberUtils.java:71-74`, used by
  `NoteUnits.widthPercentage`, `NA/sdk/scribble/utils/NoteUnits.java:67-69`) [code]. The store's `persist`
  options get `version: 1` and a `merge` that runs every stored diameter through it and replaces an unknown
  `type` with the default, so `indexOf` never sees an off-grid or out-of-range value from an older build or a
  hand-edited `localStorage` [inferred: `persist` does no validation of its own].
- Scope: global on this device. Stock stores the type globally and the widths per notebook (`SN/12` §10) [doc].
  See Q5. The tool itself is not persisted: it starts as `"pen"` on every mount
  (`hw/handwriting-note-view.tsx:182`) [code], so opening another note shows the pen with the remembered eraser
  settings behind the eraser button.

**S1.2** `hw/handwriting-note-view.tsx`:

- Replace `eraserMode` / `eraserRadius` (`:184-185`) with the store.
- **Add** `previousTool` (new state: the view has no such variable today, `hw/handwriting-note-view.tsx`
  [code, grep]). Set it to the outgoing tool whenever the tool changes to the eraser. It is never `"eraser"`
  and starts as `"pen"`. Stock likewise skips `NONE` and the eraser (`ScribbleHandler.java:1277-1283` via
  `SN/12` §7.2) [doc].
- The toolbar eraser button shows the icon of the current type (`SN/12` §3.2) [doc]. `TOOLS` (`:86-90`) holds a
  fixed `Icon`, so the eraser entry's icon has to come from the store at render.
- Tap logic is unchanged (`:540-550`): the first tap switches tools, a tap on the active eraser opens the menu.
  This matches stock (`SN/12` §3.1) [doc].
- A tap on the eraser button while its menu is open closes the menu and does not reopen it. This is stock's code
  intent (`ToolbarMenuModel.J()`, `SN/12` §3.1). What happens today [code]: the anchor is not a
  `Popover.Trigger`, so a press on it is an outside press. Base UI dismisses a pen press like a mouse press, on
  the `click` (`node_modules/@base-ui/react/floating-ui-react/hooks/useDismiss.js:181-193`: pen maps to
  `mouse`, which the popover sets to `'intentional'`, `popover/root/PopoverRoot.js:113-119`). The dismissal
  runs from a one-shot listener on the target (`useDismiss.js:315-343`), so it reaches `onOpenChange(false)`
  before React's `onClick` sets the menu open again. So the menu does not stay closed: it either closes and
  reopens within one tap or, if React batches both updates, never closes [inferred]. In both cases the `Popover`
  wrapper requests a full GC for the reported close (`src/components/ui/popover.tsx:10-16`). Fix:
  - `onOpenChange(open, details)` ignores a close with `details.reason === "outside-press"` whose
    `details.event.target` is inside `optionsAnchor`. Base UI 1.6 passes both
    (`popover/root/PopoverRoot.d.ts:29, 82`; `popover/store/PopoverStore.js:73-82`) [code].
  - The second-tap branch (`:543-547`) toggles: when the menu is open it closes it.
  - This applies to the pen and lasso options too, because the three tools share the popover.
  - Q4.
- Replace the eraser block (`:642-713`) with `<EraserMenu>` (S1.3). The pen and lasso option blocks stay as they
  are. The shared `PopoverContent` carries `className="w-auto max-w-[min(22rem,calc(100vw-1.5rem))] p-2"`
  (`:616`) on top of the wrapper's `w-72 rounded-xl p-4 shadow-popover` (`src/components/ui/popover.tsx:49`). For
  the eraser the class becomes tool-dependent: `p-0`, the S1.3 width, `shadow-none` and the S1.3 radius
  (tailwind-merge in `cn` drops the defaults). Keep `data-ink-tool-options` on it, because the existing tests
  look buttons up through it (`hw/handwriting-note-view.test.tsx:150-157`).

**S1.3** New `hw/eraser-menu.tsx`, the menu content.

The numbers in the table are **stock dp**. Every length is rendered as `n × K` CSS px, with one constant:
`K = 1.875 / window.devicePixelRatio` when the BOOX pen SDK is active (`capabilities.nativeDeviceEvents`), and
`K = 1` elsewhere. At the tablet's DPR 2.8125, K is 2/3: the menu is 322 CSS px wide, which is physically the
same as stock's 906 device px. That width also agrees with the eraser popover slot in
`docs/planning/handwriting-page-layout.md` §8 [doc]. "Device px" values (borders, radii) are rendered as
`n / devicePixelRatio`: at 2.8125 a 2 device px line is **0.71 CSS px** and a 14 device px radius is **5 CSS px**.
Chromium draws such hairlines on whole device pixels [inferred].

How to write it: set two custom properties on the popup, `--dp: ${K}px` and `--dpx: ${1 / devicePixelRatio}px`.
Every size in the menu is then a Tailwind arbitrary value such as `px-[calc(21.2*var(--dp))]` or
`h-[calc(2*var(--dpx))]`, and no size is duplicated in JS. Use **no rem** inside the menu: rem follows the system
font scale on the tablet (13.6 px at scale 0.85, `docs/planning/handwriting-page-layout.md` §1) [doc], and stock's
dp lengths do not.

Font sizes use the same K on stock's sp values: 20 sp is `20 × K` CSS px. Stock's sp and our text both follow
the system font scale [inferred] [HW H9]. Stock caps the scale for these labels (`LimitedMaxSizeTextView` with
`maxFontScale = erase_popup_text_max_scale`, `RES/layout-sw623dp/layout_shape.xml:13-21`) [code]. After the first
screenshot, adjust K for text only if the glyph heights differ from SHOT. The e-ink profile sets
`font-weight: 500` on `body` (`src/index.css:1195-1197`) [code], and SHOT's text looks regular weight [shot].
Compare stroke weight on the first screenshot as well, and decide there whether the menu sets 400.

In the table, the numbers are stock dp and the text sizes are stock sp, both scaled by K. "px" appears only
where the stock value is in device px.

| part                       | spec (stock source: `SN/12` §4, §11)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| container                  | width 483 (max `calc(100vw − 24px)`), height wraps; white; black border 3 device px; radius 19 device px; padding-top 28.6; no shadow. On e-ink the unlayered rule `[data-display="eink"] [data-slot="popover-content"] { border: 1px solid }` (`src/index.css:1224-1228`) wins over any Tailwind border class, so the border is 1 CSS px = 2.8 device px there [code]. Do not fight it. **Placement** [shot + code]: in SHOT the menu's top border is 23 device px (8 CSS) below the toolbar's bottom rule, and the menu spans 914-1819 device px (325-647 CSS). `sideOffset` is measured from the anchor button, so it is 8 CSS plus the distance from the button's bottom to the toolbar rule (today 6 px padding plus 1 px border, `:516`), which gives 15. With today's `align="center"` the menu spans about 296-618 CSS. `align="start"` alone does not reach SHOT's position: base-ui's default `collisionAvoidance.align` is `'flip'` (`node_modules/@base-ui/react/utils/useAnchorPositioning.js:85, 159-171`), so a start-aligned menu that collides flips to end alignment and lands left of the button. Reproducing SHOT needs `align="start"` together with `collisionAvoidance={{ align: "shift" }}` passed through `PopoverContent`, which gives 327-649 CSS [inferred arithmetic]. Default: `align="center"`; the shift variant is Q20 |
| title                      | `eraserTitle(...)`, 20 sp, black, left, inset 21.2. Updates live                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| type row                   | margin-top 13.8. Three equal cells between 92.2 insets, each 91.1 tall and a whole-cell button. It is a `role="radiogroup"` (`aria-label="Eraser type"`) of `role="radio"` buttons with `aria-checked` and `aria-label` = the type name, **not** `aria-pressed`: every `aria-pressed="true"` button in the tool options is painted solid black with white content (`src/index.css:1201-1213, 1237-1242`) [code], which would replace stock's dot with an inverted cell. Icon 33.9, centred. Selected type: an 11 dp black dot (`note_color_indicator_size` → `ic_note_point_size` 11dp, `RES/values-sw745dp/dimens.xml:642`, `RES/values/dimens.xml:1811`) [code], 11.7 above the cell bottom. Order Pixel, Lasso, Stroke. Tap = set type (its own stored diameter comes back), **close the menu**; tapping the selected type also closes it (`SN/12` §5.3)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| divider 1                  | full width, 2 device px (≈ 0.71 CSS px at DPR 2.8125), black                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| diameter block (not Lasso) | label `Eraser Diameter` 20 sp at inset 21.2, margin-top 21.2. On the right: `<` (36×36 outlined rounded square with a chevron, `aria-label="Decrease eraser diameter"`), a **21.2 gap**, the value (`5.00mm`, 20 sp, baseline-aligned with the label), a **21.2 gap**, `>`, right inset 21.2. The gaps are the end margins of `<`, of the value and of `>`, all `contentMarginHorizontal` (`NA/note/databinding/LayoutShapeBindingSw623dpImpl.java:475, 529, 489`) [code]; SHOT agrees (about 22 dp between the drawn box and the text) [shot]. Below: the slider, margin-top 21.2, insets 21.2                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| slider                     | Build it on `@base-ui/react/slider` (installed, 1.6.0), with `min 0`, `max positions − 1`, `step 1`, `onValueChange` for the live title and `onValueCommitted` for the store. That gives pointer capture, keyboard steps, `role="slider"` and tap-to-jump. Stock jumps too: `WaterDropSeekBar` is a plain `SeekBar`, and a tap on the track sets the progress [inferred from `AbsSeekBar`]. `aria-valuetext` `"5.00mm"`. Track: one black wedge from 1 dp (round left cap) to 27 dp (semicircular right cap), inset by half the thumb; no filled/unfilled difference (`NA/sdk/note/ui/widget/WaterDropSeekBar.java:74-110`) [code]. Thumb: a 34 dp white disc with a black ring 4/48 of its diameter (≈ 2.8 dp, `RES/drawable/pen_popup_stroke_width_seekbar_thumb_drawable.xml`) [code]. Linear over the positions of S1.1. The title and value follow the drag; see S1.5 for when the value reaches native. See H20 for what a pen drag does to the panel                                                                                                                                                                                                                                                                                                                                                                                             |
| divider 2                  | 2 device px, inset 10 on both sides, margin-top 21.2                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| "Do Not Erase Shapes"      | **omitted**. Tangleaf has no shapes, and stock hides this row itself in scenes without shape support (`showGraphMenu` false in the meeting scene, `SN/12` §4.5) [doc]. When shapes arrive, the row comes back as a global switch that removes shapes from every eraser's candidates (`SN/12` §7.1). Q8                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| "Auto-deselect Eraser" row | label 20 sp, an ⓘ button (26 dp circle-i, 10 dp after the label, `aria-label="About Auto-deselect Eraser"`) and a switch (`role="switch"`, 53.3×26.7, right inset 21.2). Vertical padding 28.6. Only the switch toggles; the label does not. The value is saved at once and the menu stays open. OFF: outlined rounded box, black square on the left, `OFF` on the right (SHOT). ON: filled black box, white `ON` on the left, white square on the right (`RES/drawable/ic_switch_on.xml`) [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ⓘ hint                     | a small popover above the ⓘ, left edges aligned, same border and radius, padding 21.2, max-width 483, 20 sp text `When enabled, auto-switch to last tool after erasing.` (`strings.xml:222` via `SN/12` §7.2). Build it as a nested `<Popover>` anchored to the ⓘ with `side="top" align="start"`. `PopoverContent` passes only `align`, `sideOffset` and `anchor` today (`src/components/ui/popover.tsx:27-37`), so S1.6 adds `side`. Give the hint `fullRefreshOnClose={false}` too, or dismissing the hint requests a full GC. A tap elsewhere in the menu closes only the hint (base-ui treats the menu as the hint's parent in its floating tree, `useDismiss.js:195-199`). A tap outside the menu closes both [inferred]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| divider 3                  | as divider 2                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| "Erase Page" button        | one outlined box across the menu (stock has two side by side, "Erase Current Layer" / "Erase All Layers"): height 56.2, margin-top 28.6, side insets 21.2, border 2 device px, radius 14 device px (≈ 5 CSS px at DPR 2.8125). Content centred: a 33.9 icon (an eraser on one sheet), 14.8 gap, `Erase Page` at 17 sp. Bottom space 28.6. Enabled whenever `!editingBusy`, including on an empty page. Stock binds no enabled state to its layer buttons, only visibility and listeners (`NA/note/note/menu/popup/ErasePopup.java:175-218`, binding `:564`) [code]. Today's "Clear sheet" is disabled on an empty page (`:700`), and that rule is dropped                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |

**Icons.** Pixel: a tilted block eraser with its split line, sitting on a baseline. Lucide's `Eraser`, already
used, has this shape. Lasso: a smaller copy of the same eraser inside a 16-dash circle. Stroke: the eraser with a
short squiggle in place of the baseline (`SN/12` §4.8) [doc]. Draw these as our own SVG components in the lucide
style, 24-unit viewBox, stroke `currentColor`. Do not copy Onyx's vector path data; see Q1.

**S1.4** Erase Page. The button calls `useConfirmation()` (`src/app/confirmation.tsx:13-16, 78`) [code] with:

- title `Erase page?`
- description `Are you sure you want to erase strokes on this page?`
- confirm label `Erase`, destructive

These adapt stock's text, `Are you sure you want to erase strokes on the current layer?` (`SN/12` §8). Stock's
dialog has no title and its buttons are `OK` / `Cancel` (`OnyxCustomDialog.getConfirmDialog(context, message, …)`,
`NA/note/note/action/erase/EraseLayerAction.java:52-69`; `RES/values/strings.xml:2252, 381`) [code]. Our
`ConfirmationOptions.title` is required (`src/app/confirmation.tsx:12-17`), and the app's other confirmations use
a verb as the confirm label (`src/features/attachments/attachments-card.tsx:77-80`,
`src/features/settings/settings-page.tsx:174-177`) [code], so the title and `Erase` stay.

- **OK**: after `await confirm(…)`, read the draft from `latestDraft.current`, not from the `draft` captured at
  the tap. If it has strokes, `change({ ...latest, strokes: [] })`. On an empty page skip `change()`: it compares
  object identity (`:451-469`), so a fresh empty draft would still become a write and an empty Undo step. Then
  `setSelected([])`, `setLimit(false)`, `setTool("pen")`, and close the menu. `setTool` closes it anyway
  (`:491`). This is stock's order: erase, switch to the drawing tool, close
  (`EraseLayerAction.java:146-149`) [code]. One `change()` is one `writer.write()` (`hw/draft-writer.ts:40-47`),
  so one Undo should restore the page [inferred from `hw/handwriting-note-view.tsx:451-469`] [HW].
  `change()` itself drops the write while `historyBusy`, `leaving` or `suspended` (`:454-462`) [code], which is
  why the button is disabled while `editingBusy`.
- **Cancel**: nothing changes. Stock keeps the menu open. Base UI probably closes the menu when the dialog takes
  focus: the popover handles a `focus-out` close (`node_modules/@base-ui/react/popover/store/PopoverStore.js:108`),
  and `FloatingFocusManager` closes on focus out by default
  (`floating-ui-react/components/FloatingFocusManager.js:127`) [code; not traced for this popover]. One cheap
  option: while the confirmation is pending (a ref), the menu's `onOpenChange` ignores a close whose reason is
  `"focus-out"`. If that is not enough, accept the close and note it in the hand-over; do not fight base-ui
  further.
- The handler lives in the note view, which owns `confirm`, `change` and `latestDraft`. `EraserMenu` only calls
  `onErasePage()`, because the menu may unmount while the dialog is open.

**S1.5** Menu behaviour:

- The slider and the `<`/`>` buttons change the title and value live. The store takes the value on slider
  release (`onValueCommitted`) and on each `<`/`>` tap, so it is persisted at once. The **applied** diameter, the
  one that reaches `InkCanvas` and native, is copied from the store only while the menu is closed. The note view
  keeps `appliedDiameter` and updates it in an effect on `[optionsOpen, stored]`:
  `if (!optionsOpen) setApplied(stored)`. A type tap closes the menu, so the new type's diameter applies at that
  close. This is stock's timing:
  the value reaches SF only at the resume after the menu closes (`SN/12` §6.5) [doc]. It also avoids a native
  `configure` per tap while the menu is open. Such a configure would run `pause()` and the helper setters with
  the pen already held (`kt/OnyxInk.kt:337-339, 377-392`; `resume()` returns early while paused, `:593-594`)
  [code], and each tap would add a pause block to the log. The previous revision committed on every tap.
- Auto-deselect: `InkCanvas` gets `onEraseFinished?: () => void`. `finishGesture` calls it when
  `current.action === "erase" && tool === "eraser"`. It does not fire for the eraser end while the pen or lasso
  tool is active (stock: only `EraseHandler`, `SN/12` §7.2) [doc]. The note view reacts with
  `if (autoDeselect) setTool(previousTool)`.

**S1.6** Menu close refresh (Q2). `src/components/ui/popover.tsx:8-21` always wraps `onOpenChange` with
`refreshPanelAfterClose`, which requests a full GC 300 ms after a close that base-ui reports
(`src/app/eink-refresh.ts:53-60`) [code]. Stock's menu close is one partial REGAL_PLUS repaint (`A7:223-231`)
[log]. Add a prop `fullRefreshOnClose?: boolean` (default `true`) to `Popover` and pass
`fullRefreshOnClose={tool !== "eraser"}` from the tool-options popover. The panel then repaints the closed menu
with the ordinary GU frame [inferred]. If the menu leaves a ghost on the tablet [HW H11], revert this one task.

In the same file, `PopoverContent` also passes `side` (for the ⓘ hint, S1.3) and, only if Q20 is taken,
`collisionAvoidance`. Both are plain pass-throughs to `PopoverPrimitive.Positioner`. The wrapper runs
`closed(open)` (the refresh) **before** the caller's handler (`popover.tsx:14-17`) [code]. So a close that S1.2's
handler later ignores (the anchor's own press) has already requested the refresh. With `fullRefreshOnClose` false
for the eraser nothing is requested. For the pen and lasso options the toggle closes the menu anyway, and the
refresh is then wanted.

H12 does not need the tablet. `PopoverStore.setOpen` is what calls `onOpenChange`
(`node_modules/@base-ui/react/popover/store/PopoverStore.js:73-82`) [code], and a controlled `open={false}` does
not go through it [inferred]. A vitest settles it (§4.5).

**S1.11** States around the open menu. Nothing new is built for these. They are listed so that the executor does
not "fix" them and the user knows what to expect. All are [code] unless tagged.

- **The pen while the menu is open.** The `Popover` wrapper holds the pen (`src/components/ui/popover.tsx:9`,
  `src/app/ink-suppression.ts:98-114`). Native pauses (`kt/OnyxInk.kt:295-300`), and stylus routing is off while
  paused (`active` includes `!pauses.isPaused`, `kt/OnyxInk.kt:271`). So every pen and eraser-end contact
  reaches the DOM, including on the part of the menu that hangs over the sheet. Taps on the menu work.
- **Pen tip or eraser end on the sheet while the menu is open.** The canvas ignores pen pointers while the native
  session is up (`hw/ink-canvas.tsx:593-598`; `useOnyxInk` returns `enabled && native`, `hw/onyx-ink.ts:361`).
  The contact only dismisses the menu on its `click` (base-ui treats the pen as a mouse, S1.2), and nothing is
  drawn or erased. A pen **drag** there also runs the framework's `WebView.onTouchEvent` scroll path. With the
  pen paused there is no scheme-3 guard, so A2 applies at once, and a full-screen update follows about 1 s
  after the lift (`SN/11-observations.md:68-82`) [doc] [HW H20]. Stock: whether an outside touch both closes
  the popup and reaches the page is [unknown] (`SN/12` §12 Q2).
- **The eraser end on the menu itself.** Chromium reports it as `pointerType "pen"` with the eraser button.
  Whether a `click` is dispatched for that button, so that the menu's buttons react and an outside eraser-end
  tap dismisses the menu, is [unknown] [HW H21]. Stock's `View.OnClickListener` fires for any tool type
  [inferred].
- **The first ~800 ms after a close.** The pen comes back 300 ms (overlay release) + 500 ms (resume delay)
  after the close (§4.3). A stroke begun in that window is neither inked nor drawn by the canvas, and a drag can
  trigger the same A2 and full-screen update as above [inferred from the two bullets above]. Stock's window
  is about 500 ms and has no WebView (Q3).
- **Tool change while open.** A tool change closes the menu (`hw/handwriting-note-view.tsx:491`). So do an
  auto-deselect and Erase Page's `setTool("pen")`. An edit becoming impossible closes it too (`:488-490`).
- **Home and back while open.** `suspend()` does not touch `optionsOpen` (`:326-335`), so the menu is still
  open on return and still holds the pen. Stock: [unknown].
- **Another pane or note.** Unmounting the view drops the overlay reason (`src/app/ink-suppression.ts:111`).
  Type, diameters and auto-deselect are global (S1.1), so a second note shows the same settings.
- **Busy.** The toolbar button is disabled while `editingBusy` (`:539`), so the menu cannot open during an
  undo, a leave or a suspended session. If the state turns busy while the menu is open, only Erase Page
  disables (S1.3). The type, diameter and switch controls do not touch the draft.

### 4.2 Width to native, and the radius back (JS + Kotlin)

**S1.7** `hw/onyx-ink.ts`:

- `useOnyxInk` takes `eraserMode` and `eraserDiameter` (`:46-85`) and keeps them in the `state` ref (`:88-126`).
- The args object gets `eraserType: state.current.eraserMode` and `eraserDiameter: state.current.eraserDiameter`
  (`:178-204`), and both go into the reconfigure deps (`:319-331`).
- `OnyxInkEvent` gets `eraserRadius?: number` (`:8-16`).

**S1.8** `kt/OnyxInk.kt`:

- `OnyxInkArgs` (`:31-52`) gets `var eraserType: String = "stroke"` and `var eraserDiameter: Int = 50`.
- `configure` validates them next to `strokeWidth` (`:313`): the type is one of the three, and the diameter is in
  `50..800`.
- `onlyOverlaysChanged` (`:407-416`) compares both. Without that, an overlay change and an eraser change that
  arrive together are taken as overlay-only (`MAP` §6.1 item 2) [doc].

**S1.9** New `kt/InkEraserSize.kt`, pure:

- `fun eraserPx(hundredths: Int, dpi: Int): Float = (hundredths / 100f) * dpi / 25.4f`. With `dpi = 300` this is
  bit-exact with stock (§2.2).
- `fun eraserRadiusUnits(px: Float, sheetWidthPx: Float, unitsWide: Double): Double = px / 2 / sheetWidthPx * unitsWide`.

`OnyxInk`:

- uses `dpi = DisplayMetrics.DENSITY_DEVICE_STABLE`, the device density without per-app overrides.
  - Stock computes with its own `densityDpi` (`NA/sdk/extension/DimensKt.java:11-13`) [code], and that is 300
    because stock has no override (§2.3) [inferred].
  - Ours is 450 (§2.3), and using it would make the eraser 1.5 times too large.
  - Whether `DENSITY_DEVICE_STABLE` stays 300 under BOOX's per-app DPI is [HW H1]. If it does not, fall back to
    `DisplayMetrics.xdpi` rounded to an integer. Stop and report before choosing.
- adds `eraserRadius` to every event in `send()` (`:764-775`);
- adds `densityStable`, `densityDpi`, `xdpi`, `eraserPx` and `eraserRadius` to `status()` (`:441-471`);
- logs `eraser type=… d=… px=… dpi=…` at `Log.d` on each configure.

**S1.10** `hw/ink-canvas.tsx`:

- The props `eraserRadius` becomes `eraserDiameter` (`:92-93, 110-111`).
- `Gesture` gets `radius: number` (`:39-55`).
- `nativeInput` / `nativeStroke` (`:425-444`) take `event.eraserRadius` when it is finite and positive.
- The pointer path (`:590-612`, desktop and mouse only) falls back to CSS millimetres:
  `radius = d / 200 × (96 / 25.4) × unitsWide / rect.width`. On the BOOX WebView CSS mm are not physical mm
  [inferred], which is why the native path uses the native value.
- The circle (`:285-290`), the overlay padding (`:307-316`) and `eraseGesture` (`:389-396, 411`) use
  `gesture.radius`.
- The hit rule itself (radius plus the pressure-scaled half-width, `hw/ink-editing.ts:338-343`) is unchanged
  until step 5.

### 4.3 Expected log after step 1

Markers `S1-MENU`, `S1-A4`. The erase sequences are unchanged from `F6:113-139`: render off at pen-down,
`HAND_WRITING_REPAINT_MODE` reconcile, render on. **No new SF transaction types** may appear. With the eraser tool
active, opening the menu shows the overlay pause:

- `setRawInputReaderEnable(false)`
- `EpdController.leaveScribbleMode` → `SF ENABLE_POST [-1, 1, pid]`
- `EpdController.setScreenHandWritingPenState(…, 3)`

Render is already off with the eraser tool (`kt/OnyxInk.kt:295-300, 615-635, 675-680`) [code].

Closing with a type tap: **no** `update_mode[1]`. The resume comes about 800 ms later (300 ms overlay release plus
500 ms resume delay, `src/app/ink-suppression.ts:18`, `kt/OnyxInk.kt:229-230`) [code]. Stock resumes about 500 ms
after the close (`A7:231-257`) [log]; see Q3. Closing with an outside tap: with S1.6, no `update_mode[1]`
either.

While the menu is open, `<`/`>` taps and slider drags add **no** `configure` lines, because the applied diameter
changes only at the close (S1.5). The first configure after the close carries the new `eraserDiameter`, and its
`eraser type=… d=…` line comes before the resume. A pen **drag** on the slider may show
`EInkHelper.beforeScroll` / `APPLY_TRANSIENT_UPDATE [2308]` during the drag and `update_mode[1]` about 1 s after
the lift. That is the WebView scroll path with the pen paused (S1.11, H20). Record it separately (marker
`S1-SLIDER`) and do not count it against the erase recordings.

### 4.4 Manual test (user)

1. Pen tool active; tap the eraser once. The eraser becomes active and no menu opens. The toolbar icon is the
   current type's icon.
2. Tap it again: the menu opens under the button. Take a screenshot and compare it with SHOT: sizes, fonts,
   dividers, icons, dot.
3. Tap Pixel: the menu closes. Reopen it: `Pixel Eraser 5.00mm`, thumb at 80/140 of the track (SHOT).
4. `<` twice: `4.90mm`, the menu stays open. Drag the slider to the ends: `1.00mm` / `8.00mm`, and `>` at 8.00
   does nothing.
5. Stroke: `Stroke Eraser 0.50mm`. Pixel again: `4.90mm` is remembered. Lasso: `Lasso Eraser`, no diameter block.
6. Pixel 5.00 mm, then erase with the eraser end in the pen tool. The JS circle is about 5 mm across on the glass
   (use a ruler). Compare it with stock's 5.00 mm trace width.
7. Auto-deselect ON: the eraser tool's tip erase returns to the pen. The eraser end with the pen tool does not
   change the tool. Tap ⓘ: the hint shows, and a tap elsewhere in the menu closes only the hint. After an
   auto-deselect erase the erased strokes are gone from the panel, and the pen writes at once.
8. Erase Page → Cancel: nothing changes. Erase Page → OK: the sheet is empty, the pen is active and the menu is
   closed. Undo restores everything in one step.
9. Restart the app: type, diameters and auto-deselect are kept.
10. Through the Tauri MCP bridge, call
    `window.__TAURI_INTERNALS__.invoke("plugin:mobile-system|configure_onyx_ink", {session: "probe", enabled: false})`.
    With a foreign session id this only returns `status()` (`kt/OnyxInk.kt:303-306`) [code]. Expect
    `densityStable: 300`, `densityDpi` presumably 450, and for Pixel 5.00 mm `eraserPx: 59.05512` [HW H1].
11. With the menu open, tap the eraser button again: the menu closes and stays closed (S1.2). The same for the
    pen and lasso options.
12. With the menu open, tap the sheet outside it with the pen tip, then (after reopening) with the eraser end:
    the menu closes and nothing is drawn or erased (S1.11). Note whether the eraser end dismisses it at all, and
    whether the eraser end can press a type cell or `<`/`>` [HW H21].
13. Drag the slider with the pen, with some vertical wobble: note any flash during the drag and about 1 s after
    the lift [HW H20]. Then change the value with `<`/`>` only and compare.
14. Tap a type, then start writing at once (within about 1 s): note whether the first stroke is lost or the
    panel flashes (S1.11, Q3).
15. Erase Page on an empty page: the dialog appears, and OK switches to the pen and closes the menu. Undo is not
    enabled by it.
16. Open a second note: the eraser type and diameters are the same as in the first note, and the tool is the
    pen.

### 4.5 Unit tests

- `hw/eraser-settings.test.ts`:
  - defaults;
  - per-type memory;
  - clamping at both ends;
  - `indexOf`/`diameterAt` round trip: pixel `500 ↔ 80`, 141 positions; stroke 151 positions;
  - titles for all three types;
  - `normalizeDiameter`: `498 → 500`, `497 → 495`, `20 → 50` (stroke), `900 → 800`; a persisted state with an
    off-grid diameter and an unknown type is repaired by `merge`.
- `hw/handwriting-note-view.test.tsx` setup (`:66-133`):
  - `mount()` renders the view without `ConfirmationProvider` (`:118-133`) [code], and `useConfirmation()` throws
    outside it (`src/app/confirmation.tsx:78-81`) [code]. Every test in the file would fail, so `mount()` wraps
    the view in `ConfirmationProvider`.
  - `beforeEach` resets the eraser store to its defaults and clears `localStorage`, as it already does for
    `useHandwritingPreference` (`:97-98`). Otherwise a persisted type leaks from one test into the next.
- `hw/handwriting-note-view.test.tsx:457-479`, rewritten:
  - the second tap opens the menu, and a third tap on `Eraser options` closes it (S1.2);
  - the type cells are `role="radio"`, come in the order Pixel, Lasso, Stroke, and none of them has
    `aria-pressed`;
  - a type tap closes the menu and changes the toolbar icon (give the icon a `data-eraser-type` to assert on);
  - no diameter block for Lasso;
  - `<`/`>` keep the menu open and change the title. `InkCanvas` gets the new diameter only after the menu
    closes (S1.5);
  - the Erase Page confirmation: Cancel, OK (switches to the pen and clears the strokes), and OK on an empty page
    (no `saveHandwritingPatch` call);
  - the auto-deselect switch persists;
  - H12: with `@/app/eink-refresh` mocked, a type tap does not call `requestFullRefreshSoon`. Neither does an
    outside press with the eraser menu (S1.6).
- Slider drags are not unit-tested: jsdom has no layout, so the rects are zero. Keyboard steps
  (`ArrowRight` on the thumb) exercise `onValueChange` / `onValueCommitted`.
- Keep `container.querySelectorAll('[role="toolbar"]')` at one (`:461, 478`): the menu must not use
  `role="toolbar"`.
- `hw/onyx-ink-session.test.tsx`:
  - `:80-85`: the configure args contain `eraserType` and `eraserDiameter`;
  - `:147-148, 159-220`: `EditorSheet` passes `eraserDiameter`, the events carry `eraserRadius: 9`, and the erase
    uses 9.
- `hw/ink-canvas.test.tsx:219-244`: the pointer eraser gets its radius from `eraserDiameter` through the CSS-mm
  fallback.
- `kts/InkEraserSizeTest.kt`:
  - `eraserPx(500, 300)` has the bits `1114388593`;
  - `eraserPx(50, 300)` has `1086126580`;
  - `eraserPx(680, 300)` has `1117823299`;
  - the radius conversion is right.

---

## 5. Step 2: eraser end with the pen tool

Scope: the eraser end while the **pen** tool is active and the type is stroke or pixel. The eraser tool, the
lasso tool and the lasso type keep today's paths until steps 3 and 4.

### 5.1 Code changes

**S2.1** `kt/RawDrawingGate.kt`:

- Rename `RawDrawingSwitches.resetDefaults()` (`:11-16`) to `pushPen()`.
- `resume(render: Boolean, input: Boolean = true)` runs `pushRects(); pushPen(); if (input) input(true); render(render)`
  (`:37-42`).
- `pause()` is unchanged.

The order "rects, then settings, then input, then render" is the device-verified one and stays.

**S2.2** `kt/OnyxInk.kt`: new `pushPenSettings()`, which is what `pushPen()` calls. It replaces
`resetPenDefaultRawDrawing()` (`:128`) and absorbs the per-tool block of `configure` (`:377-391`), which is
deleted. `configure` keeps calling `resume()` (`:392`). This is stock's `ResumeRawDrawingRequest` set
(`SN/03-erasing.md` §5) [doc]. Order:

1. `Device.currentDevice().setStrokeParameters(5, floatArrayOf(5f))`. This goes first, keeping the verified dash
   recipe of the lasso tool (`kt/OnyxInk.kt:378-381`) [code].
2. `Device.currentDevice().setStrokeParameters(8, floatArrayOf(eraserPx, 0.5f, 0.1f))`.
3. Per tool, in today's order width → colour → style (`kt/OnyxInk.kt:382-390`), which is the device-verified one
   for both tools. Stock sends colour → style → width (`A4:36-44`); nothing shows that the order matters
   [inferred], so the verified order is kept:
   - lasso tool with `fastLasso`: `setStrokeWidth(LASSO_DASH)`, `setStrokeColor(BLACK)`, `setStrokeStyle(5)`;
   - otherwise: `setStrokeWidth(pen px)`, `setStrokeColor(BLACK)`, `setStrokeStyle(1)`. This covers the eraser
     tool too, for now.
4. `setEraserRawDrawingEnabled(!interaction && eraserType != "lasso", if (eraserType == "lasso") 5 else 8)`. The
   flag is **off** while the lasso tool is active (S2.5). Until step 4 it is also off for the lasso type, whose
   eraser end still pauses at pen-down; step 4 drops that condition.
5. `setBrushRawDrawingEnabled(true)`.
6. `styleSlotDirty = false`.

Extract the decision into a pure `penSettingsCalls(tool, eraserType, eraserPx, penPx): List<PenCall>` so the order
is unit-testable. `OnyxInk` only executes the list. The helper-creation `setEraserRawDrawingEnabled(false, 0)`
(`:375`) stays: it is part of the verified open sequence, and the first resume overrides it.

**S2.3** New `kt/InkEraseHandBack.kt`, pure, with no clock. `OnyxInk` owns the `Runnable`s. It replaces
`InkEraserRenderGate` **for trace gestures only**. The old gate stays for the gestures that still pause at pen-down
(the lasso tool, and the eraser tool until step 3). States: `IDLE → AWAIT_CANVAS → POSTED → PAUSED → IDLE`.

| call                                 | from                           | effect                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ------------------------------------ | ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `eraseEnded()`                       | IDLE                           | → AWAIT_CANVAS; OnyxInk arms `canvasTimeout` (750 ms)                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `canvasReady()` / `canvasTimedOut()` | AWAIT_CANVAS                   | → POSTED, returns true; OnyxInk: `t0 = uptimeMillis()`, `EpdController.enablePost(webView, 1)`, `Log.d` the call's duration, arm `settle` for `max(0, 170 − (uptimeMillis() − t0))` ms, so render goes off 170 ms after the call **started**. The call blocks about 150 ms when SF leaves schema 7 (§2.1); arming 170 ms after its return would make the gap about 320 ms [inferred]                                                                                                                                     |
| `canvasReady()`                      | POSTED                         | no transition and no action: the settle frame reads the fence when it runs and so draws the newest ready canvas. This commit must **not** fall through to the reconcile gate, whose `SOON` path would submit a `HAND_WRITING_REPAINT_MODE` frame 120 ms later (`kt/OnyxInk.kt:487-491`) and occupy `pendingFrame`, so the settle's `refreshFrame` would return early (`:522`)                                                                                                                                            |
| `settleElapsed()`                    | POSTED                         | → PAUSED, returns true; OnyxInk: `setRender(false)` (input stays on), `submitted = refreshFrame(repaint = false, forHandBack = true)`, `frameSubmitted(submitted)`, arm `resumeDelay` (300 ms) and `hardCap` (1000 ms). If `submitted` is false, still call `webView.invalidate()`: stock's `InvalidateViewWithPenControlAction` always invalidates while paused (`SN/03-erasing.md` §3.4 step 2), and so does our `pause()` (`kt/OnyxInk.kt:631`); without it a timed-out or stale canvas leaves the trace on the panel |
| `canvasReady()`                      | PAUSED                         | a newer canvas: OnyxInk resubmits the plain frame                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| any timer                            | while `gesture.drawing`        | treated as `penDown` for that state (CANCEL or RESEND). Render never goes off, and the re-send never runs, under a pen that is on the glass. Defensive: `begin()` normally cancels first                                                                                                                                                                                                                                                                                                                                 |
| `framePresented()`                   | PAUSED                         | clears `framePending`; `ready()`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `resumeDelayElapsed()`               | PAUSED                         | sets `delayElapsed`; `ready()`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `ready()`                            | PAUSED, frame done, delay done | → IDLE, returns true; OnyxInk: `gate.resume(render = !pauses.isPaused && toolRendering(), input = false)`                                                                                                                                                                                                                                                                                                                                                                                                                |
| `hardCapElapsed()`                   | PAUSED                         | → IDLE, returns true; the same re-send. Render must never stay off                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `penDown(erasing)`                   | AWAIT_CANVAS or POSTED         | → IDLE, returns `CANCEL`: OnyxInk removes the timers. Render is still on; see S2.5 for the style                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `penDown(erasing)`                   | PAUSED                         | → IDLE, returns `RESEND`: OnyxInk re-sends and turns render on now (today's `InkEraserRenderGate.begin(false)` behaviour, `kt/InkEraserRenderGate.kt:9-10`) [code]                                                                                                                                                                                                                                                                                                                                                       |
| `reset()`                            | any                            | → IDLE, no action. Lifecycle never re-enables render; the next `resume()` pushes everything                                                                                                                                                                                                                                                                                                                                                                                                                              |

**S2.4** `kt/OnyxInk.kt` wiring:

- `private fun eraseTrace(args, erasing) = erasing && !args.eraser && !args.interaction && args.eraserType != "lasso"`.
  Step 3 and step 4 widen it.
- `begin()` (`:777-816`):
  - `when (handBack.penDown(erasing))`: `RESEND` → `gate.resume(…, input = false)`; `CANCEL` → remove the
    timers. Place it **after** `if (gesture.beginNeedsEnd()) end()` (`:782`) and **before** the overlay swallow
    (`:792`). After: a synthesized `end()` of a trace gesture calls `eraseEnded()` and arms the timers, and a
    `penDown` run before it would leave them armed under the new contact. Before the swallow: a tap on a floating
    control during PAUSED would otherwise skip the RESEND, and its `end()` → `restoreToolRendering()` (`:822`)
    would turn render on in the middle of the hand-back.
  - `val trace = eraseTrace(args, erasing)`.
  - `fastPreview = (args.fastLasso && !erasing && !args.eraser && !movingSelection) || trace` (`:799`). JS already
    uses `fastPreview` to draw nothing (`hw/ink-canvas.tsx:375, 427-428`) and to hold commits
    (`hw/onyx-ink.ts:247, 339`), and native uses it to stop previews (`kt/OnyxInk.kt:869`) [code].
  - If `trace`: no `eraserRenderGate.begin`, `colorCu(true)`, `styleSlotDirty = true`, remember
    `traceGesture = true`.
  - Otherwise the current `eraserRenderGate.begin(erasing || args.eraser)` (`:802`).
  - Everything else is unchanged: overlay swallow, `setRender(fastPreview)` for interaction tools,
    `reconcile.began`, display policy, `markNativePoint`, `cancelFrameSubmission`, `send`.
- `end()` (`:818-838`): for a trace gesture:
  - `colorCu(false)`;
  - `send("end")`;
  - `cancelFrameSubmission()`;
  - `handBack.eraseEnded()` and arm `canvasTimeout`;
  - **do not** arm the 750 ms `refresh` net (`:837`).

  The display-policy lines stay.

- `commit()` → `onComplete` (`:480-492`): after the session and fence checks, if the hand-back is AWAIT_CANVAS,
  POSTED or PAUSED, route to it and return before the reconcile gate. Only a canvas that covers the last native
  stroke counts: `args.sequence >= sequence`, the same test `InkFrameFence.canSubmit` applies at presentation
  (`kt/InkFrameFence.kt:25-26`) [code]. An older acknowledgement, for example of a commit issued before a 3 ms
  first contact of the double-contact pattern (§2.3), is dropped; otherwise it would send `ENABLE_POST(1)` for a
  canvas that the settle frame then cannot present. The commit that carries the erase is released by the `stroke`
  event, not by `end`: `gestureOpen = event.kind === "begin"` runs for every non-preview event
  (`hw/onyx-ink.ts:246-247`) and `nativeStroke` → `finishGesture` applies the erase there
  (`hw/ink-canvas.tsx:439-444`) [code]. Native `end()` still runs first, because the SDK posts the list and the end
  callback back to back to the main thread (`PUB/touch/SFTouchRender.java:69-97`) and the JS round trip is
  longer [inferred].
- `refreshFrame(repaint: Boolean = true, forHandBack: Boolean = false): Boolean` (`:521-567`):
  - skip `repaintMode.acquire()` (`:561`) when `!repaint`;
  - return whether a frame was submitted;
  - in the present callback, call `handBack.framePresented()` instead of `eraserRenderGate.framePresented()`
    (`:547`) when `forHandBack`;
  - the `finally` put-back of the region (`:548-554`) is unchanged.
- `pause()` (`:615-635`): `handBack.reset()`, remove its four `Runnable`s next to `refresh` / `penUpWait` /
  `resumeGate` (`:617-619`), `colorCu(false)`.
- `markNativePoint` (`:704-710`): the margin for erasing points is at least `eraserRadiusUnits + 2`.
- `colorCu(on)`: only a state change calls `EpdController.enableColorCU()` / `disableColorCU()`
  (`DEV/api/device/epd/EpdController.java:331-338`) [code], inside `runCatching`.
- `status()`: `handBackState`, `colorCu`, `styleSlotDirty`. `Log.d` on each hand-back transition, with
  `SystemClock.uptimeMillis()`.

**S2.5** The style-slot rule. **Never turn render on after an eraser-end contact without re-sending the tool
style.**

- `styleSlotDirty` is set in `begin()` whenever `erasing` is true while render is on and the eraser flag is on
  (the trace path, and the swallowed-overlay path at `:792-798`).
- `restoreToolRendering()` (`:641-643`) calls `pushPenSettings()` first when the flag is set.
- On **every** tip (non-erasing) pen-down while the flag is set, call `pushPenSettings()` at once, whatever the
  hand-back state. The flag outlives a `CANCEL` by an eraser-end contact (correctly: the slot does hold 8 then),
  and after a `pause()` it is cleared only by the next `resume()`, so tying the rule to `CANCEL` alone misses
  cases. With a RESEND the push has already happened and the flag is already clear. The
  first samples of that stroke may already be drawn in style 8 [inferred]. Stock has the same window
  (`SN/03-erasing.md` E3), and a stylus flip took 0.86-1.4 s on the tablet (`SN/11-observations.md:52-53`) [doc],
  so the window is not expected to be hit [HW H4].
- The eraser flag is **off** while the lasso tool is active (S2.2 step 4). SF then uses the reader that paints
  nothing and does not touch the slot (`SN/03-erasing.md` §3.1) [doc]. Stock turns it on for its selection tool,
  but stock does not erase there at all (`SN/03-erasing.md` §3.7) [doc]. Ours does (Q7), with render off at
  pen-down, one main-thread hop after SF has already chosen its reader. Without this, a lasso stroke after an
  eraser-end touch could come out in style 8.

**S2.6** The resulting post-lift sequence, the stock order:

1. CU off at the lift.
2. The JS erase result is committed; that commit is `canvasReady`.
3. `ENABLE_POST(1)` while SF is still running. The call returns about 150 ms later when SF was in schema 7 (§2.1).
4. 170 ms after that call started, render off with input on, and one plain frame.
5. Once that frame is presented and 300 ms have passed, the full re-send and render on.

Deliberate deviations from stock:

- `ENABLE_POST(1)` is sent even when nothing was erased. Stock skips it then (`SN/03-erasing.md` §3.4) [doc].
  Native does not know the result, and the pen-state-3 step leaves schema 7 anyway.
- No `resetViewUpdateMode`. `DisplayModeStack` owns the view mode (`kt/DisplayModeStack.kt:5-26`) [code]. The
  hand-back frame is simply not marked with the handwriting repaint mode.
- The exclude reset stays in the re-send (§9).

**S2.7** `close()` (`:724-748`): call `helper?.resetPenDefaultRawDrawing()` before `closeRawDrawing()`. SF's
eraser flag is global and would otherwise stay `(1, 8)` for the next app [inferred]. This is not stock (Q11). It
is its own line, so it can be dropped.

**S2.8** JS: no functional change. The erase gesture is `nativeFast` through `event.fastPreview`, so
`sampleGesture` records the point and returns (`hw/ink-canvas.tsx:375`), and no circle or preview is published.
`finishGesture` erases over the full SDK list and publishes once (`:399-424`), called from `nativeStroke` when the
`stroke` event arrives (`:439-444`). Commits are held while the gesture is open and are released by that `stroke`
event, or by `end` when the SDK delivered no list (`hw/onyx-ink.ts:246-247, 339`) [code]. Update the comments: `traceRef` means "a firmware trace (lasso or eraser) is on
screen".

### 5.2 Expected log after step 2

Markers: `S2-A4` (one eraser-end stroke over ink), `S2-A5` (eraser end, then write with the tip after about 1 s),
`S2-EMPTY` (eraser end over blank paper), `S2-QUICK` (two eraser-end strokes less than 0.3 s apart),
`S2-LASSO` (eraser end in the lasso tool, then a lasso stroke), `S2-MENU` (open and close the pen options after
an erase), `S2-BG` (Home and back).

On resume (tool select, page open, menu close):

```
TouchHelper.setSingleRegionMode()                        SF SET_SCREEN_HANDWRITING_REGION_MODE [1]
TouchHelper.setPenUpRefreshTimeMs(500)
EpdController.setScreenHandWritingRegionExclude(null, [])  SF SET_SCREEN_HANDWRITING_REGION_EXCLUDE [0, 0]
TouchHelper.setLimitRect([…], [])                        SF …REGION_LIMIT [1, 4, …] ×2   SF …REGION_EXCLUDE [1, 0]
ViewUpdateHelper.setStrokeParameters(5, [5.0])           SF SET_STROKE_PARAMETERS [5, 1, 1084227584(5.0f)]
ViewUpdateHelper.setStrokeParameters(8, [59.05512, 0.5, 0.1])
                                                         SF SET_STROKE_PARAMETERS [8, 3, 1114388593(59.05512f), 1056964608(0.5f), 1036831949(0.1f)]
TouchHelper.setStrokeWidth(<pen px>)                     SF SET_STROKE_WIDTH ×2
TouchHelper.setStrokeColor(-16777216)                    SF SET_STROKE_COLOR ×2
TouchHelper.setStrokeStyle(1)                            SF SET_STROKE_STYLE [1] ×2
TouchHelper.setEraserRawDrawingEnabled(true, 8)          SF SET_ERASER_RAW_DRAWING_ENABLED [1, 8]
TouchHelper.setBrushRawDrawingEnabled(true)              SF SET_BRUSH_RAW_DRAWING_ENABLED [1]
TouchHelper.setRawInputReaderEnable(true)
TouchHelper.setRawDrawingRenderEnabled(true)             SF SET_SCREEN_HANDWRITING_PEN_STATE [2, pid]   running
```

The line `TouchHelper.resetPenDefaultRawDrawing()` (`FA4:108-110`) is gone.

Per eraser-end gesture (Pixel 5.00 mm). The target is `A4:2-60`:

```
cb onBeginRawErasing
[HandlePenTrigger / ### EUS, s 3 c 0 when SF was not in schema 7 — A5:2-3]
EpdController.enableColorCU()          SF ENABLE_COLOR_CU [1]   CU 1
                                       (no setRawDrawingRenderEnabled(false) — it was F6:115)
cb onEndRawErasing
EpdController.disableColorCU()         SF ENABLE_COLOR_CU [0]   CU 0
EpdController.enablePost(RustWebView#…, 1)   SF ENABLE_POST [-1, 1, pid]   Post enabled!   ### EUS, s 2 c 0
  (the OnyxInk Log.d line reports the call took ≈150 ms)
  +≈170 ms from the enablePost line
TouchHelper.setRawDrawingRenderEnabled(false)   SF ENABLE_POST   SF …PEN_STATE [3, pid]   toPause   SF ENABLE_POST
                                       (ENABLE_POST and PEN_STATE [3] 1-2 ms apart: SF already left schema 7)
PANEL waveform[2] update_mode[0] flags[0x0]
  +≈300 ms: the resume block above, without setRawInputReaderEnable, ending in PEN_STATE [2] running
```

Must be absent for this gesture:

- `setViewDefaultUpdateMode(…, HAND_WRITING_REPAINT_MODE)` (it was `F6:135`);
- `beforeScroll`;
- `### EUS, s 4`;
- `update_mode[1]`.

Differences from stock that are expected:

- `SF DEBOUNCE_INC_REFRESH []` per lift (EinkWise, `SN/11-observations.md:27-31`) [doc];
- the exclude-reset lines;
- no `setStrokeParameters(1, …)` (Q10);
- no `resetViewUpdateMode`;
- the order inside the re-send: stock sends colour, style, width, colour and then the parameters `(1)`, `(5)`,
  `(8)` (`A4:36-53`, `A7:28-37`) [log]; S2.2 sends `(5)` and `(8)` first and keeps today's width → colour →
  style. Each is its own SF setter, so the order is not expected to matter [inferred], but compare the lines as a
  set, not by position.

`S2-QUICK` and the double-contact pattern of §2.3: the second `onBeginRawErasing` arrives before the 170 ms settle,
so no `setRawDrawingRenderEnabled(false)` appears between the two contacts, and only one hand-back follows the last
lift. An `enablePost` (with `Post enabled!`) may appear between the two contacts when the first contact's canvas was
acknowledged in time; the second contact then has to start its trace in a session that is running but has left
schema 7 [HW H15]. If the render-off really caused the split (§2.3), this case should now be rare [HW H14].

`S2-LASSO`: today's path. Render goes off at pen-down (`F6:115` pattern), the reconcile runs, and render comes
back as today. With the lasso tool active, the resume sends `SET_ERASER_RAW_DRAWING_ENABLED [0, 8]` (`[0, 5]` for
the lasso type), so SF paints nothing for the eraser end and the style slot keeps 5. The next lasso stroke must
be dashed.

### 5.3 Manual test (user)

- Write several strokes, then erase across them with the eraser end, Pixel 5.00 mm. During the gesture the
  soft-eraser trace shows (compare with stock A4 side by side). At the lift the trace disappears and the erased
  result appears in one partial update. No full-screen flash; the page does not move.
- Right after an erase (under 1 s), flip the pen and write. The stroke must look like normal ink, not pale or
  wide [HW H4].
- Stroke Eraser 0.50 mm: a thin trace; strokes are erased only where the trace crosses them.
- Erase over blank paper: the trace disappears, and there is no flash.
- Lasso tool: the eraser end still erases as before, and the next lasso stroke is dashed.
- Menus, Home and back, then erase again.

### 5.4 Unit tests

- `kts/InkEraseHandBackTest.kt` (new):
  1. `eraseEnded`, `canvasReady` → post; `settleElapsed` → pause, frame; `framePresented` + `resumeDelayElapsed`
     in either order → exactly one re-send.
  2. `penDown` in AWAIT_CANVAS or POSTED → `CANCEL`, and no re-send afterwards.
  3. `penDown` in PAUSED → `RESEND`, once.
  4. `reset()` in each state → no action, and later timers do nothing.
  5. `canvasTimedOut` → proceeds.
  6. `frameSubmitted(false)` → the re-send after the delay alone.
  7. `hardCapElapsed` → re-send even without the frame.
  8. `canvasReady` in PAUSED → resubmit, not a second post.
  9. `canvasReady` in POSTED → no action, no second post, no fall-through (the caller gets a "consumed" result).
  10. A timer that fires while the caller reports a pen on the glass → treated as `penDown`, never a render-off.
- `OnyxInk` settle arithmetic, if extracted as a pure helper: `settleDelay(callMs = 155) = 15`,
  `settleDelay(callMs = 2) = 168`, `settleDelay(callMs = 400) = 0`.
- `kts/RawDrawingGateTest.kt:14-31`: the resume order becomes `rects, pen, input=true, render=true`. New case:
  `resume(render = true, input = false)` gives `rects, pen, render=true`.
- `kts/PenSettingsTest.kt` (new, for `penSettingsCalls`):
  - pen tool, pixel, 500, dpi 300 → the exact list of S2.2 with `59.05512f`;
  - lasso tool → style 5, width 5, eraser flag `(false, 8)`;
  - stroke type → `(8, [5.905512f, …])`.
- `kts/InkEraserRenderGateTest.kt`: unchanged; the gate still serves the non-trace paths.
- `hw/onyx-ink-session.test.tsx` (new case): a native erase whose begin has `fastPreview: true, erasing: true`:
  - `preview` events draw nothing (no `arc`, no publish);
  - the `stroke` event erases with `eraserRadius`;
  - `commit_onyx_frame` is not invoked between `begin` and the `stroke` event, and is invoked after it with that
    event's `sequence` (S2.8; the `stroke` event, not `end`, releases the commit).

  The existing case `:159-220` keeps covering `fastPreview: false`.

---

## 6. Step 3: eraser tool (tip)

**S3.1** `eraseTrace`: `(erasing || args.eraser) && !args.interaction && args.eraserType != "lasso"`.

**S3.2** `toolRendering()` (`kt/OnyxInk.kt:638-639`): the eraser tool renders unless the type is lasso (step 4
drops that exception). Extract it as a pure `toolRenders(eraser, eraserType, interaction, fastLasso, hasSelection)`
and test it. A session opened with the eraser tool then reaches pen state 2 instead of staying in 1 (`MAP` §7.10)
[doc].

**S3.3** `penSettingsCalls`, eraser tool with a non-lasso type: `setStrokeColor`, `setStrokeStyle(8)`, and **no
`setStrokeWidth`** (`A7:19-45`) [log]. Parameters and flags as in S2.2. `styleSlotDirty` stays false for tip
strokes, because the tool's main style is already 8.

**S3.4** `penUpRefresh()` (`kt/OnyxInk.kt:501-512`): ignore it while `config.eraser` (count it in a separate
diagnostic). Stock's consumer for the eraser tool is empty (`SN/03-erasing.md` §3.3) [doc], and ours could
otherwise start a second reconcile inside the hand-back (`MAP` §6.5 item 18) [doc].

**S3.5** CU per tip gesture, as S2.4. Stock also turns CU on when the eraser tool is activated (`A7:5`) [log]. Not
copied (Q9).

**S3.6** Auto-deselect (S1.5) now runs during a hand-back. The tool change reconfigures: `pause()` resets the
hand-back, and `resume()` re-sends the pen settings. That differs from stock's order (stock's hand-back resumes
against the pen handler, `SN/03-erasing.md` §3.4) [doc], but it ends in the same SF state [inferred].

### Expected log after step 3

Markers: `S3-SELECT`, `S3-A7` (three tip strokes about 2 s apart), `S3-QUICK` (strokes under 0.4 s apart),
`S3-PEN` (switch to the pen and write at once), `S3-OPEN` (leave the page with the eraser tool active and come
back), `S3-AUTO` (auto-deselect on).

- Tool select: the resume block with `TouchHelper.setStrokeStyle(8)` ×2 and no `setStrokeWidth`, ending in
  `PEN_STATE [2] running`. The target is `A7:19-45`.
- Per tip gesture: `cb onBeginRawDrawing`, `EUS s 3`, `CU 1`, `cb onEndRawDrawing`, `CU 0`, `enablePost`, then
  after about 170 ms render off and `waveform[2] update_mode[0]`, then after about 300 ms the resume block with
  style 8. The target is `A7:48-100`.
- `cb onPenUpRefresh` arrives about 500 ms after the lift and no `HAND_WRITING_REPAINT_MODE` line follows it.
- `S3-OPEN`: the session ends in `PEN_STATE [2]`, not `[1]`.
- `S3-QUICK`: a pen-down during PAUSED shows the resume block immediately after `cb onBeginRawDrawing` [HW H5].
  A pen-down during AWAIT_CANVAS or POSTED shows no render-off and no re-send (CANCEL); an `enablePost` may already
  be in the log before it [HW H15].
- `S3-AUTO`: no hand-back for the erase that triggers the switch. The tool change's own pause and resume show
  instead: `setRawDrawingRenderEnabled(false)` (with the ~150 ms `Post enabled!` block of §2.1 when SF was in
  schema 7), `setRawInputReaderEnable(false)`, the resume block with `setStrokeStyle(1)` and `PEN_STATE [2]`, and
  then a `HAND_WRITING_REPAINT_MODE` reconcile of the erased canvas through the ordinary gate [inferred from
  `kt/OnyxInk.kt:338, 392, 487-491`].

**Manual test**:

- Erase with the tip, pixel and stroke types; the trace looks and behaves like stock A7.
- Quick successive strokes: watch whether the start of the second stroke has a trace and whether anything
  flashes [HW H5].
- Switch to the pen and write at once: normal ink.
- Auto-deselect ON: back to the pen after each erase.

**Tests**:

- `toolRenders` table test.
- `penSettingsCalls` for the eraser tool (style 8, no width).
- A `penUpRefresh` guard test, if extracted.
- `hw/onyx-ink-session.test.tsx`: an eraser-tool tip `begin` (`erasing: false` from the SDK, `erasing: true` in the
  event, `fastPreview: true`) draws nothing until the stroke arrives.

---

## 7. Step 4: lasso eraser with the firmware dash trace

**S4.1** `eraseTrace` drops the lasso exception, and so does `toolRenders`.

**S4.2** `penSettingsCalls`, lasso type:

- eraser tool: `setStrokeStyle(5)`, no width;
- the eraser flag is `(true, 5)` for every tool except the lasso tool;
- `(8, [eraserPx(50) = 5.905512f, 0.5f, 0.1f])`.

This is `A7:241-252` [log].

**S4.3** JS: no functional change. A trace gesture never publishes during the gesture, so the dashed polygon
(`hw/ink-canvas.tsx:270-282`) is drawn only on the pointer path and on non-native devices. At the lift,
`eraseGesture(…, "lasso")` runs on the full SDK list. It still deletes whole strokes until step 5.

**S4.4** The dash width: the eraser tool sends no width, so SF draws the dash at whatever width was set last
[inferred; stock the same, `A7:241-257`] [HW H8].

### Expected log after step 4

Markers `S4-TOOL`, `S4-END`.

- `S4-TOOL`: `SET_STROKE_STYLE [5]` ×2, `(5, [5.0])`, `(8, [5.905512, …])`, `[1, 5]`, then per loop the same
  hand-back with style 5 in the re-send. The target is `A7:241-330`.
- `S4-END` (pen tool, lasso type): `SET_ERASER_RAW_DRAWING_ENABLED [1, 5]` on resume. The re-send after the
  eraser-end loop restores `SET_STROKE_STYLE [1]`.

**Manual test**:

- Lasso type, eraser tool: draw a loop around strokes. SF shows the dashed loop, and at the lift the loop
  disappears and the enclosed strokes go. Compare the dash with stock.
- The eraser end in the pen tool with the lasso type, then write at once: normal ink.

**Tests**: `penSettingsCalls` for the lasso type with the eraser tool and with the pen tool.

---

## 8. Step 5: geometry the user would notice

JS only (`hw/ink-editing.ts`). Each sub-step can ship alone. The differences below are the ones from
`SN/13-eraser-geometry.md` §9 that are visible in normal use.

**S5.1 Lasso cuts vertices out** (L1, L2 in §9.4; L3 fill rule):

- New `insideNonZero(p, polygon)`, a winding-number test.
- New `cutLasso(strokes, polygon)`. For each stroke whose bounds overlap the polygon's bounds, find the vertices
  inside. None inside → the stroke is unchanged, even when a segment crosses the loop (stock L2).
- Otherwise split the stroke into runs of consecutive outside vertices and keep runs of 2 or more vertices. The
  first run keeps the id and the others get `crypto.randomUUID()`, as in pixel mode (`:371-374`) [code]. If fewer
  than 2 vertices survive, delete the stroke.
- `eraseGesture(…, "lasso")` (`:310-313`) uses `cutLasso(strokes, simplifyGesture(gesture))`, behind the existing
  degenerate-lasso guard (`:98-100`).
- `selectLasso`, the **selection** tool, is not touched.
- Deliberate deviations:
  - stock's 3-vertex quirk (P2, where the stroke stays unchanged) is not copied;
  - no integer-grid rasterisation (L4);
  - stock gives every fragment a new id and appends it at the end (P5), which cannot be seen with one ink colour.

**S5.2 Centreline radius** (S1, P3, C5):

- `cutIntervals(a, b, path, radius)` without `+ width·(0.25 + 1.5·p)/2` (`:338-343`).
- The broad phase uses a margin of `radius` (`:318`).

With the stroke eraser at 0.50 mm, today's extra half-width (up to 0.4 mm for pen width 3 [inferred]) more than
doubles the reach, which shows next to thick strokes.

Not copied, on purpose:

- The pixel eraser keeps its exact cut and its capsule sweep. Stock removes whole vertices and tests only thinned
  sample points (P1, P4). At our sampling density the cut position is not visible [inferred], and stock's gaps on
  fast swipes are a weakness.
- One-point strokes stay erasable (S4).
- Stock's 20/30 ms batching and grid thinning are not reproduced.

**Expected log**: identical to step 4. Geometry does not reach SF.

**Manual test**: draw the same page in stock and in Tangleaf, then compare.

1. Circle-lasso the middle of a long line: two pieces remain.
2. A double loop around a short stroke: it is erased.
3. Stroke Eraser 0.50 mm passing about 1 mm beside a thick stroke without touching its centre: it is kept.
4. Pixel 5.00 mm through a stroke: the gap width is close to stock's.

**Tests** (`hw/ink-editing.test.ts`):

- stroke `(0,100)…(300,100)` with a vertex every 10 units, lasso a 32-gon of radius 25 around `(150,100)` → two
  strokes `0..120` and `180..300`;
- a two-vertex line through the same lasso → unchanged;
- a double loop around a dot → erased;
- a lasso around a single-vertex stroke → erased;
- S1 of `SN/13` (`r = 12`, eraser along `y = 14`, a stroke on `y = 0`, `w = 4`, `p = 1`) → kept.

Re-check the existing cases at `:25-49, 69-75`, which used radius plus half-width.

---

## 9. Invariants to keep, and the risks

Kept as they are, device-verified:

- **Stylus routing** (`kt/InkStylusRouting.kt`, `kt/OnyxInk.kt:262-277`). Its `active` does not depend on render
  state (`:271`) [code]. Stylus gestures stay consumed during the hand-back, when render is off and input is on.
- **Dead-rectangle fixes**: the exclude reset with an empty payload and `setLimitRect(limit, NO_EXCLUDES)` stay in
  `pushRects` (`kt/OnyxInk.kt:117-127, 186-204, 653-656`). `cycleScribbleSession()` stays in real pauses and
  `close()` (`:623, 732`). The hand-back uses the render-only switch, never the cycle.
- **Frame fence** (`kt/InkFrameFence.kt`): unchanged. The hand-back adds a submission without a mode through the
  same fence, and a pen-down still cancels a dispatched frame (`kts/InkFrameFenceTest.kt:29`) [code]. The hand-back
  accepts a canvas only when its sequence covers the last native stroke, the fence's own `canSubmit` test (S2.4),
  and `commit()`'s `cancelFrameSubmission()` (`kt/OnyxInk.kt:477`) is what cancels a pending hand-back frame when a
  newer canvas arrives in PAUSED; the resubmission then comes from that canvas's acknowledgement. CLAUDE.md
  requires a device retest for fence and eraser-gate changes; steps 2-4 are exactly such retests.
- **HD EinkWise profile** (`kt/MobileSystemPlugin.kt:292-331`): untouched. `DEBOUNCE_INC_REFRESH` stays in our logs.
- **Display-mode ownership**: only `DisplayModeStack` sets the view mode. There is no direct `resetViewUpdateMode`.

Risks:

- **R1** A resume or hand-back that forgets the style leaves SF drawing the eraser with the tip (the PATCH
  failure). Mitigations: S2.5, and `pushPenSettings` on every resume, which is unit-tested.
- **R2** Render stuck off: the hard cap (S2.3) and `pause()`/`resume()` ownership. `restoreToolRendering` and the
  re-send never turn render on while `pauses.isPaused`.
- **R3** A pen-down at the settle boundary: `penDown` runs on the main thread one hop after the digitizer
  (`MAP` §4, `PUB/touch/SFTouchRender.java:40-137`) [doc]. If the settle timer runs first, the new stroke starts
  paused and `RESEND` turns render on mid-contact. What SF paints then is [unknown] [HW H5]. The hop can be
  long: for about 150 ms after the hand-back's `enablePost` the main thread is inside that call (§2.1), and a
  pen-down's `begin()` waits behind it; the settle arithmetic of S2.3 keeps the settle Runnable after it in the
  queue [inferred].
- **R4** A deferred JS `configure` during the hand-back (a selection cleared by the erase, `hw/onyx-ink.ts:254-257`)
  resets it through `pause()`. The resume path then does the re-send. That is correct, but the log differs from
  the target.
- **R5** SF globals: the eraser flag `(1, 8|5)` outlives our session unless S2.7 is kept. The style slot does not:
  `closeRawDrawing()` already sends `setStrokeStyle(0)` from both renders (`PUB/touch/SFTouchRender.java:170-173,
  271-275`; `PUB/touch/AppTouchRender.java:99-102, 193-197`) [code]. The `(8, [w, 0.5, 0.1])` parameters stay in
  SF after close; stock re-sends its own on every resume (`A4:52-53`) [log]. None of this runs when the process
  dies without `close()` [inferred].
- **R8** The main thread blocks for about 150 ms inside the hand-back's `enablePost` (§2.1), once per erase. Stock
  blocks its own main thread the same way (`A4:14`). During that time WebView input and the raw-input callbacks
  queue; the pen tool's writing is not affected because SF draws it [inferred]. If the block shows as UI lag,
  moving the call to a background thread is a follow-up, not part of step 2, because it changes the ordering
  against the main-thread settle and frame callbacks.
- **R6** Pen contacts on the new menu (slider drags, switches) reach the WebView's scroll path.
  - While the menu is open the pen is **paused**, so nothing is queued. The pause removes the scheme-3 guard,
    and A2 applies at once, with a full-screen update about 1 s after the lift (`SN/11-observations.md:68-82`)
    [doc]. The queue-until-next-pause-exit case (`:90-94`) applies only to taps while the session is running.
  - The slider drag is the likely trigger: 5 px of vertical travel is enough (`:70-72`), and 5 device px is
    about 0.4 mm [inferred] [HW H20].
  - The same holds for a pen drag on the sheet while the menu is open, and in the ~800 ms after it closes
    (S1.11).
  - That is the queued `setAutoA2ForScrolling` work. It is not addressed here, and the user should not count such
    a flash against these steps.
- **R7** The layout work (`docs/planning/handwriting-page-layout.md`, untracked at the time of writing; it keeps the
  eraser popover slot in its §8 and the 1000×1400 units) and any page-size work: the toolbar anchor, `1000` sheet
  units and the cited lines may
  move. Stop and report per §0.

## 10. Out of scope

- `setAutoA2ForScrolling`
- EinkWise `supportEAC`
- stock's selection-tool semantics (Q7)
- shapes, layers, "Do Not Erase Shapes" wiring
- finger scroll and pinch
- the per-stroke `HAND_WRITING_REPAINT_MODE` reconcile of pen strokes
- stock's popup colour CU
- stock's pen parameters `(1, [0.3, 0.6])` (Q10)
- the floating-toolbox eraser (`SN/12` §11.4)
- the dead `eraseAt` path (`hw/ink-model.ts:40-49`, `MAP` §6.5 item 19)

## 11. Hardware checks

| id  | assumption                                                                                                                                                                                                                                                                                           | step |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---- |
| H1  | `DENSITY_DEVICE_STABLE` is 300 in Tangleaf's process while its `densityDpi` is 450; stock's `densityDpi` is 300 (`SN/12` §12 Q5); `eraserPx` for 5.00 mm is `59.05512`                                                                                                                               | 1    |
| H2  | with render on and the eraser flag `(1, 8)`, SF paints the eraser-end trace in our session as it does in stock                                                                                                                                                                                       | 2    |
| H3  | `ENABLE_POST(1)` while running makes SF leave schema 7 for us too (`Post enabled!`, `EUS s 2`); our plain frame after pen state 3 shows as `waveform[2] update_mode[0]` and removes the trace                                                                                                        | 2    |
| H4  | the tip right after an eraser-end erase is drawn in pen style (`SN/03` E3 in our app)                                                                                                                                                                                                                | 2    |
| H5  | a tip stroke that starts inside the hand-back window: trace, flash or neither (`SN/03` E4)                                                                                                                                                                                                           | 3    |
| H6  | the pen-up refresh of eraser-tool strokes causes no second reconcile                                                                                                                                                                                                                                 | 3    |
| H7  | the visible effect of `ENABLE_COLOR_CU`, if any                                                                                                                                                                                                                                                      | 2    |
| H8  | the lasso-type dash width with the eraser tool                                                                                                                                                                                                                                                       | 4    |
| H9  | with `K = 1.875 / DPR` the menu and its text measure the same in device px as in SHOT                                                                                                                                                                                                                | 1    |
| H10 | no `beforeScroll`, `EUS s 4` or `update_mode[1]` after any erase                                                                                                                                                                                                                                     | 2-4  |
| H11 | closing the menu without the full GC leaves no ghost                                                                                                                                                                                                                                                 | 1    |
| H12 | base-ui does not report a controlled close through `onOpenChange` (so a type tap never requests a GC) [inferred]. Settled by the vitest of §4.5, not by the tablet                                                                                                                                   | 1    |
| H13 | the hand-back's `enablePost` takes about 150 ms when `Post enabled!` is logged (the `Log.d` of S2.3), and render goes off about 170 ms after the call started, as in `A4:14→20`                                                                                                                      | 2    |
| H14 | with render kept on at pen-down, an eraser-end gesture arrives as one begin/end pair, not the split contact of §2.3                                                                                                                                                                                  | 2    |
| H15 | after a `CANCEL` in POSTED (`ENABLE_POST(1)` already sent, render still on), the next contact logs `HandlePenTrigger` / `EUS s 3` again, draws its trace, and its own hand-back removes both traces [unknown: whether leaving schema 7 through `ENABLE_POST` re-arms the pen trigger was not traced] | 2-3  |
| H16 | the soft-eraser trace width follows the `(8, [w, 0.5, 0.1])` parameter, not `SET_STROKE_WIDTH`: S3.3 sends no width for the eraser tool, as stock does (`A7:28-45`), so SF keeps the previous tool's width (pen px, or `LASSO_DASH` after the lasso tool)                                            | 3    |
| H20 | a pen drag on the eraser menu's slider (pen paused): whether it shows `beforeScroll` / A2 during the drag and `update_mode[1]` about 1 s after the lift (S1.11, R6)                                                                                                                                  | 1    |
| H21 | the eraser end on the open menu: whether it presses buttons (a `click` for the eraser button) and whether an outside eraser-end tap dismisses the menu (S1.11)                                                                                                                                       | 1    |

## 12. Open questions for the user

**Decided by the user on 2026-09-25:** Q1 our own lucide-style icons; Q2 partial repaint on menu close, no
full GC; Q5 settings global per device; Q7 the eraser end keeps erasing in the lasso tool; Q8 no "Do Not Erase
Shapes"; Q21 our own dialog wording (title and "Erase"). The rest keep the plan's defaults until asked.

**Status 2026-09-26:** all five steps are implemented and verified with the pen on the tablet (Q12 settled:
the pressure-scaled half-width is gone). Steps 1–4 are e3a195e, e4fcdf1, 7f2c7a4; step 5 follows them.

1. **Icons**: draw our own look-alikes in the lucide style (the default here), or copy Onyx's vector path data?
   Those are Onyx's assets.
2. **Menu close**: skip the full GC for the eraser menu (S1.6), as stock does with its partial repaint?
3. **Resume after a menu close**: ours is 300 + 500 ms, stock's about 500 ms. Shorten the overlay release for this
   menu?
4. **Eraser button while the menu is open**: close only (stock's code intent, and the S1.2 default), or close and
   reopen (possible on stock through the outside-touch path, `SN/12` §12 Q1)? A look at stock settles it. Today
   Tangleaf's menu does not stay closed after that tap (S1.2) [code + inferred].
5. **Scope of the settings**: global per device (the default here), or per note like stock's widths?
6. **Slider feel**: the value is saved on release and on each `<`/`>`, and applied to the eraser when the menu
   closes (S1.5), as stock reaches SF only after its menu closes. Is live feedback in the title enough? Do
   `<`/`>` need auto-repeat on hold? Stock has none (`SN/12` §6.1).
7. **Eraser end in the lasso tool**: stock does not erase in its selection tool. Keep ours (the default), or match
   stock and ignore the eraser end there?
8. **"Do Not Erase Shapes"**: omit it until shapes exist (the default)?
9. **Colour CU at eraser-tool activation** (`A7:5`): add it for parity, or only around gestures (the default)?
10. **Pen parameters `(1, [0.3, 0.6])`**, which stock sends for its fountain pen: leave them out (the default,
    because they would change how our live ink looks)?
11. **S2.7**: reset SF's eraser and brush flags on close to protect other apps (not stock)?
12. **Step 5.2**: drop the pressure-scaled half-width so that the eraser reaches exactly what the trace covers?
13. **The small triangle** under stock's eraser button (SHOT; its meaning is unknown, `SN/12` §3.2): add it as a
    popover arrow, or leave it out (the default)? SHOT, cropped:
    - The triangle points **up**, at the button.
    - It sits on the toolbar's bottom rule, about 1195-1235 device px across.
    - It is detached from the menu, whose top border is 23 device px lower.
    - It reads more like a toolbar marker for the active tool than a popover arrow, which would touch the menu
      [shot; inferred]. SHOT is the only view of it with the menu open, and no screenshot of the eraser active
      with the menu closed exists, so this is open.
    - Stock also does not invert the active tool's button. Ours is solid black (`src/index.css:1237-1242`).
      Changing the toolbar's selected look is outside this plan (see the layout plan).

Added by the UI review:

20. **Menu placement**: keep `align="center"` (about 296-618 CSS on the tablet), or reproduce SHOT's 325-647 CSS
    with `align="start"` plus `collisionAvoidance={{ align: "shift" }}` passed through `PopoverContent` (S1.3,
    container)?
21. **Stock's dialog wording**: stock's confirmation has no title and uses `OK` / `Cancel`. Ours keeps a title
    and `Erase`, because the component and the app's other confirmations are built that way (S1.4). Switch to
    `OK`?

---

## Review (ui)

2026-09-25. An independent static check of step 1's menu and of the app behaviour around it. Nothing was run.
The tablet was not touched.

### What was checked

- **Stock resources and code.** Opened directly, not through `SN/12`:
  - `RES/layout-sw623dp/layout_shape.xml` (order, groups, margins);
  - the dimension buckets `values`, `-sw623dp`, `-sw745dp`, `-sw935dp`, `-sw990dp` for every size in the S1.3
    table;
  - `strings.xml` at the cited lines;
  - the drawables `ic_note_prev`, `pen_popup_stroke_width_seekbar_thumb_drawable`, `bg_btn_switch`,
    `ic_switch_on`/`off` and `ic_note_point`;
  - `WaterDropSeekBar.java:59-114`;
  - `ErasePopup.java` (switch, layer, stepper and slider listeners);
  - `ErasePopupViewModel.java:77-110`;
  - `LayoutShapeBindingSw623dpImpl.java:430-575`;
  - `NoteUnits.java:20-70` and `NumberUtils.java:71-74`;
  - `EraseLayerAction.java`.
- **SHOT, cropped.** Checked the menu, the stepper gaps, the switches, the layer buttons and the triangle, and
  compared with `OBS/shots/tangleaf-handwriting.png`. Tangleaf's sheet edge at 69 device px confirms DPR 2.8125
  rather than 1.875.
- **Our code.**
  - `hw/handwriting-note-view.tsx` (whole file), `hw/ink-canvas.tsx` (whole file) and `hw/onyx-ink.ts` (args,
    deps, return value);
  - `src/components/ui/popover.tsx`, `dialog.tsx`, `button.tsx` and `separator.tsx`;
  - `src/app/confirmation.tsx`, `eink-refresh.ts` and `ink-suppression.ts`;
  - `src/index.css` (the e-ink rules, 331-380 and 1165-1247);
  - `hw/input-capabilities.tsx`, `hw/draft-writer.ts`, and the test setup of `hw/handwriting-note-view.test.tsx`;
  - `kt/InkStylusRouting.kt` and `kt/OnyxInk.kt` (routing, configure, pause and resume).
- **Base UI 1.6.0 in `node_modules`.** `PopoverRoot.js` (dismiss config), `useDismiss.js` (the pen as a mouse,
  the one-shot target listener), `PopoverStore.js`, `useAnchorPositioning.js` (align flip versus shift),
  `SliderThumb.js` and `SwitchRoot.js`.

### What changed in the plan

- **S1.1**: `normalizeDiameter` (clamp, then stock's round-to-5) and a `persist` `version`/`merge` that repairs
  stored values. The note that the tool resets to the pen on every mount.
- **S1.2**:
  - `previousTool` does not exist today; "keep" became "add".
  - The toolbar icon has to come from the store.
  - The account of the anchor tap was rewritten from base-ui's code: the pen dismisses on `click` through a
    target listener that runs before React's `onClick`. The fix is now concrete: ignore an `outside-press`
    close whose target is inside the anchor, and toggle on the second tap.
  - The shared `PopoverContent` class has to become tool-dependent.
- **S1.3**:
  - The unit wording is now dp/sp throughout.
  - The device-px conversions were wrong for this DPR: 2 px is 0.71 CSS px, not 1.07, and 14 px is 5 CSS px,
    not 7.5.
  - CSS custom properties replace per-size JS, and rem is banned inside the menu.
  - The e-ink border rule wins over classes.
  - Placement: the `sideOffset` and `align` arithmetic, including why `align="start"` alone flips.
  - The type cells are a radiogroup, not `aria-pressed`, whose e-ink rule would invert them.
  - The 21.2 gaps around the stepper value were missing, and the dot size is now sourced.
  - The slider is built on base-ui's Slider, with tap-to-jump.
  - The ⓘ hint is a nested popover (needs `side`, no GC).
  - Erase Page stays enabled on an empty page, as in stock.
- **S1.4**: read `latestDraft.current` after the await, and skip `change()` on an empty page (it would write an
  empty Undo step). The handler lives in the note view. Stock's dialog wording is recorded. The likely
  `focus-out` close has a one-ref option.
- **S1.5**: the diameter now reaches `InkCanvas` and native only when the menu closes, as in stock. Before, it
  went out on every `<`/`>` tap, which ran a paused `configure` per tap.
- **S1.6**: `side` and an optional `collisionAvoidance` pass-through. The refresh runs before the caller's
  handler. H12 is a vitest.
- **S1.11** (new): the states around the open menu, among them the pen or eraser end on the sheet while it is
  open, the eraser end on the menu, the ~800 ms after a close, Home, another note, and busy.
- **§4.3**: no configure while the menu is open. A pen drag on the slider can flash; marker `S1-SLIDER`.
- **§4.4**: manual tests 11-16. **§4.5**:
  - `mount()` needs `ConfirmationProvider`; without it every test in the file throws.
  - The store is reset between tests.
  - New cases: the third tap, the radio roles, Erase Page on an empty page, H12, and no configure while open.
- **R6** corrected. With the menu open the pen is paused, so the A2 is not queued. It applies at once, and a
  full-screen update follows. The slider drag is the likely trigger.
- **§11**: H12 annotated; H20 and H21 added. **§12**: Q4, Q6 and Q13 updated; Q20 and Q21 added.

### Checked and left as they were

- Type order (Pixel, Lasso, Stroke), names, the title format, and ranges, step and defaults (1.00-8.00 /
  0.50-8.00, 0.05 mm, 5.00 / 0.50).
- Per-type memory, and closing the menu on a type tap, including the already-selected type.
- The track and thumb geometry, the switch looks, the layer-button metrics, the hint text and the confirmation
  message.
- Omitting "Do Not Erase Shapes" (Q8), and one Erase Page button instead of two layer buttons.

### Remaining doubts

- K rests on DPR 2.8125, confirmed from the screenshots. Whether the WebView's font scaling multiplies px font
  sizes the way rem was measured to scale is [inferred]. H9 settles it on the first screenshot.
- Whether React batches the dismissal and the reopen (menu never closes) or renders both (close, then reopen)
  was not run. The fix does not depend on it.
- Base UI's focus-out close when the confirmation opens was read from the store and the focus manager's
  defaults, not traced for this popover.
- The slider's thumb contains an `<input type="range">` (`SliderThumb.js:58, 348`). `installTextFocusSuppression`
  treats any `HTMLInputElement` as a text field (`src/app/ink-suppression.ts:46-53, 69-72`), so focusing the
  thumb adds a "text-focus" pen hold. That is harmless while the menu holds the pen, and it is released on
  unmount (`:73-78`). The executor should still confirm with `status()` that no reason is left after a close.
- Eraser-end clicks in Chromium (H21) and the slider flash (H20) are open. If H20 shows the full-screen update,
  the stepper is the flash-free path until `setAutoA2ForScrolling` lands.
- Stock's placement rule for the popup was not traced (`SN/12` §3.3). The Q20 arithmetic only matches SHOT.
- The triangle's meaning stays open (Q13). The reading as an active-tool marker comes from one screenshot.

## Review (native)

An independent check of every SF-visible call in steps 2-4 against the recordings, stock's code, the linked SDK
and today's `OnyxInk.kt` / `RawDrawingGate.kt`. Static only; the tablet was not touched.

**Checked and confirmed**

- The recorded stock sequences and timings of §2.1: `A4:1-60`, `A5:5-64`, `A5:128-200`, `A7:1-156`, `A7:232-340`,
  `A7:430-453` [log]. The width bit patterns of §2.2, recomputed in float32 (`59.05512f` = `1114388593`,
  `5.905512f` = `1086126580`, `80.314964f` = `1117823299`), and `DimensKt.mmToPx` (`NA/sdk/extension/DimensKt.java:11-13`)
  [code].
- Stock's `ResumeRawDrawingRequest` (`NA/note/note/request/pen/ResumeRawDrawingRequest.java:40-138`): the
  `f() || g()` gate, style 8 for eraser types 0/1 and 5 for type 2, `(5, [5.0])` then `(8, [width, 0.5, 0.1])`,
  the eraser flag `(isEraseTrackByRawRender || selection, isMoveStrokeErase ? 8 : 5)`, brush [code].
- The linked SDK: `TouchHelper.setRawDrawingRenderEnabled` / `setRawInputReaderEnable` deduplicate on cached
  booleans, while `setStrokeStyle`, `setStrokeWidth`, `setStrokeColor`, `setEraserRawDrawingEnabled`,
  `setBrushRawDrawingEnabled`, `setLimitRect` and `Device.setStrokeParameters` are not cached
  (`PUB/TouchHelper.java:104-126, 188-223, 324-351`; `PUB/EpdPenManager.java:52-54`) [code]. So `pushRects` and
  `pushPenSettings` always reach SF, and only the render and input switches can be swallowed. Every path in S2.3
  that ends in `render(true)` follows a real `render(false)` (settle, RESEND from PAUSED, the swallowed path),
  so no re-send is deduplicated away. `resume()` after a `pause()` always crosses the edge.
- Render-off = `leaveScribbleMode` (ENABLE_POST) + PEN_STATE 3 from `SFTouchRender`, plus ENABLE_POST from
  `AppTouchRender` (`PUB/touch/SFTouchRender.java:195-198, 278-283`; `PUB/touch/AppTouchRender.java:199-206`),
  matching `A4:21-24`; our helper has both renders (`kt/OnyxInk.kt:361-366`) [code].
- `closeRawDrawing()` resets the style slot to 0 from both renders (R5 corrected).
- The style slot after every path: pen-tool re-send (style 1), eraser-tool re-send (8 or 5), lasso tool (5, eraser
  flag off, so SF's reader 3 never writes the slot), tool change and every lifecycle resume (`pushPenSettings`),
  swallowed eraser-end contact (`restoreToolRendering` with the dirty flag), CANCEL by an eraser-end contact (slot
  correctly 8 until its own hand-back). With the eraser tool active in step 2 the flag is on but render is off
  (pen state 1 or 3), where SF neither inks nor switches readers (`SN/00-overview.md` §2.2) [doc], so the slot is
  untouched.
- The lasso tool keeps its dash recipe: params `(5, [5])` before style 5, flag off, and now also today's width →
  colour → style order (S2.2 corrected).
- `EpdController.enablePost(view, 1)` → `SDMDevice.enablePost` → `ViewUpdateHelper.enablePost(1)`, a synchronous
  transaction; `enableColorCU` / `disableColorCU` exist in the linked SDK (`DEV/api/device/epd/EpdController.java:234-237,
  331-338`; `DEV/device/SDMDevice.java:1769-1776, 2792-2794`) [code].
- The frame fence, the reconcile gate and the JS commit path (`kt/InkFrameFence.kt`, `kt/InkReconcileGate.kt`,
  `hw/onyx-ink.ts:228-370`, `hw/ink-canvas.tsx:365-444`) [code].

**Changed** (old → new)

- §2.1: "SF sleeps 150 ms after `Post enabled!` before it switches the schema, so pausing earlier is not known to
  be safe" → the `ENABLE_POST(1)` call itself blocks the caller about 150 ms when it takes SF out of schema 7;
  `EUS s 2` is logged at once; stock's 170 ms is that call plus a hop [log + code, inferred for the composition].
- S2.3 AWAIT_CANVAS row: "`enablePost`, arm `settle` (170 ms)" → settle measured from the start of the call,
  `max(0, 170 − callDuration)`, with the duration logged. Otherwise the gap would be about 320 ms.
- S2.3: new row `canvasReady()` in POSTED → consumed, no action (it used to fall through to the reconcile gate and
  could occupy `pendingFrame` before the settle frame).
- S2.3 `settleElapsed()`: a `webView.invalidate()` even when `refreshFrame` submits nothing, as stock and our
  `pause()` do.
- S2.3: new row: no timer acts while `gesture.drawing`.
- S2.4 `begin()`: "First `when (handBack.penDown(erasing))`" → after `beginNeedsEnd()`/`end()` (`:782`), before the
  overlay swallow (`:792`), with the two failure cases.
- S2.4 `commit()`: routes AWAIT_CANVAS, POSTED and PAUSED, and only a canvas with `args.sequence >= sequence`.
- S2.5: `pushPenSettings()` on every tip pen-down with the dirty flag, not only after a CANCEL.
- S2.6 step 4: "After 170 ms" → 170 ms after the `enablePost` call started.
- S2.2 step 3: colour → style → width → today's device-verified width → colour → style; §5.2's resume block and
  the "differences from stock" list follow.
- S2.8 and the §5.4 JS test: "the commit waits for `end`" → the `stroke` event releases the commit
  (`hw/onyx-ink.ts:246-247`), and the test checks that instead.
- §2.3: the split contact is "3 ms, then 80 ms" → 3-6 ms, then 9-80 ms, and it followed our pen-down render-off in
  4 of 5 recorded gestures, never in stock [log].
- §5.2: the enablePost duration line; the 1-2 ms render-off pair; the resume-block order; an `enablePost` that may
  sit between two quick contacts.
- §6: `S3-QUICK` CANCEL case, new `S3-AUTO` expectation.
- §9: frame-fence note; R3 (the hop can wait behind the block); R5 (style slot is reset by `closeRawDrawing`);
  new R8 (main-thread block).
- §11: H13 (block and settle timing), H14 (no split contact), H15 (trace after CANCEL in POSTED), H16 (trace width
  from the parameter, not the stroke width).
- Prefix table: `FW/` added.

**Remaining doubts**

- Whether leaving schema 7 through `ENABLE_POST(1)` with render still on re-arms the pen trigger for the next
  contact (H15). Stock never cancels its hand-back, so no recording covers it. If H15 fails, the alternative is
  stock's behaviour: no CANCEL in POSTED, render off 170 ms after the call even under a pen, and RESEND.
- The ~150 ms is inferred from gaps between two transactions on one thread, in five recordings; no SF disassembly
  of `handleEnablePost`'s sleep was redone here. The gap was 12-13 ms when SF chose scheme 4, so the block
  depends on SF's state, which is why S2.3 measures instead of assuming.
- The 150 ms main-thread block (R8) is stock's behaviour too, but our UI thread also carries the WebView's input.
  Not measured.
- The width of the soft-eraser trace is assumed to come from the style-8 parameter and to be a diameter in panel
  pixels (H16, H1). The panel is 1860 px wide at 300 dpi and the WebView is not scaled [inferred from `A4:28` and
  `FA4:70`], so view px = panel px; not verified.
- `DENSITY_DEVICE_STABLE` may be derived from `ro.sf.lcd_density` rather than the panel's ppi; only H1 settles it.
- The swallowed-overlay path with the eraser end draws some soft-eraser trace over the floating control before
  render goes off; nothing in the plan removes that trace explicitly. It is the same exposure the tip has today
  [inferred], and it is small.
- The order of SF setters is assumed not to matter (S2.2 keeps ours, stock uses another). Unverified either way.
