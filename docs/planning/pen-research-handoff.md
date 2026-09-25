# Pen and panel research: where things stand (2026-09-25)

A handoff before restarting the research on the BOOX pen path from scratch. Read this first.

## Why restart

Too many changes to the pen path were written from a theory before the theory was checked, and
several were titled as fixes before any device test. The documents in `docs/onyx-reversing/` and
the planning notes below were produced by agents answering narrow questions about one symptom at a
time; none of them describes how the stock Onyx Notes app works end to end. The next phase is
research only: understand and document the stock behaviour completely, observe it on hardware,
and only then design ours.

## Verified on hardware and committed

- Dead rectangle in handwriting (`e7946ff`, `5fdf200`): two independent SurfaceFlinger latches,
  the exclude table and the handwriting schema. Fixed and verified.
- Ordinary-UI ghosting (`5fdf200`): the app used to write the "Speed" (A2) EinkWise profile, which
  disables the firmware's own upkeep. Now HD (`refresh_mode_4`, updateMode 0), written once.
- App-wide stall on journal capture (`1da2356`): structure reconcile rewrote every block. Fixed,
  96 ms instead of minutes. Slow-call and slow-SQL warnings added (`db94b02`).
- Our own navigation-counted full refresh removed (`c89bc5f`); the firmware's tap counter
  (Settings → Display → Full Refresh Frequency) now applies to us.

## Established facts that are not yet acted on

- **Flash after every erase — mechanism proven, fix not found.** With the panel driver log (below):
  if the pen writes while this app's SurfaceFlinger session is PAUSED (pen state 3, what
  `setRawDrawingRenderEnabled(false)` / `leaveScribbleMode` produce), SurfaceFlinger switches the
  panel to update scheme 4 and drives the input with A2 (`waveform[4] flag[0x2000]`); about 1.5 s
  after the pen lifts it switches back to scheme 2 with one full GC16 update
  (`waveform[2] update_mode[1] flags[0x1000]`) — the flash. Reproduced by pausing the session with
  the debug command and drawing a plain pen stroke. Frequent WebView redraws alone do not trigger
  it; pausing without pen input does not either. Our eraser gate (`InkEraserRenderGate`) turns
  render off for erase gestures, and `toolRendering()` keeps it off whenever the eraser tool (and
  the lasso tool, mostly) is selected — so erasing always happens with the session paused.
- An attempted fix (soft-eraser style 8 as in stock's `ResumeRawDrawingRequest`, render kept on)
  was wrong on device: the sheet turned pale, a wide faded trace appeared, and the mode did not
  exit. It was reverted; the diff is kept outside the repo in the session scratchpad as
  `uncommitted-eraser-experiments.patch`. Also in that patch, untested as fixes: skip the
  handwriting repaint mode for erase reconciles, keep `interacting` false during an erase, make
  `configure()` return early when nothing changed, and honest repaint diagnostics in `status()`
  (actual mode at submit time, erase flag, trigger).
- Stock Notes has `supportEAC: false` in the firmware config store; we have `true`. That is why
  stock gets no tap-counter flashes. Whether to opt out is an open design question
  (`docs/planning/eink-panel-upkeep.md`).
- Stock's per-stroke hardware reconcile (`GrayscaleRefreshAction`, the only user of
  `HAND_WRITING_REPAINT_MODE`) does not run on this device (colour panel and tablet both disable
  it); ours marks every stroke's reconcile frame with that mode.

## Tools that proved decisive

- **Panel driver log**, no root needed: `adb logcat -b all -v time` and look for
  `onyx_tcon_send_cmd` (every panel update with waveform, update_mode, flags),
  `onyx_tcon_set_upd_scheme` / `HWEpdcManager: ### EUS` (scheme switches), and
  `SurfaceFlinger: Pid N State change to: toPause/running` (our session pausing). `update_mode[1]`
  is a full-screen update; everything else is partial.
- **Thread dump of a live process** (root): `adb shell su -c "debuggerd -b <pid>"`.
- **Firmware config store** (root): `adb shell su -c "strings /onyxconfig/mmkv/onyx_config"`.
- **Repaint state**: `plugin:mobile-system|configure_onyx_ink` with `enabled: false` returns
  `status()`; `plugin:mobile-system|debug_ink_repaint` with `reason` = `sessionCycle`, `toggleRaw`,
  `excludeReset`, … drives one firmware path at a time. Both callable through the Tauri MCP
  bridge (`adb forward tcp:9223 tcp:9223`) as `window.__TAURI_INTERNALS__.invoke(...)`.

## Plan for the next phase

1. Broad research on the stock app, one agent per subsystem, each documenting the whole mechanism
   rather than answering a symptom: pen session lifecycle; writing; erasing in every mode;
   selection and lasso; panel refresh and EinkWise; finger, palm and input routing. Then one
   agent reconciles the documents into a single description and flags contradictions.
2. Observe stock on the device with the panel driver log for each action (write, erase with the
   stylus end, erase with the eraser tool, lasso, drag a selection, open and close a menu), then
   the same actions in our app, and compare line by line.
3. Only then design our behaviour from the documented stock behaviour, one change at a time, each
   verified against the driver log.

## Where everything is

- Reverse-engineering notes: `docs/onyx-reversing/` (01 sources and firmware, 02 regions, 03 render
  flag, 04 screennote pipeline, 05 refresh levers, 06 rooting, 07 EinkWise, 08 SurfaceFlinger).
- Planning notes: `docs/planning/handwriting-logic-review.md`, `eink-panel-upkeep.md`,
  `handwriting-pen-stack-comparison.md`, `backlog.md`.
- Scratch sources: stock Notes decompiled at `~/tmp_zfs/boox-notes-inspect/decoded/`, summary at
  `~/tmp_zfs/reversed_onyx_notes_app/REPORT.md`; framework, system app, services and native libs
  (including the extracted `surfaceflinger`) under `~/tmp_zfs/onyx_framework/`; SDK under
  `~/tmp_zfs/onyx_sdk_decompiled/`; open-source reference apps under
  `~/tmp_zfs/reference_notes_apps/`; firmware packages, boot backups and the EDL client under
  `~/tmp_zfs/boox_fw/`.
