# 12 — Stock Notes eraser menu: a spec for reproduction

This spec covers the eraser popup of the stock Onyx Notes app (`com.onyx.android.note`, versionCode 45326) on
the BOOX Note Air 4C. It describes the layout, the strings, the icons, what every control does, how the diameter
is converted, how the menu opens and closes, and where the selections are stored. It was written so that
Tangleaf's handwriting UI can reproduce the menu. The method is static analysis of the decompiled APK. It also
uses two recorded artefacts from 2026-09-25: the user's screenshot of the open menu and the A7 call-tracer
log (see [11](11-observations.md)). This pass did not touch the tablet.

Claim tags, as in the other documents: **[code]** read in source or resources at the cited line;
**[inferred]** reasoned from what was read, with the reasoning given; **[unknown]** not settled statically.
**[shot]** means measured on the user's screenshot. **[log]** means read in a recorded trace.

| prefix | location                                                                                         |
| ------ | ------------------------------------------------------------------------------------------------ |
| `NA/`  | `~/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/` (jadx; `note/` app, `sdk/` SDK) |
| `RES/` | `~/tmp_zfs/boox-notes-inspect/decoded/resources/res/`                                            |
| `OBS/` | `~/tmp_zfs/pen-observation/` (recorded traces and screenshots, 2026-09-25)                       |
| `SHOT` | `OBS/shots/stock-eraser-menu.png` (1860×2480, the menu open over a page, Pixel Eraser selected)  |
| `A7`   | `OBS/a7-stock.txt` (condensed trace of action A7, eraser tool, stock)                            |

The second decompilation (`~/tmp_zfs/reversed_onyx_notes_app/src`) matches on the lines spot-checked:
`ErasePopup.java:239`, `PenTipEraseToolMenuModel.java:30`, `NoteUnits.java:33-49` [code].

---

## 1. Key facts

- The menu is `ErasePopup` (`NA/note/note/menu/popup/ErasePopup.java`). It inflates `R.layout.layout_shape`
  and binds `ErasePopupViewModel` [code, `ErasePopup.java:412-455`]. On this device the portrait layout is
  `RES/layout-sw623dp/layout_shape.xml` [inferred, §2.2].
- **Toolbar button, first tap vs second tap.** If the eraser tool is not active, the first tap on the toolbar
  eraser button only activates the eraser tool and opens no menu. When the eraser tool is already active, a tap
  opens the menu. A tap while the menu is showing closes it
  [code, `PenTipEraseToolMenuModel.java:29-36`, `ToolbarMenuModel.java:238-244, 1410-1421`].
- **Three types, left to right: Pixel Eraser, Lasso Eraser, Stroke Eraser.** Their internal values are 1, 2
  and 0 [code, `layout-sw623dp/layout_shape.xml:22-97`, `EraseType.java:12-14`]. The English label of the
  "area" eraser is **"Lasso Eraser"** [code, `RES/values/strings.xml:191`].
- **Tapping a type selects it and closes the menu** right away (`CloseErasePopupEvent`)
  [code, `EraseByTypeAction.java:103-116`].
- **Title:** `"<type name> <d>mm"` with `%.2f`, e.g. `Pixel Eraser 5.00mm`. The Lasso Eraser shows only
  `Lasso Eraser` and hides the whole diameter block [code, `ErasePopupViewModel.java:286-296`, binding
  `LayoutShapeBindingSw623dpImpl.java:560`].
- **Diameter** is in millimetres and stored in hundredths of a millimetre. Pixel Eraser: 1.00–8.00 mm. Stroke
  Eraser: 0.50–8.00 mm. The step is 0.05 mm for both the `<`/`>` buttons and the slider. The slider is linear
  in mm, with 141 or 151 positions [code, `NoteUnits.java:28-69`, `ErasePopup.java:308-410`].
  Defaults: Pixel 5.00 mm, Stroke 0.50 mm, Lasso 0.50 mm (not shown). **Each type remembers its own
  diameter** [code, §6].
- **mm → px:** `px = mm × densityDpi / 25.4`, with `densityDpi` from the app's `DisplayMetrics`
  [code, `NA/sdk/extension/DimensKt.java:11-16`]. On this tablet `densityDpi = 300` [inferred, §6.4], so
  1 mm = 11.811024 px. The same px value is the SurfaceFlinger soft-eraser width (`SET_STROKE_PARAMETERS(8, [px,
  0.5, 0.1])`) and the hit-test diameter in screen pixels [code, §6.5].
- **Correction to the brief.** 80.314964 px is **6.80 mm**, not 5.00 mm. 5.00 mm is 59.05512 px, and A7 records
  that value after the user picked the 5.00 mm eraser [log, `A7:445`]. 5.905512 px is 0.50 mm [log, `A7:250`].
- **"Do Not Erase Shapes"** (default OFF) makes every erase path skip geometric shapes: line, circle,
  rectangle, triangle, polygon, polyline and so on (§7.1) [code].
- **"Auto-deselect Eraser"** (default OFF): after each erase gesture made with the eraser tool, the app
  switches back to the previous tool. The ⓘ icon opens a hint popup that reads "When enabled, auto-switch to
  last tool after erasing." [code, §7.2].
- **"Erase Current Layer" / "Erase All Layers"** first show an OK/Cancel confirmation. OK erases the current
  page only (current layer, or all layers), records an undo step, switches to the drawing tool and closes the
  menu [code, §8].
- **Closing the menu** (outside tap, type choice, confirmed layer erase, eraser button) produces one partial
  REGAL_PLUS repaint. The pen session resumes 500 ms later [code + log, §9].
- **Storage:** switches in global MMKV. The type goes to global MMKV when the document is saved. The per-type
  widths (in px) are stored with each notebook's `NotePenInfo` and copied to MMKV [code, §10].

---

## 2. Sources and the layout variant

### 2.1 Classes and resources

