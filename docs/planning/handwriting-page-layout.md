# Handwriting page layout: full screen, one toolbar, the whole sheet visible

Written 2026-09-25 from static reading of `main` at `0f4dbee` and two screenshots taken on the tablet
(`~/tmp_zfs/pen-observation/shots/tangleaf-handwriting.png`, `stock-eraser-menu.png`). Nothing here
is implemented, and no device was touched. The stock numbers come from the decoded stock Notes app
(`~/tmp_zfs/boox-notes-inspect/decoded/`: `res/layout/fragment_scribble_normal_mode.xml`,
`res/values-sw990dp/dimens.xml`, `ScribbleMenuModel`, `DeviceUtils.adjustFullScreenStatus`,
`FitScreenRequest`).

Goal: on the Note Air 4C the handwriting page should use its chrome as sparingly as stock Notes does:
no system bars, one toolbar, and a page that is always fully on screen. Phones and desktop must keep
working.

## Hard constraints (read before changing anything)

1. **The pen path is verified on hardware and stays as it is.** Do not change the frame fence, the
   eraser gate, display-mode ownership, `InkStylusRouting`, the `configure_onyx_ink` /
   `commit_onyx_frame` protocol or any file under `kt/` except the two plugin additions in §12.2.
   A layout change only changes the sheet geometry that `onyx-ink.ts` already measures.
2. **Every geometry change must be a size change** of `[data-ink-viewport]` or of the canvas, because
   those are the only boxes `onyx-ink.ts:293-299` observes (plus window scroll/resize and
   visibilitychange). Nothing may move the sheet without also resizing one of them (§11).
3. **No persistent control may float over the sheet unless it is reported in `overlayRects`.**
   Transient popovers and menus are exempt because the `Popover`/`Menu` wrappers pause the pen while
   they are open (`src/app/ink-suppression.ts:98-114`), which is also what stock does
   (`BaseNotePopWindow.onShow` → `PopupChangeAction`).
4. **The ink format is frozen**: the sheet is 1000×1400 units (5:7). It is fitted to the screen and
   never reshaped.
5. **The eraser menu belongs to another effort** (`docs/onyx-reversing/stock-notes/12-eraser-menu.md`,
   `docs/planning/eraser-current-path.md`). This spec keeps a slot for it (§8) and does not touch its
   content.

## 1. Units and device numbers

| quantity                              | value                                                                                           |
| ------------------------------------- | ----------------------------------------------------------------------------------------------- |
| panel                                 | 1860×2480 px portrait, 300 dpi, 1 px = 0.0847 mm                                                |
| WebView (per-app DPI set by the user) | 661.33×881.78 CSS px, DPR 2.8125 (`HANDWRITING_UI_PLAN.md` "Device facts")                      |
| 1 CSS px                              | 2.8125 px = 0.238 mm                                                                            |
| compact layout                        | always on this tablet (width < 1000, `src/app/use-compact-layout.ts:3`), portrait and landscape |
| status bar                            | about 62 px ≈ 22 CSS (header rule at y=197 in the screenshot minus the 48 CSS = 135 px header)  |
| rem                                   | 16 CSS px in today's screenshot (the 32 CSS `icon-sm` button measures 90 px)                    |

- **Stock's dp are not our CSS px.** Stock runs at density 1.875, and the app runs at 2.8125. Stock
  sizes converted to our CSS px: top bar 140.4 px = 49.9 CSS, tool column 105.4 px = 37.5 CSS, eraser
  popup 906 px = 322 CSS, page area 1754.6×2339.6 px = 623.9×831.9 CSS.
- **Chrome sizes in this spec are px, not rem.** rem follows the system font scale (13.6 px was
  measured on 2026-09-06 at font scale 0.85), and a toolbar that grows with the font scale costs sheet
  height. Text inside the chrome stays in rem.
- All sizes assume DPR 2.8125. At the panel's native density (DPR 1.875, 992 CSS wide) the same CSS
  sizes are a third smaller physically (a 40 CSS button would be 6.4 mm). If the per-app DPI is ever
  reset, revisit the target sizes.

## 2. Today versus stock

| region (device px, y)            | Tangleaf today                         | stock Notes           |
| -------------------------------- | -------------------------------------- | --------------------- |
| system status bar                | 0–62                                   | hidden                |
| app header (logo, sync, search…) | 62–198                                 | —                     |
| "Back · PRIMARY · PAGE" pane bar | 198–299                                | —                     |
| title + tools row                | 299–425                                | 0–140 (one bar)       |
| grey gap                         | 425–462                                | —                     |
| sheet / page                     | 462 → 2869, bottom 16% off screen      | 140–2480, all visible |
| chrome share of the height       | 462 px (18.6%)                         | 140 px (5.6%)         |
| sheet (CSS)                      | 611×856, 717 CSS of the height visible | page 624×832          |

Also visible in the screenshot: the grid toggle is drawn inverted like a selected tool, so the bar
shows two "active" buttons (eraser and grid). Finger scrolling on the sheet does nothing
(`ink-canvas.tsx:642` `touch-none`, `startFinger` only drags a selection), so the cut-off bottom can
only be reached by scrolling on the 24 CSS side gutters.

