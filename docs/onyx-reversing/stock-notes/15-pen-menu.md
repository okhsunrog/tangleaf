# 15 — Stock Notes pen slots and pen menu: a spec for reproduction

This spec covers the pen slots in the top toolbar of the stock Onyx Notes app (`com.onyx.android.note`,
versionCode 45326) on the BOOX Note Air 4C, and the pen popup that opens from them. It describes what a slot
holds, how slots are selected, added and deleted, and where they are stored. For the popup it gives the layout,
the strings, the icons, every control, how the width is converted, and when each value reaches the Pen SDK and
SurfaceFlinger (SF). It was written so that Tangleaf's handwriting UI can reproduce both, and it follows the
structure of [12](12-eraser-menu.md). The method is static analysis of the decompiled APK. It also uses two
screenshots and two call-tracer recordings from 2026-09-25 (see [11](11-observations.md)). This pass did not
touch the tablet.

Claim tags, as in the other documents: **[code]** read in source or resources at the cited line;
**[inferred]** reasoned from what was read, with the reasoning given; **[unknown]** not settled statically.
**[shot]** means measured on a screenshot. **[log]** means read in a recorded trace.

| prefix   | location                                                                                                |
| -------- | ------------------------------------------------------------------------------------------------------- |
| `NA/`    | `~/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/` (jadx; `note/` app, `sdk/` SDK)        |
| `RES/`   | `~/tmp_zfs/boox-notes-inspect/decoded/resources/res/`                                                   |
| `OBS/`   | `~/tmp_zfs/pen-observation/` (recorded traces and screenshots, 2026-09-25)                              |
| `SHOT-P` | `OBS/shots/stock-top-swipe.png` (1860×2480, pen popup open from the 4th slot, "Pen 0.70mm", red)        |
| `SHOT-E` | `OBS/shots/stock-eraser-menu.png` (1860×2480, same notebook, eraser tool active, eraser menu open)      |
| `A1`     | `OBS/a1-stock.txt` (condensed trace of action A1, writing with slot 4 active)                           |
| `R4`     | `OBS/run-4.log` (raw call-tracer log of the same session; lines 149038-149908 hold three slot switches) |

The second decompilation (`~/tmp_zfs/reversed_onyx_notes_app/src`) matches on the lines spot-checked:
`PenPopup.java:106, 329, 356, 359, 362, 498, 534-535`, `StrokeColor.java:70-71`, `NoteMMKVHelper.QUICK_PEN_KEY`,
`PenMenuProvider.MAX_QUICK_PEN_COUNT` [code].

---

## 1. Key facts

- **A slot is a "quick pen"**: one `PenArgs` with an id, a pen type, a width in **device px**, an ARGB colour, a
  texture (`attrs`), a pressure sensitivity and a smoothing level. The toolbar shows **at most 5** slots, then
  a "+" item while there are fewer than 5 [code, §3].
- **First tap vs second tap on a slot.** A tap on a slot that is not selected selects it: the pen tool becomes
  active with that slot's type, width, colour and pressure. No menu opens. A tap on the selected slot opens the
  pen popup under it. A tap while that popup is showing closes it. While the eraser or lasso is active no slot
  counts as selected, so the first tap returns to the pen and the second opens the menu. There is no long press
  [code, `FunMenuHandler.java:72-90`, §3.5].
- **Selected marker:** a small black upward triangle (26×13 px) under the active slot, on the toolbar's bottom
  border. It marks the active tool, not the popup [code + shot, §3.4].
- **Width label:** `%.2f` of the width in mm, rounded to 0.05 mm, with no unit: `0.60`, `0.85`. It is 18 device
  px (9.6 dp) text above the nib [code + shot, §3.3].
- **Add / delete / reorder:** "+" copies the active slot into a new slot, selects it and opens the popup. The
  popup's trash icon deletes the active slot after a confirmation and closes the menu; the last slot cannot be
  deleted. There is **no reordering** [code, §3.6].
- **Fresh install:** one black "Pen" slot of 0.50 mm (5.905512 px) plus "+". The five slots in the screenshots
  are user-made [code + inferred, §3.7].
- **Storage:** global MMKV `QUICK_PEN_KEY` (JSON list plus `selectedId`), and a copy inside each notebook's
  `notePenInfo`. On open, a notebook's own list replaces the global one [code, §3.8].
- **The popup** is `PenPopup` (`NA/note/note/menu/popup/PenPopup.java`), layout `RES/layout/layout_pen.xml`, 533 dp
  wide. It is placed under the tapped slot and pushed left so that its right edge stays 40 px from the screen
  edge [code + shot, §4].
- **Rows, top to bottom** [code + shot, §5]: title `Pen 0.70mm` with a light-bulb tips icon and a trash icon; five
  pen types; nib sub-types (Pen only) with a preview; texture (Pencil only); `Line Width` `<` `0.70mm` `>` with a
  wedge slider; `Pressure Sensitivity` `30%` with an 11-tick slider (Pen only); `Stroke Stabilization` (hidden
  on this device); `Colors` `Red(#FFFF6163)` with 16 swatches in two rows of 8.
- **Five pen types, in order:** **Pen** (type 5, SF style 1, pressure sensitive), **Brush Pen** (21, style 3),
  **Ballpoint Pen** (2, style 0), **Pencil** (22, style 4 or 6), **Marker** (15, style 2). The Pen has two more nibs:
  **Calligraphy** (60) and **Fountain** (61), both style 7 at +45° and −45° [code, §6].
- **Width:** mm on a **0.05 mm** grid. The range is **0.10–2.00 mm** for every type except the **Marker, 0.50–8.00 mm**.
  The slider is linear with one position per step (39 or 151 positions). **Each type remembers its own width**
  [code, §7].
- **mm → px:** `px = mm × densityDpi / 25.4`, the same function as the eraser, so 1 mm = 11.811024 px at 300 dpi.
  Unlike the eraser, the pen width is a **page** width: SF gets `min(px, mmToPx(max)) × pageScale`. In the traces
  `pageScale` = 1755/1860 = 0.9435484, so 0.70 mm → 8.267717 px → `SET_STROKE_WIDTH 7.80099` [code + log, §7.4].
- **Pressure Sensitivity** (Pen only): 0–100 % in 10 % steps, default **30 %**, 0 shows `OFF`. It goes to SF as
  `SET_STROKE_PARAMETERS(1, [p, smooth])` with p = 0.0…1.0 [code + log, §8].
- **Colours:** 16 fixed colours, the same on colour and mono panels. There is no custom colour. The label is
  `<name>(#AARRGGBB)` [code + shot, §9].
- **When values reach SF:** a type or colour tap sends `SET_STROKE_STYLE` / `SET_STROKE_COLOR` at once, and a width
  change sends an unscaled `SET_STROKE_WIDTH` at once. The pen stays paused while the popup is open. The complete,
  correct set (style, scaled width, colour, pressure and smoothing parameters) goes out at the resume **500 ms after
  the popup closes** [code + log, §10].
- **Closing the menu:** outside tap, a second tap on the slot, or a confirmed delete. Choosing a type, colour or
  width does **not** close it. On close, the popup base class produces one REGAL_PLUS partial repaint and resumes
  the pen 500 ms later, as for the eraser menu [code, §11].

---

## 2. Sources and the layout variant

### 2.1 Classes and resources

| role                | file                                                                                                                                                                                                    |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| popup               | `NA/note/note/menu/popup/PenPopup.java` (base `BaseDocPopWindow` → `BaseEditPopWindow` → `BaseNotePopWindow`, the same chain as `ErasePopup`); `PenTipsDialog.java`; `FloatPenPopup.java` (§12)         |
| view model, binding | `NA/sdk/note/ui/viewmodel/PenPopupViewModel.java`, `NA/sdk/note/ui/common/utils/NoteUIUtils.java:361-374`, `NA/sdk/note/ui/databinding/LayoutPenBindingImpl.java`                                       |
| layout              | `RES/layout/layout_pen.xml`; items `shape_style_item.xml`, `pen_sub_shape_style_item.xml`, `texture_menu_item.xml`, `stroke_color_item.xml`; `dialog_pen_tips.xml`                                      |
| widgets             | `NA/sdk/note/ui/widget/WaterDropSeekBar.java`, `TickSeekBar.java`                                                                                                                                       |
| adapters            | `NA/note/note/menu/adapter/{ShapeAdapter,PenShapeAdapter,PenSubShapeAdapter,ShapeTextureAdapter,StrokeColorAdapter,PenStrokeColorAdapter}.java`                                                         |
| data                | `NA/note/note/menu/data/{ShapeType,ShapeTexture,StrokeColor}.java`, `NA/sdk/data/note/PenArgs.java`, `NA/sdk/scribble/utils/NotePenInfoUtils.java`, `NA/sdk/utils/NumberUtils.java`                     |
| toolbar slots       | `NA/note/note/menu/{FunMenuHandler,FunMenuProvider,PenMenuProvider}.java`, `RES/layout/layout_wrap_width_icon_menu_item.xml`, `RES/raw/note_funbar_menu.xml`                                            |
| slot state          | `NA/note/note/pen/QuickPenManager.java`, `NA/sdk/notecore/editor/pen/NotePenManager.java`, `NA/sdk/note/ui/common/utils/{NoteUtils,NoteMMKVHelper}.java`                                                |
| actions and SF      | `NA/note/note/action/menu/{ShapeChangeAction,PenWidthChangeAction,StrokeColorChangeAction}.java`, `NA/note/note/request/pen/ResumeRawDrawingRequest.java`, `NA/sdk/scribble/utils/NoteRenderUtils.java` |
| defaults            | `NA/sdk/note/ui/config/DeviceConfig.java:88-97, 205-238`, `NA/sdk/scribble/data/config/DeviceNoteConfigBean.java`, `NA/sdk/scribble/shape/ShapeFactory.java:288-299, 616-630`                           |