| role                            | file                                                                                                                                                                                                                          |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| popup                           | `NA/note/note/menu/popup/ErasePopup.java` (`FloatErasePopup.java` for the floating toolbox, §11.4)                                                                                                                            |
| view model                      | `NA/note/note/menu/viewmodel/scribble/ErasePopupViewModel.java` (base `NA/sdk/note/ui/viewmodel/PopupViewModel.java`)                                                                                                         |
| binding (portrait, this device) | `NA/note/databinding/LayoutShapeBindingSw623dpImpl.java`                                                                                                                                                                      |
| layout                          | `RES/layout-sw623dp/layout_shape.xml` (base `RES/layout/layout_shape.xml`; landscape `RES/layout-sw623dp-land/layout_shape.xml`)                                                                                              |
| type enum                       | `NA/note/note/menu/data/EraseType.java`                                                                                                                                                                                       |
| units                           | `NA/sdk/scribble/utils/NoteUnits.java`, `NA/sdk/extension/DimensKt.java`, `NA/sdk/utils/NumberUtils.java`                                                                                                                     |
| widths                          | `NA/sdk/data/note/EraserArgs.java`, `NA/sdk/data/EraseConstant.java`, `NA/sdk/note/ui/config/DeviceConfig.java`, `NA/sdk/scribble/data/NoteDrawingArgs.java`, `NA/sdk/notecore/editor/pen/NotePenManager.java`                |
| actions                         | `NA/note/note/action/erase/{EraseByTypeAction,StartEraseAction,EraseLayerAction}.java`, `NA/note/note/action/shape/{EraseCurrentLayerAction,EraseAllLayerShapesAction}.java`, `NA/note/note/action/ui/PopupChangeAction.java` |
| toolbar                         | `NA/note/note/menu/viewmodel/scribble/{ToolbarMenuModel,PenTipEraseToolMenuModel}.java`, `NA/note/note/menu/{FunMenuHandler,FunMenuProvider}.java`                                                                            |
| hint popup                      | `NA/note/note/menu/popup/AutoDeselectEraserHintPopup.java`, `RES/layout/layout_auto_deselect_eraser_hint_popup.xml`                                                                                                           |
| settings                        | `NA/sdk/note/ui/common/utils/NoteMMKVHelper.java`                                                                                                                                                                             |

### 2.2 Which layout and dimension bucket applies on the Note Air 4C

- `layout_shape.xml` exists only in `layout`, `layout-land`, `layout-sw430dp-land`, `layout-sw570dp(-land)`
  and `layout-sw623dp(-land)` [code, directory listing]. The popup is 906 px wide in the trace
  (`View.invalidate(0, 0, 906, 1044, 5)`, `A7:203`), and `erase_popup_width` is 483 dp (§4.1). So the density is
  906 / 483 = 1.875, which is `densityDpi` 300. The smallest width is then 1860 / 1.875 = 992 dp. At 992 dp
  Android picks `layout-sw623dp` for the layout and, for dimensions, the largest `values-swNNNdp` ≤ 992 that
  defines each one, often `values-sw990dp` [inferred]. The dimensions measured on SHOT match the
  `values-sw990dp` values: margins of 40 px = 21.2 dp, type cells of about 187 px, switches of about 100×50 px
  = 53.3×26.7 dp [shot].
- In `layout-sw623dp` the two layer buttons sit **side by side** as outlined boxes. In the base layout they are
  stacked full-width rows [code, diff of the two files at lines 248-303].
- No `values-en*` file overrides any eraser string or dimension. The English strings come from
  `RES/values/strings.xml` [code].
