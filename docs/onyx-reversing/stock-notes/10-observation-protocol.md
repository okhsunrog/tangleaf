# 10 — Hardware observation protocol

The plan for the next phase: perform each action on the tablet, first in stock Notes and then the
same action in Tangleaf, with the panel driver log recording, and compare what appears against the
predictions below. The predictions come from documents 01–08 and the reconciliation in 00 and 09;
each is tagged with the document it rests on and **[code]** / **[inferred]**. A prediction that
fails is a finding, not a problem with the run.

Nothing in this file has been run. The actions are for the user to perform; the capture commands
only read the device, except where marked _(changes device state)_.

## 1. Capture setup

### 1.1 Before the session (read-only)

```sh
adb shell getprop ro.build.display.id            # firmware build (4.2 as of 2026-09-25)
adb shell getprop ro.build.fingerprint
adb shell getprop ro.vendor.build.fingerprint
adb shell getprop ro.board.platform             # expect lito
adb shell getprop vendor.onyx.tablet            # expect true (image value)
adb shell getprop vendor.onyx.htcon             # expect true (image value)
adb shell getprop ro.product.model              # expect NoteAir4C
adb shell settings get global scroll_refresh_delay_v2
adb shell su -c "strings /onyxconfig/mmkv/onyx_config" > onyx_config.txt   # root; EinkWise store
adb shell su -c "sha256sum /system/bin/surfaceflinger"                      # compare with the extracted binary
```

This is action **A0**. It settles C1 (which build), C16/C17 (tablet, htcon at runtime), the
`scroll_refresh_delay_v2` value that 07 §6 predicts as the lift-to-flash delay, and stock's stored
EinkWise block (refresh index, turbo, dither threshold: 128 expected, 160 or 180 would confirm 05
§7.3). If the SF hash differs from `~/tmp_zfs/onyx_framework/native/surfaceflinger`, every SF address
in 01–08 must be re-derived before trusting predictions that depend on them.

Tangleaf's own state, read once through the Tauri MCP bridge (`adb forward tcp:9223 tcp:9223`):
`configure_onyx_ink` with `enabled:false` of a non-matching session returns `status()`
(`colorPanel`, `resumeDelayMs`, display mode) (08 §10 exp. 10).

### 1.2 Recording