## 3. Target layout on the Note Air 4C

### 3.1 Sketch, 1860×2480 px portrait

```
x: 0                                                                        1860
y: 0    +------------------------------------------------------------------+
        |[<-] Title of the note ......  [chip] |/ / /|[E][L]|[U][R]|[...]  |  toolbar 44 CSS = 124 px
y: 124  +==================================================================+  1 CSS black rule (part of the 44)
        |            gutter 6 CSS = 17 px                                  |
y: 141  |     +------------------------------------------------------+     |
y: 143  |     | canvas 588.4 x 823.8 CSS = 1655 x 2317 px              |     |
        |     | x 103..1757 px                                         |     |
        | 100 | 1000 x 1400 ink units, 1 unit = 1.655 px               | 100 |  letterbox 35.5 CSS = 100 px
        |  px |                                                        |  px |  (white on e-ink)
        |     |                                                        |     |
        |     |                                                        |     |
        |     |                                                        |     |
y: 2460 |     +------------------------------------------------------+     |
y: 2480 +------------------------------------------------------------------+  gutter 6 CSS = 17 px
```

No status bar, no app header, no pane bar, no banners. The sheet is contained: height-limited on this
3:4 screen, so it touches the top and bottom gutters and leaves 35.5 CSS on each side.

### 3.2 Toolbar positions (CSS px, 661 wide; device px = ×2.8125)

```
 4    44 48                               295 299 339 343 383 387 427 431 471 475 515 519 524 564 568 608 612 617 657
 [ <- ]   Title ..................... [chip]   [pen1][pen2][pen3][ E  ][ L  ] | [ U  ][ R  ] | [ .. ]
```

| item         | CSS x            | device px x | notes                                         |
| ------------ | ---------------- | ----------- | --------------------------------------------- |
| Back         | 4–44             | 11–124      | compact only (§4.4)                           |
| title/status | 48–295           | 135–830     | flexible, absorbs every change in the bar     |
| pen presets  | 299–427          | 842–1201    | three 40 CSS buttons (§5.1, optional task L5) |
| Eraser       | 431–471          | 1213–1326   | centre 451 CSS = 1269 px; popover anchor (§8) |
| Lasso        | 475–515          | 1337–1450   |                                               |
| separator    | 519              | 1461        | 1 CSS black, 24 CSS tall                      |
| Undo, Redo   | 524–564, 568–608 | 1475–1711   |                                               |
| separator    | 612              | 1723        |                                               |
| More         | 617–657          | 1736–1849   | note menu                                     |

### 3.3 Sheet sizes compared

| layout                                                 | canvas CSS  | canvas px     | visible |
| ------------------------------------------------------ | ----------- | ------------- | ------- |
| today                                                  | 611×856     | 1719×2407     | 84%     |
| **target** (bars hidden, 44 CSS toolbar, 6 CSS gutter) | **588×824** | **1655×2317** | 100%    |
| target with the status bar kept (fallback)             | 573×802     | 1611×2255     | 100%    |
| same, gutter 0                                         | 597×836     | 1679×2351     | 100%    |
| toolbar 48 CSS instead of 44                           | 586×820     | 1647×2306     | 100%    |
| sheet-only mode, no toolbar (deferred, §14)            | 620×868     | 1743×2441     | 100%    |
| stock page                                             | 624×832     | 1755×2340     | 100%    |

The target is 3.7% narrower than today's sheet, but all of it is on screen: the usable area grows
from 611×717 to 588×824 CSS (+11%), and nothing needs scrolling. Stock's page is wider only because
its page is 3:4; the heights match (2317 vs 2340 px).

## 4. Sheet mode (full screen while a handwriting note is open)

### 4.1 When it is on

`sheetMode = compact && android && handwritingActive`, computed once in `App.tsx`:

- `handwritingActive` already exists (`App.tsx:77-79`). In compact layout the active pane is the
  visible pane, because `show_compact_pane` sets both (`workspace-model.ts:201-204`).
- `android` uses the same user-agent test as `layout.tsx:103`. A narrow desktop window is compact
  too, but its header carries `data-tauri-drag-region` and the window controls, so desktop never
  enters sheet mode.
- It does not depend on `editing`. A read-only note (no pen, or open in another pane) gets the same
  layout without the tools.
- It is derived state, not a mount effect of the editor. Going from one handwriting note to another
  (for example with Back) keeps it on, so the bars do not flicker between notes.

### 4.2 What sheet mode hides

| chrome                                                        | where                                        | in sheet mode                          |
| ------------------------------------------------------------- | -------------------------------------------- | -------------------------------------- |
| Android status and navigation bars                            | native                                       | hidden (§4.3)                          |
| app header (brand, sync, search, assistant, settings)         | `layout.tsx:187-202`                         | not rendered                           |
| 1 px top border of `.workspace-frame`                         | `layout.tsx:204-209`                         | removed                                |
| compact "Primary / Beside" tab strip                          | `workbench.tsx:55-77`                        | not rendered                           |
| pane bar ("Back · PRIMARY · PAGE")                            | `workbench.tsx:217-281`                      | not rendered for the sheet pane        |
| active-pane ring                                              | `workbench.tsx` PaneFrame class              | dropped (it paints over the letterbox) |
| title-conflict banner, version banner, read-only line, footer | `handwriting-note-view.tsx:746-784, 820-846` | moved into the toolbar (§9)            |