- Every size is multiplied by the popup's `getFixScale()`. That is 1.0 unless it is set (`BaseNotePopWindow.java:35,
  80-86`), and the screenshot measurements fit 1.0 [code + shot].

---

## 3. Opening the menu from the toolbar

### 3.1 Tap logic

On this model `DeviceConfig.canPenTipErase` is true (it defaults to true and `note_noteair4c.json` does not set
it; see 03 §7). So the toolbar model is `PenTipEraseToolMenuModel` [code,
`ToolbarMenuModel.java:632`]. The toolbar routes the `ERASER` item to `onEraseMenuClick`
[code, `FunMenuHandler.java:217-220`]:

```java
// PenTipEraseToolMenuModel.java:29-36
if (getHandlerManager().inEraseProvider()) {
    showErasePopup(menuClickEvent);           // eraser already active: open (or close) the menu
} else {
    prepareStartEraseProvider();              // quitTextInput, quitSelection, closeAllSubMenu (ToolbarMenuModel:1299-1303)
    new StartEraseAction(editorBundle).execute();  // activeProvider(SCRIBBLE_ERASE_PROVIDER), StartEraseAction.java:29
}
```

- **Eraser not active → tap 1** activates the eraser tool with the last-used type and diameter. No menu opens
  [code]. The pen-state consequences of this tap are in 03 §3.2 (only the `onActivate` 150 ms cycle for this
  entry point).
- **Eraser active → tap** calls `showErasePopup`. If the popup for the `ERASER` menu id is already showing,
  `J()` closes it and returns. Otherwise it runs `ensureInScribbleMode(false)` (quit text input, selection,
  fill colour, universal shape; it does not close other sub-menus) and shows a new `ErasePopup` anchored to the
  button [code, `ToolbarMenuModel.java:238-244, 1410-1421, 1533-1541`].
- The one-time "eraser tips" dialog of the base toolbar model is suppressed: `showEraseTips()` returns false
  [code, `PenTipEraseToolMenuModel.java:60-62`].
- There is no long-press action on the toolbar eraser [code, grep of `FunMenuHandler`/`ToolbarMenuModel`].
- **[unknown]** What a tap on the eraser button does while the menu is open, in practice. The popup is not
  focusable (nothing calls `setFocusable`), and it is outside-touchable (`ErasePopup.java:427`). An outside
  touch may therefore dismiss it before the click reaches `J()`, in which case `J()` finds nothing showing and
  opens the menu again. Needs a look on the device.

### 3.2 The toolbar button itself

- Its icon is the icon of the current eraser type (`EraseType.getEraserIcon`). Its selected state is
  `inEraseProvider()`. Both are refreshed on `HandlerChangedEvent` and `UpdateEraserMenuEvent`
  [code, `FunMenuProvider.java:263-267`, `FunMenuHandler.java:146-149`].
- In SHOT the button shows the Pixel Eraser icon. A small filled triangle sits on the toolbar's bottom border
  under it, pointing into the menu [shot]. Whether that triangle marks the selected tool or the popup anchor is
  **[unknown]**.

### 3.3 Where the menu appears

It hangs below the toolbar under the eraser button. In SHOT its right edge is about 42 px from the right screen
edge and it overlaps the page [shot]. Placement goes through `showPopupMenu` /
`NotePopupUtils.setPositionByMenuId` (`ToolbarMenuModel.java:1543-1550`). The exact rule was not traced
**[unknown]**. The menu's type row and its click listeners are set up only when the eraser tool is active or
`canPenTipErase` is false (`ErasePopup.java:239`). On this device the menu is only reachable with the eraser
active, so the type row is always live [code + inferred].

---

## 4. Layout, top to bottom (portrait, this device)

Sizes are the resolved dimensions for this device (§2.2) in **dp**. At density 1.875, 1 dp is 1.875 device px.
For each row: the view, the visibility rule, and the values.

### 4.1 Container

- `ScrollView` → `ConstraintLayout` (`layout_shape.xml:2-12`). Popup width is `erase_popup_width` = **483 dp**
  (`values-sw935dp`) × fixScale, and the height wraps [code, `ErasePopup.java:418-420`]. Measured: 906 × 1044 px
  [log, `A7:203`].
- Background `bg_popwindow_view`: white fill, **3 px** black stroke (`pop_menu_border_width`), corner radius
  **19 px** (`global_dialog_corners`, `values-sw990dp`) [code, `RES/drawable/bg_popwindow_view.xml`,
  `NotePopupUtils.java:181-183`].
- Content top padding: `contentMarginVertical` = `erase_popup_padding_vertical` = **28.6 dp**
  [code, binding `:486`, `ErasePopupViewModel.java:84`].
- If `should_limit_erase_popup_height` is set, the height is capped at `erase_popup_max_height` and the
  content scrolls (`NotePopupUtils.java:228-253`). Whether the flag is set in this bucket was not resolved
  **[unknown]**. The recorded height of 1044 px shows no cap in practice [log].

### 4.2 Title row: `tv_selected_value`

- Text: `strokeWidthTitleValue` (§5.2). Black, `erase_popup_title_text_size` → `custom_menu_display_text_size`
  = **20 sp**. Start margin **21.2 dp** (`erase_popup_item_padding_horizontal`)
  [code, `layout_shape.xml:13-21`, binding `:535-536`].
- SHOT: "Pixel Eraser 5.00mm", left-aligned [shot].

### 4.3 Type row: three equal cells

- Views `moveEraser`, `areaEraser`, `strokeEraser`, in that order, in a packed horizontal chain. Each is a
  `View` of width 0 (equal thirds). The chain has start margin (on the first) and end margin (on the last)
  `erase_popup_type_margin_horizontal` = **92.2 dp** and top margin `erase_popup_type_margin_top` = **13.8 dp**
  [code, `layout_shape.xml:22-97`, binding `:524-526`].
- Cell height: `iconSize + 2 × contentMarginVertical` = 33.9 + 2 × 28.6 = **91.1 dp**
  [code, `ErasePopupViewModel.java:99`].
- In each cell: an icon (`ic_note_eraser_move` / `_area` / `_stroke`) of **33.9 dp** square
  (`erase_popup_icon_size`), centred. Under it, a selection dot `ic_note_point` (black filled circle) of
  **11 dp**, bottom-aligned in the cell with bottom margin **11.7 dp**. Only the selected type's dot is visible
  (GONE otherwise) [code, binding `:557, 561, 567`, style `style_eraser_item_indicator`
  `RES/values/styles.xml:5598-5600`].
- The whole cell (the `View`) is the tap target [code, `ErasePopup.java:249-277`].
- Divider `erase_type_divider`: **full width**, `list_divider_height` = **2 px**, black
  [code, `layout_shape.xml:98-103`].

### 4.4 Diameter block (hidden entirely for the Lasso Eraser)

The group `stroke_width_title, stroke_width_value, seek_bar, stroke_width_divider, increase_stroke_width,
decrease_stroke_width` is visible only if `!areaEraseSelected` [code, `layout_shape.xml:170-174`,
binding `:560`].

- `stroke_width_title`: **"Eraser Diameter"** (`@string/eraser_stroke_width`, `strings.xml:963`), 20 sp,
  top margin `erase_popup_content_title_margin_top` = **21.2 dp**, start margin 21.2 dp
  [code, `layout_shape.xml:104-113`, binding `:531-534`].
- Right-aligned on the same baseline: `decrease_stroke_width` (`<`), `stroke_width_value`, `increase_stroke_width`
  (`>`), end margin 21.2 dp [code, `layout_shape.xml:114-148`].
  - `<` / `>`: `FrameLayout` with `note_btn_click_style` (background `bg_button_click_solid`, 4 px bottom padding)
    holding `ic_note_prev` / `ic_note_next` at `note_erase_popup_stepper_icon_size` → `menu_icon_size` =
    **36 dp**. Each icon is a rounded-rectangle outline (≈ 3.25–44.75 of a 48 viewport, radius ≈ 5,
    1.5-unit stroke, white fill) with a chevron inside [code, `RES/drawable/ic_note_prev.xml`,
    `ic_note_next.xml`, `styles.xml:4958-4963`].
  - Value text: `strokeWidthValue`, e.g. **"5.00mm"**, `pen_popup_text_size` → `text_size_level_2` = **20 sp**
    [code, binding `:570`, `:537`].
- `seek_bar`: `WaterDropSeekBar`, full width with horizontal margins 21.2 dp, top margin 21.2 dp
  [code, `layout_shape.xml:149-159`, binding `:527-529`]. Drawing [code, `WaterDropSeekBar.java:59-114`]:
  - Track: one black filled wedge from the left end to the right end, inset by half the thumb. It is
    **1 dp** thick with a round cap at the left and grows linearly to **27 dp** with a semicircular cap at the
    right (`layout_pen_popup_pen_stroke_width_seekbar_min/max_track_width`). The whole track is black: the part
    before and after the thumb look the same (no "filled" colour) [code, `:79-114`; shot].
  - Thumb: `pen_popup_stroke_width_seekbar_thumb_drawable`, a white disc with a black ring 4/48 of its diameter
    thick, **34 dp** [code, drawable, `values-sw990dp`].
  - RTL layouts swap the thin and thick ends (`:85-91`) [code].
- `stroke_width_divider`: 2 px black, **inset 10 dp** left and right (`erase_divider_line_margin_horizontal`),
  top margin 21.2 dp [code, `layout_shape.xml:160-169`, binding `:545`].

### 4.5 "Do Not Erase Shapes" row (visible if the scene supports shapes)

- `disable_erase_graph` text **"Do Not Erase Shapes"** (`strings.xml:731`), 20 sp, start margin 21.2 dp,
  vertical padding 28.6 dp [code, `layout_shape.xml:175-184`, binding `:519-521`].
- `disable_erase_graph_checkbox`: `CheckBox` with background `bg_btn_switch` (a selector over
  `ic_switch_on` / `ic_switch_off`), **53.3 × 26.7 dp**, end margin 21.2 dp [code, `layout_shape.xml:185-196`,
  `RES/drawable/bg_btn_switch.xml`]. SHOT shows the OFF state as an outlined rounded box with a filled black
  square on the left and the word "OFF" on the right [shot]. Only the switch is clickable: the label has no
  listener (§7).
- Divider: inset 10 dp, 2 px.
- Row, switch and divider are visible iff `showGraphMenu` = `activeNoteScene.supportGraph()`. That is true in
  normal notebooks (`BaseNoteScene.java:851-853`) and false in the meeting scene (`NoteMeetingScene.java:386-388`)
  [code, `ErasePopup.java:171`, binding `:549-551`].

### 4.6 "Auto-deselect Eraser" row

- `auto_deselect_eraser` text **"Auto-deselect Eraser"** (`strings.xml:221`), 20 sp; then
  `auto_deselect_eraser_hint`, an `ic_note_tips` info icon (a circle outline with an "i": dot plus bar) at
  **26 dp** (`note_erase_popup_tips_icon_size`), **10 dp** start margin after the text; then the switch
  (same as §4.5) [code, `layout_shape.xml:207-238`, binding `:515-518`].
- Divider `auto_deselect_eraser_divider`: inset 10 dp, 2 px. It is always visible [code, `:239-247`].
- Landscape uses `ic_note_info` in place of `ic_note_tips` and `check_selector` in place of the switch
  [code, `layout-sw623dp-land/layout_shape.xml`].

### 4.7 Layer buttons (visible if the scene shows them)

- Two boxes side by side: `eraser_current_layer` (left) and `eraser_all_layer` (right), equal width. Each has
  `widget_rectangle_background`: white fill, **2 px** black stroke, radius **14 px** (`values-sw990dp`)
  [code, `layout-sw623dp/layout_shape.xml:248-291`, `RES/drawable/widget_rectangle_background.xml`].
- Height `erase_popup_layer_btn_height` = **56.2 dp**, top margin 28.6 dp. The left box has start margin
  21.2 dp. The right box has start margin (gap) `erase_popup_layer_btn_margin` = **17 dp** and end margin 21.2 dp
  [code, binding `:522-526`].
- Content: a centred `TextImageView` with the left drawable at 33.9 dp, drawable padding
  `erase_popup_icon_padding` = **14.8 dp**, text `erase_popup_layer_text_size` = **17 sp**
  [code, binding `:538-543`].
  - Left: icon `ic_note_eraser_cur_layer_vector` (an eraser resting on one flat rhombus "sheet"), text
    **"Erase Current Layer"** (`strings.xml:957`).
  - Right: icon `ic_note_eraser_all` (the same eraser on a sheet with a second sheet outline stacked under it),
    text **"Erase All Layers"** (`strings.xml:955`) [code, drawables; shot].
- `space_btm` below: 28.6 dp high [code, binding `:530`].
- The group (both boxes, both labels, `space_btm`) is visible iff `showEraseLayerMenu` =
  `activeNoteScene.showEraseLayerMenu()`. That is true in normal notebooks (`BaseNoteScene.java:791-793`) and
  false in the draft and meeting scenes [code, `ErasePopup.java:175`, binding `:564`,
  `layout-sw623dp/layout_shape.xml:302-303`].

### 4.8 The type icons (vector drawables, 48×48 viewport, black)

| type (value)      | drawable                | shape                                                                                                                                                                                                       |
| ----------------- | ----------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Pixel Eraser (1)  | `ic_note_eraser_move`   | a block eraser tilted 45°, drawn as an outline, with an inner line splitting off the lower-left tip, sitting on a short horizontal baseline that runs to the right (`RES/drawable/ic_note_eraser_move.xml`) |
| Lasso Eraser (2)  | `ic_note_eraser_area`   | a smaller copy of the same eraser (outline plus split line) in the centre, surrounded by a **dashed circle** of 16 dashes, radius ≈ 22, stroke ≈ 3 (`ic_note_eraser_area.xml`)                              |
| Stroke Eraser (0) | `ic_note_eraser_stroke` | the eraser outline with its split line and, at the lower right, a **squiggle** (a short wavy stroke) in place of the baseline (`ic_note_eraser_stroke.xml`)                                                 |

All three are **[code]** (path data read), and SHOT confirms them [shot].

---

## 5. The three types

### 5.1 Order, values, strings

| position | view           | `EraseType`     | value | English name (`strings.xml`) | SF main style (eraser tool) | trace parameters                                         |
| -------- | -------------- | --------------- | ----- | ---------------------------- | --------------------------- | -------------------------------------------------------- |
| left     | `moveEraser`   | `MOVE_ERASER`   | 1     | **Pixel Eraser** (`:1700`)   | 8 (soft eraser)             | `(8, [w1, 0.5, 0.1])`, eraser raw drawing `(1, 8)`       |
| middle   | `areaEraser`   | `AREA_ERASER`   | 2     | **Lasso Eraser** (`:191`)    | 5 (dash)                    | `(5, [5.0])`, `(8, [w2, 0.5, 0.1])`, eraser raw `(1, 5)` |
| right    | `strokeEraser` | `STROKE_ERASER` | 0     | **Stroke Eraser** (`:2997`)  | 8                           | `(8, [w0, 0.5, 0.1])`, eraser raw `(1, 8)`               |

Sources: the layout order [code, `layout_shape.xml:22-97`], the enum [code, `EraseType.java:12-14`], and the SF
column [code, `ResumeRawDrawingRequest.java:53-54`, 03 §5]. The A7 trace shows style 8 with 80.314964 and
59.05512 and style 5 with 5.905512 [log, `A7:32-39, 245-252, 440-447`]. What the types do is in 03 §2
("Eraser type constants"): Pixel splits strokes, Stroke removes whole strokes, Lasso removes what the drawn
loop encloses.

The default type is **0, Stroke Eraser**. `NoteDrawingArgs.defaultErase()` returns 0
(`NoteDrawingArgs.java:60-62`), `EraserArgs.createDefaultEraserArgs()` uses type 0 (`EraserArgs.java:51-55`),
and MMKV `ERASER_TYPE_KEY` defaults to it (`NoteMMKVHelper.java:834-835`) [code].

### 5.2 Title format

`ErasePopupViewModel.updateEraserWidthValue(type, widthPx)` [code, `ErasePopupViewModel.java:286-296`]:

```
name = EraseType.getEraserName(type)                    // "Pixel Eraser" | "Lasso Eraser" | "Stroke Eraser"
if (areaEraseSelected) title = name; return              // Lasso: "Lasso Eraser", no value
value = NumberUtils.formatFloat(pct / 100f) + "mm"       // String.format("%.2f") → "5.00mm"  (NumberUtils.java:27-29; DateTimeUtil.STRING_FORMAT_MM = "mm")
title = name + " " + value                               // "Pixel Eraser 5.00mm", "Stroke Eraser 0.50mm"
```

`pct` is `NoteUnits.widthPercentage(widthPx, type)` (§6.2). The title and the value next to the buttons update
on every diameter change (`ErasePopup.java:539, 542-544`) [code].

### 5.3 What choosing a type does

The cell listeners call `viewModel.onErase(editorBundle, context, EraseType.X)` → `EraseByTypeAction`
[code, `ErasePopup.java:249-306`, `ErasePopupViewModel.java:267-272`]:

1. On the main thread, `applyEraserConfig()` [code, `EraseByTypeAction.java:103-116`]:
   `drawingArgs.setCurrentEraseType(v)`; `activeEraserArgs.type = v`;
   `activeEraserArgs.updateWidth(v, drawingArgs.getEraserWidth())`, which loads the new type's own stored width
   or its default (§6.3); post `UpdateEraserMenuEvent` (toolbar icon); `activeProvider(SCRIBBLE_ERASE_PROVIDER)`
   (a no-op when the eraser is already active, `HandlerManager.java:82`); post **`CloseErasePopupEvent`** → the
   popup's `onClosePopupEvent` calls `dismiss()` (`ErasePopup.java:514-518`).
2. Then `InvalidateScreenAction`: pause render and input, `View.invalidate()`, resume after
   `DELAY_ENABLE_RAW_DRAWING_MILLS` = 500 ms on a colour panel. This resume re-sends the style and stroke
   parameters of the new type [code, `EraseByTypeAction.java:68-86`, 03 §3.2].

Tapping the type that is already selected runs the same path. There is no guard, so the menu closes
[code]. There is no "tap the selected type again to open more settings".

---

## 6. Diameter

### 6.1 Ranges, step, slider

All values are in **hundredths of a millimetre** ("percentage" in the code) [code, `NoteUnits.java:28-69`]:

| type       | min           | max           | step (buttons and slider) | slider positions  | default             |
| ---------- | ------------- | ------------- | ------------------------- | ----------------- | ------------------- |
| Stroke (0) | 50 = 0.50 mm  | 800 = 8.00 mm | 5 = 0.05 mm               | 151 (`max` = 150) | 0.50 mm (§6.3)      |
| Pixel (1)  | 100 = 1.00 mm | 800 = 8.00 mm | 5 = 0.05 mm               | 141 (`max` = 140) | 5.00 mm             |
| Lasso (2)  | 0             | 0             | —                         | — (block hidden)  | 0.50 mm (not shown) |

- The value list is `[min, min+5, …, max]` (`getEraseWidthRange`, `:37-46`). The slider's `max` is
  `list.size - 1`, and its progress is the list index of the current value [code, `ErasePopup.java:322-323,
  510-512`]. So the slider is **linear in mm** with one position per 0.05 mm [code]. SHOT: 5.00 mm on the Pixel
  range puts the thumb at index 80/140 = 0.571 of the track, which matches the picture [shot].
- `<` sets index −1, clamped at 0. `>` sets index +1, clamped at `max`. Each tap applies the value and moves the
  thumb [code, `ErasePopup.java:367-410`]. These are plain click listeners with no auto-repeat on hold
  [inferred: only `setOnClickListener` is used].
- The slider applies the value **continuously while dragging**: every user progress change calls `F(progress)`
  → `updateEraserWidth` (`ErasePopup.java:324-331, 77-79`) [code].
- The value range list is built once, when the popup is created, for the type current at that moment
  (`ErasePopup.java:412-413`). Changing the type closes the popup (§5.3), so the list never goes stale [code +
  inferred].

### 6.2 px ↔ mm and the displayed value

```
mmToPx(mm) = mm * densityDpi / 25.4             // DimensKt.java:11-13, densityDpi = app Resources DisplayMetrics
pxToMM(px) = px / densityDpi * 25.4             // DimensKt.java:15-17
widthFromPercentage(p) = mmToPx(p / 100f)       // NoteUnits.java:63-65
widthPercentage(px, t) = roundToNearestMultipleOfFive( clamp( (int)(pxToMM(px) * 100), min(t), max(t) ) )
                                                // NoteUnits.java:59-61, 67-69; NumberUtils.java:71-74 (remainder < 3 rounds down, else up)