Record the whole log in the background and grep afterwards (per the user's logcat rule):

```sh
adb logcat -c && adb logcat -b all -v time > run-<name>.log     # run in background, stop with TaskStop
adb shell pidof com.onyx.android.note                          # stock pid
adb shell pidof <tangleaf package>                              # Tangleaf pid
```

One recording per app and per action group keeps the files readable. Note wall-clock times of each
action (or film the screen) so lines can be tied to actions.

### 1.3 What to grep

```sh
grep -E 'onyx_tcon_send_cmd|onyx_tcon_set_upd_scheme|### EUS|### EBAS|HWEpdcManager: ET|State change to|State changed!|Control state|Post enabled|HandlePenTrigger|startHandwriting|EpdcManager: CU|waveform_mode|hasHwTcon|OnyxScrollRefreshManager|applyTransientUpdateMode|clearTransientUpdateMode|disable refresh|commit epdc|ignoring refresh|OnyxInk' run-<name>.log
```

| pattern                                                                                                                                                                                                         | source                             | meaning                                                                                                                                                     | always logged?                                 |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------- |
| `onyx_tcon_send_cmd … waveform[W] update_mode[M] flags[F]`                                                                                                                                                      | kernel                             | every panel update; `update_mode[1]` = mode bit `0x20` (full), only mode 98 among stock's modes (05 §4.4)                                                   | yes (handoff)                                  |
| `onyx_tcon_set_upd_scheme`                                                                                                                                                                                      | kernel                             | scheme ioctl `0x7006` (07 §5.1) [inferred]                                                                                                                  | yes (handoff)                                  |
| `HWEpdcManager: ### EUS, s N c M`                                                                                                                                                                               | SF `setEbcUpdateScheme`            | scheme 3 = handwriting started; 2/4 = left handwriting or a fixed slot changed; 4 = a fast mode sits in a fixed slot (07 §5)                                | yes, error priority [code]                     |
| `HWEpdcManager: ### EBAS n`                                                                                                                                                                                     | SF                                 | ext-buffer auto-sync on pen state 0/1 and on entering scheme 3 (07 §4.1, §5.1)                                                                              | yes [code]                                     |
| `HWEpdcManager: ET n`                                                                                                                                                                                           | SF                                 | turbo set (stock fast mode sends 5) (07 §2.2)                                                                                                               | yes [code]                                     |
| `SurfaceFlinger: Pid N State change to: <state>` + `State changed!`                                                                                                                                             | SF `handleSetHandWritingPenState`  | a pen-state transaction that differs from the live reader state; `toPause` = 3, `toResume`/`running` = 2, `start` = 1, `stop` = 0, `paused` = a 4 (07 §4.1) | yes [code]                                     |
| `Control state: a -> b`                                                                                                                                                                                         | SF reader thread                   | reader actually reached the state                                                                                                                           | yes [code]                                     |
| `SurfaceFlinger: Post enabled!`                                                                                                                                                                                 | SF `afterStopHandwriting`          | left schema 7 (pause, stop or `ENABLE_POST(1)`)                                                                                                             | yes, info [code]                               |
| `HandlePenTrigger, sid: … pid …`                                                                                                                                                                                | SF                                 | entering schema 7 (pen sample, or app `MOVE_TO`)                                                                                                            | yes [code]                                     |
| `### startHandwriting`                                                                                                                                                                                          | SF                                 | same event                                                                                                                                                  | gating not checked **[unknown]**               |
| `E EpdcManager: CU 0/1`                                                                                                                                                                                         | SF `setEnableColorCU`              | `ENABLE_COLOR_CU` (03 §4.2)                                                                                                                                 | yes, error priority [code]                     |
| `onyx_epdc_hwscreenRefresh(): waveform_mode  wrong!`                                                                                                                                                            | SF `hwScreenRefresh`               | a frame with nibble 0 or 7–15, e.g. stock's `REGAL_PLUS` one-shot; sent with waveform 0 (05 §4.4)                                                           | yes [code]                                     |
| `SurfaceFlinger: hasHwTcon: n`                                                                                                                                                                                  | SF `ExtBufferSchema::onDeactivate` | the ExtBuffer schema ended (one-shot 98 ×2 follows) (05 §8.2)                                                                                               | gating not checked                             |
| `OnyxScrollRefreshManager` (`checkHasScrollCallback topScrollBounds`, `hadChildInWhiteList`, ` beforeScroll`, ` afterScroll`)                                                                                   | framework, in the app's process    | the framework scroll-A2 path (07 §6 Entry A); never for `com.onyx`                                                                                          | yes (D/E) [code]                               |
| `### applyTransientUpdateMode`, `### clearTransientUpdateMode, clearFlag n`, `### disable refresh.`, `### warning: commit epdc in handwriting mode.`, `### warning: sys/app is in fast mode, ignoring refresh.` | SF                                 | fixed-slot and schema-7 decisions                                                                                                                           | only with SF debug on (byte `0xac9ea0`) [code] |
| `OnyxInk`                                                                                                                                                                                                       | Tangleaf                           | our plugin's own log                                                                                                                                        | yes                                            |

Optional, _(changes device state)_: SF debug logging via transaction `APPLY_SF_DEBUG` (1048642,
`ViewUpdateHelper.applySFDebug(true)` by reflection, 07 §2.2). It makes the `###` debug lines visible
and prints `pen control state…, penTriggered…, current epdc schema…, androidDrawing…`. Stock's own
`PenLogger` output can be enabled with the debug broadcast (`KNoteCommonReceiver`, 03 §9, 06 §8);
also a state change. Neither is needed for the first pass.

### 1.4 Shorthand used in the predictions

- `P3` = `Pid <pid> State change to: toPause` (+ `Control state: toPause -> paused`).
- `P2` = `State change to: running` / `toResume` (+ `Control state: toResume -> running`).
- `P1` = `… start`, `P0` = `… stop`.
- `T` = `HandlePenTrigger` + `EUS, s 3 c 0` (and possibly `### EBAS 0` just before it).
- `X` = `Post enabled!` then, ~150 ms later, `EUS, s 2 c 0` (or `s 4` if a fast mode is in a fixed
  slot).
- `F` = the observed flash signature: `EUS, s 2 c 0` followed by `onyx_tcon_send_cmd … waveform[2]
update_mode[1] flags[0x1000]` (handoff).
- `A2` = `EUS, s 4 c 0` followed by `waveform[4]` updates.
- "partial" = an `onyx_tcon_send_cmd` with `update_mode[0]`.

`T` appears only on the first drawn sample after the session last left schema 7 (07 §4.2) [code]:
after a stroke, further strokes do not repeat it until something ends handwriting.

## 2. The actions

Each action: what to do, then the predicted stock lines, then the predicted Tangleaf lines, then
what the result decides. Do each action in stock Notes, then in Tangleaf, on a page with some
existing strokes, pen tool selected, unless stated. Wait 3 s between actions.

### A1. Open a note page, wait 3 s, draw one stroke

- **Stock** (01 §4.2–§4.3, 07 §3) [code; timing inferred]: `P1` (the second 1 is ignored), `P3`
  (park); about 600 ms after the page appears, `P2`. No `EUS`, no `update_mode[1]` of the app's own.
  On the stroke: `T`, live-ink updates. Open question: which waveform/flags the live ink uses
  (02 §11 exp. 2).
- **Tangleaf** (08 §5.2) [code]: `P3` (session cycle; SF logs it only if the reader state differs),
  `P1`, `P2` with the pen tool; a partial update of the whole WebView ≈ 120 ms after the WebView
  acknowledges (reconcile mode 524290). On the stroke: `T`.
- Decides: the park and the resume delay (01 E1); Tangleaf's open cost.

### A2. Write ten strokes without touching the UI, then wait 60 s

- **Stock** (02 §5, §8) [code for the calls; panel effect inferred]: one `T` at the first stroke, then
  only live-ink updates. No update ≈ 0–100 ms after each lift and none ≈ 500 ms after the last lift
  (per-stroke reconcile gated off by `vendor.onyx.tablet`). No `X`, no scheme line afterwards; no
  `P3` during 60 s unless auto-save runs (then a pause and a resume). No `OnyxScrollRefreshManager`
  lines.
- **Tangleaf** (08 §5.3) [code; SF commit inferred]: `T` once; after each stroke's pen-up refresh
  (~500 ms) a partial update, expected with the handwrite flag, since mode `0x80002` is the entry SF
  commits in schema 7 (02 §5.6). Record its rectangle: whole WebView or the stroke only (08 exp. 5).
  If Entry B (`WebView.onTouchEvent`, no pen-state gate) fires on vertical strokes, it cannot switch
  the scheme while it is 3 (07 §5.2); look for a full-screen update ~1 s after a lift
  (`update_mode[0]` or `[1]`) or, with SF debug, `commit epdc in handwriting mode` [inferred, 07 §6].
- Decides: 02 exp. 1, 3, 5, 12; whether plain app frames reach the panel in schema 7 (gap 5 in 09);
  whether Entry B fires during normal Tangleaf writing.

### A3. Write with a red pen

- **Stock** (02 §6.3) [inferred]: raw colour is sent to SF (`isColorStrokesShowOriginal` defaults to
  colour device). Watch whether the live trace is coloured before the next hand-back.
- **Tangleaf** (08 §3) [code]: stroke colour is always black to SF; the colour appears only with the
  reconcile frame.
- Decides: 02 exp. 6.

### A4. Erase one stroke with the stylus eraser end (pen tool), lift, wait 3 s

Repeat after selecting each eraser type (stroke, partial, area) in the eraser menu and returning to
the pen.

- **Stock** (01 §1 item 7, 03 §3.3–§3.4, 07 §4.4) [code; timing inferred]: `CU 1` at down; `T` at the
  first eraser sample if schema 7 was not active; SF draws a soft-eraser trace (types 0/1) or a dashed
  trace (type 2); **no `P3` while the eraser is on the glass**. At lift: `CU 0`; if a visible stroke
  was erased, `X` (from `ENABLE_POST(1)` in `EraseFinishAction.render`); then `P3` (render-only
  pause); a partial update (the erased area, mode 5); ≈ 300 ms later `P2`. No `A2`, no `F`, no
  `OnyxScrollRefreshManager` lines.
- **Tangleaf** (08 §5.4, 07 §6) [code for our calls; framework part inferred]: `P3` shortly after
  contact (after the main-thread hop; possibly after a `T` from the first samples); `X` if schema 7
  was active; if the stroke has a vertical component, ` beforeScroll` from `OnyxScrollRefreshManager`
  and `A2` during the stroke; WebView frames with `waveform[4]`; after lift ` afterScroll`, the
  reconcile frame, `P2`; ≈ `scroll_refresh_delay_v2` + 100–200 ms after lift, `F`.
- Decides: 01 E2, 03 E2, 07 E8, 08 exp. 1, 06 E8; and, with the eraser-type variants, which type the
  eraser end uses (07 §4.4 [inferred]). Timestamp `P3` against the first ink update (08 exp. 1).

### A5. Eraser end, then the tip within 200 ms (post-erase window)

Erase with the eraser end, flip the pen and start a stroke within 200 ms of the lift. Repeat with

> 1 s between.

- **Stock**: the stroke starts in state R (pen state 3, input on) (01 E3, 03 E3/E4).
  - If 07 holds: stock is exempt from Entry A and has no WebView, so no `A2`/`F`; the first part of
    the stroke is not inked by SF (reader paused) and appears only as the app's redraw after `P2`
    [inferred].
  - If the handoff's "pen input into a paused session" holds inside SF: `A2` during the stroke and `F`
    ~1.5 s after lift.
  - Also watch the style: a pale or wide first stroke would confirm the single SF style slot (03 E3).
- **Tangleaf**: render is back only after the reconcile frame; a quick tip stroke restores render
  immediately (`InkEraserRenderGate.begin(false)`, 08 §5.3) → `P2` at the tip down [code].
- Decides: C12 (09), the central question.

### A6. Select the eraser tool from the eraser menu; then change only the eraser type

- **Stock** (03 §3.2) [code; interleaving inferred]: `CU 1`; `P3`; `P2` ≈ 150 ms later; `P3`
  again and `P2` ≈ 500 ms later (or `P3 P3 P2` merged); on a type-only change one `P3` … `P2`
  (500 ms).
- **Tangleaf** (08 §5.9) [code]: `P3` (if render was on); no `P2` afterwards (eraser tool keeps
  render off). The popover close may add a mode-98 GC 300 ms later if closed by an outside tap
  (08 §5.9 step 3).
- Decides: 01 E17, 03 E14.

### A7. Erase with the eraser tool (tip), one gesture per type 0, 1, 2

- **Stock** (03 §3.3, §4.1) [code]: as A4 but without the eraser-end reader: `T`, SF paints with main
  style 8 (types 0/1) or 5 (type 2), `CU 0` at the first gesture's end only (it stays off after); lift
  → `X` (if erased), `P3`, partial, `P2` ≈ 300 ms. No `A2`, no `F`.
- **Tangleaf** (08 §5.5) [code; SF inferred]: no `State change` lines per gesture (steady 3);
  ` beforeScroll` + `A2` on vertical movement; after lift the reconcile frame; `F` ≈ 1–1.5 s later.
  If the page was opened with the eraser tool (pen state 1, 08 §5.2), note whether SF inks.
- Decides: 08 exp. 2, 03 E2 (eraser-tool variant).

### A8. Lasso tool: enter, lasso around nothing, lasso around one stroke

- **Stock** (04 §5.1–§5.5, §6.1) [code; order inferred]: entry `CU 1`, `P3`, partial, `P2` ≈ 500 ms.
  Lasso stroke: `T`, dashed SF trace. Nothing hit: `P3` (+ `X` if schema 7), a partial update of the
  page, `P2` ≈ 500 ms. Stroke hit: `P3` (+ `X`), partial updates (dirty rect, overlay), `ET 5`, `A2`
  (`EUS s 4` either at the transient apply or right after `Post enabled!` if the apply came while the
  scheme was still 3), popup appears, and **no** `P2`. With SF debug:
  `### applyTransientUpdateMode, updateMode 2308`.
- **Tangleaf** (08 §5.6) [code]: dashed SF trace with the session running; lift with a hit →
  configure → `P3`, then no `P2`. Lift without a hit: no `State change` (render stays on) (08 exp. 9).
- Decides: 04 E1, E2 (does any update cover the whole lasso path), E4.

### A9. Drag the selection with the stylus for 2 s, lift, wait 7 s; repeat with a finger

- **Stock** (04 §5.7) [code for absence of calls; panel inferred]: no `State change` lines; overlay
  frames with `waveform[4]` (transient 2308 active since A8). No new scheme switch. The session is
  paused (state R, brush flag 0). Prediction split as in A5: with 07, no `F`; with the handoff
  mechanism, `F` after the stylus drag but not after the finger drag (04 E3, E14).
- **Tangleaf** (08 §5.7) [code]: stylus: `APPLY_TRANSIENT_UPDATE(2308)` at down → `A2` (if the scheme
  was 2); Entry A ` beforeScroll` as well; after lift, ` afterScroll` → ≈ 1 s later `EUS s 2` and a
  full-screen update (`F` if `gcAfterScrolling` is true) — this also clears our own transient early
  [inferred, 07 §6]; then configure (`P3`-free, render already off), a reconcile frame; at +5 s our
  `CLEAR_TRANSIENT_UPDATE(0)` (no effect if the slot is already clear). Finger: no native call during
  the drag; Entry A applies to fingers (`beforeScroll`, `A2`, ≈ 1 s later `EUS s 2` + full-screen
  update) [inferred].
- Decides: 04 E3/E4/E14, 08 exp. 8, 07 E5.

### A10. Tap empty space to deselect, wait 7 s; select again and delete

- **Stock** (04 §5.8–§5.10) [code; ≈ times inferred]: `P3` (named pause), partial updates (old and new
  areas), `CU 0` (provider back to pen), `P2` ≈ 500 ms after the named resume; about 5 s after the
  release `EUS s 2 c 0` and a full-screen update (07 §5.3: mode 5; 05: no GC). With SF debug:
  `### clearTransientUpdateMode, clearFlag 0`. Delete: the removed area repainted while transient A2
  is still active (`waveform[4]` expected), then as the tap.
- **Tangleaf**: selection cleared by JS → configure → render back per tool (`P2` for the pen tool);
  partial updates from the reconcile.
- Decides: 04 E5, E10, E15 (put the pen down at ≈ 4.5 s and hold past 5 s: stock's clear should not
  come), E13 (turbo stays 5: look for `ET` lines).

### A11. Rectangle lasso: drag a rectangle with the stylus

- **Stock** (04 §5.3) [code]: no `P2` (render and input refused); dashed rectangle drawn by the app
  every ≥ 30 ms under `A2` (FM on at touch-down); FM off at up → 5 s later `EUS s 2` + full-screen
  update. Pen input in state P: split prediction as A5 (04 E6).
- **Tangleaf** (08 §5.6) [code]: steady 3; JS polygon; Entry A ` beforeScroll` / `A2` on vertical
  movement and `F` after lift [inferred].
- Decides: 04 E6.

### A12. Eraser end inside a shown selection (lasso tool, TRANSFORM)

- **Stock** (04 §5.11, 03 §3.7) [code app side]: no erase (`useDrawErase` false); session paused with
  eraser raw drawing on and brush off; SF paused → no trace [inferred]; split prediction as A5
  (04 E11b).
- **Tangleaf** (08 §5.4) [code]: inside the grab box, `APPLY_TRANSIENT_UPDATE(2308)` at down; the JS
  erase clears the selection → configure after lift; transient cleared 5 s later. Outside the box: no
  transient (08 exp. 13).
- Decides: 04 E11, 08 exp. 13.

### A13. Open the pen-width popup, wait 2 s, dismiss it; then dismiss and write within 300 ms

- **Stock** (05 §5.1–§5.2, 01 §5.4; 09 C2) [code; panel effect unknown]: show → `P3` (+ `X`), a
  partial update, `CU 1` if the popup turns CU on (pen tool does not own CU). Dismiss →
  `waveform_mode  wrong!` then an `onyx_tcon_send_cmd` with `waveform[0]` for the editor (the
  `REGAL_PLUS` one-shot, possibly carried by another invalidate), `CU 0`, `P2` ≈ 500 ms after dismiss.
  Writing within 300 ms: the stroke lands in state P (02 exp. 8) → split prediction as A5.
- **Tangleaf** (08 §5.10) [code]: open → `P3` and, because the reader is already paused, a second
  `P3` from `cycleScribbleSession` (07 §4.1); close by outside tap → ≈ 300 ms later a mode-98 GC
  (`update_mode[1]`, waveform expected 2) and ≈ 800 ms later `P2`. Close by choosing an option → no
  GC (08 exp. 7).
- Decides: 01 E5, 05 E3/E4, 08 exp. 7, 02 exp. 8, 01 E13 (open a second popup within 200 ms of the
  first dismiss: a brief `P2` between the two `P3`s would confirm that the resume sleep blocks the
  pen thread).

### A14. Rename the note (dialog with keyboard), type a letter, close

- **Stock** (01 §7.2) [code; focus path inferred]: dialog focus loss → INV → `P3`; keyboard →
  pause (no new line if already paused); on close focus regain → INV → `P2` ≈ 500 ms; rename ticks
  the counted GC (every 10th: `update_mode[1]`).
- **Tangleaf** (08 §5.10) [code]: overlay + text focus + IME → one pause transition (`P3`, second
  `P3`); keyboard hide → GC after 300 ms; overlay close → GC after 300 ms if `onOpenChange(false)`;
  `P2` after 300 + 500 ms.

### A15. Tap the empty canvas with one finger ten times

- **Stock** (06 E1) [code]: no `State change`, no `EUS`, no `update_mode[1]`.
- **Tangleaf**: no native pen call expected; WebView pointer handling only [inferred from 08 §8].

### A16. Two-finger scroll (zoomed in, or a scrollable page) for 3 s, lift, wait 7 s; pinch; write within 200 ms after a pinch

- **Stock** (06 §3.6, 05 §5.4; 09 C6) [code; timing inferred]: at scroll begin `ET 5`, `A2`, `P3`;
  `P2` ≈ 500 ms later while the fingers still move; A2 frames; at the end region re-push (no line);
  ≈ 5 s after lift `EUS s 2 c 0` and a full-screen update (mode 5 per 07; 05 predicts no
  `update_mode[1]`). Pinch: the same, plus `P3` / `P2` from the zoom-finish translate. Writing right
  after a pinch: resume is immediate (`resumeRawDrawingImmediately`), so the stroke should land in
  state D (06 E3); `T` then `X` at the next pause gives `EUS s 4` (fast slot still set).
- **Tangleaf** (08 §5.11) [code; framework inferred]: one `P3`/`P2` pair per scroll event whose args
  change; whole-WebView reconcile frames; Entry A applies to fingers: ` beforeScroll`, `A2`,
  ` afterScroll`, then `EUS s 2` + full-screen update ≈ `scroll_refresh_delay_v2` after lift. Pinch:
  not described in 08 (gap 3 in 09) — record what happens.
- Decides: 06 E2/E3, 05 E5, 08 exp. 4, 07 E5; C10 (what the mode-5 full-screen update looks like).

### A17. Turn pages ten times (swipe or page button)

- **Stock** (05 §5.3, 01 §7.2) [code; which frame carries the GC inferred]: each turn `P3`, partial
  update of the new page, `P2` ≥ 500 ms later; on the 10th turn (default settings) one
  `waveform[2] update_mode[1]`. If fast mode was on, an immediate `EUS s 2`.
- **Tangleaf** (08 §5.13) [code]: each switch `P3`, second `P3`, `P0` (reader stops; repeated 0s
  ignored), `P3`-class lines from the extra cycle, a mode-98 GC ≈ 300 ms later on every switch, then
  the new session (`P3`, `P1`, `P2`).
- Decides: 05 E2, 01 E10.

### A18. Undo once, redo once

- **Stock** (00 §3.10) [code]: `P3`, a partial update of the changed rect, `P2` ≈ 500 ms. No GC.
- **Tangleaf** (08 §5.12, exp. 6) [code]: `P3`, second `P3`, `P0`, GC ≈ 300 ms later, then
  `P3 P1 P2`. With the eraser tool selected before the undo: the reopened session stays at `P1` with
  no `P2`/`P3`; then write with the tip and watch for SF ink, `A2`, `F` (08 exp. 3).

### A19. Leave the page to the library

- **Stock** (01 §4.6, E12) [code]: `P3` (+ `X`), `P0`; `resetEpdPost` sends another 0 (ignored if the
  reader is already stopped). No GC request from the app; if fast mode was on, `EUS s 2` at once.
- **Tangleaf** (08 §5.13): as the close in A17 without the reopen; GC ≈ 300 ms later.

### A20. Press Home, wait 5 s, return; then open recents and return

- **Stock** (00 §3.12, 01 E6, E16) [code; focus path inferred]: focus loss → `P3`; the `onPause`
  save adds no line (already paused); no `P0`. Return → `P2` ≈ 500 ms after focus regain.
- **Tangleaf** (08 §5.14, exp. 12): `P3`, second `P3`, `P0`, GC ≈ 300 ms later (possibly while
  hidden); return → new session `P3 P1 P2` and a whole reconcile.

### A21. Stock only: fill-colour tool, erase a stroke with the eraser end

- (03 §3.7, E11; 01 E4) [code app side]: at tool selection `P3` and no `P2` (render refused, input
  on). Erase: no live trace. Split prediction as A5: with 07, no `A2`/`F`; with the handoff mechanism,
  `A2` and `F`.
- Decides: 03 E11 — the cleanest stock counterpart to Tangleaf's eraser tool.

### A22. Tangleaf only: pause with the debug command, then vertical, horizontal and wobbly strokes

Use `debug_ink_repaint` with the pause/toggle reason (handoff) to put the session in pen state 3,
then draw (a) one vertical stroke, (b) one strictly horizontal stroke, (c) one horizontal stroke
with 3–7 px of vertical wobble. Repeat (a) with the pen held still at the down point for 0.5 s first.

- Prediction (07 §6, E1, E4) [code for each gate; attribution inferred]:
  (a) `checkHasScrollCallback topScrollBounds …` at window start (once), ` beforeScroll`, `A2`,
  ` afterScroll`, then `F` ≈ `scroll_refresh_delay_v2` + 100–200 ms after lift;
  (b) no `beforeScroll` (Entry A needs |dy| > 8 px and |dy| > |dx|), no `A2` unless Entry B fires;
  (c) `A2` without a `beforeScroll` line would identify Entry B (WebView, ≥ 5 px, no pen gate).
- Also look for `SurfaceFlinger: hasHwTcon: 1` just before the full update: present → 05's
  `ExtBufferSchema` candidate; absent with `beforeScroll` present → 07's path.
- _(changes device state, user decides)_ variant: set Tangleaf's EinkWise profile to "Speed" and
  repeat (a): 07 predicts no `beforeScroll`-triggered `A2`/`F` (07 E6).
- Decides: C11 (09).

### A23. Stock only: floating toolbar, press and hold an item 2 s, release, write immediately

- (06 §3.10, E4) [code]: `P3` and `CU 1` at press; no `P2` at release; `P2` only after the action's
  own resume (≥ 500 ms). Writing before `P2`: split prediction as A5.

### A24. System full-refresh gesture/key while a note is shown

- **Stock** (05 E12, 02 exp. 14) [code app side]: `P3`, a partial mode-5 update, `P2` ≈ 500 ms; any
  `update_mode[1]` comes from the firmware side.
- **Tangleaf**: no handler for the broadcast [inferred from 08 §6.2]; record what the firmware does.

### A25. Stock only: draw with each shape tool (line, rectangle, circle) and each pen type

- (07 §8, E13) [code app side]: shapes go through `NormalShapeHandler` → `PenShape` → `MOVE_TO`
  transactions: `HandlePenTrigger` / `EUS s 3` can appear while the last state line is `toPause`. Note
  which pen types do this.

### A26. Stock only: the degenerate exclude rect

- (01 E7; 09 C5) [inferred]: with the widest pen, start strokes exactly at the top-left corner of the
  canvas (just below the toolbar). A missing ink box of about half the stroke width at the corner
  confirms that stock's `[Rect(0,0,0,0)]` exclude is stored and inflated.

### A27. Tangleaf only: edge cases from 08

- Pen tap on the floating selection menu (08 exp. 11): `P3` at the tap only if render was on; look
  for an ink dot.
- Scroll the sheet fully out of view and back (08 exp. 14): one `P3` and no `P2` while off screen.
- Freehand lasso that selects nothing, twice (08 exp. 9).

## 3. Deduplicated catalogue of open experiments

Every experiment proposed in 01–08, mapped to the action above that covers it. "—" means it needs a
setup the actions above do not provide.

| topic                                                                               | source experiments                                 | action                                                                                                 |
| ----------------------------------------------------------------------------------- | -------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| firmware identity, runtime properties, stored configs                               | 03 E10, 05 E0/E1, 07 E0/E2, 02 exp. 10, 08 exp. 10 | A0                                                                                                     |
| park on open, first resume                                                          | 01 E1                                              | A1                                                                                                     |
| live-ink scheme and waveform, schema 7 persistence, idle timeout                    | 02 exp. 1/2/5, 03 E5, 07 E7                        | A1, A2                                                                                                 |
| plain app frames during schema 7; Tangleaf reconcile rectangle and flags            | 02 exp. 3/12, 08 exp. 5                            | A2 (SF debug helps)                                                                                    |
| colour ink                                                                          | 02 exp. 6                                          | A3                                                                                                     |
| eraser end keeps render on in stock; timing of Tangleaf's pause                     | 01 E2, 03 E2, 06 E8, 07 E8, 08 exp. 1              | A4                                                                                                     |
| eraser detection (`BTN_TOOL_RUBBER`), digitizer device and hover reports            | 03 E1, 06 E6                                       | — (`getevent -lt`, root, read-only)                                                                    |
| post-erase window, style slot, `ENABLE_POST(1)` while DRAWING                       | 01 E3/E18, 03 E3/E4                                | A4, A5                                                                                                 |
| eraser-tool entry cycles                                                            | 01 E17, 03 E14                                     | A6                                                                                                     |
| eraser tool per gesture                                                             | 03 E2, 08 exp. 2                                   | A7                                                                                                     |
| session opened with a non-rendering tool (pen state 1)                              | 08 exp. 3                                          | A18                                                                                                    |
| lasso entry, trace removal, transient apply                                         | 04 E1/E2/E4, 01 E8 (shape-tool no-op pause)        | A8, A25                                                                                                |
| stylus vs finger drag of a selection; brush flag 0                                  | 04 E3/E14, 08 exp. 8, 07 E5                        | A9                                                                                                     |
| deselect, fast-mode release, turbo, pen held at 5 s, page turn with a selection     | 04 E5/E10/E13/E15/E16                              | A10, A17                                                                                               |
| rectangle lasso                                                                     | 04 E6                                              | A11                                                                                                    |
| eraser end in the lasso tool                                                        | 04 E11, 03 E13, 08 exp. 13                         | A12                                                                                                    |
| popup show/dismiss, REGAL frame, pen-thread blocking, writing within the resume gap | 01 E5/E13, 05 E3/E4, 08 exp. 7, 02 exp. 8, 04 E7   | A13                                                                                                    |
| colour CU                                                                           | 03 E8, 04 E12, 05 E6/E11, 06 E11                   | A4, A6, A8, A13, A23 (compare `flags` with and without `CU 1`)                                         |
| dialogs, keyboard, auto-save dialog                                                 | 02 exp. 11                                         | A14 (auto-save: set the shortest interval, write, move the pen away, wait)                             |
| finger tap, touch-down resume, locked zoom preview, hover suppression               | 06 E1/E7/E12/E13/E14                               | A15; E7/E12/E13/E14 need their own short runs (status bar pull, zoom preview lock, hover over toolbar) |
| scroll and pinch, fast-mode exit cost                                               | 06 E2/E3, 05 E5, 08 exp. 4                         | A16                                                                                                    |
| counted GC                                                                          | 05 E2, 04 E9, 01 E10                               | A17 (and repeated selection-menu clicks for 04 E9)                                                     |
| undo                                                                                | 08 exp. 6                                          | A18                                                                                                    |
| close, background, focus, screen off                                                | 01 E6/E9/E12/E16, 08 exp. 12                       | A19, A20 (screen off: press power, 5 s, unlock)                                                        |
| split screen, rich-text session                                                     | 01 E14/E15                                         | — (own runs)                                                                                           |
| fill-colour tool, universal-shape tool                                              | 03 E11/E12, 01 E4                                  | A21 (universal shape: same procedure with that tool)                                                   |
| flash source: Entry A vs Entry B vs ExtBuffer                                       | 07 E1/E3/E4/E6, 05 E7                              | A22                                                                                                    |
| pen state 4 and brush-raw-off in SF                                                 | 07 E9/E10, 03 E6/E7                                | — (need debug calls in Tangleaf; state changes)                                                        |
| floating toolbar press                                                              | 06 E4                                              | A23                                                                                                    |
| system refresh gesture                                                              | 05 E12, 02 exp. 14                                 | A24                                                                                                    |
| app-driven `MOVE_TO`                                                                | 07 E13                                             | A25                                                                                                    |
| degenerate exclude rect                                                             | 01 E7, 02 exp. 15                                  | A26                                                                                                    |
| overlays and exclude rects (stale native table)                                     | 06 E5                                              | — (open page navigator, write across it, close, write again)                                           |
| palm                                                                                | 06 E10                                             | — (palm on canvas while writing; `getevent` on the touch panel)                                        |
| material centre, EinkWise side effects                                              | 05 E8/E9/E10                                       | — (own runs; E8 needs a system dialog within 5 s of a pinch)                                           |
| meeting-scene hand-back, stray-ink paths                                            | 02 exp. 9/13                                       | — (own runs)                                                                                           |
| disable finger touch                                                                | 06 E9                                              | — (toggle in the toolbar)                                                                              |
| `EPDC+0x8` / `+0x280` consumers                                                     | 07 E12                                             | — (SF debug)                                                                                           |
| selection-menu GC cadence                                                           | 04 E9                                              | — (repeat a selection-menu action 20 times)                                                            |
| layer erase                                                                         | 03 E9                                              | — ("erase all layers" in stock)                                                                        |
| Tangleaf edge cases                                                                 | 08 exp. 9/11/14                                    | A27                                                                                                    |

## 4. What each outcome would settle

- **A4 + A5 + A21 + A9 + A11 in stock** decide C12: whether stock ever shows `A2`/`F` from pen input
  alone. None → the paused session alone does not trigger the flash for a `com.onyx` package, and
  the framework path (07) is the prime suspect for Tangleaf. Some → SF itself reacts to pen input in
  a paused session, and 07 §1 item 1 needs another look.
- **A22** decides which framework entry (or `ExtBufferSchema`) produces Tangleaf's flash, and whether
  the full update is the clear-flag-1 GC or something else (C10, C11).
- **A2** decides whether plain frames reach the panel during schema 7 and whether Tangleaf's
  `0x80002` frames are the ones that do.
- **A0** is done (2026-09-25): the tablet runs the analysed 4.2 build, SF and `framework.jar` are
  byte-identical, so the SF addresses in 01–08 apply (00-overview §0).