### 2.2 Which layout and dimension bucket applies

- `layout_pen.xml` exists only in `layout`, `layout-land`, `layout-sw430dp-land` and `layout-sw490dp-land`
  [code, directory listing]. In portrait the Note Air 4C (smallest width 992 dp, density 1.875; see 12 §2.2)
  therefore uses the **base `layout/layout_pen.xml`** with `LayoutPenBindingImpl`.
- Dimensions resolve to the largest `values-swNNNdp` ≤ 992 that defines each one. No `values-en*` or
  `values-port` file overrides any pen-popup dimension, string or bool [code, resolved by applying Android's
  qualifier precedence to every `values*` directory]. The bucket is given next to each value below.
- The popup scale `getFixScale()` is 1.0 (`BaseNotePopWindow.java:80-86`). It is never set for this popup, and
  SHOT-P fits 1.0 [code + shot].
- Device config: there is no `RES/raw/config_note_noteair4c.json`, so `DeviceNoteConfigBean` keeps its defaults.
  Two of them shape the popup: `supportActivePen` = false (`getH()`, `:85-88`), so the tips bulb is **shown**, and
  `supportConfigSmoothLevel` = false (`getL()`, `:95-98`), so the stabilization row is **hidden**. Both match
  SHOT-P. Twelve other models ship a `config_note_*.json` that sets one of them [code + shot].

---

## 3. The pen slots in the toolbar

### 3.1 What a slot holds

`PenArgs` fields, as stored in JSON: `id` (string; a UUID for added slots, `"1"` for the default one), `type`
(the `ShapeFactory` constant, §6), `width` (device px, float), `color` (ARGB int), `attrs` (`{texture}`, Pencil
only), `pressureSensitivityV2` (0…1 or null), `smoothLevel` (0…1 or null). The list is a `QuickPenList`
`{quickPens: [...], selectedId}` [code, `NoteMMKVHelper.java:964`, `PenPopup.java:657-666`, `NotePenManager.java:45-54`].

### 3.2 The slot view

- The top bar is menu bar `FUN`, a horizontal RecyclerView that does not scroll (`ScribbleFragment.java:480`,
  `DisableScrollLinearManager`). Items, left to right: every quick pen (`FunMenuProvider.L()`, `:88-96`), then
  `PEN` ("+"), `ERASER`, `UNDO`, `REDO`, `PAGE_PREV`, `PAGE_TITLE`, `PAGE_NEXT` (`RES/raw/note_funbar_menu.xml`),
  with separator lines before `ERASER` and `PAGE_PREV` [code + shot].
- One `WrapWidthIconMenuItem` per quick pen with `width ≥ 0` (`PenMenuProvider.java:44-56, 58-70`). Menu id
  `"PEN" + id`, tag `"PEN"`. The pen-type name is set as the item title but this layout has no title view, so it
  is never shown [code].

| part                          | value                                                                                         | source                                                                       |
| ----------------------------- | --------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| content box                   | **43 × 56 dp**                                                                                | `fun_menu_pen_content_width` (sw935dp), `_height` (sw990dp) [code]           |
| margins                       | 5 dp outer (`fun_menu_horizontal_margin`, sw829dp) + 3 dp inner (`menu_item_margin`, sw936dp) | [code]                                                                       |
| pitch                         | 5+3+43+3+5 = **59 dp** = 110.6 px; measured 111 px                                            | [code + shot]                                                                |
| icon                          | **36 dp**, centred                                                                            | `func_pen_menu_icon_size` → `menu_icon_size` [code]                          |
| colour bar (part of the icon) | measured **50 × 26 px** (26.7 × 13.9 dp) at the bottom of the icon                            | [shot]                                                                       |
| width label                   | **18 px** (9.6 dp) black, `includeFontPadding=false`, top-centre of the 43×56 box             | `pen_item_line_width_text_size` (sw990dp); `FunMenuProvider.java:205` [code] |

### 3.3 Icon, colour bar and label

- **Icon per type** (`ShapeType.java:14-20`; Pencil via `NoteMenuUtils.java:111-113`) [code + shot]:
  Pen `ic_note_pen_fountain`, Brush Pen `ic_note_pen_soft` (the flame), Ballpoint Pen `ic_note_pen_hard`, Pencil
  `ic_note_charcoal_pen` (texture 1) or `ic_note_texture_charcoal_pen` (texture 2), Marker `ic_note_marker_pen`,
  Calligraphy `ic_note_pen_calligraphy_latin`, Fountain `ic_note_pen_calligraphy_asia`. Unknown types fall back
  to the Brush Pen icon. The shapes are described in §5.10.
- **Colour bar.** Every pen icon is a 48-unit vector that is black except for one white (`#ffffff`) rounded
  rectangle at the bottom (e.g. `RES/drawable/ic_note_pen_fountain.xml:7-9`). The toolbar tints the whole drawable
  with `PorterDuffColorFilter(penColor, MULTIPLY)` (`KuiDataBindingUtil.java:246-249`, colour from
  `PenMenuProvider.java:62`). The white bar takes the pen colour and the black outline stays black. A colour of
  −1 (white) means "no tint", so a white pen shows an empty bar [code + shot].
- **Label** = `NumberUtils.formatFloat(pxToMM(width))` = `String.format("%.2f", roundFloat(mm))`, no unit
  (`PenMenuProvider.java:36-38, 63`). `roundFloat` rounds to the nearest 0.05 mm (§7.2). It uses the default locale,
  so a comma-decimal locale would show `0,60` [inferred from the `String.format` overload]. SHOT-P: `0.60`,
  `0.85`, `0.65`, `0.70`, `0.70` [shot].

### 3.4 The selected marker

- The item's `iv_bottom_indicator` shows `ic_tips_label_bottom`, a black upward triangle of **26 × 13 px**
  (`tips_label_width/height`, sw935dp), at bottom-centre of the item, on the toolbar's bottom line. It is visible
  only when the item is selected (`LayoutWrapWidthIconMenuItemBindingImpl.java:424-441, 507`; alignment TOP set
  for every top-bar item, `FunMenuProvider.java:271`) [code].
- A slot is selected iff it is the active quick pen **and** the pen tool is active
  (`PenMenuProvider.java:61`: `isPenSelected(id) && inPenProvider()`) [code].
- It is therefore the active-tool marker, not a popup anchor. `PenPopup.canSelectMenuItem()` returns false
  (`PenPopup.java:528-531`), so opening the popup does not change it [code]. SHOT-P shows it under slot 4 (the
  active pen), SHOT-E under the eraser with no slot marked [shot]. This settles open point 3 of
  [12 §12](12-eraser-menu.md).

### 3.5 Tap logic

```java
// FunMenuHandler.d(), FunMenuHandler.java:72-90 (slot tap)
boolean wasShowing = toolMenuModel.isMenuPopupShowing(item.getMenuId());   // this slot's popup open?
toolMenuModel.ensureInScribbleMode();          // closes every popup; quits text, lasso, fill, shape modes
PenArgs pen = notePenManager.handlePenChange(id);   // selectedId = id (NotePenManager.java:128-137)
if (item.getSelected().get()) {                // this slot is the active pen AND the pen tool is active
    showPenPopup(event, wasShowing);           // ToolbarMenuModel.java:1480-1488: if (wasShowing) return; else new PenPopup(...)
    return;
}
updateQuickPenManagerSelectedPen(pen.getId()); // QuickPenManager.selectPen → MMKV
selectPenShape(pen, false);                    // ShapeChangeAction(type).setPenAttrs(attrs) (:259-270)
```

- **Slot not selected → tap:** the slot becomes active (type, width, colour, texture, pressure, smoothing). No menu
  opens [code].
- **Slot selected → tap:** the popup opens. **Tap again while it shows:** `ensureInScribbleMode()` closes it and
  `showPenPopup` returns without reopening, so the tap closes the menu [code]. The popup is outside-touchable
  (`PenPopup.java:362`), so the tap on the slot may dismiss it as an outside touch before the click arrives; in
  that case `wasShowing` is false and the menu would reopen. Which one happens is **[unknown]**, as for the eraser
  (12 §3.1).
- **Another tool active (eraser, lasso, shapes):** no slot is selected, so the first tap on any slot switches to the
  pen with that slot, and the second tap opens the menu [code].
- **No long press:** `MenuItem.enableLongClick` defaults to false and the top bar never enables it;
  `BaseDocMenuHandler.onMenuLongClick` is empty (`:139-141`) [code].
- **What the switch does** (`ShapeChangeAction.java:36-111`): the active handler's `prepareShapeChangeObservable`,
  `activeFreeProviderByShapeType(type)` (`:80`), `InvalidateScreenAction.setResumeRawDrawing(false)` (`:38`), then
  on the drawing args `setCurrentShapeType`, `setPenAttrs`, `NoteManager.setStrokeStyle(getStrokeStyle(type, attrs))`
  and `setStrokeColor(drawingArgs.strokeColor)` (`:106-109`), then `resumePenDelay(100)` (`:69`). Afterwards
  `FunMenuHandler.j()` (`:99-107`) posts `CustomPenUpdateEvent` (toolbar rebuild), `QuickPenColorUpdateEvent` and
  `PenPopupUpdateShapeEvent` [code].