What becomes unreachable while the note is open: dashboard via the brand, the sync indicator, search,
the assistant, settings, and pane switching. All of them come back after Back. Stock's editor has none
of them either. The note's own sync state stays visible as "Unsent" (§9).

### 4.3 System bars (native, see §12.2)

- Hide `WindowInsetsCompat.Type.systemBars()` with
  `BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE`. This is the WindowInsetsController form of stock's
  `setSystemUiVisibility(0x3516)` (HIDE_NAVIGATION | FULLSCREEN | IMMERSIVE_STICKY and the layout
  flags). Stock hides them for the whole `ScribbleActivity`. We have one activity, so the page asks for
  it on entry and gives it back on exit.
- The activity is already edge-to-edge (`MainActivity.kt:20-28`), so the WebView does not move or
  resize when the bars go. Only the insets change. That keeps the native view coordinates the ink
  session uses unchanged; the sheet only moves through the CSS layout, which ends in a
  `[data-ink-viewport]` resize and a normal `configure`.
- The insets reach the page through a new `safeAreaChanged` plugin event (§12.2), because
  `appearance.tsx:139-163` re-reads them only on `resize`/`orientationchange`, and hiding a bar under
  edge-to-edge fires neither. Without the event `#root` keeps its 22 CSS top padding and
  `html::before` paints a blank band.
- **Entering**: call `setImmersiveMode(true)` and hold `suppressOnyxInk("system-bars", true)` until
  the first `safeAreaChanged` after the call, or 500 ms. The layout settles while the pen is paused,
  so the session never draws across the transition where stock showed a dead ink rectangle
  (`docs/onyx-reversing/03-raw-drawing-render-flag.md:259`,
  `docs/onyx-reversing/02-handwriting-regions.md:519`). Then call the existing
  `requestFullRefreshSoon()`, which merges with the navigation's own refresh.
- **Leaving**: write the cached windowed insets back into the CSS variables first, then call
  `setImmersiveMode(false)`, so the header never sits under a bar that has just reappeared. The
  following `safeAreaChanged` corrects any difference.
- The native side remembers the wanted state and applies it again in `onResume`, so switching apps
  does not leave the note with bars.
- A swipe from the top edge shows the bars transiently over the toolbar (not over the sheet, whose
  top is 143 px down while the bar is 62 px tall). Pulling the shade takes window focus, which already
  pauses the pen (`OnyxInk.kt:585-591`). Stock pauses explicitly on its status-bar broadcast
  (`stock-notes/01-pen-session-lifecycle.md:618`). Ours is expected to be enough, but it is unverified
  (device check 6 in §13).

### 4.4 How to leave

- **Back button** at the left end of the toolbar, like stock's back arrow. It does exactly what the
  compact pane-bar Back does today (`workbench.tsx:219-236`): `mayLeavePane(paneId)`, then `go_back`
  if the pane has history, else `backToHome()`. Extract that handler into one `usePaneBack(paneId)`
  hook and use it in both places, so the leave guard (`handwriting-note-view.tsx:417-433`) keeps a
  single entry point.
- **Android Back** (hardware or gesture) keeps working through `layout.tsx:99-121`; no change.
- **Any other way the note stops being the visible page** (Delete in the note menu, a pane closing)
  turns `sheetMode` off, and the bars come back.
- There is no separate "exit full screen" button. Stock's top-left toggle hides the app's own bars,
  not the system bars; that is the deferred sheet-only mode (§14).

### 4.5 Other pages and other screens

| context                                  | behaviour                                                                                                                                                                |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| any non-handwriting page, Home, settings | unchanged: bars, header and pane bar as today                                                                                                                            |
| Note Air 4C landscape (882×661 CSS)      | sheet mode; canvas 431×603 CSS, 225 CSS letterbox per side. Fit-width is a zoom-level job (§7.4)                                                                         |
| Android phone portrait (e.g. 390×844)    | sheet mode; width-limited, canvas 376×526 CSS, top-aligned, 260 CSS free below                                                                                           |
| Android phone landscape (844×390)        | sheet mode; canvas 237×332 CSS; the 280 CSS width floor makes it scroll a little (§7.2)                                                                                  |
| Android tablet ≥ 1000 CSS wide           | desktop layout, no sheet mode; the sheet is fitted inside the pane                                                                                                       |
| desktop, any window size                 | shell, header and pane bar stay; the editor toolbar has no Back; the sheet is fitted inside the pane with a 16 CSS gutter (for example 521×729 CSS in a 1440×900 window) |

## 5. The toolbar

### 5.1 Items, left to right

