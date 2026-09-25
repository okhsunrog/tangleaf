# Stock Notes research: how the BOOX Notes app drives the pen and the panel

Research, begun 2026-09-25, into how the stock Onyx BOOX Notes app (`com.onyx.android.note`,
"knote2") drives the pen session and the e-ink panel on a BOOX Note Air 4C. It also records
Tangleaf's current path in the same terms. The goal is a complete description with a source for
every claim, observed on hardware before anything in Tangleaf changes. These documents contain no
fixes and no design proposals.

Start with [00-overview.md](00-overview.md). Then read [09](09-contradictions-and-gaps.md) for what
is disputed or missing, and [10](10-observation-protocol.md) for the next step on the device.

## Documents

| file                                                                 | content                                                                                                                                                                                       |
| -------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [00-overview.md](00-overview.md)                                     | the reconciled end-to-end description: the session state machine, every user action's call sequence and SurfaceFlinger-visible effect, a stock vs Tangleaf table, and the list of divergences |
| [01-pen-session-lifecycle.md](01-pen-session-lifecycle.md)           | session creation, open and park, pause and resume with the `PenEventHandler` gates, close, a second `TouchHelper` for rich text, and the trigger → transaction table                          |
| [02-writing.md](02-writing.md)                                       | one stroke end to end: SF live ink, handwriting schema 7, pen-up handling, the reconcile that is gated off, colour                                                                            |
| [03-erasing.md](03-erasing.md)                                       | every eraser mode and tool; SF's eraser reader and style slot; the post-erase pause and resume                                                                                                |
| [04-selection-and-lasso.md](04-selection-and-lasso.md)               | lasso types, hit test, TRANSFORM, fast mode, deselection, menu actions                                                                                                                        |
| [05-panel-refresh-and-einkwise.md](05-panel-refresh-and-einkwise.md) | the app's refresh policy, `InkRenderManager`, counted GC, fast mode, dead code, EinkWise and `supportEAC:false`, SF update schemes                                                            |
| [06-input-routing.md](06-input-routing.md)                           | the two input channels, finger gestures, palm and hover, eraser end and side button, overlays and exclude rects                                                                               |
| [07-firmware-state-machine.md](07-firmware-state-machine.md)         | the SurfaceFlinger and framework state machine: pen states, schema 7, schemes 2/3/4, fixed slots, the framework scroll-A2 path behind the flash                                               |
| [08-tangleaf-current-path.md](08-tangleaf-current-path.md)           | Tangleaf's current pen path at commit `e078293`, described action by action in the same terms                                                                                                 |
| [09-contradictions-and-gaps.md](09-contradictions-and-gaps.md)       | contradictions between the documents and the older notes, their resolution against the sources, and the gaps                                                                                  |
| [10-observation-protocol.md](10-observation-protocol.md)             | the hardware observation plan: capture setup, grep patterns, actions A0–A27 with the predicted driver-log sequences for stock and Tangleaf, and all open experiments, deduplicated            |
| [11-observations.md](11-observations.md)                             | what was recorded on the tablet on 2026-09-25 with the driver log and the call tracer, including the cause of the flash after erasing                                                         |
| [12-eraser-menu.md](12-eraser-menu.md)                               | the stock eraser menu: types, diameters in mm, toggles, layout and storage, with what Tangleaf needs to reproduce                                                                             |
| [13-eraser-geometry.md](13-eraser-geometry.md)                       | how each stock eraser type decides what it removes, compared with Tangleaf's hit tests                                                                                                        |
| [15-pen-menu.md](15-pen-menu.md)                                     | the stock pen slots and pen menu: pen types and their SF styles, widths, pressure, colours, storage, with what Tangleaf needs to reproduce                                                    |
| [14-page-storage.md](14-page-storage.md)                             | how stock stores pages and strokes: page size in device pixels (1860×2480 here), point file layout, per-page fit, and what it means for Tangleaf's page size                                  |

Documents 01–08 were each written by one research agent and then corrected by an independent
verifier. Each ends with its own "Verification notes". Document 00 and documents 09–10 come from a
reconciliation pass that re-read the sources wherever 01–08 disagreed. In that pass 08 was not
edited; its one correction is recorded in 09 C23.

## Sources