- **Recorded slot switch** (slot 1 → slot 2, `R4`) [log]:

  | R4 line         | time        | event                                                                                                   |
  | --------------- | ----------- | ------------------------------------------------------------------------------------------------------- |
  | 149138 / 149141 | 02.052/.109 | finger down / up on the slot                                                                            |
  | 149150-149161   | 02.118-.119 | pen paused: `setRawDrawingEnabled(false)`, `ENABLE_POST(1)`, pen state 3                                |
  | 149158-149173   | 02.119-.122 | main thread: `SET_STROKE_STYLE [3]` ×2 (new type), `SET_STROKE_COLOR [black]` ×2 (still the old colour) |
  | 149390, 149431  | 02.396/.422 | two `[UI] waveform[2] update_mode[0]` panel updates (the toolbar redraw [inferred])                     |
  | 149440-149473   | 02.426-.433 | resume: `SET_STROKE_WIDTH 9.472631` (0.85 mm × 0.9435), `SET_STROKE_COLOR [-40605]`, pen state 2        |

  The pen is back about 315 ms after the finger lifted. The first `SET_STROKE_COLOR` of each resume still carries
  the previous slot's colour, because `applyDrawingArgs` runs before `applyPenArgs` (02 §3) [log + code].

### 3.6 Adding, deleting, reordering

- **"+"** is the XML item `PEN`: icon `ic_note_quick_pen_add` (a tray with a nib and a circled +), title
  `@string/pen` = "Brushes" (`strings.xml:2405`, not shown), same 43×56 dp box, right after the last slot. It is
  hidden once there are 5 slots (`FunMenuProvider.java:88-96`: `if (size >= 5) m.add(PEN)`, `m` being the hidden
  set), which is why SHOT-P shows no "+" [code + shot]. `PenMenuProvider.MAX_QUICK_PEN_COUNT = 5` is declared but
  unused; the string `note_quick_pen_add_max_tips` ("Exceeded the largest quantity.") is never used [code].
- **Tap on "+"** (`FunMenuHandler.a()`, `:53-60`): `NotePenManager.addQuickPen()` copies the active slot (type,
  width, attrs, colour, pressure, smoothing) under a new UUID, appends it and selects it
  (`NotePenManager.java:56-67, 73-82`); the same goes into `QuickPenManager` and MMKV (`:134-138`); then
  `showPenPopup(event, false)` opens the popup on the new slot, `selectPenShape`, `CustomPenUpdateEvent` and an
  `InvalidateScreenAction` [code].
- **Delete** is the trash icon in the popup (§5.2). Visible iff the scene `supportMultipleBrushes()`: true in normal
  notebooks (`BaseNoteScene.java:879-881`), false in the meeting scene; otherwise INVISIBLE, keeping its space
  (`PenPopup.java:441`) [code]. A tap (`PenPopup.java:179-187, 228-262`):
  - no selected pen: toast **"Please select a custom brush"** (`strings.xml:2096`);
  - more than one slot: confirm dialog **"Are you sure you want to delete the current brush?"** (`:2097`), **OK / Cancel**;
  - one slot: dialog **"Please keep at leat 1 brush."** (`:2098`, typo in the source), **OK** only, nothing deleted.
  - OK with more than one slot runs `j()` (`:368-372`): `ensureInScribbleMode()` first, which closes all popups
    through `closeAllSubMenu()` (`ToolbarMenuModel.java:674-676`, `ScribbleMenuModel.java:258-261, 110-112`), so
    **the pen menu closes**. Then the active slot is removed from `NotePenManager` and `QuickPenManager`, index 0
    becomes the active slot, its type, width, colour and attrs go into the drawing args, MMKV is written
    (`QuickPenManager.java:158-175`), and the toolbar is rebuilt [code].
- **Reorder:** none. There is no `ItemTouchHelper` or list swap in `note/` or `sdk/note`, and no long click on
  these items [code, grep].
- **Limits:** at least 1 slot, at most 5 [code].

### 3.7 Defaults on a fresh install

- With `QUICK_PEN_KEY` missing, `NoteUtils.getDefaultQuickPens()` (`:150-153`) uses the model config's
  `quickPenList`. `RES/raw/note_noteair4c.json` has none, so `buildDefaultQuickPens()` (`:77-91`) returns one pen,
  id `"1"`, type `defaultShape()` = **5 (Pen)**, width `defaultStrokeWidth()`, colour **0xFF000000**, plus a
  placeholder `{id "-1", type -1, width -1}` that is filtered out, with `selectedId "1"` [code].
- **Default width**: `DeviceConfig` sets `defaultStrokeWidthWithMM = 0.5` (`DeviceConfig.java:91`) and at config load
  `PenConstant.DEFAULT_STROKE_WIDTH = mmToPx(0.5)` = **5.905512 px, label `0.50`** (`:205-209`, `ConfigManager.java:29-32`)
  [code]. That this runs before the default is read is **[inferred]** from the startup order; if it did not,
  the default would be the class constant 7.2 px, label `0.60` (`PenConstant.java:19`) [unknown which].
- A new slot made by `NotePenManager.a()` (`:45-54`) has type 5, the default width, black, pressure 0.3 and
  smoothing 0.6 [code].
- So a fresh install shows **one black Pen slot plus "+"**. The five slots of SHOT-P (Pen 0.60 black, Brush Pen 0.85
  red, Pen 0.65 navy, Pen 0.70 red, Pen 0.70 green) are user data [inferred]. Their px widths at 300 dpi are 7.087,
  10.039, 7.677, 8.268 and 8.268; R4 records 0.60, 0.85 and 0.65 exactly (§7.4) [log].
- Default width per pen type, used when a type is chosen in the popup and has no stored width yet
  (`ShapeFactory.getShapeDefaultStrokeWidth`, `:288-299`; `DeviceConfig.java:91-95`) [code]:

  | type            | default                |
  | --------------- | ---------------------- |
  | Pen 5, 60, 61   | 0.50 mm = 5.905512 px  |
  | Brush Pen 21    | 1.00 mm = 11.811024 px |
  | Ballpoint Pen 2 | 0.50 mm = 5.905512 px  |
  | Pencil 22       | 0.30 mm = 3.543307 px  |
  | Marker 15       | 5.00 mm = 59.055118 px |

### 3.8 Storage

- **Global:** MMKV instance `"NOTE"`, key **`QUICK_PEN_KEY`** (`NoteMMKVHelper.java:257, 964`), value the
  `QuickPenList` JSON of §3.1. `QuickPenManager` writes it on select, add, edit (every popup change,
  `updateSaveQuickData`), delete, and when it strips invalid entries (`QuickPenManager.java:31-36, 42-55, 125-128,
141-175`) [code].
- **Per notebook:** `NoteModel.notePenInfo` → `NotePenInfo.quickPenList`, stored as JSON in the notebook's
  `notePenInfo` field (`NoteInfoPBDocument.java:48, 95`). On save the notebook gets `QuickPenManager`'s current list
  (`GlobalNoteBundle.java:130-135`, `SaveDocumentAction.java:350-351`) [code]. The same `NotePenInfo` also holds
  the per-type width map `penWithMap` and the pressure/smoothing of the drawing args (§12).
- **Which wins:** on open, a non-empty notebook list **replaces** the global list in memory
  (`OpenDocumentAction.java:116-124` → `GlobalNoteBundle.updateDocDrawingArgs:364-377` →
  `QuickPenManager.loadFromDrawArgs:101-109`) and is cloned into `NotePenManager` (`ScribbleMenuModel.java:137-150`).
  A new notebook starts from the global list (`CreateDocumentAction.java:103`) [code]. Since every change is
  written back to MMKV, new notebooks inherit the pens of the notebook changed last [inferred].
- `NOTE_PEN_INFO_KEY` receives the whole `NotePenInfo` on each save (`SaveDocumentAction.java:355-361`,
  `NoteMMKVHelper.java:1879`). Its reader `getNotePenInfo()` (`:1619-1623`) has no caller, so it is written but never
  read [code, grep].
- **Toolboxes** (`ToolBox`, `ToolBoxesPopup`) are presets of the floating toolbar, with their own pen lists in MMKV
  `FLOAT_MENU_BEAN_KEY`. They are used only in the floating menu styles and are not part of the top-bar slots
  [code, `NotePenManager.java:117-125, 209-225`].

---

## 4. Opening the menu and where it appears

- `showPenPopup` → `new PenPopup(context, scribbleMenuModel, P())` (`ToolbarMenuModel.java:1480-1488`), with the
  slot's `MenuItem` set on it, shown by `d1(event, popup, true)` → `showPopupMenu` with a `NotePopupContext` whose
  menu id is `PEN` (`:484-499`) [code].
- `PEN` is in the scene's `getAtBottomModeMenuIds()` (`BaseNoteScene.java:266-268`), so `setPositionByMenuId` sets
  `toParentPosition = 3` (`NotePopupUtils.java:272-281`). Then [code, `NotePopupUtils.java:23-41, 66-79, 128-174, 185-191`]:
  - x: `a()` with the default `alignParent = 3` gives offset 0, i.e. the popup's left edge at the slot's left edge.
    `h()` then shifts it left so that `left + width ≤ windowWidth − 40` (`note_popup_margin_bottom` = 40 px,
    `values`), and right so that `left ≥ 40`.
  - y: `c(..., true)` places it directly under the slot (`showAsDropDown` offset 0), or above it if it would not
    fit below.
- On this device: 1860 − 999 − 40 = **821 px** left edge, whatever slot is tapped. SHOT-P measures the popup at
  x 821-1819, y 160-1359 (999 × 1200 px) [code + shot]. The eraser popup of SHOT-E ends at the same right edge,
  so the same rule explains 12 §3.3 [shot].