| #   | item           | size (CSS)    | behaviour                                                                                                                                                                                         |
| --- | -------------- | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Back           | 40×40         | only when the view gets `onBack` (sheet mode)                                                                                                                                                     |
| 2   | title          | flex, min 0   | tap opens RenamePageDialog (as today); truncates                                                                                                                                                  |
| 3   | status chip    | auto, max 160 | the note's highest-priority status (§9); absent when there is nothing to say                                                                                                                      |
| 4   | pen presets ×3 | 40×40 each    | one tap selects pen _and_ width (2, 3, 5 units). A nib icon over a 16 CSS sample bar 1, 2 and 3 CSS thick. `aria-label` "Pen, fine/medium/bold", `aria-pressed` = `tool === "pen" && width === w` |
| 5   | Eraser         | 40×40         | first tap selects, second tap opens the eraser popover (§8)                                                                                                                                       |
| 6   | Lasso          | 40×40         | first tap selects, second tap opens the lasso options (existing)                                                                                                                                  |
| –   | separator      | 1×24          | black on e-ink                                                                                                                                                                                    |
| 7   | Undo, Redo     | 40×40 each    | as today                                                                                                                                                                                          |
| –   | separator      | 1×24          |                                                                                                                                                                                                   |
| 8   | More (…)       | 40×40         | NoteMenu: Grid paper (checkbox), Add to/Remove from favorites, Rename, Delete note                                                                                                                |

- Bar: `h-[44px]` including a 1 px bottom rule, `px-1`, `gap-1`, white. On e-ink the rule is black
  and there is no shadow (stock uses a 1.5 dp = 2.8 px black line, which is our 1 CSS).
- Targets are 40 CSS = 9.5 mm square, and icons 24 CSS = 5.7 mm (stock: 36 dp icon, about 8 mm
  target). The current `icon-sm` buttons are 32 CSS with 16 CSS icons.
- The tools are right-aligned and never move. Anything that appears or disappears (the chip,
  "Unsent", a longer title) changes only the width of the title area.
- The grid toggle moves into the More menu. It is a sheet property, changed rarely, and as an
  `aria-pressed` button it is inverted like a selected tool (see §2). Stock keeps it under "Template"
  in its column, a menu as well.
- Read-only (`!editing`): items 3–7 are replaced by the read-only text, set inline in the right part
  of the bar (`text-xs`, at most two lines, `title` with the full text). Back, title and More stay.
- `role="toolbar"` and `aria-label="Handwriting tools"` stay on the bar. The e-ink inversion and
  `transition: none` rules (`index.css:1230-1247`) key on them.

### 5.2 Narrow widths

The wide bar needs 48 + 358 + 4 = 410 CSS, plus room for the title. Measure the bar with a
`ResizeObserver` (not a container query, so the DOM has one set of buttons):

- width ≥ 600: as above (tablet portrait 661, tablet landscape 882, desktop panes).
- width < 600: the three presets collapse into one Pen button whose second tap opens the existing
  width popover (`handwriting-note-view.tsx:618-641`). The right cluster is then 270 CSS, and a
  360 CSS phone still fits Back, pen, eraser, lasso, undo, redo and More with 38 CSS left for the
  title.
- width < 480: the chip shows its icon only (label in `aria-label`).

### 5.3 E-ink styling

- Selected tool: inverted (existing rule). Separators and the rule are solid black. There are no
  tinted states, hover effects, shadows or transitions.
- Disabled undo/redo keep today's reduced opacity, and their size never changes.
- Letterbox: `bg-neutral-100 eink:bg-white`. A light grey dithers on the Kaleido panel and suggests a
  surface that is not there. The sheet border becomes `eink:border-black`, which gives the page a hard
  edge.

## 6. Left tool column: not now

Decision: **no column**. All our tools fit in one bar.

- Our tool set is Back, title, three pens, eraser, lasso, undo, redo and More: 9 buttons. At 40 CSS
  that is 410 CSS of fixed width, and a 661 CSS bar has about 250 CSS left for the title. Stock needs
  a column because it offers 18 tools.
- A column would cost no sheet size. The 5:7 sheet is height-limited on this 3:4 screen, so up to
  about 58 CSS of width is free. It would still add nothing: every item would be moved out of a bar
  that has room. It would also push the sheet off centre and add a second edge that the pen has to
  travel to.
- **When to reconsider**: when the bar can no longer hold the tool set at 661 CSS with a title of at
  least 150 CSS, for example once text boxes, shapes, templates or page navigation exist. Then add a
  column of at most 56 CSS on the left (stock: 37.5 CSS at our DPR) instead of a second row, since a
  second row costs sheet height and a column does not.

## 7. Fitting the sheet

### 7.1 Rule

