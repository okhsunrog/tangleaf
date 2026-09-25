# 09 — Contradictions between the documents, resolutions, and gaps

Reconciliation pass over 01–08 in this directory and the older `docs/onyx-reversing/*.md`. For each
disagreement: what each side says, what the primary source shows, and the status. "Resolved" means
the source was re-read in this pass (citation given); "open" means only hardware or further
disassembly can decide, and the entry names the experiment in [10](10-observation-protocol.md).
Static analysis only.

Path prefixes: `N/` = `~/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/`,
`PUB/` = `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-pen-1.5.4.3/sources/com/onyx/android/sdk/`,
`SF@` = virtual address in `~/tmp_zfs/onyx_framework/native/surfaceflinger`,
`IMG/` = `~/tmp_zfs/boox_fw/imgs/`.

## 1. Contradictions

### C1. Android 11 or Android 13, firmware 4.1 or 4.2

- Brief: Android 11, firmware 4.1. `docs/onyx-reversing/README.md`, 01, 03, 07: Android 13, 4.2.
  02, 05: Android 11 (vendor fingerprint `…NoteAir4C:11/…`). 07 verification: probably the same
  build.
- Source: `IMG/vendor.img:/build.prop` has `ro.vendor.build.version.release=11`, sdk 30, fingerprint
  `Onyx/NoteAir4C/NoteAir4C:11/2026-04-28_17-50_4.2-rel_04282_555977efe/8548`;
  `IMG/system.img:/system/build.prop` has `ro.system.build.version.release=13`, sdk 33, fingerprint
  `qti/qssi/qssi:13/2026-04-28_17-50_4.2-rel_04282_555977efe/548` [code, read in this pass].
- **Resolved**: the extracted build is firmware 4.2 with an Android 11 vendor partition and an
  Android 13 system partition. Both labels are correct for their partition.
- **Closed on the tablet (2026-09-25, A0)**: the device runs this 4.2 build; SF, `framework.jar` and
  the pen native libraries are byte-identical to the analysed copies (00-overview §0). The brief's
  "Android 11, firmware 4.1" was an unverified statement and wrong.

### C2. Popup resume delay: 500 ms or 800 ms

- 01, 03, 04, 06: `POPUP_RESUME_PEN_TIME_MS` = 500 on colour (app `note/note/event/PenEvent.java:20`).
  05 §6, §9, §12.2: 800 on colour (`sdk/scribble/event/pen/PenEvent.java:18`).
- Source: `PenEventHandler` imports `com.onyx.android.note.note.event.PenEvent`
  (`N/note/note/eventhandler/PenEventHandler.java:14`) and uses `PenEvent.POPUP_RESUME_PEN_TIME_MS`
  at `:358` and `:444`. The SDK class with 800/400 is referenced only by
  `N/sdk/scribble/data/pen/BasePenEventHandler.java`, which has no subclass in the app (01 §5.5).
- **Resolved: 500 ms on colour, 300 ms on mono.** 05's correction was wrong.

### C3. Does pen state 0 (STOP) end the handwriting schema?

- 02 §5.6 ("Only state 3 ends the handwriting schema"), 01 §3.2 table (STOP without
  `stopHandwriting`), older `08-surfaceflinger.md` §3: no. 07 §4.1: yes.
- Source: `SF@0x566740` in the STOP branch is `b 0x5667e4`; `0x5667e4..0x5667f4` restores the frame
  and tail-branches to `0x558630` (`EpdcManager::stopHandwriting`), the same tail PAUSE uses
  [code, disassembled in this pass].
- **Resolved: STOP also runs `stopHandwriting`.**

### C4. Does stock send an empty exclude list on every resume?

- 02 §3 and §11 exp. 15, 06 §3.10: `AppTouchInputReader` sends any non-null list, so a resume
  without overlay slots sends an empty `SET_SCREEN_HANDWRITING_REGION_EXCLUDE`. 01 §4.4: the list is
  `[Rect(0,0,0,0)]`, never empty.