- `ensureInScribbleMode()` runs before the popup is built (quits text input, selection, fill colour and universal
  shape, and closes other sub-menus) [code, `ToolbarMenuModel.java:1533-1541`].
- The popup subscribes to the document event bus while shown (`BaseEditPopWindow.java:76-82`), which is how the
  toolbar and the popup stay in sync (`CustomPenUpdateEvent`, `PenPopupUpdateShapeEvent`, `QuickPenColorUpdateEvent`)
  [code].

---

## 5. Layout, top to bottom (portrait, this device)

Sizes are dp unless marked px (device px, 1 dp = 1.875 px). "Inset" means the 21 dp horizontal margin
`layout_pen_popup_padding_horizontal` (sw990dp) that the binding applies to every block except the first divider.
Measured values are from SHOT-P.

### 5.1 Container

- `ScrollView` → vertical `LinearLayout` (`layout_pen.xml:2-13`). Width `note_pen_popup_width` = **533 dp**
  (sw935dp) = 999 px; height wraps (`PenPopup.java:359-360`). Measured 999 × 1200 px [shot].
- Background: the common popup drawable, white fill, **3 px** black border, radius **19 px**, as for the eraser menu
  (`BaseNotePopWindow.java:145-149`, `NotePopupUtils.java:181-183`; 12 §4.1) [code + shot].
- Top padding `layout_pen_popup_padding_vertical` = **21 dp** (sw990dp) (binding `:249`). There is no bottom
  padding; the colour grid's own padding closes the popup [code].
- `should_limit_pen_popup_height` = false (sw745dp), so there is no height cap (`NotePopupUtils.java:240-256`)
  [code]. The root is a plain `ScrollView`, so the `AutoFastModeNestedScrollView` hook in `PenPopup.v()` does
  nothing here (`:459-464`) [code].
- Dividers are `1dp` black `View`s = 2 px (`layout_pen.xml:59-62` and others) [code + shot].

### 5.2 Header row

A `ConstraintLayout`, inset 21 dp (binding `:256`) [code, `layout_pen.xml:14-58`]:

- `pen_type_title`, left: `r() + " " + "<w>mm"`, e.g. **`Pen 0.70mm`**. `r()` is the type's name, or "Preview"
  for a type not in the pen list (`PenPopup.java:327-337, 410-413`). 20 sp black (`pen_popup_title_text_size` →
  `text_size_level_2`, sw990dp) [code + shot].
- `pen_tips` (light bulb, `ic_note_pen_tips`), right after the title: **25 dp** square (`page_tips_icon_size`),
  start margin **21 dp** (`layout_pen_popup_tip_margin_title`) (binding `:282-286`). Visible iff
  `!supportActivePen` (§2.2). A tap opens `PenTipsDialog` (§5.11) [code + shot].
- `delete` (trash, `ic_note_delete`: lid bar, handle, bin with two vertical slots), right-aligned: **36 dp**
  square (`popup_note_stroke_width_button_size` → `menu_icon_size`) (binding `:242-243`), margins 4 dp top and bottom
  (`layout_pen_popup_delete_icon_margin`), `bg_button_click_solid` with 8 px bottom padding. Behaviour in §3.6
  [code + shot].
- Then the pen-type row (§5.3), and a **full-width** divider (not inset; measured x 824-1815) [code + shot].

### 5.3 Pen-type row

- `pen_shape_list`: a `GridLayoutManager` with span = number of types = **5**, one row, top margin **35 dp**
  (`layout_pen_popup_pens_margin_top`) (`PenPopup.java:423-429`, binding `:270`). No extra spacing:
  `note_pen_popup_pen_list_item_flush_to_edge` = false (sw623dp) [code].
- Cells are equal fifths of 533 − 2×21 dp; SHOT-P icon centres are 184 px apart, starting 131 px from the
  popup's left edge [shot].
- Item `shape_style_item.xml`: icon **36 dp**, minimum item width 58 dp, bottom padding 10 dp; under the icon the
  indicator `menu_item_indicator`, **11 dp** with 5 dp top margin: `ic_note_point` (a black dot, radius 6 of a
  14-unit viewport, ≈ 9.4 dp) when selected, transparent otherwise [code + shot].
- Selected = the item's type equals the active slot's type, or the active type is one of its sub-types; and the
  pen or shape tool is active (`ShapeAdapter.java:84-95`) [code].
- Order, names and icons: §6.1. What a tap does: §6.2.

### 5.4 Sub-type row (Pen only)

- `pen_subtype_layout`, inset 21 dp, visible iff the current type (mapped to its parent) has sub-types; only the
  Pen does (`PenPopup.java:501-526`) [code + shot].
- A horizontal row, vertical margin **30 px** (`pen_popup_sub_pen_content_margin_vertical`, sw623dp) (binding `:269`):
  - `pen_subtype_preview`: **160 × 42 dp**, start margin 7 dp, end margin 0 [code]. It shows the stroke preview
    (§6.3).
  - a vertical divider 1 dp wide, **42 dp** tall (measured at x 1173) [code + shot].
  - `pen_subtype_list`: start margin **25 dp** (sw745dp), end margin **17 dp**, a grid of span 3: **Pen, Calligraphy,
    Fountain** (`PenPopup.java:512-519`). Item `pen_sub_shape_style_item.xml`: 36 dp icon and the 11 dp dot. The
    selected nib's icon is tinted with the pen colour (MULTIPLY), so its bar shows red in SHOT-P
    (`PenSubShapeAdapter.java:76-78, 107-112`) [code + shot].
- Then an inset divider [code + shot].

### 5.5 Texture row (Pencil only)

- `pen_texture_layout`, inset 21 dp, visible iff the type has textures; only the Pencil (22) does
  (`ShapeTexture.java:11-12`, `PenPopup.java:150-158`) [code].
- Title **"Texture"** (`note_texture`, `strings.xml:2199`), 20 sp, top margin 14 dp; list top margin 30 px; a
  horizontal list of two items (`texture_menu_item.xml`: 36 dp icon, dot indicator, 28 dp end padding): texture 1
  `charcoal_texture_v1` and texture 2 `charcoal_texture_v2`, 49×49 grainy PNGs with no labels [code]. Then an inset
  divider.

### 5.6 Line Width block

A `ConstraintLayout`, inset 21 dp, vertical margin **14 dp** (`pen_popup_label_margin_vertical`, set in
`PenPopup.java:355`, binding `:251-252`) [code, `layout_pen.xml:117-181`]:

- `line_width_title` **"Line Width"** (`line_width`, `strings.xml:1526`), 20 sp, left [code + shot].
- Right-aligned: `<` (`decrease_stroke_width`), the value, `>` (`increase_stroke_width`). The steppers are
  `note_btn_click_style` frames holding `ic_note_prev` / `ic_note_next` at **36 dp** (`note_pen_popup_stepper_icon_size`),
  the same outlined-square chevron buttons as the eraser menu. `<` and the value have end margin **21.2 dp**
  (`erase_popup_item_padding_horizontal`) [code + shot].
- Value `line_width_value`: **`0.70mm`** (no space), 20 sp, minimum width **39 dp** (`pen_stroke_width_text_min_width`),
  centred (binding `:245-248`) [code + shot].
- `seekBar`: `WaterDropSeekBar`, full width, top margin **30 px** (binding `:259`), the same wedge slider as the eraser
  menu: black track from 1 dp to **27 dp** thick, round caps, thumb a **34 dp** white disc with a black ring
  (`PenPopup.java:478-480`; 12 §4.4) [code + shot].
- Then an inset divider (binding `:260`) [code + shot].

### 5.7 Pressure Sensitivity block (Pen only)

- `pressure_sensitivity_layout`, inset 21 dp, visible iff `supportPressureSensitivity(type)`, i.e. type 5
  (`PenPopup.java:450-457`) [code + shot].
- Title **"Pressure Sensitivity"** (`strings.xml:2407`), 20 sp, left; value right-aligned, 20 sp, min width 39 dp:
  **`30%`** (`NumberFormat.getPercentInstance()`), or **`OFF`** (`pen_pressure_sensitivity_off`, `:2408`) at 0
  (`PenPopup.java:281-297`) [code + shot].
- `pressure_seekBar`: `TickSeekBar`, top margin 30 px (binding `:262`) [code, `TickSeekBar.java:67-148, 182-190`;
  dimensions from `layout_pen.xml:217-228`]:
  - track **3 dp** black line between the thumb centres at the two ends (`..._track_height`, sw623dp);
  - progress part (left of the thumb) **6 dp** thick black (`..._progress_height`, sw623dp);
  - ticks: **max + 1 = 11**, each **2 dp × 12 dp** black, at every step, except the one under the thumb
    (`..._tick_width` sw490dp, `..._tick_height` sw623dp);
  - thumb: the same **34 dp** white disc with black ring.
  - SHOT-P: thumb at 0.299 of the track (4th tick) for 30 % [shot].
- Its own inset divider, then a second inset divider (binding `:263`). The two are adjacent, so the line under
  this block is 4 px thick (measured y 1043-1046) [code + shot].

### 5.8 Stroke Stabilization block (hidden on this device)

`smooth_level_layout`: same structure as §5.7, title **"Stroke Stabilization"** (`pen_smooth_level`,
`strings.xml:2409`), value `NN%` or `OFF`. Visible iff `supportConfigSmoothLevel && supportSmoothLevel(type)`
(types 5, 60, 61) (`PenPopup.java:466-473`). `supportConfigSmoothLevel` is false here (§2.2), so the row never
shows [code + shot].

### 5.9 Colors block