- **Contain, top-aligned, horizontally centred**, with a uniform gutter `g` (6 CSS in compact,
  16 CSS from 1000 CSS up). This is stock's `FitScreenRequest`, `scale = min(viewW/pageW,
  viewH/pageH)`, except that stock centres vertically. We align to the top because when the sheet is
  width-limited (phones) it then stays put if the viewport height changes. On the tablet the sheet is
  height-limited, so the two are the same.
- Width cap 900 CSS (as today, bounding the canvas buffer on large desktops). Width floor 280 CSS
  (below that the sheet overflows and the viewport scrolls).
- No margins inside the sheet. Grid insets belong to the template, as in stock.

### 7.2 Mechanism: CSS only, the canvas untouched

- `[data-ink-viewport]` (`handwriting-note-view.tsx:785-788`) keeps its element, its `overflow-auto`
  and `overscroll-contain`, and gets `[container-type:size]` plus padding `g`. Container units then
  measure its content box.
- The sheet wrapper (`:794`) gets
  `width: max(280px, min(100cqw, (100cqh - 3px) * 5 / 7 + 2px, 900px))` and `mx-auto`.
  - `2px` is the wrapper's 1 px border on each side.
  - The extra `1px` of slack stops rounding from making the viewport scrollable by one pixel. A 1 px
    scroll would still fire `configure`, which pauses and re-arms raw drawing for nothing.
- The canvas stays `block aspect-[5/7] w-full` (`ink-canvas.tsx:642`). Its size still comes from the
  wrapper width, so the canvas `ResizeObserver` (`ink-canvas.tsx:482-493`, buffer resize) and both
  observers in `onyx-ink.ts:293-296` keep working with no change.
- `container-type: size` also applies layout containment, so the viewport becomes the containing
  block for fixed descendants. Nothing inside it is fixed (popovers and menus portal to `body`), and
  `[data-workspace-scroll] > *` already does the same (`index.css:1163-1168`).
- Support: container query units are in the Android WebView (Chrome 151) and WebView2, and in
  WebKitGTK 2.40 and later.
- No JS measurement and no state: the fit changes only when the viewport box does.

### 7.3 Keeping zoom possible later

- Leave a `--sheet-zoom` custom property on the viewport, default 1, and multiply the wrapper width by
  it. A later zoom sets it (fit-width is `100cqw / fit`), and the viewport scrolls as it does today.
  The ink mapping is rect-based (`ink-model.ts:20-21`), so any size works.
- Two things the zoom effort must solve first, and which this layout does not:
  - **Finger panning on the sheet.** The canvas is `touch-none`. On this firmware every WebView
    scroll switches the panel to A2 and ends in a full GC16 flash (`InkStylusRouting.kt` header).
  - **Horizontal clipping.** `configure` passes only `clipTop`/`clipBottom`. A sheet wider than a
    desktop pane needs a horizontal clip as well, which means new `configure` arguments, a protocol
    change that needs its own review.

## 8. Eraser popover slot

- **Anchor**: the Eraser button in the toolbar, below it (`side` bottom is Base UI's default, so
  `popover.tsx` needs no `side` prop while the tools stay in the top bar). Keep today's mechanics:
  the second tap on the active eraser sets `optionsAnchor` and opens the shared `Popover` with
  `data-ink-tool-options` (`handwriting-note-view.tsx:540-547, 610-617`). The popover closes on a
  tool change and when editing ends (`:488-491`).
- **Geometry on the tablet**: the eraser centre is at 451 CSS. A popover of stock's width (322 CSS)
  centred there spans 290–612 CSS, inside the 12 CSS collision padding, with its top at
  44 + `sideOffset` CSS. Stock's box spans x 914–1819 px from y 160 px, which is 325–647 CSS from
  57 CSS. The eraser effort decides the alignment (`align="center"` as now, or `"end"` to match
  stock).
- **Width**: express it in px (`w-[min(322px,calc(100vw-24px))]`), not rem. Today's
  `max-w-[min(22rem,…)]` is 352 CSS at a 16 px rem but only 299 CSS at the font scale measured on
  2026-09-06.
- **Pen while open**: paused by the `Popover` wrapper (constraint 3), so the popover needs no overlay
  rect. Closing runs `refreshPanelAfterClose`.
- The optional stock-style marker triangle under the button would be a `PopoverPrimitive.Arrow`,
  which `popover.tsx` does not export yet. That belongs to the eraser effort.
- This spec does not change the eraser popover's content, the eraser button's icon, or the test at
  `handwriting-note-view.test.tsx:457-479`.

## 9. Banners, status lines, footer and toasts

Today these are rows in the editor column (`:746-784, 820-846`). Under a contained sheet each one
would rescale the whole sheet when it appears, and a banner that appears while the footer disappears
moves the sheet without resizing anything `onyx-ink.ts` observes. All of them move into the toolbar,
so nothing above or below the viewport ever changes height.

| status (priority order)        | chip label                        | chip style (e-ink)     | details popover content and actions                         |
| ------------------------------ | --------------------------------- | ---------------------- | ----------------------------------------------------------- |
| save error (`saveState` Error) | "Not saved"                       | inverted, `role=alert` | "Could not save your changes. {message}" + **Retry saving** |
| title conflict                 | "Title conflict"                  | inverted               | existing text + **Use remote** / **Keep mine**              |
| version conflict               | "{n} versions"                    | outlined               | existing text + **Compare**                                 |
| sheet full (`limit`)           | "Sheet full"                      | outlined               | existing text                                               |
| native pen error               | "Pen input"                       | outlined               | "Fast pen input is unavailable: {error}"                    |
| unsent changes                 | "Unsent" (plain text, no popover) | plain                  | —                                                           |

- The chip opens one `Popover` (with `PopoverTrigger`, marked `data-ink-status`) that lists **every**
  current status with its full text and actions, not only the top one. The pen is paused while it is
  open, as for any popover.
- On colour displays the chips keep today's amber (conflicts) and destructive (save error) tints.
- The read-only notice is inline text in the bar (§5.1), not a chip, because the tools are gone then.
- The loading text and the load error stay inside the viewport. They never coexist with the sheet.
- **Toasts**: sonner sits bottom-right (`main.tsx:48-55`), which is over the sheet in sheet mode. A
  pen tap on a toast action there is swallowed by `InkStylusRouting`, and the firmware ink draws over
  it. In sheet mode, render the Toaster `top-center` with a 4 px top offset, so toasts cover the
  toolbar row. A toast taller than 44 CSS still reaches about 15 CSS into the sheet's top gutter and
  edge; that residue is accepted. Note-level errors do not use toasts anyway (they are chips).

## 10. E-ink rules for this page

1. **Nothing above or below the viewport changes height while the page is open.** The toolbar is a
   fixed 44 px and the statuses live in it. The only sheet geometry changes left are entering or
   leaving sheet mode, rotation, and window or pane resizes. Each is a viewport resize followed by one
   `configure`, and `configure` already waits for the pen to lift (`onyx-ink.ts:164-167`).
2. **Toolbar changes are local.** A chip, "Unsent" or an undo/redo enable state repaints a few
   40 CSS boxes outside the handwriting region. Tools never shift.
3. **No animation.** The toolbar and chip use `transition: none`. Popover enter animations are
   already collapsed by the e-ink motion rule (`index.css:1173-1178`) and by
   `prefers-reduced-motion`.
4. **One full refresh per transition, never per frame.** Entering or leaving sheet mode requests
   `requestFullRefreshSoon()` once, which collapses with the navigation's own refresh.
5. **A pen-down on the toolbar never inks.** The toolbar lies outside the limit rect, with the gutter
   and border (7 CSS = 20 px) between its rule and the canvas. `InkStylusRouting` decides at pen-down,
   so a stroke that starts on the sheet and runs up into the toolbar stays ink.
6. **High contrast.** Black rules, inverted selection, white letterbox, black sheet border. The
   meaning never rests on colour alone (the chips have text).
7. **No reflow loops.** The fit is pure CSS; there is no JS measuring the sheet. The only JS
   measurement is the toolbar-width observer for the preset collapse, which fires on resize only.

## 11. Ink geometry contract (checklist for the implementer)

- `[data-ink-viewport]` stays a `div` ancestor of the canvas (the test harnesses in
  `onyx-ink-session.test.tsx` and `ink-canvas.test.tsx` mock `getBoundingClientRect` on it).
- Every change that moves the sheet also resizes the viewport:
  - header, pane bar, tab strip or safe-area top removed or added: the viewport height changes;
  - safe-area bottom (`pb-[var(--safe-area-inset-bottom)]` on the editor root): the height changes;
  - rotation or window resize: both observers fire, plus `resize`.
- No row may be added to the editor column above or below the viewport.
- The only persistent control over the sheet is still the selection menu, reported at
  `ink-canvas.tsx:466`. Toolbar, chips and letterbox are outside the canvas rect.
- `onyx-ink.ts` and `ink-canvas.tsx` get **no** change.

## 12. Code changes by file

### 12.1 Frontend (TypeScript/CSS)

| file                                                                     | change                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| ------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/app/sheet-mode.ts` (new)                                            | A small zustand store `{ sheet: boolean }` (the Toaster reads it outside the React tree under `App`), plus `useSheetSystemBars(sheet)`. The hook: on true, caches the current safe-area values, holds `suppressOnyxInk("system-bars", true)`, calls `setImmersiveMode(true)`, and releases the hold on the next `safeAreaChanged` or after 500 ms, then `requestFullRefreshSoon()`. On false or unmount, restores the cached values into the CSS variables and calls `setImmersiveMode(false)`. It is a no-op off Android.                                      |
| `src/App.tsx`                                                            | Compute `sheetMode` (§4.1) before the early returns. Call `useSheetSystemBars(sheetMode)`, and pass `sheet={sheetMode}` to `AppLayout` and `sheetPaneId={sheetMode ? activePane.id : null}` to `Workbench`.                                                                                                                                                                                                                                                                                                                                                     |
| `src/app/layout.tsx`                                                     | New prop `sheet?: boolean`. When it is true, do not render the `<header>` (`:187-202`) and add `border-t-0` to `.workspace-frame` (`:204-209`). The Back handler and `CompactBackToHomeContext` stay unchanged.                                                                                                                                                                                                                                                                                                                                                 |
| `src/features/workspace/workbench.tsx`                                   | `WorkbenchProps.sheetPaneId`. `Workbench` skips the compact tab strip when it is set. `PaneFrame` skips the pane bar and the active ring for `compact && sheetPaneId === paneId`. Extract `usePaneBack(paneId)` from `:219-236` and use it in the pane bar. `PagePane` passes `onBack={usePaneBack(paneId)}` to `HandwritingNoteView` only for the sheet pane.                                                                                                                                                                                                  |
| `src/features/handwriting/handwriting-note-view.tsx`                     | Optional `onBack` prop renders Back. Toolbar per §5: `h-[44px]`, 40 px buttons, 24 px icons, separators, tools right-aligned. Grid toggle moves into `NoteMenu` as a checkbox item, disabled unless `editing && draft && !editingBusy`. Banners, read-only line and footer are replaced by the status chip plus inline read-only text (§9). Viewport: `[container-type:size]`, padding `g`, `bg-neutral-100 eink:bg-white`, top-aligned. Wrapper: the width formula in §7.2 and `eink:border-black`. The tool-options `Popover` stays where and how it is (§8). |
| `src/features/handwriting/handwriting-status.tsx` (new)                  | The status chip and its details popover: priority, labels, actions (§9). It is a pure presentational component fed by the view's state.                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `src/features/handwriting/handwriting-note-view.tsx` (task L5, optional) | Three pen presets instead of one Pen button when the bar is at least 600 CSS wide (§5.2), with the single-Pen fallback below that.                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `src/main.tsx`                                                           | Wrap the Toaster in a small component that reads the sheet store: `top-center` with a 4 px top offset in sheet mode, `bottom-right` otherwise.                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `src/app/appearance.tsx`                                                 | Also listen to the plugin event `safeAreaChanged` (`addPluginListener("mobile-system", "safeAreaChanged", …)`) and write the four CSS variables from it. The existing `resize`/`orientationchange` path stays.                                                                                                                                                                                                                                                                                                                                                  |
| `src/lib/api.ts`                                                         | `export const setImmersiveMode = checkedCommand(commands.setImmersiveMode)`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `src/lib/bindings.ts`                                                    | Regenerated with `vp run bindings:generate`, never edited by hand.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `src/index.css`                                                          | E-ink rules for `[data-ink-status]` (inverted when it is an alert, 1 px black outline otherwise, `transition: none`) and for the toolbar separators. The toolbar inversion rules at `:1230-1247` stay as they are and keep applying through `role="toolbar"`.                                                                                                                                                                                                                                                                                                   |
| `src/components/ui/popover.tsx`                                          | No change. It gets `side` only if a column is ever added (§6).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `src/features/handwriting/ink-canvas.tsx`, `onyx-ink.ts`, `ink-*.ts`     | **No change.**                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |

### 12.2 Native and Rust changes (marked: each needs a device retest, §13)

| #      | file                                                                   | change                                                                                                                                                                                                                                                                                                                                                                                          |
| ------ | ---------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **N1** | `plugins/tauri-plugin-mobile-system/android/.../MobileSystemPlugin.kt` | `@Command fun setImmersiveMode(invoke)` with args `{ enabled: Boolean }`. On the UI thread: `WindowCompat.getInsetsController(window, decorView)`, set `systemBarsBehavior = BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE`, then `hide` or `show` `WindowInsetsCompat.Type.systemBars()`. Store `immersiveWanted`, and resolve at once.                                                                |
| **N2** | same file, `onResume()` (`:133-137`)                                   | If `immersiveWanted`, hide the system bars again. This goes before `onyxInk?.onResume()` and touches nothing in `OnyxInk`.                                                                                                                                                                                                                                                                      |
| **N3** | same file, `load()` (`:70-80`)                                         | Extend the **one existing** WebView insets listener; do not add a second one. After the unchanged `imeChanged(...)` call, compute the safe area the way `getSafeAreaInsets` does (factor that into `safeAreaOf(insets, density)`). If it differs from the last value sent, `trigger("safeAreaChanged", {top, right, bottom, left})` in CSS px. `getSafeAreaInsets` switches to the same helper. |
| **N4** | `plugins/tauri-plugin-mobile-system/src/mobile.rs`, `models.rs`        | `set_immersive_mode(enabled)` → `run_mobile_plugin("setImmersiveMode", ImmersiveModeArgs { enabled })`, modelled on `set_system_bars_style` (`mobile.rs:62-70`).                                                                                                                                                                                                                                |
| **N5** | `src-tauri/src/commands/system.rs`, `src-tauri/src/lib.rs`             | App command `set_immersive_mode(app, enabled)`: calls the plugin on Android and returns `Ok(())` elsewhere, like `set_system_bars_style` (`system.rs:167-182`). Register it next to that command (`lib.rs:30`).                                                                                                                                                                                 |
| **N6** | permissions                                                            | None. Rust calls to the plugin bypass the ACL, and the event uses the existing `register_listener` permission (`permissions/ink-events.toml`, part of `mobile-system:default`). `build.rs` COMMANDS lists only commands invoked directly from JS, so it is unchanged.                                                                                                                           |