- Source: `NoteDocViewInfo.getExcludeRectList()` returns a list holding one `new Rect()` when no slot
  is registered (`N/sdk/notecore/editor/data/NoteDocViewInfo.java:51-65`);
  `EditorBundlesKt.getTouchExcludeRectList` starts from that list (`:315-338`); `PenEventHandler.q()`
  passes `CollectionKt.copy(...)` of it (`PenEventHandler.java:171`), and `copy` is a plain
  `toMutableList` (`N/sdk/extension/CollectionKt.java:346-349`) [code]. `UpdateRawDrawRectAction` uses
  the same getter.
- **Resolved**: the note session never sends an empty exclude list. Every resume and every overlay
  change sends at least one degenerate rect, twice (SF render leg, which skips only empty lists, and
  App render leg), offset by the view's screen origin. 06's "an empty list does clear SF's exclude
  table" applies to the SDK mechanics, not to what stock sends. Whether `HandWritingHandler`'s own
  exclude list (`z()`) can be empty was not checked.

### C5. What SF does with that one degenerate exclude rect

- 01 §4.4: disputed, **[unknown]**. Older `README.md`: a degenerate rect "cannot work" as a reset
  because the native test inflates every exclusion. Older `08-surfaceflinger.md` §2 ("The null view
  / {0,0,0,0} reset"): a one-rect payload stores one degenerate rect, inflated by `strokeWidth/2`,
  i.e. a small dead box; only an empty array (n ≤ 0) clears. Older `02-handwriting-regions.md:348`:
  same reading for the native reader. Older `README.md` also says the vendor's clearing call is
  `setScreenHandWritingRegionExclude(null, [Rect(0,0,0,0)])`, which 08-surfaceflinger contradicts
  (the true reset is the empty-array form).
- **Open.** The 08-surfaceflinger reading predicts a dead box of about half the stroke width at the
  `EditorView`'s top-left screen corner in stock (A26 in 10). The README's clearing idiom is
  contradicted by the later SF disassembly and should be treated as superseded [inferred].

### C6. Does a finger scroll pause the pen in stock?

- 01 §7.2 row "finger scroll": no pause found, pen state during scroll **[unknown]**. 06 §3.6: scroll
  begin runs FM on + `PauseResumeRawPenAction`.
- Source: `ViewportEventHandler.onViewportScrollBeginEvent` posts `ApplyFastModeEvent(true)` and runs
  `PauseResumeRawPenAction` (`N/note/note/eventhandler/ViewportEventHandler.java:112-117`) [code].
- **Resolved: it pauses, then resumes after 500 ms**, as for pinch. The event is posted only when
  `ScrollGesturesKt.canScroll` holds (06 §3.6).

### C7. What stops the 5 s fast-mode exit: a finger or the pen?

- 05 §5.4 and §9: "return if a finger is still down", "the exit waits for all fingers up". 02 §5.7,
  04 §5.8, 06 §3.6: the pen raw-touch flag.
- Source: `EpdEventHandler.e()` returns if `touchStateInfo(...).getG()`
  (`N/note/note/eventhandler/EpdEventHandler.java:145-150`); `TouchStateInfo.getG()` is the getter
  jadx renamed from `isPenRawTouch`, field `g`, set by `setPenRawTouch`
  (`N/sdk/notecore/editor/data/TouchStateInfo.java:24, 104-105, 193-194`) [code].
- **Resolved: the pen raw-touch flag.** A finger does not delay the exit.

### C8. Can fast mode be on while the pen or eraser tool is active?

- 03 §3.3: the raw-pen-up subscriber acts only in fast mode, "which is only possible for handlers
  with `renderInFastMode()` true (selection, universal shape)". 02 §5.7, 05 §5.4, 06 §3.6: fast mode
  is entered by scroll, pinch, drags for any handler.
- Source: `EpdEventHandler.onApplyFastModeEvent` enters fast mode on `enable` without checking the
  handler; only `k()` (the exit request) checks `renderInFastMode()` (`EpdEventHandler.java:185-190,
203-217`) [code].
- **Resolved: fast mode can be on with any tool** (for 5 s after a scroll or pinch). With the pen or
  eraser tool active, a raw pen-up inside that window re-arms the exit and marks the extra
  pause/resume.

### C9. Which handlers post `RawPenDownEvent` / `RawPenUpEvent`

- 04 §5.8: eraser strokes and shape-tool strokes, "not plain pen strokes". 02 §5.2/§5.4: the pen
  handler posts them.
- Source: `EpdShapeHandler.java:324` posts `RawPenDownEvent`, `:390` posts `RawPenUpEvent` [code].
- **Resolved: plain pen strokes post them too.**

### C10. What `clearTransientUpdate(false)` costs on the panel

- 05 §1, §5.4: "leaving fast mode costs no GC"; E5 expects no `update_mode[1]`. Older
  `07-einkwise.md` §5.4.6: `true` fires a full GC flash, `false` does not. 07 §5.3: the clear lambda
  also queues, unconditionally, a full-screen update in mode 5 (verified at `SF@0x5644cc-0x564580`).
  06 §6 cites "a free full GC hidden in `clearTransientUpdate`".
- Reconciliation: all agree that flag 0 queues no mode-98 GC. 07 adds a full-screen mode-5 entry.
  By 05 §4.4, `update_mode` in the driver line is bit `0x20` of the mode word, so a mode-5 entry
  should log `update_mode[0]` with waveform 255 (kernel's choice) [inferred]. 06's "free full GC"
  refers to the flag-1 case of the older document.
- **Partly resolved**: no GC for flag 0 [code]; a full-screen AUTO update exists [code]; how it
  looks on the panel and in the driver line is **open** (A10, A16 in 10).

### C11. Where the observed flash comes from

- Handoff: when the pen writes into a paused session, "SF switches the panel to update scheme 4",
  then ~1.5 s after lift scheme 2 and one full GC16 (`waveform[2] update_mode[1] flags[0x1000]`).
- 07: SF has no path from a pen state to scheme 4 [code]. The framework's
  `OnyxScrollRefreshManager` (Entry A, `ViewRootImpl`, skips `com.onyx` packages, exempts a stylus
  only when `getPenState()==2` at ACTION_DOWN) or `WebView.onTouchEvent` (Entry B, no pen gate)
  calls `ScrollHelper.beforeScroll` → `applyTransientUpdate(2308)` → scheme 4; `afterScroll` after
  `scroll_refresh_delay_v2` (default 1000 ms) → `clearTransientUpdate(gcAfterScrolling)` → scheme 2 +
  full-screen update [code for each link].
- 05 §8.2: `ExtBufferSchema::onDeactivate` always ends with a one-shot 98 (count 2) on an HW-Tcon
  device, a candidate for the post-lift GC; what activates that schema is **[unknown]**.
- Reconciliation [inferred]: the observed `update_mode[1]` and `waveform[2]` fit a mode-98 entry, not
  the mode-5 full-screen entry of 07 §5.3 (C10). Under 07's mechanism that requires
  `gcAfterScrolling` = true on the device (clear flag 1), which is a firmware config value not read;
  under 05's, an `ExtBufferSchema` activation. The two predict different log markers:
  `D OnyxScrollRefreshManager:  beforeScroll` / ` afterScroll` for 07, `SurfaceFlinger: hasHwTcon: 1`
  for 05.
- **Open** (A22 in 10). The handoff's causal phrasing ("SF switches because the session is
  paused") is not supported by the SF code; the correlation it observed is compatible with 07's
  mechanism, where pen state 3 removes the framework's stylus exemption.

### C12. Does stock flash when pen input lands in a paused session?

- 01 E3, 03 E4/E11, 04 H3/E6/E11, 06 E4 predict, following the handoff, that stock flashes in its
  paused-with-pen states (post-erase window, fill colour, stylus drag of a selection, rectangle lasso,
  eraser end in TRANSFORM, writing within the resume delay).
- 07 §1 item 3: stock is exempt from Entry A (`isOnyxApp`) and its canvas is not a WebView, so no
  framework scroll-A2 [code for the gate, inferred for the conclusion]. Stock does run its own
  transient A2 for scroll, pinch, drags and transforms, whose exit queues the full-screen mode-5
  update.
- **Open**, and decisive: if stock never shows `EUS s 4` + `update_mode[1]` from pen input alone, the
  07 mechanism is favoured; if it does, something inside SF (C11, 05's candidate) reacts to pen input
  in a paused session (A5, A9, A11, A12, A13, A21, A23 in 10).

### C13. What `ENABLE_POST(0)` does

- 01 §3.2: **[unknown]**; older `03-raw-drawing-render-flag.md` §1.3 infers a per-pid suppression;
  02 §10: can call `stopHandwriting` when `androidDrawing == 1`; 07 §4.4: inert.
- Reconciliation: `handleEnablePost(v)` calls `stopHandwriting` if `v >= 0 && androidDrawing != v`
  (02 and 07 agree at `SF@0x560c18-0x560c30`). With `v = 0` that happens only when
  `androidDrawing == 1`, i.e. outside schema 7, where `stopHandwriting` returns (`### not in
handwriting schema.`). **Resolved: no effect** [code]. The per-pid suppression reading of the older
  document has no support in the handler.

### C14. `REPAINT_EVERY_THING` and `stopHandwriting`

- Older `08-surfaceflinger.md` §1 groups them. 07 §2.1: provider `refresh` only, no
  `stopHandwriting` (`SF@0x56007c`, `0x4f68a4`). **Resolved per 07** [code there].

### C15. The "do not switch epd Schema in handwriting mode" guard

- Older `08-surfaceflinger.md` §3: schema 7. 07 §5.2: `FBDev` scheme == 3, and the slot is updated
  before the guard (verified in 07 at `SF@0x558d90-0x558da8`). **Resolved per 07.**

### C16. `vendor.onyx.tablet`

- 02 §7: true, read from the image. 05 §6, 06 §5: **[unknown]**, property not read.
- Source: `IMG/vendor.img:/build.prop` has `vendor.onyx.tablet=true` [code, this pass]. Not an `ro.`
  property, so the runtime value is [inferred] equal. **Resolved at image level.** The per-stroke
  reconcile is off on this device regardless of the stored `enablePenUpRefresh` (05 §5.8).

### C17. `vendor.onyx.htcon`

- 04, 05, 06: [inferred] true from the SF provider choice. 02: [code] from the image.
- Source: `vendor.onyx.htcon=true` in the same file [code, this pass]. **Resolved at image level.**

### C18. Scheme choice when handwriting ends

- 02 §5.6: from "the app's current EPD mode". 03 §6: from a mode read through `0x559ab0`/`0x559304`.
  07 §4.3: the fixed update entry passed through `applyDither`, and the return value is tested
  (verified in 07). **Resolved per 07**: the fixed (transient or app-scope) slot decides; 02 and 03
  describe the same function less precisely.

### C19. The eraser end on stock: no-op reader or SF eraser reader

- 07 first draft: stock's eraser end selects the no-op reader 3. 07 verification, 01, 03: every
  resume sends `SET_ERASER_RAW_DRAWING_ENABLED(1, 8 or 5)` for types 0–2, so reader 4 draws.
  **Resolved in 07's verification** [code]. The no-op state applies between a full pause
  (`resetPenDefaultRawDrawing`) and the next resume, and while the rich-text `HandWritingHandler`
  last touched the flags.

### C20. The eraser menu entry: one or two pause/resume cycles

- 01 §7.2: PR then RS(150), "whether an INV is also issued … not re-traced". 03 §3.2: two overlapping
  cycles (`EraseHandler.onActivate` 150 ms + `EraseByTypeAction.t()` INV 500 ms); one cycle for a type
  change or `StartEraseAction`. **Resolved per 03** (code cited there).

### C21. `0xff0021`

- Older `08-surfaceflinger.md` §1: repaint-with-mode. 05 §10: `setTrigger`; repaint-with-mode is
  `0xff0023`. **Resolved per 05** (jump table decoded there).

### C22. `setDisplayScheme` and `supportRegal`

- Older `05-panel-refresh-levers.md`: `setDisplayScheme` a lever; `supportRegal()` false. 05 §4.2,
  §10: `0xff0015` hits the common exit (no-op); `supportRegal` is a kernel ioctl, value **[unknown]**.
  **Resolved per 05.**

### C23. `08-tangleaf-current-path.md`: does Tangleaf's App render push regions?

- 08 §3 row `setLimitRect(limit, NO_EXCLUDES)`: "App render → `AppTouchInputReader` lists only".
- Source: public `AppTouchRender.setLimitRect(Rect, List)` calls `AppTouchInputReader.setLimitRectList`
  and `setExcludeRectList` (`PUB/pen/touch/AppTouchRender.java:270-273`), and both send
  transactions: `EpdController.setScreenHandWritingRegionLimit` (`PUB/pen/touch/AppTouchInputReader.java:146-149`)
  and, for any non-null list including an empty one, `setScreenHandWritingRegionExclude`
  (`:151-157`) [code, this pass].
- **Resolved, 08 corrected here**: each Tangleaf `setLimitRect(limit, NO_EXCLUDES)` sends LIMIT twice
  and one EXCLUDE with an empty array and view flag 1, which by `08-surfaceflinger.md` (`n <= 0`)
  clears SF's exclude table. The per-configure transaction counts in 08 §5.2, §5.11 and §6.1 are
  therefore one EXCLUDE short per `setLimitRect` call. 08 itself was not edited.

### C24. Finger drag "pause" in older docs

- Older `03-raw-drawing-render-flag.md`: finger drag-gesture down is a pause trigger. 06 §7: only for
  dragging float-menu / zoom-preview layouts through `DragGestureListener`; a canvas finger-down
  does not pause. **Resolved per 06.**

### C25. "An empty exclude list is discarded inside the SDK"

- Older `README.md`. 06 §7, 02 §3: true for the SF render half, false for the App render half.
  **Resolved per 06** (both bundled and public SDK, see C23). With mask 3 an empty list does reach SF.

### C26. SF reader numbering

- 03 and 07: SF has five readers (0 `BtnReader`, 1 `BaseDrawReader`, 2 `DrawReader`,
  3 `BaseEraseReader`, 4 `EraseReader`). 06 §3.2: four readers (0 `BtnReader`, 1 `DrawReader`,
  2 `EraseReader`, 3 `SideEraseReader`). Not a contradiction: 06 describes the in-process
  `libonyx_pen_touch_reader.so`, 03/07 describe `surfaceflinger`. Both are [code] in their binaries.

### C27. `RawPenDownEvent` from `EraseHandler` duplicated

- 03 §3.3: the eraser tool posts `RawPenDownEvent`/`RawPenUpEvent` twice. Not contradicted
  elsewhere; recorded because 04 §5.8 counts the sources differently (C9).

### C28. Pen-state-3 reasoning in the handoff vs the SDK guard

- 08 §1 item 2: the SDK skips a render call with the same value. 07 §4.1: SF compares a pen-state
  transaction with the **live** reader state, so a 3 sent while the reader is already paused (4) runs
  `pause()` + `stopHandwriting` again and logs another `State change to: toPause`. Not a contradiction,
  but it means Tangleaf's `cycleScribbleSession` after a render-off produces a second state-change
  line (used in the predictions of 10).

## 2. Claims that still rest on one reading

Not contradicted, but only one document looked, and they carry weight in the predictions:

- `hasHWRepaint` tests for exactly `0x80002` (02, disassembled). Tangleaf's reconcile mode is 524290
  = `0x80002`, so Tangleaf's reconcile frames are the kind SF commits in schema 7 [inferred link from
  the view mode to the layer entry word].
- The SF clear lambda's unconditional full-screen `addEpdc(…, 5)` (07, verified once).
- `ExtBufferSchema::onDeactivate` one-shot 98 count 2 (05, verified once).
- `EraseReader::onActivate` overwriting the single style slot (03, spot-checked).
- The `OnyxScrollRefreshManager` / `ScrollHelper` / `WebView` gates (07, whole files re-read in its
  verification).

## 3. Gaps nobody covered

1. **HWR paths**: hold-to-convert (`RxPenGesture` → `StartRecognizeDiagramAction`), scratch-out erase
   gestures, the HWR/placement handlers' pen and refresh behaviour, and how stray firmware ink from a
   discarded stroke is cleared (02 §5.4 step 4).
2. **Text input and the rich-text session**: `TextInputHandler` flow, `QuitFastModeImmediatelyEvent`
   GCs, and the second `TouchHelper` in `HandWritingHandler` (its SF effects were never traced; 01 §4.7,
   06 §3.13). Whether it can be open at the same time as the note session is unknown.
3. **Pinch zoom in Tangleaf**: 08 does not describe what a pinch does in the WebView (native zoom,
   JS gesture or blocked) or its pen-session effects.
4. **Live ink waveform and colour**: which waveform and flags SF uses for its own ink, and whether
   colour pens ink in colour on the Kaleido panel.
5. **Schema-7 output with plain frames**: who enqueues entries for `EpdcManager::next`; whether a
   plain mode-5 invalidate ever reaches the panel during schema 7.
6. **`ExtBufferSchema` activation**: which event activates it (05 §8.2).
7. **SF `PenManager+0x2054` and pen input while paused**: 07 names it the "paused" byte and says
   `BaseReader::onEvent` returns while it is set; 03 left its meaning open. Whether a paused SF
   reader still reports anything else (e.g. to the framework) is not traced.
8. **In-process reader state 5 after `nativePausePen()`** (06): whether hover events keep flowing
   while the reader is paused.
9. **Framework → SF update-mode path for hardware-accelerated frames**, especially Chromium's
   WebView frames in Tangleaf (`ViewRootImpl.addEpdc`), and whether the WebView's default update mode
   actually reaches SF as the layer entry word.
10. **`ENABLE_COLOR_CU`**: the panel effect of flag `0x400000` (05 §5.6).
11. **SF style 8 and style 5 painters** (`PainterSoftEraser`, `PainterDash`): what they draw.
12. **Cross-app leakage**: SF-global brush/eraser raw-drawing flags, turbo (stock sets 5 and never
    restores), dither threshold writes to the EinkWise store, transient slot. What Tangleaf inherits
    after stock ran, and vice versa, was not traced.
13. **Firmware focus-gain exclude reset**: older `README.md` says the firmware resets the exclude
    table on window-focus gain for recognised note views; none of 01–08 re-checked it.
14. **Screen off / on and the keyguard** in both apps.
15. **Auto-save dialog**: 02 §8 infers that the auto-save progress dialog pauses the pen; the dialog →
    pause link was not traced.
16. **Split screen and multi-window** in both apps beyond 01's unfocused-bind note.
17. **Stock's `supportEAC:false` vs Tangleaf's `true`** in the framework scroll path: 07 finds that
    `getEacScrollStyle()` ignores `supportEAC`, but the per-app `scrollRefreshDelay`, `gcAfterScrolling`
    and `enableA2Scrolling` values on the device were not read.
18. **Undo in stock** was traced only in this pass, at the action level (`UndoRedoAction`); the
    requests it runs were not checked for implicit pauses beyond the explicit `pauseArgs`.