- Inset 21 dp; title row top margin 14 dp (binding `:267`) [code, `layout_pen.xml:290-316`]:
  **"Colors"** (`note_color`, `strings.xml:1884`) left, 20 sp; right `textView_color_string`, 20 sp:
  **`Red(#FFFF6163)`** (§9.3) [code + shot].
- `color_list`: top margin **30 px** (binding `:239`), a grid of span `max(8, 5)` = **8**, two rows for 16 colours
  (`PenPopup.java:415-422`), plus `HorizontalSpaceItemDecoration(8, 40 px)` (`stroke_color_item_horizontal_space`,
  sw745dp) [code].
- Item `stroke_color_item.xml`: swatch icon **36 dp** (the drawn disc is 44/48 of it: measured 63 px), the 11 dp dot
  indicator with 5 dp top margin, bottom padding 5 dp [code + shot].
- Measured: swatch pitch **120 px (64 dp)**, first swatch 41 px from the popup's left edge, rows 106 px apart,
  42 px from the last row to the popup's bottom edge [shot].

### 5.10 Icons (vector drawables, 48-unit viewport, black with a white body)

| type (value)      | drawable                        | shape                                                                              |
| ----------------- | ------------------------------- | ---------------------------------------------------------------------------------- |
| Pen (5)           | `ic_note_pen_fountain`          | fountain-pen nib (two curved halves, centre slit, breather hole) on a rounded body |
| Brush Pen (21)    | `ic_note_pen_soft`              | flame-shaped brush tip on a trapezoid ferrule, on the body                         |
| Ballpoint Pen (2) | `ic_note_pen_hard`              | a cone outline with a round ball at the apex, on the body                          |
| Pencil (22)       | `ic_note_charcoal_pen`          | pencil "house" (pentagon) outline with a zigzag wood line and a centre line        |
| Pencil, texture 2 | `ic_note_texture_charcoal_pen`  | the same pencil with straight bands in place of the zigzag                         |
| Marker (15)       | `ic_note_marker_pen`            | trapezoid shoulder with a slanted chisel block on top, on the body                 |
| Calligraphy (60)  | `ic_note_pen_calligraphy_latin` | a nib like the Pen's with a longer slit and a larger hole                          |
| Fountain (61)     | `ic_note_pen_calligraphy_asia`  | a flat, wide chisel top with a V notch                                             |
| tips              | `ic_note_pen_tips`              | a light bulb with rays (36×35 viewport)                                            |
| delete            | `ic_note_delete`                | trash can: lid bar, handle, bin with two vertical slots                            |

In the popup the type icons are untinted (all bodies white); the toolbar and the selected sub-type tint the body
with the pen colour [code + shot].

### 5.11 The tips dialog

`PenTipsDialog` (`PenPopup.java:264-266`, `dialog_pen_tips.xml`): centred, **697 × 478 dp** (sw935dp), not
cancelable, closed only by its button [code]. Content: **"Usage Tips"** (`strings.xml:2100`, 25 sp bold);
**"Writing Unavailability in Certain Areas"** (`:2092`) with the paragraph `:2093` about magnetic fields; **"Strokes
Appearing without Contact"** (`:2094`) with `:2095` about a loose nib; button **"OK"** (`understood`, `:3381`)
[code]. It is device help text and not needed for Tangleaf.

---

## 6. Pen types

### 6.1 The list

`ShapeType.getFilteredPenShape()` (`ShapeType.java:64-75`) takes the enum values in order, removes the graphical
shapes and the sub-type values 60 and 61. It has no device, colour/mono or tilt filter [code + shot].

| #   | enum (`ShapeType.java`)       | value = `ShapeFactory` constant           | English name (`strings.xml`)    | SF style (`getStrokeStyle`) | width range  | pressure | smoothing | sub-types |
| --- | ----------------------------- | ----------------------------------------- | ------------------------------- | --------------------------- | ------------ | -------- | --------- | --------- |
| 1   | `FOUNTAIN_PEN` (:14)          | 5 `SHAPE_BRUSH_SCRIBBLE`                  | **Pen** (`fountain_pen`, :1207) | 1 FOUNTAIN                  | 0.10–2.00 mm | yes, 0.3 | yes, 0.6  | 60, 61    |
| 2   | `SOFT_PEN` (:15)              | 21 `SHAPE_NEO_BRUSH`                      | **Brush Pen** (:365)            | 3 NEO_BRUSH                 | 0.10–2.00 mm | no       | no        | —         |
| 3   | `HARD_PEN` (:16)              | 2 `SHAPE_PENCIL_SCRIBBLE`                 | **Ballpoint Pen** (:286)        | 0 PENCIL                    | 0.10–2.00 mm | no       | no        | —         |
| 4   | `CHARCOAL_PEN` (:17)          | 22 `SHAPE_CHARCOAL_SCRIBBLE`              | **Pencil** (:2413)              | 4 CHARCOAL; 6 if texture 2  | 0.10–2.00 mm | no       | no        | —         |
| 5   | `MARKER_PEN` (:18)            | 15 `SHAPE_MARKER_SCRIBBLE`                | **Marker** (:1595)              | 2 MARKER                    | 0.50–8.00 mm | no       | no        | —         |
| –   | `CALLIGRAPHY_PEN_LATIN` (:19) | 60 `SHAPE_LATIN_CALLIGRAPHY_PEN_SCRIBBLE` | **Calligraphy** (:377)          | 7 SQUARE_PEN, angle +45°    | 0.10–2.00 mm | no       | yes, 0.6  | —         |
| –   | `CALLIGRAPHY_PEN_ASIA` (:20)  | 61 `SHAPE_ASIA_CALLIGRAPHY_PEN_SCRIBBLE`  | **Fountain** (:376)             | 7 SQUARE_PEN, angle −45°    | 0.10–2.00 mm | no       | yes, 0.6  | —         |

- The UI names do not follow the SDK constants: SDK "brush" (5) is the UI "Pen", SDK "pencil" (2) is "Ballpoint
  Pen", SDK "charcoal" (22) is "Pencil", SDK "asia calligraphy" (61) is "Fountain" [code].
- Styles are from `ShapeFactory.getStrokeStyle` (`:616-630`); pressure and smoothing support from
  `NotePenInfoUtils.java:107-113`; parameters in §10.3. Types 3 and 4 (also style 0) cannot be chosen in the UI
  [code].

### 6.2 What a tap on a type does

1. `PenShapeAdapter.onShape` (`:52-65`) does nothing if the type is already active or is the parent of the
   active sub-type (`shouldKeepShapeType`, `:67-72`) [code].
2. Otherwise it sets the slot's type, **clears its attrs** (the texture goes back to 1), and runs
   `ShapeChangeAction(type).setPenAttrs(null)` (§3.5): new provider, new drawing-args type, immediate
   `SET_STROKE_STYLE` and `SET_STROKE_COLOR` [code].
3. It then posts `PenPopupUpdateShapeEvent(type)` without pen args (`:30-38`). `PenPopup.onPenPopupUpdateShapeEvent`
   (`:598-614`) rebuilds the width list for the new type, **loads the width stored for that type**
   (`NoteDrawingArgs.getStrokeWidth(type)`, the `penWithMap`, default §3.7), applies it through `changeWidth`
   (§7.3), and rebuilds the texture, sub-type, pressure and smoothing rows [code].
4. So a type switch keeps the slot's colour and pressure but takes the new type's own width. The title updates
   (e.g. `Brush Pen 1.00mm`). **The popup stays open** [code].
5. A sub-type tap (`PenSubShapeAdapter.java:130-139`) does the same without clearing attrs. A texture tap
   (`ShapeTextureAdapter.java:122-135`) sets type 22 and `attrs.texture`, runs `ShapeChangeAction(22)` with it
   (style 4 or 6), and saves the slot [code].

### 6.3 The preview (Pen, Calligraphy, Fountain)

`RenderPenPreviewAction` (`:53-136`) renders into the 160 × 42 dp preview, only for types 5, 60 and 61
(`PenPopup.java:268-270`): two full sine periods over the first 2/3 of the width (margin w/18, amplitude h/4),
starting and ending at the low point, then a straight line to w − w/18. It uses the slot's type, width, colour,
attrs, pressure and smoothing [code + shot: a red wave in SHOT-P]. It is redrawn on every width, colour and type
change (`PenPopup.java:171-175, 617-625`) [code].

---

## 7. Line width

### 7.1 Range, step, slider

`NotePenInfoUtils` [code, `NotePenInfoUtils.java:47-71, 97-101`]:

| types                 | min     | max     | step    | list                | slider positions  |
| --------------------- | ------- | ------- | ------- | ------------------- | ----------------- |
| all except the Marker | 0.10 mm | 2.00 mm | 0.05 mm | 0.10, 0.15, …, 2.00 | 39 (`max` = 38)   |
| Marker (15)           | 0.50 mm | 8.00 mm | 0.05 mm | 0.50, 0.55, …, 8.00 | 151 (`max` = 150) |

- The list is built from min to max in steps of 0.05, each value through `roundFloat`; there are no finer steps at
  small widths. The slider's `max` is `list.size − 1`, its progress the list index (`PenPopup.java:475-499, 331`),
  so the slider is **linear in mm** [code]. SHOT-P: 0.70 mm is index 12/38 = 0.316; the thumb measures 0.315 [shot].
- `<` sets index − 1, clamped at 0; `>` sets index + 1, clamped at `max`. Plain click listeners, no auto-repeat
  (`PenPopup.java:197-209, 483-494`) [code].
- The slider applies the value **continuously while dragging** (`PenPopup.java:77-96`: every user progress change
  calls `c0`) [code].