**Unchanged on purpose**: `OnyxInk.kt`, `InkStylusRouting.kt`, `InkFrameFence.kt`,
`InkEraserRenderGate.kt`, `DisplayModeStack.kt`, `ViewDisplayMode.kt`, `MainActivity.kt` (it stays
edge-to-edge), the manifest, and the `configure_onyx_ink` / `commit_onyx_frame` / `suppress_onyx_ink`
arguments.

### 12.3 Tests

- `handwriting-note-view.test.tsx`:
  - Replace `button("Grid paper").click()` (`:224, 243, 294, 333, 367, 384, 437`) with a
    `toggleGrid()` helper that opens "Note options" and clicks the "Grid paper" item. At `:306`,
    assert the item's `aria-checked` instead of `aria-pressed`.
  - The tests that look for "Could not save your changes" (`:234, 296, 308`) or click "Retry saving"
    (`:312`) open the status chip first.
  - Keep `:278-287` (read-only: no pressed buttons, text inline) and `:457-479` (one toolbar, eraser
    second tap) as they are.
  - New: Back is rendered only with `onBack` and calls it; the chip shows the top-priority status and
    its popover lists all of them; with L5, a preset sets both tool and width.
- New pure test for the `sheetMode` derivation (compact, android, page kind).
- `onyx-ink-session.test.tsx`, `ink-canvas.test.tsx`: unchanged, and they must pass.
- Kotlin: a unit test for `safeAreaOf` and for the "send only on change" comparison.