```

Setting a diameter stores `widthFromPercentage(p)` px (`ErasePopup.java:530-540`). The display converts px back
through `widthPercentage` (§5.2) [code]. The truncating `(int)` is corrected by the round-to-5: 59.05512 px →
499.99 → 499 → 500 → "5.00mm" [inferred arithmetic from the code].

### 6.3 Defaults and per-type memory

- Defaults: `DeviceConfig.defaultEraserMoveWidthWithMM = 5.0` and `defaultEraserStrokeWidthWithMM = 0.5`
  (`DeviceConfig.java:96-97`). At config load they become `EraseConstant.DEFAULT_ERASER_MOVE_WIDTH =
  mmToPx(5.0)` and `DEFAULT_ERASER_STROKE_WIDTH = mmToPx(0.5)` (`DeviceConfig.java:205, 226-233`,
  `ConfigManager.java:29-31`). `getDefaultEraserWidth(type)` returns the move width for type 1 and the stroke
  width for every other type, including 2 (`EraseConstant.java:33-38`) [code]. `note_noteair4c.json` does not
  override them [code].
- **Each type keeps its own width.** `EraserArgs.eraseWidthMap: Map<String, Float>` is keyed by the type.
  `getWidth()` reads the current type's entry and falls back to the default (`EraserArgs.java:128-136`), and
  `updateWidth(type, w)` writes one entry (`:157-159`). The notebook-level copy is
  `NotePenInfo.eraseWidthMap: Map<Integer, Float>`, written by `NoteDrawingArgs.setEraserWidth(type, w)` and read
  by `getEraserWidth(type)` (`NoteDrawingArgs.java:141-143, 281-284, 359-361, 385-388`). The popup writes both
  (`ErasePopup.java:534-535`). A type switch loads the new type's entry (`EraseByTypeAction.java:110`) [code].
- **Clamp on load:** when the notebook's eraser args are built, `EraseUtils.migrateLegacyEraserWidthLimitIfNeed`
  lowers any type 0/1 width above 8.00 mm to 8.00 mm (`EraseUtils.java:32-…`, `NoteDrawingArgsExtKt.java:18-33`)
  [code]. It sets no minimum. Values below the minimum are only clamped for display (§6.2).

### 6.4 The DPI on this device, and the values in the brief

The code reads `densityDpi` from the app's `DisplayMetrics` [code]. The value on the tablet was not read
directly. Three recorded facts give **300** [inferred]:

- The popup is 906 px wide for 483 dp, a density of 1.875 = 300/160 [log, `A7:203`].
- The area eraser's width parameter is 5.905512 px [log, `A7:250`]. That is exactly `mmToPx(0.5)` at 300, the
  type-2 default.
- After the user switched to the 5.00 mm eraser the parameter is 59.05512 px [log, `A7:445`]. That is exactly
  `mmToPx(5.0)` at 300.

At 300 dpi, **80.314964 px = 6.80 mm** (80.314964 × 25.4 / 300 = 6.7999…, displayed as "6.80mm"). This is the
style-8 width at the start of A7 and at page open in A1 [log, `A7:36-37`]. Which of the two style-8 types
carried 6.80 mm was not recorded **[unknown]**. The "5.00mm" in SHOT belongs to 59.05512 px. The brief's pairing
"5.00 mm → 80.314964 px" is therefore wrong. Its "0.5 mm → 5.905512 px" is right (that is also the Lasso
Eraser's default).

Widths are stored in **px**, so they depend on the density. If the app's DPI changed (for example through
EinkWise per-app DPI), stored widths would show as different mm values [inferred from the storage type].

### 6.5 Where the width goes

- **SurfaceFlinger trace:** `ResumeRawDrawingRequest` sends `setStrokeParameters(8, [activeEraserArgs.width, 0.5,
  0.1])` on every resume [code, `ResumeRawDrawingRequest.java:54`]. A diameter change while the menu is open
  reaches SF only at the resume after the menu closes. The popup keeps the pen paused, and
  `EraseWidthChangedEvent` has one subscriber, which saves the float menu (`FragmentFloatMenu.java:372-375`)
  [code + inferred].
- **Hit test:** `ScribbleHandler.d()` builds `EraseArgs` with `eraserWidth = w`, `drawRadius = w / 2` and
  `normalizeScale = 1 / renderScale` (`ScribbleHandler.java:318-323`). The requests test with radius
  `drawRadius × normalizeScale` in page units (`StrokesErasingRequest.java:87, 188`,
  `OverlayErasingRequest.java:77`) [code]. So the setting is the **diameter in screen pixels at any zoom**.
  The Lasso Eraser does not use it for hit testing (03 §2).
- The erasers never touch images, text, attachments or rich text, and they act on the current layer only
  (`ShapeUtils.java:121-129`, `OverlayErasingRequest.java:82`, `AreaEraseFinishedRequest.java:100`) [code].

---

## 7. The two switches

Both switches are wired the same way. The popup reads MMKV into the view model once, at creation
(`ErasePopup.java:99-103`). A tap on the **switch itself** flips the MMKV value immediately
(`ErasePopup.java:110-156`). The `CheckBox` toggles its own drawable state, and the popup stays open [code].
The label text has no click listener [code]. The two rows appear in the view model as
`disableEraseGraphChecked` and `autoDeselectEraserChecked`.

### 7.1 "Do Not Erase Shapes"

- MMKV `ERASE_GRAPHICS_SWITCH_KEY`, default **false**, global (`NoteMMKVHelper.java:143, 1089-1090,
  1931-1932`) [code].
- When ON, "graphical" shapes are excluded from erasing. The list is `ShapeFactory.isGraphicalShape`: line (7),
  polyline (31), polygon (17), wave line (25), triangle (8), circle (0), rectangle (1), trapezoid (26), regular
  polygon (24), regular hexagon (27), arrow line (28), rhombus (18), curve (39), universal shape (40)
  (`ShapeFactory.java:94-139, 382-384`; `ShapeUtils.java:514-516, 833-846`) [code]. Every eraser applies it:
  Stroke (`StrokesErasingRequest.java:122, 188`, flag from `ScribbleHandler.d()`), Pixel
  (`OverlayErasingRequest.java:78-82`), Lasso (`AreaEraseFinishedRequest.java:99-100`), both layer buttons
  (`EraseCurrentLayerAction.java:44`, `EraseAllLayerShapesAction.java:64` → `EraseCurrentLayerRequest.java:56`,
  `ClearAllFreeShapesRequest.java:65`), and the HWR scratch-out gesture (`HWRHandleEraseGestureAction.java:121`)
  [code]. Freehand ink is never affected.
- The value is read when each erase runs, so a change applies to the next gesture [code].

### 7.2 "Auto-deselect Eraser"

- MMKV `AUTO_DESELECT_ERASER_KEY`, default **false**, global (`NoteMMKVHelper.java:77, 1071-1072,
  1358-1359`) [code].
- When ON, `EraseHandler.onEraseFinished` → `y0()` → `switchToPreviousProvider()` runs after **every finished
  erase gesture with the eraser tool**. It activates the tool that was active before the eraser, unless that
  was `NONE` or the eraser (`EraseHandler.java:29-32, 99-101`, `ScribbleHandler.java:1277-1283`) [code]. Only
  `EraseHandler` does this. Erasing with the stylus's eraser end while the pen tool is active is not affected
  [code].
- **Info icon (ⓘ):** a tap opens `AutoDeselectEraserHintPopup` [code, `ErasePopup.java:81-97, 140-162`]:
  - Text **"When enabled, auto-switch to last tool after erasing."** (`strings.xml:222`), black,
    `erase_popup_title_text_size` (20 sp), padding 21.2 dp, `maxWidth` 483 dp, wrap content, in the same
    white/3 px border/rounded background [code, `RES/layout/layout_auto_deselect_eraser_hint_popup.xml`,
    `AutoDeselectEraserHintPopup.java` constructor].
  - Position: the left edge aligns with the icon's left edge (when the hint is wider than the icon), and the
    bottom edge sits `max(list_divider_height, 4 px)` above the icon's top [code, `show()`].
  - It closes on an outside tap (`setOutsideTouchable(true)`). The eraser menu stays open under it [code +
    inferred].

---

## 8. "Erase Current Layer" and "Erase All Layers"

Tap → `EraseByTypeAction(CURRENT_LAYER_ERASER | ALL_LAYER_ERASER)`. `applyEraserConfig` returns early for layer
types. `t()` runs `EraseLayerAction` in place of `InvalidateScreenAction` [code, `ErasePopup.java:182-218`,
`EraseByTypeAction.java:68-77, 103-105`].

1. **Confirmation dialog**: `OnyxCustomDialog.getConfirmDialog` with the alert icon `ic_dialog_alert_vector`,
   full-screen immersive, buttons **OK** / **Cancel** (`strings.xml:2252, 381`) [code,
   `EraseLayerAction.java:52-69`, `OnyxCustomDialog.java:820-830`]. Message:
   - current layer: **"Are you sure you want to erase strokes on the current layer?"** (`strings.xml:958`)
   - all layers: **"Are you sure you want to erase strokes on all layers?"** (`strings.xml:956`)
2. **Cancel** completes the chain with no item, so nothing is erased, the tool does not change and the eraser
   menu stays open [code for `subject.onComplete()` at `:81`; the rest inferred from Rx semantics].
3. **OK**:
   - Current layer: `EraseCurrentLayerRequest` removes every shape on the **current layer of the current (centre)
     page** (`EraseCurrentLayerAction.java:39-47`, `EraseCurrentLayerRequest.java:47-67`) [code].
   - All layers: `ClearAllFreeShapesRequest` removes all free shapes on **every layer of the current page**
     (`EraseAllLayerShapesAction.java:59-67`, `ClearAllFreeShapesRequest.java:57-76`) [code]. Other pages are
     untouched in both cases.
   - Both honour "Do Not Erase Shapes", both skip attachments (`getFilterAttachShape`), and both clear the page's
     HWR data [code].
   - **Undoable:** the removal goes to `CommandManager.removeShapes`, which pushes an `UndoRedoCommand`
     (`CommandManager.java:146-150, 208-215`). It is called from the request and again from the action
     (`EraseCurrentLayerRequest.java:60`, `EraseCurrentLayerAction.java:60`, `EraseAllLayerShapesAction.java:55`)
     [code]. Whether one undo restores everything in one step was not traced **[unknown]**.
   - Afterwards the app **leaves the eraser**: `activeProviderByShapeType(currentShapeType)` activates the
     drawing tool for the current pen shape. Then `CloseErasePopupEvent` closes the menu
     (`EraseLayerAction.java:146-149`) [code].
   - Pen control around it: the requests are plain `BaseNoteRequest`s. They pause render and input first, and
     the pen resumes 500 ms after the re-render (03 §3.5) [code].

---

## 9. Dismissal, and what the panel does

### 9.1 Ways the menu closes

| trigger                                         | code                                                                                                                     |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| tap outside the popup                           | `setOutsideTouchable(true)`, `ErasePopup.java:427` [code]                                                                |
| tap on a type cell                              | `CloseErasePopupEvent`, `EraseByTypeAction.java:115` [code]                                                              |
| OK in a layer-erase dialog                      | `CloseErasePopupEvent`, `EraseLayerAction.java:148` [code]                                                               |
| tap on the toolbar eraser button while it shows | `ToolbarMenuModel.J()` → `closeMenuPopup` (`:238-244`); interplay with the outside-touch path **[unknown]** (§3.1)       |
| sub-menu close (for example back)               | `ToolbarMenuModel.closeSubMenu()` → `closeShownPopupWindow()` (`:666-668`); the key binding was not traced **[unknown]** |

Moving the slider, tapping `<`/`>`, flipping a switch, opening the ⓘ hint and cancelling a layer dialog do
**not** close the menu [code].

The popup is not focusable (no `setFocusable` in `BasePopWindow` / `BaseNotePopWindow`, by grep) [code]. So an
outside touch probably also reaches the window underneath [inferred from Android `PopupWindow` behaviour];
whether a pen tap on the page both closes the menu and draws is **[unknown]**.

### 9.2 Pen and panel on show and on dismiss

- **Show:** `BasePopWindow.onShow` → `windowChangeEvent(true)` → `PopupChangeAction(true)`. That runs
  `InvalidateScreenAction` (no regal, no render resume) and posts `PopupWindowChangeEvent(true)`.
  `PenEventHandler.onPopupWindowChange` then increments the popup count and runs
  `PauseRawPenAction(pauseArgs)` [code, `BasePopWindow.java:203`, `BaseEditPopWindow.java:76`,
  `PopupChangeAction.java:37, 53-58`,
  `PenEventHandler.java:436-442, 147-149`]. While the popup count is above zero, every resume is refused (03 §5,
  gate `A()`). Recorded: `ENABLE_COLOR_CU [1]`, `setRawDrawingEnabled(false)` twice, pen state 3, and
  `0x400000`-flagged partial updates for the popup [log, `A7:159-180`].
- **Dismiss:** `BaseEditPopWindow.dismiss()` posts `EnableColorCUEvent(false)` (`BaseEditPopWindow.java:43-45`);
  `BasePopWindow.dismiss()` →
  `windowChangeEvent(false)` → `PopupChangeAction(false)` → `InvalidateScreenAction.setCheckUseRegalMode(true)`
  → `enableRegalModeOnce()`. The next `InkRenderManager.invalidate` therefore sets the view's update mode to
  **REGAL_PLUS** (the default of `viewUpdateConfigModel`), calls `EpdController.enableRegal()`, invalidates and
  resets the mode. Then `PopupWindowChangeEvent(false)` decrements the count and resumes the pen after
  `POPUP_RESUME_PEN_TIME_MS` = **500 ms** (colour) [code, `BasePopWindow.java:156-160`,
  `PopupChangeAction.java:37, 46-50`, `InvalidateScreenAction.java:89-93`, `InkRenderManager.java:30, 63-65,
  79-95, 113-122`, `PenEventHandler.java:443-445`, `PenEvent.java:20`].
- Recorded on dismiss: `ENABLE_REGAL [1]`, `setViewDefaultUpdateMode(REGAL_PLUS)`,
  `SET_APPLICATION_DISPLAY_SCHEME [3]`, one **partial** `waveform[16] update_mode[0]` repaint, and the pen
  resume (`SET_STROKE_STYLE` … pen state 2) about 520 ms later [log, `A7:223-252`; same pattern at `419-447`,
  `781-822`]. It is not a full-screen GC16 (11 §Menus).

---

## 10. Where the selections are stored

| what                    | in memory                                                                                                                 | persisted                                                                                                                                                                                                                                                                                                          |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| current type            | `NoteDrawingArgs.c` (`setCurrentEraseType`) and `NotePenManager` note-owned `EraserArgs.type`                             | MMKV `ERASER_TYPE_KEY` when the document is saved (`SaveDocumentAction.java:357`); restored for every document on open and create (`OpenDocumentAction.java:117`, `CreateDocumentAction.java:156`). So it is **global, last saved wins** [code]                                                                    |
| per-type widths (px)    | `NotePenInfo.eraseWidthMap` in `NoteDrawingArgs`; `EraserArgs.eraseWidthMap` in `NotePenManager` (`NOTE_OWNED_ERASER_ID`) | with the notebook: `NoteModel.notePenInfo` JSON (`NoteDocument.java:300-305` writes, `:829` reads); globally: MMKV `NOTE_PEN_INFO_KEY` and `ERASER_WIDTH_KEY` (current type only) on save (`SaveDocumentAction.java:359-360`) [code]. Which one a new notebook starts from beyond `ERASER_WIDTH_KEY` **[unknown]** |
| note-owned eraser args  | rebuilt from `NoteDrawingArgs` by `ScribbleMenuModel.M()` (`ScribbleMenuModel.java:130-135`)                              | —                                                                                                                                                                                                                                                                                                                  |
| Do Not Erase Shapes     | read from MMKV at each use                                                                                                | MMKV `ERASE_GRAPHICS_SWITCH_KEY`, written on tap [code]                                                                                                                                                                                                                                                            |
| Auto-deselect Eraser    | read from MMKV at each erase end                                                                                          | MMKV `AUTO_DESELECT_ERASER_KEY`, written on tap [code]                                                                                                                                                                                                                                                             |
| floating-toolbox eraser | per-toolbox `EraserArgs` (`NotePenManager.updateFloatMenuEraserArgs`)                                                     | float menu config, saved on `EraseWidthChangedEvent` (`FragmentFloatMenu.java:372-375`) [code]                                                                                                                                                                                                                     |

---

## 11. What Tangleaf needs to reproduce

Each item gives the exact value and whether it is from code, the screenshot or inference. The sizes are the
dp values of this device. In the Tangleaf WebView, 1 CSS px is presumably 1 dp, because the WebView's
devicePixelRatio 1.875 was measured on 2026-09-06 (992 CSS px portrait viewport, recorded in the project's
device notes) and matches the density found in §2.2 [inferred].

### 11.1 Toolbar eraser button

1. Icon = the icon of the current type (§4.8). The button shows a selected state while the eraser tool is active.
2. Tap while another tool is active: activate the eraser with the last type and diameter. **No menu.**
3. Tap while the eraser is active: open the menu under the button. Tap again while it is open: close it.
4. No long press.

### 11.2 Menu container

- Width 483, height wraps. White, 3 px black border, radius 19 px (device px, about 10 CSS px). Anchored under
  the toolbar at the eraser button and kept inside the screen.
- Horizontal content inset 21.2. Top padding 28.6. Dividers are 2 device px black: the first one full width,
  the others inset 10 on both sides.
- Opening it pauses the pen session. Closing it produces one partial repaint, and the pen session resumes
  500 ms after that (§9.2).

### 11.3 Controls, in order

| #   | control                 | exact content / values                                                                                                                                                                                                                                                | behaviour                                                                                                                                                                                                                                                                         |
| --- | ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | title                   | `Pixel Eraser 5.00mm` / `Stroke Eraser 0.50mm` / `Lasso Eraser`; `"<name> " + d.toFixed(2) + "mm"` (no space before `mm`); 20 sp, left                                                                                                                                | follows the current type and diameter live                                                                                                                                                                                                                                        |
| 2   | type row                | three equal cells between 92.2 insets, height 91.1, top margin 13.8: **Pixel** (eraser on baseline), **Lasso** (eraser in dashed circle), **Stroke** (eraser with squiggle); icons 33.9; 11 black dot under the selected one, 11.7 from the cell bottom               | tap = select that type, store it, **close the menu**; the new type brings back its own diameter; the pen session is reconfigured about 500 ms later                                                                                                                               |
| 3   | "Eraser Diameter" block | label `Eraser Diameter` 20 sp; right side `<` [value] `>` with 36 outlined-square chevron buttons and value `5.00mm` 20 sp; below, a full-width wedge slider: black track 1 → 27 thick with round ends, thumb a 34 white disc with a black ring; **hidden for Lasso** | range Pixel 1.00–8.00, Stroke 0.50–8.00 mm; step 0.05 mm for `<`, `>` and each slider position; linear; `<`/`>` clamp at the ends; slider applies live while dragging; defaults Pixel 5.00, Stroke 0.50; each type keeps its own value; the menu stays open                       |
| 4   | "Do Not Erase Shapes"   | label 20 sp + ON/OFF switch 53.3 × 26.7 (OFF: outlined rounded box, black square left, "OFF"); default OFF                                                                                                                                                            | only the switch toggles; global setting; ON = no eraser (any type, layer buttons, scratch-out) removes geometric shapes (§7.1); menu stays open                                                                                                                                   |
| 5   | "Auto-deselect Eraser"  | label 20 sp + ⓘ icon 26 (10 gap) + switch; default OFF; hint text `When enabled, auto-switch to last tool after erasing.` in a bordered popup above the ⓘ, max width 483, padding 21.2, 20 sp                                                                         | ON = after each eraser-tool erase gesture, return to the previous tool (not for the stylus eraser end with the pen tool); ⓘ opens the hint, an outside tap closes it                                                                                                              |
| 6   | layer buttons           | two equal outlined boxes side by side, height 56.2, gap 17, white, 2 px border, radius 14 px; centred icon 33.9 + 14.8 gap + text 17 sp: `Erase Current Layer` (eraser on one sheet), `Erase All Layers` (eraser on stacked sheets)                                   | tap → confirm dialog `Are you sure you want to erase strokes on the current layer?` / `…on all layers?` with **OK / Cancel**; OK erases the current page only (current layer / all layers), is undoable, switches to the drawing tool and closes the menu; Cancel changes nothing |

### 11.4 Units and the pen session

- Store the diameter per type as an integer in hundredths of a mm: Pixel 100…800, Stroke 50…800, step 5.
  Convert only when sending it: `devicePx = mm × 300 / 25.4` (11.811024 px/mm, `densityDpi` 300 [inferred]).
  In CSS px that is `mm × 6.2992` at DPR 1.875 [inferred].
- SF soft-eraser width = `devicePx` (the diameter). Stock sends `setStrokeParameters(8, [devicePx, 0.5, 0.1])` for
  Pixel/Stroke. For Lasso it sends style 5 with `(5, [5.0])` and still `(8, [0.5 mm → 5.905512, 0.5, 0.1])`
  [code + log].
- Hit-test diameter = the same `devicePx` in **screen** pixels, divided by the zoom for page units [code].
- The default type for a fresh install is **Stroke Eraser** [code].
- The floating-toolbox variant (`FloatErasePopup`) has the same layout. It keeps the eraser per toolbox and does
  not write the notebook's `NoteDrawingArgs` (`FloatErasePopup.java:19-45`). It is not needed for the main
  toolbar [code].

---

## 12. Open points for the device

1. A tap on the toolbar eraser button while the menu is open: does it close, or close and reopen (§3.1)?
2. Does an outside pen or finger tap on the page both close the menu and reach the page (§9.1)?
3. Is the triangle under the toolbar eraser button the selected-tool marker or the popup anchor (§3.2)?
4. Does one undo restore a whole layer erase (§8)?
5. Does the Notes app really run at `densityDpi` 300 (`adb shell dumpsys activity` / `wm density`)? The code
   depends on it and §6.4 infers it from three values.