- The list is rebuilt when the type changes (`onPenPopupUpdateShapeEvent` → `x()`), so switching to the Marker
  switches the range [code].

### 7.2 Display and rounding

```
roundFloat(mm)  = roundToNearestMultipleOfFive(Math.round(mm * 100)) / 100   // NumberUtils.java:67-78: remainder 0-2 down, 3-4 up
formatFloat(mm) = String.format("%.2f", roundFloat(mm))                      // NumberUtils.java:27-29
value text      = formatFloat(mm) + "mm"                                     // "0.70mm"   (PenPopup.java:327-330; DateTimeUtil.STRING_FORMAT_MM)
title           = typeName + " " + value text                                // "Pen 0.70mm" (PenPopup.java:332)
toolbar label   = formatFloat(pxToMM(slot.width))                            // "0.70"     (PenMenuProvider.java:36-38)
```

On show, the popup reads the slot's px width and converts it with `pxToMM` (`PenPopup.java:398-400, 628-634`)
[code].

### 7.3 mm ↔ px and storage

```
mmToPx(mm) = mm * densityDpi / 25.4     // NA/sdk/extension/DimensKt.java:11-13 (the same function as the eraser)
pxToMM(px) = px / densityDpi * 25.4     // DimensKt.java:15-17
changeWidth(mm):                        // PenPopup.java:533-541
    activeQuickPen.width = mmToPx(roundFloat(mm))
    PenWidthChangeAction(mmToPx(mm), type): drawingArgs.setStrokeWidth → penWithMap[currentType] = px   (PenWidthChangeAction.java:28-53)
                                            PenWidthChangeRequest: penWithMap[type] = px; NoteManager.setStrokeWidth(px)  (PenWidthChangeRequest.java:17-20)
    then save the slot (MMKV), rebuild the toolbar, redraw the preview (PenPopup.java:171-175)
```

- At `densityDpi` 300 (the value inferred in 12 §6.4): 1 mm = 11.811024 px; 0.10 mm = 1.181102 px; 0.70 mm =
  8.267717 px; 2.00 mm = 23.622047 px; 8.00 mm = 94.488189 px [inferred arithmetic].
- **Compared with the eraser** (12 §6): the same `mmToPx`. The eraser stores hundredths of a mm per type and converts
  when used; the pen stores **px** in the slot and in the per-type map. Both display on a 0.05 mm grid. The eraser
  width goes to SF unscaled, as a screen-pixel diameter; the pen width is scaled by the page zoom (§7.4) [code].
- Widths stored in px depend on the density: a DPI change (for example EinkWise per-app DPI) would show stored
  widths as different mm values [inferred, as for the eraser].
- On document load `clampStokeWidthPX` clamps the per-type map and the slots to `[mmToPx(min), mmToPx(max)]`
  (`NotePenInfoUtils.java:27-29`; `NotePenInfosKt.java:18-44`, called from `NoteDocument.java:832`) [code].

### 7.4 Width on the wire: page units

- At every resume `applyPenArgs` sends `SET_STROKE_WIDTH = getRenderStrokeWidth(px, type, renderContext)` =
  `min(px × s, mmToPx(maxMm(type)) × s)` (`NoteManager.java:215`, `ShapeUtils.java:1115-1119, 1153-1155`), where s is
  `RenderMeasure.getStrokeWidthDisplayScale()`, the page-to-screen scale [code].
- In the recorded session the page view is 1755 × 2340 px (`A1:186`, `setLimitRect([[0,0][1755,2340]])`), so
  s = 1755/1860 = 2340/2480 = 0.9435484 [log + inferred]. Recorded [log]:

  | trace        | slot           | px (mm × 300/25.4) | `SET_STROKE_WIDTH` | style |
  | ------------ | -------------- | ------------------ | ------------------ | ----- |
  | `R4:149038`  | 0.60 black Pen | 7.086614           | 6.686563           | 1     |
  | `R4:149440`  | 0.85 red Brush | 10.039370          | 9.472631           | 3     |
  | `R4:149908`  | 0.65 navy Pen  | 7.677165           | 7.243777           | 1     |
  | `A1:201-202` | 0.70 red Pen   | 8.267717           | 7.80099            | 1     |

  All four equal `px × 0.9435484` to six digits. The pen width is therefore a width on the **page** (a 0.70 mm
  line is 0.70 mm on a page 1860 px = 157.5 mm wide) and appears 0.66 mm wide on the glass at this fit [inferred].

- While the popup is open, a width change also sends `SET_STROKE_WIDTH(px)` at once, unscaled and unclamped, through
  `PenWidthChangeRequest` → `NoteManager.setStrokeWidth` (`NoteManager.java:914-923`). The pen is paused, and the
  resume after closing overwrites it with the scaled value [code + inferred]. This was not recorded **[unknown]**.

---

## 8. Pressure sensitivity and stabilization

### 8.1 Pressure Sensitivity (Pen only)

- Supported only for type 5; default **0.3** (`NotePenInfoUtils.java:39-41, 107-109`). A slot with no value uses the
  default (`PenPopup.java:374-380`) [code].
- Slider `max` = 10 (`:73-75`); `progress = clamp((int)(value × 10), 0, 10)`; `value = progress / 10f`
  (`:15-17, 31-33, 77-87`). So the values are **0.0, 0.1, …, 1.0**, displayed as **`0%`… `100%`** in 10 % steps, and
  `OFF` in place of `0%` [code]. SHOT-P: `30%`, thumb at the 4th of 11 ticks [shot].
- A change (`PenPopup.java:98-117, 543-547`) sets `slot.pressureSensitivityV2`, writes the drawing args
  (`NoteDrawingArgs.setPenPressureSensitivity`, into `NotePenInfo`), and saves the slot to MMKV. Nothing goes to SF
  while the popup is open [code].
- At the resume, `ResumeRawDrawingRequest.m()` (`:71-82`) calls `NoteRenderUtils.setPressureSensitivity`
  (`:89-102`): read SF's style-1 parameters, set `[0] = value`, write back if it changed. Then
  `setBrushSmoothLevel` (`:64-87`) always writes style 1 with `[1] = smooth`. Recorded:
  `SET_STROKE_PARAMETERS [1, 2, 0.3f, 0.6f]` (`A1:207-208`) [code + log].
- `PenConstant.DEFAULT_PRESSURE_SENSITIVITY = 0.375` exists but the popup does not use it [code].
- How SF turns `[0]` into a pressure curve is **[unknown]**.

### 8.2 Stroke Stabilization (hidden here)

- Supported for types 5, 60 and 61, default **0.6**, same 0–10 slider and mapping, `OFF` at 0
  (`NotePenInfoUtils.java:43-45, 89-95, 103-113`; `PenPopup.java:119-138, 304-320, 549-553`) [code].
- The row is hidden on this device (§5.8), but the default 0.6 is still sent: style 1 `[1]` for type 5, style 7
  `[3]` for types 60/61 (`NoteRenderUtils.java:80-87, 104-111`). Recorded as `0.6f` in every resume (`A1:208`)
  [code + log].

### 8.3 Tilt and calligraphy angle

- No tilt control exists. Style 4/6 `[0]` would be set to 1 only if `supportTilt`, which is false here
  (`NoteRenderUtils.java:128-137`) [code].
- The calligraphy nibs are fixed: style 7 `[1] = min(slot px width, 10)`, `[2] = +45` (60) or `−45` (61)
  (`NoteRenderUtils.java:113-126`). The `min(…, 10)` is in unscaled px, i.e. 0.847 mm at 300 dpi [code + inferred].

---

## 9. Colours

### 9.1 The palette

The grid shows `StrokeColor.LIST` (16 entries, `StrokeColor.java:38-71`; `StrokeColorAdapter.java:27, 52-54`) in
this order. Hex values are ARGB from `RES/values/colors.xml`; every one matches the pixel colour in SHOT-P
[code + shot].

| #   | constant (line)  | ARGB       | English name (`strings.xml`) | swatch drawable       | swatch look                |
| --- | ---------------- | ---------- | ---------------------------- | --------------------- | -------------------------- |
| 1   | BLACK (38)       | `FF000000` | Black (:340)                 | tinted ring           | black disc                 |
| 2   | DARK_GRAY (40)   | `FF404040` | Dark Gray (:611)             | `ic_note_dark_grey`   | dark grey disc             |
| 3   | MEDIUM_GRAY (42) | `FF808080` | Medium Gray (:1614)          | `ic_note_medium_grey` | grey disc                  |
| 4   | LIGHT_GRAY (44)  | `FFC0C0C0` | Light Gray (:1522)           | `ic_note_light_grey`  | light grey disc            |
| 5   | WHITE (46)       | `FFFFFFFF` | White (:3489)                | `ic_note_color_white` | white disc, letters **WT** |
| 6   | RED (48)         | `FFFF6163` | Red (:2606)                  | `ic_note_color_red`   | red disc, letters **RD**   |
| 7   | GREEN (50)       | `FF00B036` | Green (:1268)                | `ic_note_color_green` | green disc, letters **GN** |
| 8   | BLUE (52)        | `FF000084` | Navy Blue (:343)             | `ic_note_color_blue`  | navy disc, letters **BU**  |
| 9   | QAUA [sic] (54)  | `FF00F0FF` | Aqua Blue (:189)             | tinted ring           | cyan disc                  |
| 10  | VIOLET (56)      | `FFEE00FF` | Violet (:3452)               | tinted ring           | magenta disc               |
| 11  | ORANGE (58)      | `FFFFAA00` | Orange (:2308)               | tinted ring           | orange disc                |
| 12  | LEMON (60)       | `FFF0FF00` | Lemon (:1509)                | tinted ring           | yellow disc                |
| 13  | CHARTREUSE (62)  | `FF008000` | Chartreuse (:435)            | tinted ring           | dark green disc            |
| 14  | GRAPE (64)       | `FF9338BE` | Grape (:1263)                | tinted ring           | purple disc                |
| 15  | SKY_BLUE (66)    | `FF00AAFF` | Sky Blue (:2907)             | tinted ring           | light blue disc            |
| 16  | ORANGE_RED (68)  | `FFFF4400` | Orange Red (:2310)           | tinted ring           | orange-red disc            |