## 13. Order of work, gates, device checks

| track | content                                                                                                                  | native  | can ship alone                                               |
| ----- | ------------------------------------------------------------------------------------------------------------------------ | ------- | ------------------------------------------------------------ |
| L1    | Contain fit, letterbox, `--sheet-zoom` hook (§7)                                                                         | no      | yes: the whole page becomes visible even with today's chrome |
| L2    | Toolbar sizing and grouping, grid into More, status chip replacing the banners and footer (§5, §9)                       | no      | yes                                                          |
| L3    | Sheet mode in the shell: header, pane bar and tab strip hidden, editor Back, sheet store, Toaster placement (§4.2, §4.4) | no      | yes, with the status bar still showing (canvas 573×802 CSS)  |
| L4    | System bars: N1–N6, `useSheetSystemBars`, the `safeAreaChanged` listener (§4.3)                                          | **yes** | after L3                                                     |
| L5    | Pen presets (§5.1 item 4, §5.2), optional                                                                                | no      | yes                                                          |

- Gates after every track: `vp check`, `vp test`. For L4 also `vp run bindings:generate`,
  `cargo fmt --all -- --check`,
  `cargo clippy --workspace --locked --all-targets --all-features -- -D warnings`,
  `cargo test --workspace --locked --all-features`, and the plugin unit tests (from
  `src-tauri/gen/android`: `./gradlew :tauri-plugin-mobile-system:testDebugUnitTest`).
- Stop and report instead of improvising if a track seems to need a change in any file listed as
  unchanged, or a new argument to an ink command.
- **Device checks** (the user runs them on a `just build-android-debug` build after L3 and again
  after L4):
  1. Open a note from Home. After L4 the status bar is gone, there is no blank band, the toolbar
     starts at y=0, and the canvas ends near y=2460 with the whole page visible.
  2. Write along all four sheet edges, especially the top centimetre where the status bar was and the
     bottom edge. Ink appears everywhere, with no dead band or rectangle.
  3. Tap every toolbar button with the pen: the action happens and nothing inks.
  4. Tap Eraser twice, then Lasso twice: the popover opens under the button, the pen is paused, and
     closing leaves a clean panel.
  5. Leave with the toolbar Back and with Android Back, ten times in a row quickly. The bars return,
     the header sits below the status bar and never under it, and ink works on the next open.
  6. Pull the shade or control centre down while in a note, then close it: the pen does not ink
     while the shade is down and inks afterwards.
  7. Swipe from the top edge: the transient bars cover only the toolbar, hide again, and ink is
     unaffected.
  8. Switch to another app and back: the note is still full screen, and the pen works.
  9. Rename the note, which brings up the keyboard. The bars stay hidden, and the pen works after the
     dialog closes. Note whether the sheet rescales while the keyboard is up (see risk 3 below).
  10. Rotate to landscape and back: one rescale each way, and ink lands under the pen.
  11. With a sync conflict or a save error present, write: the chip changes, and the sheet does not
      move.
  12. Trigger a toast (for example, sync offline): it appears at the top over the toolbar.

Risks to watch:

1. **Dead rectangle on the transition.** Stock showed one at a full-screen→windowed transition, and
   the fault is below the app. The pause held until the insets settle (§4.3) is the mitigation.
   Checks 2 and 5 test it.
2. **Status-bar shade.** We rely on losing window focus, where stock uses an explicit broadcast pause.
   Check 6 tests it.
3. **Keyboard resize.** If the WebView's height follows the keyboard, the contained sheet rescales
   twice per rename. The pen is paused by the dialog then, so the cost is two redraws. If check 9
   shows it, hold the fit while the keyboard is visible.
4. **Phones with 3-button navigation.** Hiding the navigation bar makes Home a two-step gesture while
   a note is open. This is the same trade stock makes on BOOX, which has no navigation bar by default.

## 14. Deferred, with triggers

- **Sheet-only mode** (stock's top-left toggle, hiding our own toolbar): canvas 620×868 CSS =
  1743×2441 px, 5% larger in each dimension. It needs a persistent corner control over the sheet,
  reported in `overlayRects` the way stock registers its type-10 exclude rect, and another
  transition of the kind stock itself glitches on. Revisit once the L4 transition has passed device
  checks 2 and 5 reliably.
- **Left tool column**: see §6 for the trigger.
- **Zoom and pan**: see §7.3 for the two prerequisites.
- **An option to keep the status bar while writing**: add it only if someone asks. The layout
  already handles both states.
- **Editable pen presets** (width and colour, like stock's long-press), and page navigation for
  multi-page notes. Both are out of scope for the frozen single-sheet format.