| prefix used in the documents          | location                                                            | what it is                                                                                                                |
| ------------------------------------- | ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `N/`, `app/`, `sdk/`, `NA/`, `stock/` | `~/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/`    | stock Notes APK, jadx output; `note/…` is the app, `sdk/…` the SDK copy bundled in the APK (the copy stock actually runs) |
| —                                     | `~/tmp_zfs/reversed_onyx_notes_app/src`                             | a second decompilation of the same APK, used for spot checks                                                              |
| `res/`, `RAW/`                        | `~/tmp_zfs/boox-notes-inspect/decoded/resources/res/`               | resources, including the per-model `note_noteair4c.json`                                                                  |
| `P/`, `PUB/`, `pen/`, `dev/`          | `~/tmp_zfs/onyx_sdk_decompiled/src/`                                | the public Onyx SDK (`onyxsdk-pen-1.5.4.3`, `onyxsdk-device-1.3.5.2`) that Tangleaf links                                 |
| `FW/`, `fw/`, `aosp/`, `fwv/`         | `~/tmp_zfs/onyx_framework/out/sources/`                             | firmware `framework.jar` (`android.onyx.*`, patched `View`, `ViewRootImpl`, `WebView`)                                    |
| `fws/`                                | `~/tmp_zfs/onyx_framework/out-services/sources/`                    | `services.jar`                                                                                                            |
| `kcb/`                                | `~/tmp_zfs/onyx_framework/out-kcb/sources/`                         | the `com.onyx` system app                                                                                                 |
| `SF`, `sf`, `SF@0x…`                  | `~/tmp_zfs/onyx_framework/native/surfaceflinger`                    | extracted SurfaceFlinger; symbols from its `.gnu_debugdata`, addresses are virtual addresses                              |
| `PTR@0x…`, `native/`                  | `~/tmp_zfs/onyx_framework/native/libonyx_pen_touch_reader.so`       | the in-process pen reader loaded by the SDK                                                                               |
| `IMG/`, `vendor.img`                  | `~/tmp_zfs/boox_fw/imgs/`                                           | firmware 4.2 build 8548 partition images (`vendor.img` Android 11, `system.img` Android 13)                               |
| `kt/`, `hw/`, `app/` (in 08)          | this repository                                                     | Tangleaf's plugin (`plugins/tauri-plugin-mobile-system/…`) and frontend (`src/…`)                                         |
| —                                     | `docs/onyx-reversing/*.md`, `docs/planning/pen-research-handoff.md` | the earlier notes and the handoff; used as context and checked against the sources (09)                                   |

Everything is static analysis. The one class of hardware fact used is the driver-log behaviour the
handoff recorded (the flash after pen input into a paused session). Every SF address belongs to the
extracted 4.2 binary, which is byte-identical to the one on the tablet (verified 2026-09-25, action
A0; see [00-overview §0](00-overview.md)).

## Tagging convention

Every claim carries one tag:

- **[code]**: read in source or disassembly at the cited file and line, or at the cited address.
- **[inferred]**: reasoned from code that was read, but not read directly. The reasoning is
  written out.
- **[unknown]**: cannot be settled statically. Where it matters it becomes an experiment.

Some documents add **[prior doc]** / **[doc]** for a claim taken from an earlier document without
re-checking it. Line numbers are jadx line numbers of the file named. Kotlin classes carry a long
`@Metadata` line near the top, so their numbers look shifted compared with other tools. Method names
such as `a()` or `q()` are jadx's renamed private methods. Where the Kotlin metadata gives the
original name, it is written next to them.

## Vocabulary

- **Pen state** 0 STOP, 1 START, 2 DRAWING (SF log `running`), 3 PAUSE (`toPause`), 4 `PEN_ERASING`
  in the SDK (a no-op in SF).
- **Render / input flags**: `TouchHelper`'s cached `setRawDrawingRenderEnabled` /
  `setRawInputReaderEnable`. The render flag maps to pen state 2/3 plus `ENABLE_POST(1)` on
  render-off. The input flag only affects the in-process reader.
- **Schema 7**: SF's handwriting schema, entered on the first inked sample (`HandlePenTrigger`,
  `### EUS, s 3`). It is left on pause, stop or `ENABLE_POST(1)` (`Post enabled!`).
- **Scheme 2/3/4**: the EPDC update scheme (`### EUS, s N`). 3 is handwriting. 4 means a fast (A2
  or DU class) mode sits in a fixed update slot. 2 is normal.
- **Fixed slots**: the app-scope and transient update-mode slots in SF
  (`applyAppScopeUpdate`, `applyTransientUpdate`).
- **Fast mode**: stock's `EpdEventHandler` state. It sets transient A2 (2308), turbo 5 and dither
  160, and leaves them 5 s after the gesture ends.
- **PR / RS(d) / INV / FM**: shorthands defined in 00 §2.1.
