# Reference note apps on the Onyx Pen SDK, compared with stock Notes and Tangleaf

Five open-source apps that drive the Onyx Pen SDK, each documented in the terms of
[`../stock-notes/`](../stock-notes/README.md) so that they can be compared action by action with the
stock Notes app and with Tangleaf. Sources are the clones under `~/tmp_zfs/reference_notes_apps/`.
Everything here is **static analysis**; driver-log behaviour is predicted, not observed. The
predictions are to be checked with the protocol in
[`../stock-notes/10-observation-protocol.md`](../stock-notes/10-observation-protocol.md).

Device: Note Air 4C, firmware 4.2 (`2026-04-28_17-50_4.2-rel_04282_555977efe`), system Android 13,
vendor Android 11; SurfaceFlinger and `framework.jar` byte-identical to the analysed copies
(verified 2026-09-25, [`../stock-notes/00-overview.md`](../stock-notes/00-overview.md) §0).

| document                 | app       | SDK                                         | canvas                                       |
| ------------------------ | --------- | ------------------------------------------- | -------------------------------------------- |
| [notable.md](notable.md) | Notable   | pen 1.5.4.1, device 1.3.5.2                 | `SurfaceView`, SF render                     |
| [notate.md](notate.md)   | notate    | pen 1.5.4, device 1.3.4                     | `SurfaceView`, SF render                     |
| [saber.md](saber.md)     | Saber     | pen 1.5.4, device 1.3.4                     | Flutter; hidden dummy `SurfaceView` per page |
| [mokke.md](mokke.md)     | Mokke Ink | pen 1.5.2, device 1.3.3 (traced in 1.5.4.3) | `SurfaceView`, SF render                     |
| [pngnote.md](pngnote.md) | PngNote   | pen 1.4.1, device 1.2.9                     | `SurfaceView`; **SDK dead on this device**   |

PngNote's `device-1.2.9` does not know the `lito` platform and falls back to a device class with
empty pen methods, and its method lookup targets `android.view.View` where firmware 4.2 has them on
`ViewUpdateHelper`. As shipped it should not reach SurfaceFlinger at all on this tablet
(check: `Empty region detected when mapping!!!!!` in logcat on every `setLimitRect`).

## What stock has, and who else has it

"Paused" = the app's SurfaceFlinger session in pen state 3 while the pen touches the panel, the
proven precondition of the scheme-4/A2 + delayed full GC16 flash (see
`../../planning/pen-research-handoff.md`). All rows are per the documents; `?` = not settled
statically.

| stock capability                                                                        | stock                                       | Notable                                                                            | notate                                                          | Saber                                            | Mokke                                  | Tangleaf                                                          |
| --------------------------------------------------------------------------------------- | ------------------------------------------- | ---------------------------------------------------------------------------------- | --------------------------------------------------------------- | ------------------------------------------------ | -------------------------------------- | ----------------------------------------------------------------- |
| pen never lands on a paused session while erasing                                       | yes (render stays on)                       | yes, except 500 ms pause windows after tool switch, undo, page change, scroll step | no: stroke/pixel eraser, rectangle select, text tool pause      | no: eraser tool / eraser-end hover pause         | yes (no eraser tool; erase = gestures) | no: eraser tool and eraser end pause                              |
| no flash after erase (predicted)                                                        | yes (observed)                              | yes                                                                                | no — flashes 0.1–0.2 s after lift, caused by its own A2 request | probably (depends on framework scroll bounds)    | yes                                    | no (observed, ~1.5 s after lift)                                  |
| live eraser trace drawn by firmware                                                     | yes, style 8 / 5                            | yes, style 8 / 5; re-enabled after each resume                                     | only the lasso eraser's dashed trace                            | no                                               | no                                     | no (JS draws it)                                                  |
| erase modes                                                                             | stroke, soft, area, clear                   | eraser end, lasso eraser                                                           | stroke, pixel, lasso eraser                                     | stroke only                                      | scratch-out gestures                   | see 08                                                            |
| stylus eraser end                                                                       | yes                                         | yes                                                                                | yes, tool switched on hover before contact                      | yes (pauses on hover)                            | ignored                                | yes                                                               |
| no per-stroke panel refresh; screen handed back to the app later                        | yes (gated off by `vendor.onyx.tablet`)     | per-update `SurfaceView` entries in mode 524290 (lost after first erase)           | partial GC of the last stroke 2 s after hover exit              | pause+resume 1 s after last pen-up               | pause+resume pair                      | no: whole-WebView repaint in mode 524290 after every stroke       |
| one session per note, paused rather than closed, gated delayed resume with full re-send | yes (500 / 300 / 150 ms)                    | partly (150 / 500 ms resume with re-send after erase)                              | no (re-opens without close)                                     | no (closes on `onStop`)                          | no (closes on background, resize, …)   | no: closes and reopens on undo, page switch, leave, background    |
| fast mode for drags/scroll, cleared 5 s later without GC                                | yes                                         | A2 during finger gestures, cleared with a GC (flash on purpose)                    | mode 4 / 2308, cleared at once with a GC                        | no                                               | no                                     | 2308 on selection drag, cleared after 5 s                         |
| full refresh only on 10th page action / menu click, Refresh button, text fast-mode exit | yes                                         | GC after scroll, zoom, deselect                                                    | GC after almost every UI action                                 | GCs hit the hidden view, never the visible layer | none                                   | GC after every overlay close and keyboard hide, page switch, undo |
| exempt from EinkWise (`supportEAC:false`) and framework scroll-A2                       | yes (firmware lists it; `com.onyx` package) | no                                                                                 | no                                                              | no                                               | no                                     | no                                                                |
| exclude regions for floating UI                                                         | one degenerate rect, never empty            | toolbar strip (older report)                                                       | toolbar and sidebar as real excludes                            | none; SF ink over toolbar, dialogs, keyboard     | never writes the table (inherits)      | cleared on every resume; pen taps on overlays swallowed           |

## Techniques the documents flag as worth studying

Observations, not proposals; each needs the hardware log first.

- Keeping render on for erasing and letting SF's eraser reader draw the trace, re-enabling the eraser
  channel after every resume (Notable, stock).
- `ENABLE_POST(1)` to leave handwriting mode before posting a repaired frame, without pausing first
  (Notable, PngNote, stock's erase hand-back).
- Handing the screen back to the app only after the pen goes idle (Saber 1 s after pen-up, notate 2 s
  after hover exit) instead of a repaint per stroke.
- Switching to the eraser tool on hover of the eraser end, before contact (notate).
- Real exclude rects for floating UI (notate).
- Erasing decided after lift from ordinary strokes, so the session never pauses during input (Mokke).
- A reference-counted owner for the single transient-update slot (Notable).