- `StrokeColor.DEFAULT` (the first 8) is used only for the span count (§5.9) [code].
- **Swatch drawing** (`DataBindingUtils.setStrokeColor`, `:179-189`): an entry with a drawable shows it as is, a
  disc of radius 20.5 in the colour inside a black ring from radius 19 to 22 (48-unit viewport). An entry without
  one shows `ic_note_color_black` (white disc, black ring) tinted with the colour (MULTIPLY) [code].
- **WT / RD / GN / BU** are black letter paths baked into those four drawables (e.g. `ic_note_color_red.xml:13-18`),
  present on every device. They label the four original colours, which a mono panel shows as greys [code +
  inferred for the purpose].
- **Selected:** the swatch whose value equals the slot colour shows the 11 dp dot under it
  (`PenStrokeColorAdapter.java:25-27`, `StrokeColorAdapter.java:76-78, 88`) [code + shot].

### 9.2 What a tap does

`PenStrokeColorAdapter.onColor` (`:36-40`) sets the slot colour; `StrokeColorAdapter.onColor` (`:93-100`) runs
`StrokeColorChangeAction`: `NoteManager.setStrokeColor(getRenderColor(c))` → immediate `SET_STROKE_COLOR`
(`StrokeColorChangeRequest.java:15-17`, `NoteManager.java:896-903`), then `NoteDrawingArgs.setStrokeColor`
(`StrokeColorChangeAction.java:47-50`). Then `QuickPenColorUpdateEvent` → `PenPopup.java:617-625`: save the slot to
MMKV, rebuild the toolbar (new bar colour), update the label, re-tint the sub-type icons, redraw the preview.
**The popup stays open** [code].

### 9.3 Label, custom colours, colour vs mono

- Label: `format_color_string` = `%1s(%2s)` (`strings.xml:1203`) with the name and `"#" +
Integer.toHexString(argb)` upper-cased (`PenPopup.java:272-274`, `ColorUtils.java:211-213`): **`Red(#FFFF6163)`**,
  no space, 8 hex digits including alpha [code + shot].
- **No custom colours.** `StrokeColor.addStrokeColor` / `setList` are never called and `StrokeColorPopup` is never
  constructed [code, grep]. A slot colour outside the list would show `(#FF……)` with an empty name [inferred].
- **Colour vs mono:** the palette and the menu are identical; no menu code checks the panel [code]. Only the ink
  differs: `RenderContext.getRenderColor` (`:357-363`) passes the colour unchanged with colour config 1 and turns
  any non-grey colour into black with config 0. The config follows `isColorStrokesShowOriginal()`, which defaults
  to `isColorDevice()` (`OnyxSystemConfig.java:100-105`; 02 §6.3). On the Note Air 4C the traces show the raw red
  `-40605` = `0xFFFF6163` sent to SF (`A1:195-196`), so config 1 is in effect [code + log].

---

## 10. How the values reach the Pen SDK and SurfaceFlinger

### 10.1 The popup pauses the pen

`PenPopup` has the same base classes as `ErasePopup`. On show, `PopupChangeAction(true)` posts
`PopupWindowChangeEvent(true)`; `PenEventHandler` increments the popup count and pauses the raw pen. While the count
is above zero every resume is refused (`PenEventHandler.java:105-107, 147-149, 155-165, 436-446`; 12 §9.2) [code].
The stroke setters themselves (`TouchHelper.setStroke*` → `SFTouchRender` and `AppTouchRender`, each sent twice)
are not gated, so some values reach SF at once [code, 02 §3].

### 10.2 Per control

| control                  | stored at once                                               | sent to SF at once (popup open)                                                                        | sent at the resume after the popup closes           |
| ------------------------ | ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------ | --------------------------------------------------- |
| pen type / nib / texture | slot type (+ attrs); drawing-args type and attrs; slot saved | `SET_STROKE_STYLE(style)`, `SET_STROKE_COLOR(drawing-args colour)`, then the new type's width as below | everything in §10.4                                 |
| width (slider, `<`, `>`) | slot px; `penWithMap[type]`; slot saved                      | `SET_STROKE_WIDTH(px)`, unscaled, unclamped                                                            | `SET_STROKE_WIDTH(min(px, max) × s)`                |
| colour                   | slot colour; drawing-args colour; slot saved                 | `SET_STROKE_COLOR(renderColor)`                                                                        | `SET_STROKE_COLOR` ×2 (drawing args, then the slot) |
| pressure                 | slot `pressureSensitivityV2`; `NotePenInfo`; slot saved      | nothing                                                                                                | `SET_STROKE_PARAMETERS(1, [p, smooth])`             |
| stabilization (hidden)   | slot `smoothLevel`; `NotePenInfo`; slot saved                | nothing                                                                                                | style 1 `[1]` or style 7 `[3]`                      |

Sources: `ShapeChangeAction.java:106-109`, `PenWidthChangeRequest.java:17-20`, `StrokeColorChangeRequest.java:15-17`,
`PenPopup.java:533-553`, `NoteManager.java:205-219`, `ResumeRawDrawingRequest.java:52-55, 71-113` [code].

### 10.3 Pen type → SF style and parameters

This is 02 §6.2, re-checked [code: `ShapeFactory.java:616-630`, `ResumeRawDrawingRequest.java:71-82`,
`NoteRenderUtils.java:64-137`]:

| UI type           | value | `SET_STROKE_STYLE` | `SET_STROKE_PARAMETERS` on resume                                    | SF width cap |
| ----------------- | ----- | ------------------ | -------------------------------------------------------------------- | ------------ |
| Pen               | 5     | 1 FOUNTAIN         | style 1: `[0]` = pressure (0…1), `[1]` = smoothing (0.6)             | 2 mm × s     |
| Brush Pen         | 21    | 3 NEO_BRUSH        | —                                                                    | 2 mm × s     |
| Ballpoint Pen     | 2     | 0 PENCIL           | —                                                                    | 2 mm × s     |
| Pencil, texture 1 | 22    | 4 CHARCOAL         | style 4 `[0]` = 1 only with `supportTilt` (false here): nothing sent | 2 mm × s     |
| Pencil, texture 2 | 22    | 6 CHARCOAL_V2      | same                                                                 | 2 mm × s     |
| Marker            | 15    | 2 MARKER           | —                                                                    | 8 mm × s     |
| Calligraphy       | 60    | 7 SQUARE_PEN       | style 7: `[1]` = min(px, 10), `[2]` = +45, `[3]` = smoothing         | 2 mm × s     |
| Fountain          | 61    | 7 SQUARE_PEN       | style 7: `[1]` = min(px, 10), `[2]` = −45, `[3]` = smoothing         | 2 mm × s     |

Every resume also sends the eraser pair `(5, [5.0])` and `(8, [eraserPx, 0.5, 0.1])` (`ResumeRawDrawingRequest.java:52-55`;
12 §6.5).
Parameter writes are read-modify-write through `GET_STROKE_PARAMETERS`, and are skipped if SF returns too short an
array [code + log: `R4` shows `GET_STROKE_PARAMETERS [1]` before the write].

### 10.4 Wire trace of the resume with the Pen slot (A1:186-219)

`REGION_LIMIT` ×2 → `REGION_EXCLUDE` ×2 → `REGION_MODE [1]` → `SET_STROKE_COLOR [-40605]` ×2 → `SET_STROKE_STYLE [1]`
×2 → `SET_STROKE_WIDTH 7.80099` ×2 → `SET_STROKE_COLOR [-40605]` ×2 → `SET_STROKE_PARAMETERS [1, 2, 0.3, 0.6]` →
`[5, 1, 5.0]` → `[8, 3, 80.314964, 0.5, 0.1]` → `SET_ERASER_RAW_DRAWING_ENABLED [1, 8]` →
`SET_BRUSH_RAW_DRAWING_ENABLED [1]` → `SET_SCREEN_HANDWRITING_PEN_STATE [2, pid]` [log]. It matches the order in
02 §3 [code].

---

## 11. Dismissal, and what the panel does

### 11.1 Ways the menu closes

| trigger                                         | code                                                                                                                                     |
| ----------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| tap outside the popup                           | `setOutsideTouchable(true)`, `PenPopup.java:362` [code]                                                                                  |
| tap on the selected slot while it shows         | `FunMenuHandler.d()` → `ensureInScribbleMode()` closes it, `showPenPopup` returns (§3.5); interplay with the outside touch **[unknown]** |
| tap on another slot, the eraser or another tool | `ensureInScribbleMode()` / `closeAllSubMenu()` in the handlers [code]                                                                    |
| OK in the delete dialog                         | `PenPopup.j()` → `ensureInScribbleMode()` (§3.6) [code]                                                                                  |
| sub-menu close (for example back)               | `ToolbarMenuModel.closeSubMenu()` → `closeShownPopupWindow()` (`:665-667`) [code]                                                        |

Choosing a type, nib, texture, width or colour, moving a slider, opening the tips dialog, and cancelling the delete
dialog do **not** close it [code].

### 11.2 Pen and panel on show and on dismiss

- The popup uses the same `BaseEditPopWindow` / `BasePopWindow` show and dismiss path as the eraser menu, so 12 §9.2
  applies unchanged [code, `BaseEditPopWindow.java:43-51, 76-82, 94-96`; `BaseNotePopWindow.java:58-66, 124-132`]:
  - **Show:** the constructor calls `EpdController.enableColorCU()` (`BaseNotePopWindow.java:37`); on show,
    `PopupChangeAction(true)` (an `InvalidateScreenAction`), pen paused, popup count + 1.
  - **Dismiss:** `EnableColorCUEvent(false)`; `PopupChangeAction(false)` arms REGAL_PLUS for one invalidate, so the
    next repaint is one partial REGAL_PLUS update; then the popup count − 1 and the pen resumes after
    `POPUP_RESUME_PEN_TIME_MS` = **500 ms** on a colour panel (`PenEvent.java:20`), sending §10.4.
- The eraser-menu recording shows that pattern: one partial repaint and the pen resume about 520 ms after the
  dismiss (12 §9.2, `A7:223-252`) [log]. **No recording of the pen menu itself exists yet** [unknown].
- A slot switch without the menu is lighter: pause, immediate style and colour, toolbar redraw, resume 100 ms after
  the action finishes, about 315 ms after the tap (§3.5) [code + log].

---

## 12. Where the values are stored

| what                                 | in memory                                                  | persisted                                                                                                            |
| ------------------------------------ | ---------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| slot list, active slot               | `NotePenManager` (note-owned list), `QuickPenManager`      | MMKV `QUICK_PEN_KEY` on every change; notebook `notePenInfo.quickPenList` on save (§3.8) [code]                      |
| slot type, width (px), colour, attrs | the slot `PenArgs`                                         | as above [code]                                                                                                      |
| slot pressure, smoothing             | the slot `PenArgs`; also `NotePenInfo` of the drawing args | as above; and in the notebook's `NotePenInfo` [code]                                                                 |
| width per pen type                   | `NotePenInfo.penWithMap` (type → px)                       | with the notebook; seeded for a new notebook from the per-type defaults (`UIBundle.java:245-266`) [code]             |
| current type, attrs, colour          | `NoteDrawingArgs`                                          | with the notebook; restored from the selected slot on open (`UIBundle.java:327-333`) [code]                          |
| floating-toolbox pens                | per toolbox (`FloatPenPopup`, `NotePenManager` float map)  | MMKV `FLOAT_MENU_BEAN_KEY`; `FloatPenPopup` never writes `QUICK_PEN_KEY` (`FloatPenPopup.java:46-51, 96-119`) [code] |

---

## 13. Corrections to earlier documents

- 02 §6.2 gives the width clamp as `ShapeUtils.java:1113-1116`; it is at `:1115-1119` (wrapper) and `:1153-1155`
  (core). The 8 mm cap applies to the Marker only; every other type is capped at 2 mm [code].
- 02 §6.2's "min(width, 10)" for style 7 is in **unscaled slot px** [code].
- 12 §3.2 left open whether the triangle under a toolbar button marks the selection or the popup. It marks the
  selected tool (§3.4) [code + shot].
- 12 §3.3 left the popup placement rule open. It is the rule of §4: the left edge at the anchor's left edge,
  shifted to keep the right edge 40 px from the screen edge [code + shot].

---

## 14. What Tangleaf needs to reproduce

Sizes are the dp values of this device; in the Tangleaf WebView 1 CSS px is presumably 1 dp (devicePixelRatio 1.875;
12 §11) [inferred]. Device px are marked px.

### 14.1 Toolbar slots

1. Up to **5** slots, left of the eraser, one per pen, in list order; after them a **"+"** item while there are
   fewer than 5.
2. Slot: 43 × 56 box, pitch 59; the type's 36 icon centred; its white body tinted with the pen colour (a bar of
   about 27 × 14 at the bottom); the width label above, `mm.toFixed(2)` rounded to 0.05, no unit, 9.6 (18 px),
   black.
3. Selected (active pen with the pen tool active): a black upward triangle 26 × 13 px centred under the slot on the
   toolbar's bottom line. No other selected style.
4. Tap an unselected slot: activate it (type, width, colour, pressure). **No menu.** Tap the selected slot: open the
   menu under it. Tap again: close it. No long press.
5. "+": copy the active slot, append, select, open the menu on it.
6. No reordering. Minimum 1 slot.
7. Fresh install: one slot, Pen, 0.50 mm, black `#FF000000`, pressure 0.3.

### 14.2 Menu container

- Width 533, height wraps (640 in SHOT-P). White, 3 px black border, radius 19 px. Left edge at the slot's left
  edge, moved left to keep 40 px (about 21) from the right screen edge; top at the slot's bottom.
- Top padding 21; blocks inset 21 left and right; dividers 2 px black, the first one full width, the rest inset
  21; the divider under Pressure is doubled (4 px).
- Opening pauses the pen session; closing gives one partial repaint and resumes the pen 500 ms later with the
  slot's full settings (§10.4).

### 14.3 Controls, in order

| #   | control                    | exact content / values                                                                                                                                                                                | behaviour                                                                                                                                                                                           |
| --- | -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | title                      | `"<type name> " + mm.toFixed(2) + "mm"`, e.g. `Pen 0.70mm`, 20 sp, left                                                                                                                               | follows type and width live                                                                                                                                                                         |
| 2   | tips bulb                  | 25 icon, 21 after the title                                                                                                                                                                           | opens a help dialog; optional for Tangleaf                                                                                                                                                          |
| 3   | delete (trash)             | 36 icon, right                                                                                                                                                                                        | 1 slot: "Please keep at least 1 brush." (stock: "leat") OK; else "Are you sure you want to delete the current brush?" OK/Cancel; OK deletes the active slot, selects the first, **closes the menu** |
| 4   | pen types                  | 5 equal cells, top margin 35, icons 36, 11 dot under the selected one: **Pen, Brush Pen, Ballpoint Pen, Pencil, Marker**; then a full-width divider                                                   | tap = set the slot's type (clear texture), load that type's own width, keep colour and pressure; menu stays open                                                                                    |
| 5   | nibs (Pen only)            | preview 160 × 42 (a two-period wave in the pen colour and width), 1 × 42 divider, then 3 cells: **Pen, Calligraphy (+45°), Fountain (−45°)**; the selected nib's body in the pen colour, dot under it | tap = set the slot's type to 5, 60 or 61; menu stays open                                                                                                                                           |
| 6   | texture (Pencil only)      | "Texture", two grainy squares (1, 2), dot under the selected                                                                                                                                          | tap = texture; style 4 or 6                                                                                                                                                                         |
| 7   | Line Width                 | label; right `<` [value] `>`: 36 outlined-square chevrons, value `0.70mm` 20 sp min width 39; below, the wedge slider (1 → 27 black, 34 thumb)                                                        | range 0.10–2.00 mm (Marker 0.50–8.00), step 0.05, linear, 39 / 151 positions; buttons ±1 step, clamped; live while dragging; each type keeps its own width; menu stays open                         |
| 8   | Pressure Sensitivity (Pen) | label; value `30%` or `OFF`; ticked slider: 3 track, 6 progress left of the thumb, 11 ticks 2 × 12, 34 thumb                                                                                          | 0–100 % in 10 % steps, default 30 %; sent as style 1 `[0]` = p/100 at the next resume                                                                                                               |
| 9   | Stroke Stabilization       | hidden on this device                                                                                                                                                                                 | Tangleaf can omit it and send 0.6                                                                                                                                                                   |
| 10  | Colors                     | "Colors" left, `Name(#AARRGGBB)` right; 8 × 2 swatches (36 icon, 44/48 disc, pitch 64), 11 dot under the selected; the 16 colours of §9.1 in order; WT/RD/GN/BU letters on white, red, green, navy    | tap = slot colour; toolbar bar and label update; menu stays open; no custom colours                                                                                                                 |

### 14.4 Units and the pen session

- Keep the width per slot in hundredths of a mm on the 0.05 grid: 10…200 (Marker 50…800), step 5. Keep a separate
  last-used width per pen type for type switches (defaults §3.7).
- Convert only when sending: page px = `mm × 300 / 25.4`; SF width = `min(pagePx, maxMm × 300/25.4) × pageScale`,
  where `pageScale` is the page-to-screen scale of the current view (0.9435 at stock's fit) [code + log].
- Styles and parameters per type: §10.3. Pressure goes to style 1 `[0]`, smoothing 0.6 to style 1 `[1]` or style 7
  `[3]`; the calligraphy angle is ±45 and `[1]` = min(px, 10). Send them at every resume, after style, width and
  colour.
- Colours are sent as the raw ARGB int on this colour panel.
- Stock sends style, colour and width at the moment of a change even while paused; the values that matter are those
  of the resume. A reproduction may send everything once at resume.

---

## 15. Open points for the device

1. A tap on the selected slot while the menu is open: close, or close and reopen (§3.5)? Same question as 12 §12.1.
2. The pen-menu show and dismiss sequence (repaint mode, the 500 ms resume, the unscaled width sent while paused)
   has not been recorded; a trace of "open menu, change width and colour, close" would confirm §10-11.
3. The stored slot widths (MMKV `QUICK_PEN_KEY`) and whether a fresh install starts at 0.50 or 0.60 mm (§3.7).
4. What SF does with the pressure parameter: stroke width against pen pressure at 0 %, 30 % and 100 %.
5. Does an outside pen or finger tap on the page both close the menu and reach the page (as for the eraser, 12 §12.2)?
6. `densityDpi` 300 for the Notes app (12 §12.5); every mm value here depends on it.
