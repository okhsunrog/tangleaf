# Stock Notes: panel refresh policy and EinkWise

Subsystem report 05 of the stock-Notes research. Scope: every place where the stock BOOX Notes app
(`com.onyx.android.note`, "knote2", versionCode 45326, `c2a16c15d32`) asks for a panel update or
changes display policy, what each request becomes in the framework and in SurfaceFlinger, which of
them run on a Note Air 4C, and what `supportEAC:false` changes for the app. Static analysis only; no
device was touched.

Evidence tags used on every claim:

- **[code]**: read in source or disassembly at the cited location.
- **[inferred]**: reasoned from code, not read directly.
- **[unknown]**: not decidable statically; see §11 for the experiment.
- **[prior doc]**: taken from an earlier report in `docs/onyx-reversing/` and not re-verified here.

## Source shorthands

| Shorthand | Path                                                                                                                                         |
| --------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `app/`    | `~/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/note/` (stock app code)                                                       |
| `sdk/`    | `~/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/sdk/` (SDK bundled in the stock APK)                                          |
| `res/`    | `~/tmp_zfs/boox-notes-inspect/decoded/resources/res/raw/`                                                                                    |
| `fw/`     | `~/tmp_zfs/onyx_framework/out/sources/android/onyx/` (framework.jar)                                                                         |
| `aosp/`   | `~/tmp_zfs/onyx_framework/out/sources/android/` (framework.jar, patched AOSP classes)                                                        |
| `kcb/`    | `~/tmp_zfs/onyx_framework/out-kcb/sources/` (the `com.onyx` system app)                                                                      |
| `SF`      | `~/tmp_zfs/onyx_framework/native/surfaceflinger`; addresses are virtual addresses, symbol names come from its `.gnu_debugdata` MiniDebugInfo |

A second decompilation of the same APK (`~/tmp_zfs/reversed_onyx_notes_app/src`) was spot-checked:
`EpdEventHandler.java` is byte-identical between the two.

**Firmware caveat.** The framework, `kcb` and SurfaceFlinger dumps were taken from build 8548,
`2026-04-28_17-50_4.2-rel` (`docs/onyx-reversing/01-sources-and-firmware.md:4-5`). The task brief
said the device runs firmware 4.1; that was wrong. Verified on the tablet 2026-09-25 (E0 done, see
[00-overview §0](00-overview.md)): it runs this 4.2 build, the system partition is Android 13 (SDK 33)
and the vendor partition Android 11 (SDK 30) — which is why Onyx fingerprints carry `NoteAir4C:11/`.
The "Android 13" in the earlier reports is correct. The device's SurfaceFlinger and `framework.jar`
are byte-identical to the dumps, so the framework/SF analysis applies.

---

## 1. Summary

1. **Stock's app code issues no full-update request for writing, erasing, scrolling or zooming.**
   (Whether SurfaceFlinger adds one on its own, e.g. at the end of a pen session, is a separate
   question, §8.2.) Its whole repaint policy on a Note Air 4C is:
   - plain, unbracketed `View.invalidate()` calls on the editor view whenever content is redrawn
     (after every stroke, after erase/undo/page render, via `RenderManager.invalidate()` /
     `InkRenderManager.invalidate()`); mode 5 unless a one-shot is pending (§5.1a);
   - an ordinary full-view `View.invalidate()` of the editor view (default mode 5 = AUTO, a partial
     update), bracketed by a raw-pen pause and a delayed resume (500 ms on a colour panel);
   - on some of those, a one-frame `REGAL_PLUS` tag (mode 9), which the HW-Tcon colour path of
     SurfaceFlinger rejects as an invalid waveform and sends with waveform field 0 (§4.4);
   - one full GC16 on every 10th "screen change" (page turn, selection-menu click, text-menu click,
     rename), plus the toolbar Refresh button;
   - a transient A2 "fast mode" (`applyTransientUpdate(2308)`) during pinch, scroll, drags and
     transforms, released 5 s after the gesture ends with `clearTransientUpdate(false)`. The `false`
     flag means leaving fast mode costs no GC.
     [code, §5]
2. **The per-stroke handwriting reconcile (`GrayscaleRefreshAction`, the only user of
   `HAND_WRITING_REPAINT_MODE`) does not run on this device.** It is gated three times: by the
   `vendor.onyx.tablet` property, by `enablePenUpRefresh = !isColorDevice()`, and by the settings
   toggle being hidden (`showPenUpRefresh:false` in `note_noteair4c.json`, and hidden on any HW-Tcon
   device). [code, §5.8]
3. **`repaintEveryThing(GC)` in the editor is dead code.** Its only editor callers hang off
   `RefreshPageEvent`, which nothing in the APK constructs. Several other EPD calls are also
   unreachable (§5.8). The library screen's Refresh button is the one live
   `repaintEveryThing(GC)`. [code]
4. **Stock never delegates refresh to EinkWise at runtime.** The only EAC-writing code
   (`UpdateModeEventHandler`) is unreachable. Stock does write into its own EAC config indirectly:
   every fast-mode entry and exit calls `EInkHelper.setDitherThreshold(160/128)`, and the OEC service
   persists that value into stock's app config **and into the device-wide fallback display config**.
   [code, §7.3]
5. **What `supportEAC:false` changes for stock** (§7): the per-input-event pipe message to
   `OECService` is suppressed in stock's own process, so the tap counter behind "Full Refresh
   Frequency" is never fed; SurfaceFlinger's debouncer (post-input quality repaint and counted GC)
   is cleared on every resume; the in-app EAC getters (paint, refresh, note, DPI, rotation, extra
   config) return dummies; DPI override is disabled. It does **not** stop `OECService` from applying
   stock's stored refresh block (app-scope slot, turbo, anti-flicker, dither) and display block
   (gamma, CFA saturation and brightness, dither threshold, enhance) on every activity resume. In
   addition, being an Onyx package (`"com.onyx"` substring) independently excludes stock from the
   automatic scroll-refresh manager and from the per-canvas EAC paint hooks. [code]
6. **SurfaceFlinger update scheme.** Every `HWEpdcManager: ### EUS, s N c M` line comes from
   `HWEpdcManager::setEbcUpdateScheme`, whose only callers are `setFBSchema`, `syncFBSchema` and
   `clearHandwritingFBSchema`. All three pick scheme **4 if the app-scope or transient slot holds a
   "fast" mode, otherwise 2**. `ExtBufferSchema::onDeactivate` always ends with a full GC (mode 98).
   That makes it a candidate source of the post-pen-lift flash; which pen path activates
   ExtBufferSchema is **[unknown]**. [code, §8]
7. On this device the driver log's `update_mode[1]` is exactly bit `0x20` (`EINK_UPDATE_MODE_FULL`)
   of the mode word, as decoded in `FBDev::hwScreenRefresh`. So `UI_GC_MODE` (98 = `0x62`) is the
   full update, and `UI_GU_MODE` (2), AUTO (5), A2 (2308) and `HAND_WRITING_REPAINT_MODE` (524290)
   are partial. [code, §4.4]

---

## 2. Central classes and entry points

| Class / file                                                                                      | Role in refresh policy                                                                                                                         |
| ------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `app/note/eventhandler/EpdEventHandler.java`                                                      | Owns "fast mode" (transient A2, turbo, dither threshold) and the 5 s exit timer; subscribes to all fast-mode events                            |
| `app/common/utils/EpdDeviceManager.java`                                                          | The app's own counted full-GC ("full refresh every N")                                                                                         |
| `app/note/action/render/InvalidateScreenAction.java`                                              | Standard "repaint the editor" pipeline: optional REGAL_PLUS one-shot, optional delay, posts `InvalidateViewWithPenControlEvent`                |
| `app/note/action/render/InvalidateViewWithPenControlAction.java`                                  | Pause raw pen → `InkRenderManager.invalidate()` → resume raw pen after a delay                                                                 |
| `sdk/notecore/editor/display/InkRenderManager.java`                                               | The actual repaint: set the one-shot view update mode, `View.invalidate()`, reset the mode                                                     |
| `sdk/api/device/epd/EpdController.java` (bundled)                                                 | Static facade; every call goes to `Device.currentDevice()`                                                                                     |
| `sdk/api/device/EpdDeviceManager.java` + `EpdSDM.java` (bundled)                                  | `setUpdateMode` / `resetUpdateMode` wrappers used by `InkRenderManager`                                                                        |
| `sdk/device/SDMDevice.java` (bundled)                                                             | Reflection into `android.onyx.ViewUpdateHelper`, `View` and `EInkHelper`; `UpdateMode` → int mapping `h()`                                     |
| `fw/ViewUpdateHelper.java`                                                                        | Parcels each request into a SurfaceFlinger binder transaction                                                                                  |
| `fw/optimization/EInkHelper.java`, `OECService.java`, `impl/Tablet*Impl.java`                     | EinkWise (EAC): per-app config, applied on resume by `system_server`                                                                           |
| `SF` `SurfaceFlingerHelper::processOnyxRequests` @ `0x55f260`                                     | Transaction demux; `EpdcManager`/`HWEpdcManager` schema, fixed slots, update scheme                                                            |
| `res/note_noteair4c.json`, `sdk/note/ui/config/DeviceConfig.java`                                 | Per-model app config: `epdTurbo`, `showPenUpRefresh`, `isTabDevice`, `transformUpdateModel`                                                    |
| `sdk/scribble/device/DeviceNoteConfig.java`, `sdk/scribble/data/config/DeviceNoteConfigBean.java` | Per-model `editorViewUpdateModel` (the one-shot mode): no NoteAir4C file exists, so the default `REGAL_PLUS` (bean constructor, `:39`) applies |

`Device.currentDevice()` resolves to `SDMDevice` on this device: `Device.b()` maps board platform
`lito` to `DeviceIndex.SDM` (`sdk/device/Device.java`, map at the `BOARD_PLATFORM_LITO` put), and
`detectDevice()` returns `SDMDevice.createDevice()`. [code; that `ro.board.platform` is `lito` is
**[prior doc]**, `01-sources-and-firmware.md:4`]

---

## 3. The app-level policy in one table

Rows are grouped by mechanism. "NA4C" says whether the row runs on a Note Air 4C and why.
Mode numbers are the ints that reach SurfaceFlinger (§4.1). "Full?" means `update_mode[1]` in the
driver log (§4.4).

| #   | Event                                                                                                                                                                                                                                      | App call                                                                                                                                                               | Mode reaching SF                                                         | Full?                                      | NA4C                                                                                                                                                 |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ | ------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| P0  | Content redraw: stroke flushed to the page cache, erase/undo re-render, page render, scroll/zoom display (`RenderManager.invalidate()` / `InkRenderManager.invalidate()` / `displayBoxList`)                                               | plain `View.invalidate()` on `EditorView`, **no pen pause/resume**                                                                                                     | 5 (or 9 if a REGAL one-shot is pending, §5.1)                            | no                                         | yes [code: `RenderManager.java` many `invalidate()` sites, `InkRenderManager.java:47-54,79-97,138-141`; see 02-writing §5.4 for the per-stroke call] |
| P1  | Erase finished, before A4: re-render of the changed slices                                                                                                                                                                                 | `forceRenderVisibleDirtyScreenRectToDisplay(..., DisplayArgs{invalidate=false, enablePost=true})` → `InkRenderManager.invalidate` → `EpdController.enablePost(view,1)` | none (no view invalidate); `ENABLE_POST(-1,1,pid)`                       | —                                          | yes [code: `EraseFinishAction.java:343-370`, `ForceRenderSliceListActionsKt.java:51-55`]                                                             |
| A1  | Any `InvalidateScreenAction` (focus change, popup show, toast, tool change, float-menu move, system `REFRESH_SCREEN` broadcast, …; list in §5.2)                                                                                           | pause raw pen (render+input) → `InkRenderManager.invalidate()` → resume after 500 ms                                                                                   | per-frame view mode **5** (AUTO → waveform 255 on the HW path)           | no                                         | yes [code]                                                                                                                                           |
| A2  | A1 with `setCheckUseRegalMode(true)` (popup **dismiss**), page goto/insert, status-bar change, menu style change, catalog/thumbnail close, page-margin dialog                                                                              | `enableRegalModeOnce()` then A1; `EpdController.enableRegal()` + `setViewDefaultUpdateMode(REGAL_PLUS)`                                                                | per-frame **9** (`UI_REGAL_PLUS_MODE`)                                   | no                                         | runs; SF HW path logs "waveform_mode wrong" for nibble 9 [code SF; visual effect unknown]                                                            |
| A3  | Settings/template sub-fragment closes (`RegalInvalidateWithPenControlEvent`)                                                                                                                                                               | `InvalidateViewWithPenControlAction(DisplayArgs.enableRegalDisplayArgs())`                                                                                             | 9                                                                        | no                                         | yes, as A2                                                                                                                                           |
| A4  | Erase finished (stroke or area eraser)                                                                                                                                                                                                     | `InvalidateViewWithPenControlAction(pause+resume, resume delay 300 ms)` + `resetViewUpdateMode(editorView)`                                                            | 5                                                                        | no                                         | yes (details in the erasing report)                                                                                                                  |
| A5  | Lasso pen-up while not moving                                                                                                                                                                                                              | `InvalidateScreenAction.setDelayRefreshTime(300)`                                                                                                                      | 5                                                                        | no                                         | yes                                                                                                                                                  |
| A6  | Any A1–A5 while a selection is non-empty                                                                                                                                                                                                   | `SelectionHandler.onInvalidateViewWithPenControl` drops it                                                                                                             | none                                                                     | —                                          | yes                                                                                                                                                  |
| G1  | Page turn, add, delete, goto (every `BasePageAction.p`)                                                                                                                                                                                    | `EpdDeviceManager.applyGCWithInterval()`; on the 10th tick `byPass(0)`, `applyGCOnce()`, then A1                                                                       | one-shot **98** (`UI_GC_MODE`) on the next composed frame; then 5        | **yes** (1 in 10)                          | yes if `enableFullRefresh` (default `isColorDevice()` = true) [code; stored value unknown]                                                           |
| G2  | Selection-menu click, text-input-menu click, rename note                                                                                                                                                                                   | same counter                                                                                                                                                           | 98 every 10th                                                            | yes (1 in 10)                              | yes                                                                                                                                                  |
| G3  | Toolbar "Refresh" button                                                                                                                                                                                                                   | `applyGCImmediately()`                                                                                                                                                 | 98 now                                                                   | **yes**                                    | yes                                                                                                                                                  |
| G4  | `QuitFastModeImmediatelyEvent(true)` (text input start/finish), **only if fast mode is active and the active handler does not render in fast mode**                                                                                        | leave fast mode + `applyGCImmediately()`                                                                                                                               | 98                                                                       | **yes**                                    | yes [code: `EpdEventHandler.java` `onQuitFastModeImmediatelyEvent`, guard `!renderInFastMode() && this.b`]; outside fast mode the event does nothing |
| F1  | Pinch/scroll begin, float-menu or widget drag, selection transform, rectangle-selection drag, zoom preview drag, canvas seekbar, thumbnail list, universal-shape edit                                                                      | `ApplyFastModeEvent(true)` → `setDitherThreshold(160)`, `setEpdTurbo(5)`, `applyTransientUpdate(ANIMATION_QUALITY)`                                                    | transient slot **2308** (`UI_A2_QUALITY_MODE`) → **scheme 4**            | no                                         | yes                                                                                                                                                  |
| F2  | Gesture end                                                                                                                                                                                                                                | `ApplyFastModeEvent(false)` → counter reset, 5 s timer + wakelock, `PenEvent.resumeRawDrawingImmediately()`                                                            | —                                                                        | —                                          | yes                                                                                                                                                  |
| F3  | 5 s after F2, no finger down                                                                                                                                                                                                               | `setDitherThreshold(128)`, `clearTransientUpdate(false)`; if a pen stroke happened in fast mode, `PauseResumeRawPenAction`                                             | transient slot cleared → **scheme 2**; clear flag 0 → no GC              | no                                         | yes                                                                                                                                                  |
| F4  | Page change or document close while in fast mode                                                                                                                                                                                           | immediate `d()` (as F3, without the 5 s wait)                                                                                                                          | as F3                                                                    | no                                         | yes                                                                                                                                                  |
| M1  | Material centre opened (web store)                                                                                                                                                                                                         | `applyAppScopeUpdate("MaterialCenterFragment$applyAppScopeFastMode$1", true, false, ANIMATION_QUALITY, MAX)` + dither 160                                              | app slot 2308, scheme 4                                                  | no                                         | yes                                                                                                                                                  |
| M2  | Material centre `onPause`                                                                                                                                                                                                                  | `clearAppScopeUpdate(true)` + dither 128                                                                                                                               | clear flag 1: fast→none action 8 → `applyGCOnce`                         | **yes** [prior doc for the 8 → GC mapping] | yes                                                                                                                                                  |
| C1  | Erase / lasso / universal-shape / normal-shape handler activated, any popup shown while the pen handler is active, **or a raw erasing stroke begins in a handler that uses draw-erase (pen handler included, e.g. the stylus eraser end)** | `EpdController.enableColorCU()`                                                                                                                                        | sets `EpdcManager+0x254`; every normal-schema entry gets flag `0x400000` | no                                         | yes [code]; visual effect unknown                                                                                                                    |
| C2  | Those handlers deactivated, last popup closed, document finished, raw erasing points of a draw-erase stroke received                                                                                                                       | `disableColorCU()`                                                                                                                                                     | clears it                                                                | —                                          | yes                                                                                                                                                  |
| R1  | Any Onyx dialog shown / hidden                                                                                                                                                                                                             | `EpdController.disableRegal()` / `enableRegal()`                                                                                                                       | `/dev/ebc` ioctl `0x700a` 0/1                                            | —                                          | yes                                                                                                                                                  |
| S1  | `ScribbleFragment.onDestroy`, `EpdEventHandler.destroy`                                                                                                                                                                                    | `EpdController.resetEpdPost()`                                                                                                                                         | `ENABLE_POST(-1,1,pid)` + pen state 0                                    | no (session teardown)                      | yes                                                                                                                                                  |
| S2  | Save-progress popup dismissed                                                                                                                                                                                                              | `useGCForNewSurface(true)`                                                                                                                                             | `EpdcManager+0x30 = 1`                                                   | [unknown]                                  | yes                                                                                                                                                  |
| L1  | Library screen Refresh button                                                                                                                                                                                                              | `EpdController.repaintEveryThing(UpdateMode.GC)`                                                                                                                       | `REPAINT_EVERY_THING_WITH_MODE` 98                                       | **yes**                                    | yes (outside the editor)                                                                                                                             |
| X   | Pen-up reconcile, `onRefreshPage` GC, EAC mode switch, `handwritingRepaint`, `waitForUpdateFinished`, `BaseRenderer` modes                                                                                                                 | —                                                                                                                                                                      | —                                                                        | —                                          | **no**: dead or gated off (§5.8)                                                                                                                     |

There is no timer-driven EPD request, no per-stroke EPD mode/GC request and no per-scroll GC
anywhere in the app [code: census in §5.9]. Two qualifications (added in verification): every stroke
does end in a plain `View.invalidate()` of the editor view (row P0; 02-writing §5.4), and
`RawInputReader` runs a 500 ms pen-up timer whose `onPenUpRefresh` reaches
`EpdShapeHandler.onPenUpRefresh` → `P0()`, which is gated off on this device (§5.8). Neither sends an
update mode or a GC.

---

## 4. From an app call to the panel

### 4.1 `UpdateMode` enum → integer (bundled `SDMDevice.h()`)

`sdk/device/SDMDevice.java:1593-1634` switches on the enum through the synthetic map at `:353-420`.
The integers are read by reflection from `android.onyx.ViewUpdateHelper` in `createDevice()`
(starts at `:447`; the mode ints at `:453-485`, the method handles continue to about `:710`); their
values are in `fw/ViewUpdateHelper.java:63-98`. [code]

| `UpdateMode`                | SDMDevice field → `ViewUpdateHelper` constant    | Value    | Waveform nibble / flags              |
| --------------------------- | ------------------------------------------------ | -------- | ------------------------------------ |
| `None` (default branch)     | `H = UI_DEFAULT_MODE`                            | 5        | AUTO                                 |
| `GU_FAST`, `DU`             | `t = UI_DU_MODE`                                 | 1        | DU                                   |
| `DU4`                       | `J = UI_DU4_MODE`                                | 2312     | DU4 + dither Y1                      |
| `DU_QUALITY`                | `u = UI_DU_QUALITY_MODE`                         | 2305     | DU + dither Y1                       |
| `GU`                        | `v = UI_GU_MODE`                                 | 2        | GC16, partial                        |
| `GC`                        | `w = UI_GC_MODE`                                 | **98**   | GC16 \| FULL (0x20) \| WAIT (0x40)   |
| `GCC`                       | `x = UI_GCC_MODE`                                | 107      | nibble 11 (rejected on the HW path)  |
| `DEEP_GC`                   | `y = UI_DEEP_GC_MODE`                            | 108      | nibble 12 (rejected on the HW path)  |
| `ANIMATION`                 | `z = REGIONAL\|NOWAIT\|ANIM\|PARTIAL`            | 4        | A2                                   |
| `ANIMATION_QUALITY`         | `A = UI_A2_QUALITY_MODE`                         | **2308** | A2 + dither Y1; a "fast" mode for SF |
| `ANIMATION_MONO`            | `B = UI_MONO_A2_MODE`                            | 33554436 |                                      |
| `ANIMATION_X`               | `C = UI_X_A2_MODE`                               | 16777220 |                                      |
| `GC4`                       | `D = UI_GC4_MODE`                                | 3        |                                      |
| `REGAL`                     | `E = UI_REGAL_MODE` (falls back to GU if 0)      | 6        |                                      |
| `REGAL_D`                   | `F = REGIONAL\|NOWAIT\|REAGLD\|REAGL\|PARTIAL`   | 4102     |                                      |
| `REGAL_PLUS`                | `G = UI_REGAL_PLUS_MODE` (falls back to GU if 0) | **9**    | nibble 9: rejected on the HW path    |
| `HAND_WRITING_REPAINT_MODE` | `I = HAND_WRITING_REPAINT_MODE`                  | 524290   | GC16 partial + bit 19                |

### 4.2 The transports

`SDMDevice` reaches the framework by reflection; `ViewUpdateHelper` then parcels a SurfaceFlinger
transaction (`transactData`, `fw/ViewUpdateHelper.java:1421`). The rows below are the ones stock
uses. SF handlers were decoded from the jump tables of `processOnyxRequests` @ `0x55f260`: table B at
`0x341444` (base `0x55f314`) for `0xff00xx`, table A at `0x341094` (base `0x55f2e0`, first code
`0x10002a`) for `0x1000xx`. [code]

| App call (bundled `EpdController`, line)                                  | `SDMDevice` (line) → framework                                                 | SF code                     | SF handler @ address → effect                                                                                                                                                                                                                                                                                                                                          |
| ------------------------------------------------------------------------- | ------------------------------------------------------------------------------ | --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `applyGCOnce()` `:57`                                                     | `:826` → `ViewUpdateHelper.applyGCOnce()` `:350`                               | `0xff0026`                  | `0x560214` → EPDC vtable `+0x18` = `HWEpdcManager::applyGCOnce` @ `0x566430`: writes the global one-shot entry at `0xac9dd8` with mode **98**, count 1. **If the FBDev scheme (`FBDev+0x30`) is 4, the mode is taken from `[0xaca250]` instead of 98**. Nothing is sent until the next composed frame                                                                  |
| `repaintEveryThing(UpdateMode)` `:537`                                    | `:2546` → `repaintEverything(int)` `:1061`                                     | `0xff0023`                  | `0x5601a4`: `EPDC+0x2c = 0`, one-shot `0xac9dd8 = mode`, count 1, then provider vtable `+0x10` (full recomposite)                                                                                                                                                                                                                                                      |
| `repaintEveryThing()` `:316`                                              | `:2020` → `repaintEverything()` `:1057`                                        | `0xff0014`                  | `0x56007c`: `EPDC+0x2c = 0`, provider `+0x10`; no mode                                                                                                                                                                                                                                                                                                                 |
| `applyTransientUpdate(m)` `:75`                                           | `:870` → `applyTransientUpdate(int)` `:404`                                    | `0xff0066`                  | `handleApplyTransientScopeUpdate` → `EpdcManager::applyTransientUpdateMode` @ `0x559924`: `FixedUpdateEntryManager::apply(slot 1, hash 0, enable 1, clearFlag 0, mode, INT_MAX)`, then `applyUpdateAction`. Dropped while the panel is powering off (`EPDC+0x24 - 1 < 3`)                                                                                              |
| `clearTransientUpdate(b)` `:550`                                          | `:902` → `clearTransientUpdate(boolean)` `:494` (sends 0/1)                    | `0xff0067`                  | `EpdcManager::clearTransientUpdateMode` @ `0x5599e4`: `apply(slot 1, 0, enable 0, clearFlag=b, -1, 0)`, then `applyUpdateAction`                                                                                                                                                                                                                                       |
| `applyAppScopeUpdate(s, true, false, m, MAX)` `:47`                       | `:2566` → `applyAppScopeUpdate(String,bool,bool,int,int)` `:320` (hash of `s`) | `0xff0004`                  | `handleApplyAppScopeUpdate` → `EpdcManager::applyAppScopeUpdate` → `applyUpdateAction`                                                                                                                                                                                                                                                                                 |
| `clearAppScopeUpdate(true)` `:514`                                        | `:2467` → `clearAppScopeUpdate(boolean)` `:480`                                | `0x100040`                  | `handleClearAppScopeUpdate` → `EpdcManager::clearAppScopeUpdate` → `applyUpdateAction`                                                                                                                                                                                                                                                                                 |
| `setEpdTurbo(n)` `:366`                                                   | `:2140` → `setEpdTurbo(int)` `:1161`                                           | `0x100055`                  | EPDC vtable `+0x98`; turbo is one global value, ioctl `0x7024` [prior doc 07 §5.4.8]                                                                                                                                                                                                                                                                                   |
| `Device.setDitherThreshold(n)` (no `EpdController` wrapper)               | `:2119` → **`EInkHelper.setDitherThreshold`** (OEC binder, not SF)             | then `0x100080`             | `system_server` → `TabletEACDisplayImpl.setDitherThreshold` → `ViewUpdateHelper.setDitherThreshold` (drops values < 128) → `EpdcManager::setDitherThreshold` @ `0x559c88`. Persisted, see §7.3                                                                                                                                                                         |
| `enableColorCU()` / `disableColorCU()` `:135/:109`                        | `:998` → `enableColorCU(boolean)` `:565`                                       | `0x100075`                  | `0x55f7c0` → `EpdcManager::setEnableColorCU` @ `0x559dcc`: logs `CU %d`, stores the byte at `EPDC+0x254`                                                                                                                                                                                                                                                               |
| `enableRegal()` / `disableRegal()` `:149/:118`                            | `:1026` → `enableRegal(boolean)`                                               | `0xff002a`                  | `handleEnableRegal` @ `0x561588` → `FBDev::enableRegal` @ `0x55a458`: `/dev/ebc` ioctl `0x700a`                                                                                                                                                                                                                                                                        |
| `supportRegal()` `:434`                                                   | `ViewUpdateHelper.supportRegal()`                                              | `0xff001c`                  | `handleSupportRegal` @ `0x5610f8` → `FBDev::supportRegal` @ `0x55a378`: ioctl `0x7009`; the answer comes from the kernel **[unknown]**                                                                                                                                                                                                                                 |
| `useGCForNewSurface(b)` `:450`                                            | `:2457` → `useGCForNewSurface(boolean)` `:1450`                                | `0x100051`                  | `0x55f6b0`: `EPDC+0x30 = (b > 0)`                                                                                                                                                                                                                                                                                                                                      |
| `byPass(0)` `:80`                                                         | `:875` → `byPass(int)` `:410` (owner-guarded)                                  | `0x10002d`                  | `0x55f538`: writes `EPDC+0x2c` (bypass counter)                                                                                                                                                                                                                                                                                                                        |
| `resetEpdPost()` `:328`                                                   | `:2025` → `resetEpdPost()` `:1071`                                             | `0xff000c`, then `0xff000d` | `ENABLE_POST(-1, 1, pid)` then `setScreenHandWritingPenState(0)`: the SF effects are in `08-surfaceflinger.md` §3                                                                                                                                                                                                                                                      |
| `setDisplayScheme(3)` via `resetUpdateMode` → `useFastScheme` `:336,:446` | `:2113` → `setDisplayScheme(int)` `:1131`                                      | `0xff0015`                  | **table B entry points to `0x5605fc`, the common exit (returns 0): a no-op** [code, re-decoded in verification: `ldrh [0x341444 + 2*0x15]` → `0x5605fc`; the Onyx-code predicate at `0x55f1d0` does accept `0xff0015`, so it is not handed to another dispatcher from there. Handling of the code anywhere before `processOnyxRequests` was not searched exhaustively] |
| `setViewDefaultUpdateMode(v, m)` `:420`                                   | `:2337` → `View.setDefaultUpdateMode(int)` (`aosp/view/View.java:14577`)       | none                        | a field on the view; used by the next `invalidate`                                                                                                                                                                                                                                                                                                                     |
| `resetViewUpdateMode(v)` `:342`                                           | `:2034` → `View.resetUpdateMode()` (`View.java:13718`)                         | none                        | field back to 5                                                                                                                                                                                                                                                                                                                                                        |

### 4.3 How a view mode reaches SurfaceFlinger

`View.invalidate()` passes `getDefaultUpdateMode()` into `invalidateInternal(..., mode)`
(`aosp/view/View.java:10814-10816`), and `ViewGroup.invalidateChild(view, rect, mode)` on a
hardware-accelerated window calls `ViewRootImpl.addUpdateEntry(view, mode)` **synchronously**
(`aosp/view/ViewGroup.java:4762-4768`). The mode is therefore captured at `invalidate()` time, and
`InkRenderManager`'s set → invalidate → reset sequence (§5.1) does tag the frame. [code]

In SurfaceFlinger, `HWNormalSchema::output` adds candidates in the order layer entry → `next()`
one-shot → layer change → bypass → fixed slot, and `UpdateEntryDetector::output()` lets the last
valid entry win, except that 98 and 108 short-circuit. **[prior doc 07 §5.4.1-2]** The one-shot slot
written by `applyGCOnce` and `repaintEverything(mode)` is the global entry at `0xac9dd0`/`0xac9dd8`
[code, above].

### 4.4 Which of these are "full" in the driver log

`FBDev::hwScreenRefresh` @ `0x55b074`, the HW-Tcon path [code]:

- `nibble = mode & 0xF`. Values 1–6 go through a 6-entry table: 1→1, 2→2, 3→3, 4→4, 5→**255**
  (the kernel picks), 6→5 (REGAL, plus flag `0x8000` when bit 12 is set). **Anything else (0, 7–15)
  logs `onyx_epdc_hwscreenRefresh(): waveform_mode  wrong!`** (priority 4, tag `SurfaceFlinger`, via
  `0x55b154`) and continues to the ioctl. The waveform slot of the ioctl struct (`[sp,#16]`) was
  zeroed at `0x55b0a4` and is not written on this branch, so **the update is sent with waveform 0**
  and flags 0 (`0x55b180`). [code, re-read in verification; the table bytes at `0x340e45` decode
  to targets `0x55b0f4/134/128/140/11c/170` = 1,2,3,4,255,5]. What the kernel does with waveform 0
  (INIT per `EINK_WAVEFORM_MODE_INIT = 0` in `fw/ViewUpdateHelper.java`, or a substitution) is
  **[unknown]**.
- `update_mode = (mode >> 5) & 1` (`ubfx w9, w24, #5, #1` at `0x55b190`), i.e. `EINK_UPDATE_MODE_FULL`.
  **So `update_mode[1]` in `onyx_tcon_send_cmd` means that bit 0x20 was set in the mode word.** Of
  the modes stock uses, only 98 has it.
- Bit 18 adds ioctl flag `0x40000` (`0x55b1a0`); bits 16/17 branch elsewhere.

Whether normal composed frames on this device take `hwScreenRefresh` or the software
`FBDev::refreshScreen` path is **[inferred]**: the HW path, because `vendor.onyx.htcon` selects
`HWEpdcManager` (`08-surfaceflinger.md` §1). Experiment E3 checks it.

---

## 5. The mechanisms, step by step

### 5.1 The basic repaint: `InkRenderManager.invalidate(DisplayArgs)`

`sdk/notecore/editor/display/InkRenderManager.java` (`invalidate(DisplayArgs)` `:79-97`,
`enableRegalModeOnce` `:63-65`, `resetViewUpdateMode` `:103-108`, `setViewUpdateModeOnce` `:115-123`,
constructor default `i = REGAL_PLUS` `:30`) [code]:

1. If `displayArgs.enableRegalModeOnce`, set the pending one-shot `h = i`. `i` is the
   "view update config model", initialised to `REGAL_PLUS` and overwritten in
   `DisplayBundle.init()` from `DeviceNoteConfig.ensureConfig().getEditorViewUpdateModel()`
   (`sdk/notecore/editor/DisplayBundle.java:81-86`, `init()`).
2. If `displayArgs.enablePost`, `EpdController.enablePost(view, 1)` (`ENABLE_POST(-1,1,pid)`). It
   is false by default. **Correction:** the erase paths do set it:
   `EraseFinishAction.render()` (`app/note/action/erase/EraseFinishAction.java:352-354`,
   `invalidate=false, enablePost=true`), `ErasingShapeAction` (`:124-125`, only when the erase track
   is not raw-rendered, `:105-109`), and `DisplayActionsKt.displayNoteDirtyRect(..., z)`
   (`app/common/extension/DisplayActionsKt.java:151`, callers pass `z`). The A-class repaints in
   this report do not set it. [code]
3. `setViewUpdateModeOnce()`: if a one-shot is pending and it is `REGAL` or `REGAL_PLUS`, call
   `EpdController.enableRegal()` (ioctl `0x700a`, 1), then
   `EpdDeviceManager.setUpdateMode(view, h)` → `EpdSDM.setUpdateMode`
   (`sdk/api/device/EpdSDM.java:54-57`) → `setViewDefaultUpdateMode(view, h)` **and
   `useFastScheme()`** (`setDisplayScheme(3)`, a no-op in SF, §4.2).
4. **Only if `displayArgs.invalidate`** (default true): `view.invalidate()`, which records the
   whole-view update entry with the current mode (§4.3).
5. `resetViewUpdateMode()`: if a one-shot was pending, `EpdDeviceManager.resetUpdateMode(view)` →
   `EpdSDM.resetUpdate` (`:48`) → `EpdController.resetUpdateMode(view)`, i.e.
   `View.resetUpdateMode()` (mode 5) plus `useFastScheme()` → `setDisplayScheme(3)`, and then
   `EpdSDM.resetUpdate` calls `useFastScheme()` a second time. These are no-ops in SF (§4.2). Then
   `h = None`.

**5.1a Other entry points into the same method.** `InkRenderManager.invalidate()` (no args,
`:138-141`) and `displayBoxList(eb, list, displayArgs)` (`:47-54`) both run the same
`invalidate(DisplayArgs)`; the base `RenderManager.invalidate()` (`RenderManager.java:931-936`) is a
bare `view.invalidate()` and is called from 17 sites inside `RenderManager` (page render
completion, high-quality render, slice updates). Since `InkRenderManager` overrides `invalidate()`,
those calls go through the one-shot logic too [inferred from Java dispatch: `this$0.invalidate()`
in `RenderManager` resolves to the override]. So **a pending REGAL_PLUS one-shot is consumed by
whichever of these invalidates runs first**, not necessarily by the `InvalidateScreenAction` that
armed it [inferred]. None of these content invalidates pause or resume the pen [code].

`DeviceNoteConfig` loads `res/config_note_<model>.json`
(`sdk/scribble/device/DeviceNoteConfig.java`, prefix `"config_note_"`). There is **no
`config_note_noteair4c.json`** in the APK (only `noteair3`, `notex3*`, `go103*`, `t10c*`, …), so
`ensureConfig()` returns a fresh `DeviceNoteConfigBean` whose `editorViewUpdateModel` field defaults
to `"REGAL_PLUS"` (`DeviceNoteConfigBean` constructor). The five files that set the key all set
`"REGAL"`. **On a Note Air 4C the one-shot mode is `REGAL_PLUS` = 9.** [code]

With no one-shot pending, the frame carries the editor view's default mode. Nothing in the live app
sets that mode persistently (§5.8), so it is `UI_DEFAULT_MODE` 5: a partial update where the kernel
chooses the waveform (255 on the HW path). [code for the value; the SF mapping is from §4.4]

**What REGAL_PLUS does on this panel.** The per-frame entry is 9 and nibble 9 is outside the HW-Tcon
table, so SF logs `waveform_mode wrong` and sends the update with waveform field 0 (§4.4). The
arbitration is last-wins and 9 is neither a short-circuit value nor a "fast" slot value, so nothing
in SF replaces it. [code for the rejection; inferred for the arbitration outcome from 07 §5.4] What the panel
visibly does then (no update, a default waveform, or the kernel substituting one) is **[unknown]**
(experiment E4). `EpdController.enableRegal()` is sent before every such frame regardless of
`supportRegal()`. [code]

### 5.2 Repaint bracketed by the pen: `InvalidateScreenAction` and `InvalidateViewWithPenControlAction`

`app/note/action/render/InvalidateScreenAction.java` [code]:

- The constructor (`:29-39`) builds `RawPenArgs` with pause render, pause input,
  resume render, resume input, and `resumeDelayTime = PenEvent.DELAY_ENABLE_RAW_DRAWING_MILLS`.
  That is `RawPenArgs.f = isColorDevice() ? 500 : 150` (`sdk/notecore/editor/data/RawPenArgs.java:69`),
  so **500 ms on this panel**.
- `create()` (`:103-127`): on the main thread run `z()` (`:89-93`, the regal one-shot if
  `setCheckUseRegalMode(true)`, setter `:130`), then an optional `delay(s)` ms, then `y()` (`:83-87`): if a document is open, post
  `InvalidateViewWithPenControlEvent(rawPenArgs)`.
- Consumers: `ScribbleHandler.onInvalidateScreenEvent` → `onInvalidateViewWithPenControl` →
  `new InvalidateViewWithPenControlAction(editorBundle, event.rawPenArgs)`
  (`app/note/handler/scribble/ScribbleHandler.java:1008-1017`). `SelectionHandler` overrides it and
  runs it only when its selection list is empty
  (`app/note/handler/scribble/SelectionHandler.java:2174-2180`).

`app/note/action/render/InvalidateViewWithPenControlAction.java` [code]: `pauseRawPen(args)` →
`observeOn(main)` → `renderManager.invalidate(displayArgs)` (§5.1) → `resumeRawPen(args)`.

- `PauseRawPenAction.n()` (`app/note/action/pen/PauseRawPenAction.java`): if both render and input
  are paused, `NoteManager.setRawDrawingEnabled(false)`; otherwise the individual switches.
  `TouchHelper.setRawDrawingEnabled(z)` (`sdk/pen/TouchHelper.java:276-284`) calls
  `setRawDrawingRenderEnabled(z)` and `setRawInputReaderEnable(z)`, **each of which returns early if
  the flag already has that value** (`:286-288`, `:298-300`), and then always
  `resetPenDefaultRawDrawing()` = `setBrushRawDrawingEnabled(true)` +
  `setEraserRawDrawingEnabled(false, 5)` (`:182-185`). So a pause while render is already paused
  sends no second pen-state transaction. [code, added in verification]
- `ResumeRawPenAction.n()`: posts `PenEvent(resumeRender, resumeInput, resumeDelayTime)`. The
  resume happens later, in the pen event handler.
- The SF-visible effect of the pause, in the bundled `SFTouchRender`
  (`sdk/pen/touch/SFTouchRender.java:263-285`): `EpdController.leaveScribbleMode(view)`, i.e.
  `enablePost(1)` = `ENABLE_POST(-1,1,pid)`, plus `pauseDrawing()` = pen state **3**. The resume is
  `resumeDrawing()` = pen state **2**. What SF does with them (stop handwriting, restore the normal
  schema, full recomposite) is in `08-surfaceflinger.md` §3 and the pen-lifecycle report.
  [code for the calls, prior doc for SF]

So the standard stock repaint on this device is: pen state 3 and post enabled → one whole-view
partial update in mode 5 (or 9) → pen state 2 after 500 ms. It never uses GC and never uses
`HAND_WRITING_REPAINT_MODE`.

Triggers I verified in this pass, with the flags that change the recipe [code]:

| Trigger                                                                                                                                                                                                                                                                                                                                                                                                                                   | Site                                                                                                                                                                                                                             | Recipe                                                                                                                                                                                                           |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Popup show / dismiss                                                                                                                                                                                                                                                                                                                                                                                                                      | `app/note/action/ui/PopupChangeAction.java:37`                                                                                                                                                                                   | `setCheckUseRegalMode(!show)` (**dismiss gets REGAL_PLUS**), `setResumeRawDrawing(false)` (render not resumed by this action)                                                                                    |
| Toast appear / disappear                                                                                                                                                                                                                                                                                                                                                                                                                  | `app/note/action/ui/ToastChangeAction.java:40`                                                                                                                                                                                   | `setResumeRawDrawing(false)`                                                                                                                                                                                     |
| Window focus gained or lost                                                                                                                                                                                                                                                                                                                                                                                                               | `app/note/eventhandler/PenEventHandler.java:345`                                                                                                                                                                                 | default                                                                                                                                                                                                          |
| Floating nav ball show / hide                                                                                                                                                                                                                                                                                                                                                                                                             | `PenEventHandler.java:367`                                                                                                                                                                                                       | `setDelayResumePenTimeMs(500)`                                                                                                                                                                                   |
| `onFullScreenRefreshEvent`, posted by `KNoteCommonReceiver.i()` on the system broadcast `onyx.android.intent.action.REFRESH_SCREEN` (`app/common/receiver/KNoteCommonReceiver.java:62-64`, action switch in `onReceive`). The firmware sends it from `PhoneWindowHelper.fullRefreshScreen()` (`fw/utils/PhoneWindowHelper.java:1351-1353`), reached from the key/gesture mapping cases `REFRESH_SCREEN` (`:414-415`) and `6` (`:754-755`) | `app/note/handler/common/EpdShapeHandler.java:359-362`                                                                                                                                                                           | default. **Stock answers the system "full refresh" gesture with a partial mode-5 repaint only**; whether the firmware also issues its own GC for that gesture is **[unknown]** (not traced beyond the broadcast) |
| Lasso pen-up, not moving                                                                                                                                                                                                                                                                                                                                                                                                                  | `SelectionHandler.java:2155-2161`                                                                                                                                                                                                | `setDelayRefreshTime(300)`                                                                                                                                                                                       |
| Counted GC (§5.3)                                                                                                                                                                                                                                                                                                                                                                                                                         | `app/common/utils/EpdDeviceManager.java:19`                                                                                                                                                                                      | default, after `applyGCOnce`                                                                                                                                                                                     |
| Pen/shape change                                                                                                                                                                                                                                                                                                                                                                                                                          | `FloatPenChangeAction.y()` (`:63-64`), `ShapeChangeAction.java:38` (their own private `refreshScreen()`; the earlier citation of `ScribbleHandler.refreshScreen()` `:1254-1256` was wrong, that method has no caller in the APK) | **`setResumeRawDrawing(false)`** (render not resumed by this action)                                                                                                                                             |

Census of every `InvalidateScreenAction` / `InvalidateViewWithPenControlAction` /
`RegalInvalidateWithPenControlEvent` construction in `app/` (grep, verification pass) [code]:

- `InvalidateScreenAction`, default recipe: `SlideToolbarVisibilityToggleDetector:70,77`,
  `HWRHandler:152` (dead, §5.8), `ScribbleHandler:1163` (dead), `ZoomSelectionHandler:89`,
  `SelectionHandler:2159` (300 ms delay), `RichTextHandler:334`, `EpdShapeHandler:361`,
  `FragmentHWRPlacement:589`, `ScribbleFragment:1210,1223,1988`, `CommonEventHandler:160`,
  `DeviceReceiverEventHandler:233`, `MenuEventHandler:174`, `PenEventHandler:345,367`,
  `TogglePenTypeAction:46`, `StartUniversalShapeAction:37`, `SetEditorViewVisibilityAction:39`,
  `StartFillColorAction:55`, `QuitFillColorAction:34`, `EraseByTypeAction:79`,
  `RenderToEditorViewAction:55`, `BaseDocMenuHandler:150`, `FunMenuHandler:59`,
  `ToolbarMenuModel:584`, `FragmentFloatMenu:214`, `FloatToolMenuHandler:555`,
  `AudioMenuModel:338`, `common/utils/EpdDeviceManager:19`.
- `InvalidateScreenAction` with `setResumeRawDrawing(false)`: `FloatPenChangeAction:64`,
  `ShapeChangeAction:38`, `StartTextInputAction:69`, `PopupChangeAction:37` (plus REGAL on
  dismiss), `ToggleFingerTouchAction:70`, `ToastChangeAction:40`.
- `InvalidateScreenAction` with `setPauseRawInputReader(false)`: `HWRDiagramConvertAction:711`.
- `ColorDeviceUtils:15` builds one with `setPauseRawDrawingRender(false)`, but
  `ColorDeviceUtils.refreshScreen*` has no caller in the APK (grep) — dead.
- `InvalidateViewWithPenControlAction` directly, default `pauseResumeArgs()` (all four flags, delay
  500 ms): `MenuBundle:85`, `NestedListFloatMenuMode:234`, `StartSelectionAction:77`,
  `FillColorAction:252`, `EpdEventHandler:296` (REGAL display args); with the event's own args:
  `ScribbleHandler:982` / `EraseHandler:102` / `UniversalShapeHandler:639` (erase finish, 300 ms
  delay from `EraseFinishAction.E()` `:156-158`), `ScribbleHandler:1016` (the `InvalidateScreenAction`
  consumer).
- `InvalidateViewWithPenControlEvent` posted directly with `RawPenArgs.resumeArgs()` (no pause; the
  consumer still runs the mode-5 invalidate, then resumes render and input after 500 ms):
  `FloatMainMenuFixedModeProvider:100`.
- `RegalInvalidateWithPenControlEvent`: `FragmentMenuSettingBase:348`, `FragmentDraftTemplate:194`,
  `FragmentMainBkGround:661`.

This replaces the earlier reliance on `docs/planning/eink-panel-upkeep.md` §1.3; the
`ScribbleHandler.onRefreshPage()` / `HWRHandler.onRefreshPage()` entries there are unreachable (§5.8).

### 5.3 The counted full GC: `app/common/utils/EpdDeviceManager`

Whole file, `app/common/utils/EpdDeviceManager.java:8-63` [code]:

```java
private int a = 10;          // gc interval - 1 after setGcInterval
private int b;               // counter
private void a(EditorBundle eb) { EpdController.byPass(0); EpdController.applyGCOnce(); new InvalidateScreenAction(eb).execute(); }
public void applyGCImmediately(EditorBundle eb) { b = a; applyGCWithInterval(eb); }
public void applyGCWithInterval(EditorBundle eb) { int i = b; b = i + 1; if (i >= a) { b = 0; a(eb); } }
public void checkIntervalChanges(int i) { if (i - 1 == getGcInterval()) return; setGcInterval(i); }
public void resetRefreshCount() { b = 0; }
public EpdDeviceManager setGcInterval(int i) { a = i - 1; b = 0; return this; }
```

- The interval comes from the app's own settings:
  `EpdDeviceManager.instance().checkIntervalChanges(noteSetting.isEnableFullRefresh() ? noteSetting.getFullRefreshCount() : Integer.MAX_VALUE)`
  (`app/note/action/setting/LoadAndApplySettingAction.java:38`). The defaults are
  `fullRefreshCount = 10` and `enableFullRefresh = DeviceInfoUtil.isColorDevice()`
  (`sdk/note/ui/setting/bean/NoteSetting.java:46-47`). **On a colour panel the default is a GC on
  every 10th tick; on a mono panel the default is never.** The user's stored values are
  **[unknown]**.
- **The GC itself is `applyGCOnce()`, which only arms SF's one-shot slot** (mode 98, or `[0xaca250]`
  while the FBDev scheme is 4; §4.2). `InvalidateScreenAction` then produces the frame that carries
  it: pause pen → invalidate → resume after 500 ms. `byPass(0)` beforehand clears any bypass count.
  On this panel the GC is therefore a **full** GC16 (`update_mode[1]`) on the frame of the following
  invalidate [inferred from §4.4, since 98 has bit 0x20]. Which of the "one-shot", "layer" and
  "fixed slot" candidates wins in that frame follows §4.3; 98 short-circuits.
- Ordering caveat [inferred]: `applyGCOnce()` arms SF synchronously on the calling thread, while the
  `InvalidateScreenAction` first pauses the pen on the pen scheduler and then invalidates on the main
  thread. Any other composed frame of this app in between (for a page turn, the new page's own
  content `invalidate()`, row P0) can carry the one-shot instead. Which frame carries the GC is
  therefore not fixed by the code; the GC may land before the pen pause (pen state 3) arrives.
- Page actions also post `PenEvent.resumeRawDrawing(max(600 − elapsed, 500))` just before the tick
  unless the handler reloads (`app/note/action/page/BasePageAction.java:117-126`). [code]
- Ticks: every page action (`app/note/action/page/BasePageAction.java:124`, in `p(long,int)` after
  `PageChangedEvent`), selection-menu click (`app/note/menu/SelectionMenuHandler.java:346`),
  text-input-menu click (`app/note/menu/TextInputMenuHandler.java:258`), rename note
  (`app/note/menu/viewmodel/scribble/FuncBarMenuModel.java:229`).
- Immediate GC: Refresh button (`app/note/menu/viewmodel/scribble/ToolbarMenuModel.java:1051-1054`),
  and `QuitFastModeImmediatelyEvent(true)` (`EpdEventHandler.java:262-276`, posted from
  `TextInputHandler.java:270` and `TextInputHelper.java:180`).
- Counter reset without a GC: leaving fast mode (`EpdEventHandler.k()` `:185-190`), and
  `QuitFastModeImmediatelyEvent(false)` (`:273`).

Strokes, erases, lasso, undo/redo, scrolling and zooming never tick the counter.

### 5.4 Fast mode: `app/note/eventhandler/EpdEventHandler.java`

State: `b` = in fast mode, `c` = the `clearTransientUpdate` flag, `f3199d` = "a raw pen stroke
ended while in fast mode", `e` = a 5 s wakelock timer, `f` = the 5 s exit timer observer, and
`g = ConfigManager.getDeviceConfig()` [code, fields `:50-64`].

- **`c` is `false`.** It is `private final boolean c` and is never assigned (the constructor at
  `:85-107` does not touch it), and the Kotlin metadata names it `resetTransientUpdate`. The
  constructor also pushes it into the textbox module:
  `DefaultRefreshListener.setResetTransientUpdate(false)` and `setQuitTransientUpdate(false)`
  (`:91-93`). [code]
- `h` (static) = `EPD_HIGH_CONTRAST = isColorDevice() ? 160 : 255` (`:80-82`). **160 on this panel.**

Enter, `a()` (`:109-116`): cancel timers → `Device.currentDevice().setDitherThreshold(160)` →
`EpdController.setEpdTurbo(deviceConfig.epdTurbo)` → `EpdController.applyTransientUpdate(i())` →
`b = true` → post `FastModeStateChangedEvent(true)`.

- `epdTurbo = 5` both as the `DeviceConfig` default (`sdk/note/ui/config/DeviceConfig.java:82`) and
  in `res/note_noteair4c.json`.
- `i()` = `deviceConfig.getTransformUpdateModel() ?: ANIMATION_QUALITY` (`:176-179`).
  `note_noteair4c.json` has no `transformUpdateModel` (13 other models set `"DU_QUALITY"`), so it is
  **`ANIMATION_QUALITY` = 2308**.

Exit request, `k()` (`:185-190`): if the active handler's `renderInFastMode()` is false and `b`:
`resetRefreshCount()` and `b(false)`, which arms the 5000 ms timer and a 5000 ms wakelock
(`:118-129`). Exit proper, `e()` (`:145-154`): **return if a finger is still down**
(`touchStateInfo.getG()`), else `d()`. Then, if a pen stroke ended during fast mode, run
`PauseResumeRawPenAction`. The timer is one-shot (`RxTimerUtil.timer(5000L, f)`) and `e()` does not
re-arm it on the early return, so if a finger is down when it fires, fast mode (transient 2308,
scheme 4, dither 160) stays on until the next event that calls `k()`/`d()` (pen up in fast mode,
`ApplyFastModeEvent(false)`, handler change, page change, close). [code for the missing re-arm;
the consequence is inferred]

`d()` (`:136-142`): `b = false` → `setDitherThreshold(128)` → **`clearTransientUpdate(false)`** →
post `FastModeStateChangedEvent(false)` → release the wakelock.

Event wiring [code]:

| Event                                         | Handler lines | Effect                                                                                                                                               |
| --------------------------------------------- | ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ApplyFastModeEvent(true)`                    | `:203-208`    | enter unless already in (or `force`)                                                                                                                 |
| `ApplyFastModeEvent(false)`                   | `:209-217`    | `k()`, then post `PenEvent.resumeRawDrawingImmediately()`                                                                                            |
| `DelayApplyFastModeEvent`                     | `:237-249`    | the same after `event.delay` ms; only poster: `ZoomMenuPopup.java:141` (`false`)                                                                     |
| `HandlerChangedEvent`                         | `:252-259`    | enter if the new handler renders in fast mode, else `k()`                                                                                            |
| `QuitFastModeImmediatelyEvent(applyGC)`       | `:262-276`    | if not a fast handler and `b`: `e()` now, `PenEvent.resumeRawDrawing()`, then `applyGCImmediately()` or `resetRefreshCount()`                        |
| `RawPenDownEvent`                             | `:279-282`    | cancel the exit timer                                                                                                                                |
| `RawPenUpEvent`                               | `:285-291`    | if in fast mode: `k()` and remember `f3199d = true`                                                                                                  |
| `BeforePageChangeEvent`, `CloseDocumentEvent` | `:221-234`    | immediate `d()`                                                                                                                                      |
| `ResumeScribbleComponentEvent`                | `:300-305`    | re-enter if the active handler renders in fast mode (posted by `ScribbleActivity.resumeComponentsImpl`, `app/note/ui/ScribbleActivity.java:510-512`) |
| `ZoomPreviewPopMoveEnableEvent`               | `:308-311`    | forwarded as `ApplyFastModeEvent`                                                                                                                    |
| `RegalInvalidateWithPenControlEvent`          | `:294-297`    | A3 in §3                                                                                                                                             |
| `destroy()`                                   | `:193-200`    | cancel timers, `k()` if in fast mode, **`EpdController.resetEpdPost()`**, textbox reset, wakelock exit                                               |

`renderInFastMode()`: false in `BaseHandler` (`sdk/notecore/editor/handler/BaseHandler.java:280`) and
hence in the pen, eraser and text handlers (`TextInputHandler.java:1182` returns false);
`SelectionHandler` returns `isTransformMode()` (`:2861`, `:1444-1446`); `UniversalShapeHandler`
returns `isValidAction()` (`:799`). **Writing never enters fast mode.** [code]

Producers of `ApplyFastModeEvent(true)` include viewport scale/scroll begin
(`app/note/eventhandler/ViewportEventHandler.java:77-82`, `:111-116`, each followed by
`PauseResumeRawPenAction`), rectangle-selection touch-down (`RectangleSelectionDrawingProxy.java:114`),
selection transform start (`StartTransformAction.java:227`), drags (`DragGestureListener.java:124`,
`FloatDraggableMenuBar.java:94`), zoom (`ZoomBeginAction.java:50`), and the SDK's
`AutoFastModeNestedScrollView` / `AutoFastModeRecyclerViewOnScrollListener`. The full grep list is
reproduced in §5.9. [code]

**SF effect.** Enter: transient slot = 2308, which is in SF's "fast" whitelist. `applyUpdateAction`
→ `setFBSchema` → `setEbcUpdateScheme(4, 0)`: driver log `### EUS, s 4 c 0`, panel scheme 4, plus
panel-side Y1 dither on (07 §5.4.5). Exit with flag 0: slot cleared → scheme 2
(`### EUS, s 2 c 0`) and **no** clear-screen action, because only clear flag 1 (IMMEDIATELY) turns
the fast→none transition into `applyGCOnce` (07 §5.4.6). [scheme choice: code §8; the clear-flag
table: prior doc]. Turbo 5 is never reset by stock; it stays until something else (the next EAC
resume, §7.2) writes it. [code for stock; inferred for persistence from 07 §5.4.8]

### 5.5 Other transient / app-scope users inside the stock APK

| Site                                                                                                                           | Call                                                                                                                                                                                                      | Reachable in the note flow?                                                                                                                                                                                                                                          |
| ------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `app/note/datarepository/view/MaterialCenterFragment.java:116-126`, `:410-415`                                                 | `RxTimerUtil.shortTimer` → `applyAppScopeUpdate("MaterialCenterFragment$applyAppScopeFastMode$1", true, false, ANIMATION_QUALITY, MAX)` + dither 160; `onPause`: `clearAppScopeUpdate(true)` + dither 128 | yes (material store; `MaterialCenterActivity.resumeComponentsImpl` is empty). The `true` clear gives a GC when the store is left [inferred via 07 §5.4.6]                                                                                                            |
| `sdk/textbox/listener/DefaultRefreshListener.java` (`startMove`, `onQuitEdit`)                                                 | `applyTransientUpdate(ANIMATION_QUALITY)` on textbox move; `clearTransientUpdate(b)` on quit edit only if `c` ("quitTransientUpdate") is true                                                             | yes, but `EpdEventHandler` sets `quitTransientUpdate = false` (`:93`), so the textbox never clears its own transient; the next `EpdEventHandler.d()` clears it [code; the interaction is inferred]                                                                   |
| `sdk/kui/dialog/OnyxBaseDialog.java:117`, `:213`, `:306` (and `OnyxAlertDialog*Fragment`, `BaseSupportDialogFragment:142-149`) | `show()` → `disableRegal()`; `hide()`/`clear()`/invisible → `enableRegal()`                                                                                                                               | yes (every Onyx dialog). A `ViewDragHelper` inside draggable dialogs uses `applyTransientUpdate(ANIMATION_QUALITY)` and `clearTransientUpdate(true)` (`sdk/kui/utils/ViewDragHelper.java:91`, `:101`): a GC at drag end [code; reachability of the drag not checked] |
| `sdk/note/ui/common/base/BaseActivity.java:164-170`                                                                            | `resumeComponentsImpl()` default: return if `isFlexibleScreen()`, else `clearTransientUpdate(false)` + `useGCForNewSurface(true)` (the flexible-screen value on NA4C is **[inferred]** false)             | only for activities that do not override it; **`ScribbleActivity` overrides it** (`:510-512`), so the editor does not run it [code]                                                                                                                                  |
| `sdk/kui/utils/FastModeController`, `OnyxLinearSnapHelper`, `recyclerdraghelper/ItemDragHelper`                                | transient A2 / `applyGCOnce` / clear(true)                                                                                                                                                                | `FastModeController` and `OnyxLinearSnapHelper` have no referencing class in `app/` or `sdk/note/`; `ItemDragHelper` only in the custom-menu settings adapters [code]                                                                                                |

### 5.6 Colour "CU" (`enableColorCU`)

App side [code]:

- `GlobalNoteBundle.enableColorCU()/disableColorCU()` (`app/note/GlobalNoteBundle.java:149`, `:186`).
- `ScribbleHandler.onActivate()`/`onDeactivate()` call them when `isEnableColorCU()`
  (`ScribbleHandler.java:910`, `:948`). That is true for `EraseHandler` (`:46`), `SelectionHandler`
  (`:1958`), `UniversalShapeHandler` (`:549`) and **`NormalShapeHandler` (`:164`, missing from the
  first draft)**, and false in `BaseHandler` (`:91`), so false for the pen handler.
- **Added in verification:** the pen handler still turns CU on for the duration of a raw erasing
  stroke. `ScribbleHandler.onBeginRawErasing` (`:931-936`) posts `RawPenDownEvent` and calls
  `a(point)`, which, if `canDrawErase()` (`:764-772`: `useDrawErase()` and not
  `PenEventHandler.getF()`), runs `EraseBeginAction` and **`enableColorCU()`** (`:238-246`);
  `l0(pointList)` (`:463-468`, from the raw erasing points callback `:1143`) calls
  **`disableColorCU()`**. `useDrawErase()` is true for `EpdShapeHandler` (the pen handler,
  `:498`), `NormalShapeHandler`, `UniversalShapeHandler`, `EraseHandler`, `FillColorHandler`, and
  false in the base `ScribbleHandler` and the HWR / zoom-selection handlers. So an erase with the
  stylus eraser end while the pen tool is active produces `CU 1` … `CU 0` around the stroke [code
  for the calls; that the eraser end arrives as `onBeginRawErasing` is from the pen/erasing reports].
- `ColorModeEventHandler` (`app/note/eventhandler/ColorModeEventHandler.java`): when the active
  handler does not own CU, it enables CU while any popup is showing (a counter of
  `PopupWindowChangeEvent`s) and disables it when the last one closes; `EnableColorCUEvent` forces
  it on or off.
- The `BaseNotePopWindow` constructor enables it unconditionally (`app/common/base/BaseNotePopWindow.java:37`),
  as do `NestedListMenuBar.java:63` and `FloatDraggableMenuBar.java:81`. `ScribbleMainFragment.java:427`
  disables it when the document finishes.

SF side [code]: `EpdcManager::setEnableColorCU` stores `EPDC+0x254`. It is read in
`HWNormalSchema::output` @ `0x566e5c` (and `NormalSchema::output` @ `0x55b4e0`). When set,
`EpdcManager::appendFlag(list, 0x400000)` ORs bit 22 into every update entry of the frame before the
image handler runs. What bit 22 means to the kernel/T-CON is **[unknown]**; 08 notes a
`0x500000 → 0x200000` flag translation in `FBDev::refreshScreen` **[prior doc]**. Consequence: on this
device **every normal-schema frame drawn while the eraser, lasso or shape tool is active, while a
popup is open, or during a draw-erase stroke (including the stylus eraser end with the pen tool),
carries an extra flag that ordinary pen-tool frames do not** (experiments E6, E11).

### 5.7 Session-level calls that touch refresh

- `EpdController.resetEpdPost()` in `ScribbleFragment.onDestroy()` (`app/note/ui/ScribbleFragment.java:1492`)
  and `EpdEventHandler.destroy()` (`:197`): `ENABLE_POST(-1, 1, pid)` then pen state 0
  (`fw/ViewUpdateHelper.java:1071-1077`). [code]
- `EnablePostAction` → `EpdController.enablePost(1)` from the meeting-notes view models only
  (`MeetingViewModel.java:91`, `:380`; `MeetingRecognizerViewModel.java:902`). [code]
- `SaveProgressPopup` (`app/note/menu/popup/SaveProgressPopup.java:95-110`): on dismiss,
  `useGCForNewSurface(true)` (posted to the decor view if attached). No caller ever passes `false`,
  and `Constant.DEFAULT_USE_GC_FOR_NEW_SURFACE = isCfaDevice()` is already true on this device
  (**[prior doc]** 05 §2.1), so this is at most a re-assertion. [code / inferred]
- `ClearSelectionPathRequest` → `resetViewUpdateMode(penHostView)`
  (`app/note/request/selection/ClearSelectionPathRequest.java:19`), and
  `EraseFinishAction.z0()` → `resetViewUpdateMode(editorView)` (`:505-507`). Both put the view back
  to 5; neither sends anything to SF.

### 5.8 Code that exists but does not run on a Note Air 4C

| Item                                                                                                                                                                     | Why it does not run                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Pen-up grayscale reconcile**: `EpdShapeHandler.P0()` → `GrayscaleRefreshAction` (`setViewDefaultUpdateMode(HAND_WRITING_REPAINT_MODE)`, `displayNoteDirtyRect`, reset) | `P0()` requires `OnyxSystemConfig.isUseHWUpdateForPenUpRefresh()` (`app/note/handler/common/EpdShapeHandler.java:248-263`). That returns **false if `SystemPropertiesUtil.isTablet()`** (property `vendor.onyx.tablet`, `sdk/utils/SystemPropertiesUtil.java:10,57-58`), else `SystemConfigBean.enablePenUpRefresh`, whose default is `!DeviceInfoUtil.isColorDevice()` = false (`sdk/data/config/system/data/SystemConfigBean.java:22`; `isColorDevice` = `getColorType() > 0`, `sdk/utils/DeviceInfoUtil.java:136-138`). The only setter is `OnyxSystemConfig.updateUseHWUpdateForPenUpRefresh` (`:241-244`), driven by a settings toggle that is shown only if `DeviceConfig.showPenUpRefresh && !hasHTCon()` (`SettingViewModel.java:83`, `LoadSettingsDataAction.java:71`), and `note_noteair4c.json` sets `"showPenUpRefresh": false`. [code; that `vendor.onyx.tablet` is true and `getColorType() > 0` on the device is inferred] **Verification note:** `SystemConfigBean` is not only its defaults: `LoadOnyxSystemConfigRequest.execute()` parses it from `Device.currentDevice().readSystemConfig(ctx, ONYX_SYSTEM_CONFIG_KEY)`, a device-wide store, reloaded on `com.onyx.action.system.config.changed` (`KNoteCommonReceiver`). If `vendor.onyx.tablet` is false, a stored `enablePenUpRefresh:true` (written by another app, a previous firmware or a mono-era setting) would enable the reconcile; the stored value is **[unknown]** (E0/E1). |
| `ScribbleHandler.onRefreshPage()` / `HWRHandler.onRefreshPage()` → `repaintEveryThing(GC)`                                                                               | Called only from their `refreshPageEvent(RefreshPageEvent)` subscribers (`ScribbleHandler.java:1250-1252`, `HWRHandler.java:190-191`). **`RefreshPageEvent` is never constructed** anywhere in the APK (grep over all sources). [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `UpdateModeEventHandler` (EAC `updateMode 2`, turbo `hasHTCon() ? 2 : 0`, via `ChangeAppUpdateModeAction` → `ApplyAppConfigRequest`)                                     | Registered (`app/note/handler/NoteEditBundles.java:77`), but `RichTextApplyFastUpdateModeEvent` / `RichTextRestoreUpdateModeEvent` are never constructed. `destroy()` calls the restore handler, which returns early because the saved config is null. [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `DataBindingUtils.repaintView` → `EpdController.handwritingRepaint`                                                                                                      | Binding adapter with no user: none of the generated `*BindingImpl` classes calls it. [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `WaitViewUpdateRequest` → `waitForUpdateFinished`                                                                                                                        | No referencing class. [code] (`OnyxAppCompatActivity.dismissProgressDialog` also calls it; not traced.)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `BaseRenderer.setViewUpdateModeOnce/resetViewUpdateMode`, `beforeUnlockCanvas`                                                                                           | `RenderContext.q` (the mode) defaults to `None` and is only ever set to `None` (`sdk/scribble/shape/RenderContext.java:77`, `BaseRenderer.java:333`); `beforeUnlockCanvas` has no caller. [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `setDisplayScheme(3)` sent by every `resetUpdateMode`                                                                                                                    | Reaches SF but hits the default exit of the jump table (§4.2). `ViewUpdateHelper.setUpdateScheme`/`applySysScopeUpdate` are empty on this firmware **[prior doc]** 05 §1.5. [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `ColorDeviceUtils.refreshScreen/refreshScreenDelay`                                                                                                                      | No callers: grep for `ColorDeviceUtils` over the whole decompiled APK finds only the class itself. [code, re-checked in verification]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `ScribbleHandler.refreshScreen()` (`:1254-1256`)                                                                                                                         | No caller in the APK (`FloatPenChangeAction`/`ShapeChangeAction` use their own private `refreshScreen()`). [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `sdk/api/device/EpdDeviceManager.enterAnimationUpdate/exitAnimationUpdate` (transient 2308 / clear)                                                                      | No caller in the APK. [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `BaseRenderer.beforeUnlockCanvas` → `enablePost(surfaceView, 1)`                                                                                                         | No caller (already noted in the `BaseRenderer` row). [code]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `app/note/test/*` activities (`TconTestActivity`, `HWPenTestActivity`, `SWPenTestActivity`, `BasePenTestActivity`, `ScribbleTestActivity`)                               | Debug activities: transient DU/A2/HWR modes, random app-scope modes and turbos, `applyGCOnce`. Not part of the note flow [inferred from their names and that they are separate activities; launch paths not traced].                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |

### 5.9 Census of refresh-related calls in the APK (non-test)

92 call sites were grepped over `com/onyx/**` in the bundled sources, excluding the `sdk/device` and
`sdk/api/device` definitions and `note/test/`
(scratch list `…/scratchpad/epdcalls.txt`, not preserved; the count was not reproduced in
verification, but a fresh grep of the refresh-related `EpdController` methods in `note/`, `sdk/`,
`libsetting/` found no live site missing from the groups below except the ones added in §5.2,
§5.6 and §5.8). Grouped [code]:

- **Editor, live**: `EpdEventHandler` (fast mode, resetEpdPost), `common/utils/EpdDeviceManager`
  (GC), `InkRenderManager` (one-shot mode, enableRegal), `GlobalNoteBundle`/`BaseNotePopWindow`/
  `NestedListMenuBar`/`FloatDraggableMenuBar` (colour CU), `EraseFinishAction` and
  `ClearSelectionPathRequest` (reset view mode), `ScribbleFragment.onDestroy` (resetEpdPost),
  `SaveProgressPopup` (useGCForNewSurface), `MaterialCenterFragment` (app scope), the textbox
  `DefaultRefreshListener` (transient), `SFTouchRender`/`AppTouchRender` (post/pen state; pen
  report), and the meeting `EnablePostAction`.
- **Outside the editor, live**: `LibraryViewModel.onRefresh` → `repaintEveryThing(GC)`
  (`sdk/note/ui/library/viewmodel/LibraryViewModel.java:432-435`); `DialogNoteMenu.java:585` and
  `StatusLayoutRecognizerCheck.java:92` → `invalidate(view, GU)`; `CropImageView.java:156` →
  `refreshScreen(view, GC)`; the dialogs' regal toggles; `CreateNotePopup` colour CU;
  `MeetingMinutesWebView`/`TipTapEditor` → `setWebViewContrastOptimize(true)` (`WebView.setCssInjectEnabled`).
- **Bundled but not traced into the note flow**: `libsetting/.../AppsFragment.java:233`
  (`repaintEveryThing(GC)`), `PowerSettingFragment`/`TPFragment` (EMTP power).
- **Dead**: §5.8.

---

## 6. Device-specific branches, resolved for a Note Air 4C

| Branch                                                                 | Where                                                                                               | Value on NA4C and how it was determined                                                                                                                                                                                                                                                                                                                        |
| ---------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Device.currentDevice()` class                                         | `sdk/device/Device.java`                                                                            | `SDMDevice` (platform `lito`) [code + prior doc for the platform]                                                                                                                                                                                                                                                                                              |
| `DeviceInfoUtil.isColorDevice()` = `getColorType() > 0` (SF `1048659`) | `sdk/utils/DeviceInfoUtil.java:136-138`                                                             | true [inferred: CFA panel, `epdColorMode:1` in `noteair4c_eac.json`, prior doc 05 §0; not read from SF]                                                                                                                                                                                                                                                        |
| `SystemPropertiesUtil.isTablet()` = `vendor.onyx.tablet`               | `SystemPropertiesUtil.java:57-58`                                                                   | **[unknown]**: property not read. If true, the pen-up reconcile is off regardless of config. If false, it depends on the stored device-wide `SystemConfigBean.enablePenUpRefresh` (default false on colour; stored value unknown, §5.8)                                                                                                                        |
| `SystemPropertiesUtil.hasHTCon()` = `vendor.onyx.htcon`                | `SystemPropertiesUtil.java:14`                                                                      | true [inferred: SF uses `HWEpdcManager` exactly when this is true, 08 §1; the driver log shows `HWEpdcManager` lines]                                                                                                                                                                                                                                          |
| Per-model app config `note_noteair4c.json`                             | `res/note_noteair4c.json`, loader `DeviceConfig.readConfig` (`"note_" + Build.MODEL.toLowerCase()`) | `canDisableCTPTouch:true, showReadNote:true, largeObjectSize:0, loadingShapePointThreshold:20000, overlayEraserBufferTimeMs:30, strokeEraserBufferTimeMs:20, showPenUpRefresh:false, isTabDevice:true, epdTurbo:5` [code for the file; that `Build.MODEL` lower-cases to `noteair4c` is **inferred** from the file's existence, not read from the device (E0)] |
| `transformUpdateModel` (fast-mode waveform)                            | `DeviceConfig.getTransformUpdateModel()`                                                            | absent → `ANIMATION_QUALITY` (2308) [code]                                                                                                                                                                                                                                                                                                                     |
| `editorViewUpdateModel` (one-shot mode)                                | `DeviceNoteConfig` (`config_note_<model>.json`)                                                     | no file → `REGAL_PLUS` (9) [code]                                                                                                                                                                                                                                                                                                                              |
| `EPD_HIGH_CONTRAST`                                                    | `EpdEventHandler.java:80-82`                                                                        | 160 [code + colour inference]                                                                                                                                                                                                                                                                                                                                  |
| Raw-pen resume delay / popup resume                                    | `RawPenArgs.java:69`, `sdk/scribble/event/pen/PenEvent.java:16-19`                                  | 500 ms / **800 ms** (`DELAY_ENABLE_RAW_DRAWING_MILLS = isColorDevice() ? 500 : 150`, `POPUP_RESUME_PEN_TIME_MS = isColorDevice() ? 800 : 400`) [code + colour inference; the first draft said 500 / 500]                                                                                                                                                       |
| Auto full refresh                                                      | `NoteSetting.java:46-47`                                                                            | on, every 10 ticks by default [code + colour inference; stored setting unknown]                                                                                                                                                                                                                                                                                |
| Pen-up grayscale reconcile                                             | §5.8                                                                                                | off [code]                                                                                                                                                                                                                                                                                                                                                     |
| Zoom-preview overlay colour in fast mode                               | `ZoomPreviewContainerHandler.java:162-171`                                                          | colour branch: overlay tinted while in fast mode (UI only) [code]                                                                                                                                                                                                                                                                                              |
| Phone vs tablet UI (`isPhone()`, `vendor.onyx.phone`)                  | menus, float-menu mode (`FloatMenuModeFactory.java:24`)                                             | tablet layout [inferred]; no refresh-policy consequence found                                                                                                                                                                                                                                                                                                  |
| `isOnyxApp(pkg)` (`"com.onyx"` substring)                              | `fw/utils/ActivityManagerHelper.java:224-229`                                                       | true for stock → excluded from `View.updateCanvasImpl` EAC paint (`aosp/view/View.java:6094-6097`) and from `OnyxScrollRefreshManager.deliverMotionEvent` (`fw/optimization/OnyxScrollRefreshManager.java:135`) [code]                                                                                                                                         |
| SF panel path                                                          | `setSurfaceFlingerProvider` reads `vendor.onyx.htcon`                                               | `HWEpdcManager`, `hwScreenRefresh` [prior doc 08 §1]                                                                                                                                                                                                                                                                                                           |

---

## 7. EinkWise (EAC) and stock

### 7.1 What stock itself does with EAC

- `NoteApplication.s()` → `RxEACManager.init(this, new DataManager(), EventBus.getDefault())`
  (`app/NoteApplication.java:364-366`). It only stores references (`sdk/eac/rx/RxEACManager.java:110-117`).
  No config is read or pushed at startup. [code]
- The only code that writes an EAC refresh config is the dead `UpdateModeEventHandler` (§5.8).
- Stock never calls `EInkHelper.setAppScopeRefreshMode`, `setTurbo`, `applyAppConfigToService`, or
  `setSystemRefreshMode` (grep of `app/`). [code]
- Stock **does** call `EInkHelper.setDitherThreshold` indirectly through
  `SDMDevice.setDitherThreshold` (`:2119-2121` → method `X2`, resolved on `android.onyx.optimization.EInkHelper`
  in `createDevice()`), on every fast-mode entry and exit and in the material centre (§3 F1/F3/M1/M2). [code]

### 7.2 What the firmware does to stock on every resume, `supportEAC` or not

Trigger: the `/dev/onyx/listener` activity-state pipe → `OECService.handleActivityResumeImpl`
(`fw/optimization/OECService.java:461-471`), gated on a **component** change
(`bundle/OptimizationBundle.java:79-84`), so switching between stock's own activities counts too. The
five `EACBaseImpl.onResume` implementations then run [code]:

- `TabletEACRefreshImpl.onResume` (`impl/TabletEACRefreshImpl.java:240-250`) reads
  `appConfig.getRefreshConfig(component)` **directly, not through the `supportEAC`-aware
  `getRefreshConfigByCurrentComponent`**, and then:
  - `applyUpdateMode` (`:95-121`): `useGCForNewSurface(true)` if the config says so; resolved mode
    0/3/5 → `clearAppScopeUpdate(multiWindow ? 0 : 2)`, mode 1/2/4 → `clearSFDebouncer` +
    `applyAppScopeUpdate(null, toEpdMode(mode), flag)`, mode −1 → `whiteClear(2, true)`.
  - `applyEpdParameter` (`:81-86`): `setEpdTurbo(resolved turbo)`, `antiFlicker(cfg)`,
    `enableDither(ditherBitmap && raw updateMode ∈ {0,3,5} && enable)`, and
    **`handleSFDebouncer`, which for `supportEAC == false` is `clearSFDebouncer()` =
    `debouncer(false,0,0,0,0)`** (`:143-149`).
- `TabletEACDisplayImpl.onResume` (`impl/TabletEACDisplayImpl.java:78-83`) →
  `applyDisplayConfig(deviceConfig.getDisplayConfig(component))`: gamma, CFA brightness, saturation
  minimum, saturation, bwMode, dither threshold, enhance (the order is **[prior doc]** 07 §3.5). This
  is not gated on `supportEAC`.
- Both return early only if the package is in `OnyxSystemConfig.KEY_MAINTAIN_PREV_PKG_REFRESH_PKG_SET`
  (`impl/EACBaseImpl.java:59-64`). Whether stock is in that set is **[unknown]**.

What stock's stored refresh block resolves to is **[unknown]** without reading
`/onyxconfig/mmkv/onyx_config`. The code path points to "HD" [inferred]:

- `EACAppConfig.createOnyxAppConfig` (`fw/optimization/data/v2/EACAppConfig.java:100-111`):
  `supportEAC=false`, refresh block `enable=true`, display block `enable=true`, paint off, DPI off.
- `kcb` `InitApplyEACConfigAction` (`kcb/com/onyx/reader/apps/action/InitApplyEACConfigAction.java:150-159`)
  sets `refreshModeIndex = REFRESH_MODE_INDEX_FOR_ONYX_OR_SYSTEM_DEFAULT` for Onyx/system apps that
  do not support Regal. That constant is `refresh_mode_4` = mode 0 ("HD") on this device
  **[prior doc 07 §3.1]**.
- `ValidateAppConfigAction.validateRefreshConfig` (`fw/optimization/action/ValidateAppConfigAction.java:167-186`)
  turns a REGAL-class index into auto mode for Onyx apps, and otherwise uses
  `refreshMigrateMapForSystemApp`.

If that holds, stock's resume sends `clearAppScopeUpdate(2)`, `setEpdTurbo(0)`, anti-flicker 10,
`enableDither(false)` and the debouncer clear, plus the display block. `clearAppScopeUpdate(2)` only
acts if the app slot currently holds a fast mode (07 §5.4.3). So coming from an app that left an A2
pin, stock's resume would clear it, with clear flag 2 (NEW_SURFACE): the action is deferred to the
next new layer [prior doc 07 §5.4.6, inferred for stock].

### 7.3 Stock's writes into the EAC store (side effect of fast mode)

`EInkHelper.setDitherThreshold(i)` → `OECService.setDitherThreshold` (`OECService.java:1908-1920`) →
`TabletEACDisplayImpl.setDitherThreshold(topComponent, cfg, i)` (`impl/TabletEACDisplayImpl.java:142-150`) [code]:

1. `applyDitherThreshold(i)` → `ViewUpdateHelper.setDitherThreshold(i)` (drops `i < 128`, `fw/ViewUpdateHelper.java:1138-1145`) → SF.
2. `appConfig.setDitherThreshold(component, i)`, which also sets `displayConfig.enable = true`
   (`data/v2/EACAppConfig.java:411-416`).
3. **`deviceConfig.getFallbackDisplayConfig().setDitherThreshold(i)`**: the fallback used for apps
   without their own config (`data/v2/EACDeviceConfig.java:59-61`).
4. `saveDeviceConfig(cfg, 2)`: persisted to MMKV.

Consequences [inferred]:

- Every stock fast-mode entry writes 160 and every exit writes 128 into stock's config and the
  global fallback, and persists both. If the process dies while in fast mode, 160 stays stored; at
  the next `system_server` start `ValidateAppConfigAction.validateDitherThreshold` rewrites any value
  other than 128/180 to **180** (`ValidateAppConfigAction.java:147-153`). On the next resume the
  display block then applies it.
- The write targets `getCurrentTopComponent()`. If stock is not the top component at that moment
  (the 5 s delayed exit after a system dialog took the top), the 128 lands in **another app's**
  config (`OECService` setters operate on the foreground component, **[prior doc]** 05 §2.0).

### 7.4 Exactly what `supportEAC:false` changes (firmware side)

`supportEAC` consumers in `fw/**`, all checked [code]:

| Consumer                                                                                                                                                                                 | Effect of `false` for stock                                                                                                                                                                                                                                                                          |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `EInkHelper.inputEventChangedAsync` filter `isAppConfigSupportEAC()` (`EInkHelper.java:1127-1146`, `:1164-1169`), called from `ViewRootImpl` for key/motion down/up                      | **No input pipe messages from stock's process.** So `OECService` never runs `EACBaseRefreshImpl.handleInputEventImpl` for stock: no `debounceIncRefresh()` tick (the "Full Refresh Frequency" counter) and no `enableRegal` + debouncer transient on input-up (`impl/EACBaseRefreshImpl.java:25-52`) |
| `TabletEACRefreshImpl.handleSFDebouncer` (`:143-149`)                                                                                                                                    | Debouncer cleared on every resume, config apply and system-window hide (`configDebouncerBySysWindowChanged`, `:190-201`). SF runs no post-input quality repaint and no counted GC for stock                                                                                                          |
| `EACBaseRefreshImpl.getRefreshConfigByCurrentComponent` (`:134-140`)                                                                                                                     | Returns a bare `EACRefreshConfig` → `getConfigUpdateMode` gives 0 for stock. Used by `getConfigUpdateMode` callers, **not** by `onResume`                                                                                                                                                            |
| `EACBaseRefreshImpl.calculateScrollingRefreshMode` (`:83-100`)                                                                                                                           | Device default scrolling mode is used                                                                                                                                                                                                                                                                |
| `EInkHelper.isAppConfigActive()` (`:1156-1162`) gating `getAppExtraConfig`, `getApplicationDPI`, `getNoteConfig`, `getPaintConfig`, `getRefreshConfig`, `getRotationConfig` (`:510-957`) | In-process getters return disabled/dummy configs. So `EACScreenNoteManager` (system pen pipeline) never engages, `EACConfigChangedImpl` redraw decisions see dummies, `AccessibilityHelper` refresh reads dummies                                                                                    |
| `EACSplashScreenManager`, `EACRotationManager`, `EACKeyboardManager` (`:221`)                                                                                                            | Features off                                                                                                                                                                                                                                                                                         |
| `ValidateAppConfigAction.supportEAC` / `validateDPI` (`:288-293`, `:315-322`)                                                                                                            | DPI override forced off                                                                                                                                                                                                                                                                              |
| `OECService.java:375` (force-stop on EAC enable toggle), `:907` (config-change broadcast), `:1209` (`getDisallowedEACApps`)                                                              | Stock is not killed when EAC is toggled, gets no `onyx.action.oec.config.change` for a same-state rewrite, and is listed as disallowed (the EinkWise panel refuses it, **[prior doc]** 04)                                                                                                           |
| `EpdcUpdateDebounceWithDelay.java:100`                                                                                                                                                   | Screen-note debounce inactive                                                                                                                                                                                                                                                                        |

Not changed by `supportEAC:false`: the resume-time refresh block (app-scope slot, turbo, anti-flicker,
dither enable) and display block (§7.2), and the `setDitherThreshold` persistence (§7.3).

Independent of `supportEAC` but also true for stock, because the package contains `"com.onyx"`: no
`View.updateCanvasImpl` EAC paint config and no `OnyxScrollRefreshManager` scroll handling (§6).

---

## 8. SurfaceFlinger: update scheme, fixed slots and implicit GCs

### 8.1 Where `### EUS` comes from

`HWEpdcManager::setEbcUpdateScheme(int scheme, int c)` @ `0x565de0` logs
`"### EUS, s %d c %d"` (string `0xa96aa`, tag `HWEpdcManager` `0x104f6d`) and ends in
`FBDev::setEbcUpdateScheme` @ `0x55a970` (ioctl `0x7006` per **[prior doc]** 07 §5.4.5). Before
switching it sleeps **100 ms** when going to scheme 2 or 5 from 4, and 150 ms when going from 3
(`0x565ea4-0x565ef0`, `usleep(100000/150000)`). With scheme 4 it sets a byte at `*(this+0x280)+8`,
otherwise clears it (`0x565ef4-0x565f58`). [code]

It is reached only through the vtable slot `+0x78` (address point `0xa354d0`, entry `0xa35548`). The
only `+0x78` callers are [code]:

| Caller                                                               | Scheme chosen                                                                                                                                   | Also does                                                                    |
| -------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| `HWEpdcManager::setFBSchema(int mode, bool, int clear)` @ `0x565c78` | fast(mode) ? 4 : 2 (**[prior doc]** 07 §5.4.5; the `mov w1,#2` default is visible at `0x565d88`)                                                | `applyDither`, `clearScreen(clearType)`, enhance on only if fast             |
| `HWEpdcManager::syncFBSchema()` @ `0x565f74`                         | `fixedUpdateEntry()` → `applyDither(mode)`: 4 if the returned mode is one of 1, 4, 0x901, 0x904, 0x908, 0x1000001, 0x1000004, 0x2000004, else 2 | `clearScreen(fast ? 1 : 0)`, enhance, `resetSaturation()`, `signalRefresh()` |
| `HWEpdcManager::clearHandwritingFBSchema()` @ `0x566134`             | same rule as `syncFBSchema`                                                                                                                     | `clearScreen(0)`, enhance, sets `this+0x8` if not in fast mode               |

So **scheme 4 means "the app-scope or transient fixed slot holds a fast mode"** as far as these
callers go. Stock puts 2308 in the transient slot during fast mode (§5.4) and in the app slot in the
material centre (§5.5); nothing else in stock does. [code + §5]

Who calls `syncFBSchema` (`+0x60`): `HWNormalSchema::output` (`0x566f34`) and
`ExtBufferSchema::onDeactivate` (`0x55ba84`). `setFBSchema` (`+0x68`) is called from
`EpdcManager::clearFBSchema` @ `0x559908` as `setFBSchema(5, 1, 0)`, i.e. scheme 2, and that is the
shared `onActivate` of `ExtBufferSchema`, `DreamSchema` and `HWDreamSchema` (identical-code-folded
symbol at `0x55ba4c`). [code]

### 8.2 Implicit full GCs inside SurfaceFlinger

| Path                                                 | Code                                                                                                                                                                        | Trigger relevant to stock                                                                                                                |
| ---------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `HWEpdcManager::clearScreen(t)` @ `0x565bf8`         | `t == 8` or `t == 0x65` → tail-call vtable `+0x18` = `applyGCOnce`; other `t` looked up in a `std::map` at `0xaca280`, and the found value is written to the one-shot entry | fast→none with clear flag 1 (**[prior doc]** 07 §5.4.6); stock uses flag 0 in the editor, flag 1 in the material centre and dialog drags |
| `ExtBufferSchema::onDeactivate` @ `0x55ba5c`         | power state −1, `EPDC+0x2c = 0`, `syncFBSchema()`, then if `vendor.onyx.htcon` the one-shot entry = **98 with count 2**, else `applyGCOnce()`                               | **[unknown]** which pen situation activates the ExtBuffer schema; a candidate for the "GC16 ~1.5 s after lift" (E7)                      |
| `HWEpdcManager::applyGCOnce` while `FBDev+0x30 == 4` | uses `[0xaca250]` instead of 98                                                                                                                                             | a stock counted GC or Refresh button pressed during fast mode: the waveform/flags differ **[unknown]** (E5)                              |
| `repaintEverything(mode)` (`0xff0023`)               | one-shot = mode, count 1, full recomposite                                                                                                                                  | library Refresh button only                                                                                                              |

### 8.3 Driver-log vocabulary, mapped to code

| Driver / logcat line                                                 | Produced by                                                                                                                                              |
| -------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `HWEpdcManager: ### EUS, s N c M`                                    | `setEbcUpdateScheme(N, M)`, callers in §8.1 [code]                                                                                                       |
| `onyx_tcon_set_upd_scheme`                                           | kernel side of ioctl `0x7006` [inferred]                                                                                                                 |
| `onyx_tcon_send_cmd … waveform[W] update_mode[M] flags[F]`           | kernel side of `FBDev::hwScreenRefresh`: W from the nibble table, M = bit 0x20 [code for the SF side; that the kernel prints them unchanged is inferred] |
| `SurfaceFlinger: onyx_epdc_hwscreenRefresh(): waveform_mode  wrong!` | a frame whose mode nibble is 0 or 7–15, e.g. stock's REGAL_PLUS one-shot [code]                                                                          |
| `EpdcManager: CU %d`                                                 | `setEnableColorCU` [code]                                                                                                                                |
| `SurfaceFlinger: hasHwTcon: %d`                                      | `ExtBufferSchema::onDeactivate` (and the `EpdcSchemaManager` ctor) [code]: a direct marker for E7                                                        |

---

## 9. Interactions with the other subsystems

- **Session lifecycle.** Every standard repaint (A1–A5) is a pen-session pause (post=1, pen state 3)
  followed by a resume (pen state 2) after 500 ms (300 ms after erase). Popups and toasts pause
  render and input but resume **input only** (`setResumeRawDrawing(false)`): render comes back
  through the popup's own resume path (`PenEvent.POPUP_RESUME_PEN_TIME_MS` = **800** on a colour
  device, `PenEvent.java:18`), which the session report covers. `resetEpdPost` (post=1, pen state 0)
  on destroy. Content redraws (row P0) do not touch the pen session at all. [code]
- **Writing.** No refresh on pen-up on this device (§5.8). A stroke that ends during fast mode sets
  `f3199d` so that the fast-mode exit runs `PauseResumeRawPenAction` (a pause/resume cycle) after
  clearing the transient slot (`EpdEventHandler.e()`). `RawPenDownEvent` postpones the fast-mode exit.
  [code]
- **Erasing.** Erase finish = P1 (slice re-render with `enablePost=true` and no view invalidate:
  one `ENABLE_POST(-1,1,pid)`) then A4 (pause both, repaint in mode 5, resume after 300 ms), no GC,
  no special mode. While the eraser tool is active, colour CU is on; with the pen tool active, CU
  is on for the duration of each draw-erase stroke (stylus eraser end) (§5.6). Stock's erase path never
  enters fast mode and never touches the fixed slots, so scheme 4 is **not** entered by stock's own
  code during an erase [code]. If a scheme-4 switch appears in the driver log while erasing in
  stock, it comes from SF's pen path, not from app policy (E7).
- **Selection / lasso.** Lasso pen-up refresh after 300 ms (A5); transforms enter fast mode (2308,
  scheme 4) and leave 5 s later without a GC; while a selection exists, all A-class repaints are
  dropped (A6); colour CU on while the selection tool is active; selection-menu clicks tick the GC
  counter. [code]
- **Refresh.** All app-initiated full GCs go through one-shot 98 via `applyGCOnce` + invalidate
  (G1–G4); no `refreshScreen`/`handwritingRepaint`/`HAND_WRITING_REPAINT_MODE` in the editor. [code]
- **Input routing.** Scroll/zoom begin is where fast mode starts (`ViewportEventHandler`), with a
  `PauseResumeRawPenAction`; the exit waits for all fingers up (`touchStateInfo.getG()`). [code]
- **Firmware / EinkWise.** §7: resume-time refresh and display blocks apply; the tap counter and
  debouncer do not; the dither-threshold writes persist into stock's config and the fallback.

---

## 10. Corrections to earlier notes

| Earlier statement                                                                                                        | Correction                                                                                                                                                   |
| ------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `docs/planning/eink-panel-upkeep.md` §1.3 lists `ScribbleHandler.onRefreshPage()` → `repaintEveryThing(GC)` as a trigger | Unreachable: `RefreshPageEvent` is never constructed (§5.8)                                                                                                  |
| Same doc §1.1 counts `repaintEveryThing(GC)` ×2 as live primitives                                                       | Both editor sites are dead; the live one is the library screen's Refresh button                                                                              |
| Earlier reports say "Android 13"                                                                                         | Correct: system partition Android 13; the `:11/` in the fingerprints is the vendor partition (verified on the tablet, 00-overview §0)                        |
| 05 §1.5 / §2.1: `supportRegal()` "returns false here"                                                                    | It is a kernel ioctl (`0x7009`); the value is not in the dump and is **[unknown]**. Stock does not consult it before the REGAL_PLUS one-shot anyway          |
| `08-surfaceflinger.md` §1 labels `0xff0021` as "REPAINT_EVERYTHING (mode)"                                               | Per the jump table `0xff0021` (16711713 = Java `setTrigger`) writes a 5/`-1` entry at `0xac9da8`; `REPAINT_EVERY_THING_WITH_MODE` is `0xff0023` → `0x5601a4` |
| `setDisplayScheme` is listed as an app lever (05 §2.1, "acceptance unverifiable")                                        | SF has no handler for `0xff0015`: it is a no-op                                                                                                              |

---

## 11. Open questions as hardware experiments

All with `adb logcat -b all -v time` running and a stopwatch or screen recording. "Driver lines" means
`onyx_tcon_send_cmd`, `onyx_tcon_set_upd_scheme`, `HWEpdcManager: ### EUS`,
`SurfaceFlinger: Pid … State change`, and `waveform_mode  wrong`.

**E0: firmware identity.** `adb shell getprop ro.build.fingerprint`, `getprop vendor.onyx.tablet`,
`getprop vendor.onyx.htcon`, `getprop ro.board.platform`. Confirms whether the dumped 4.2 sources
match, and resolves the `isTablet`/`hasHTCon` branches (§6).

**E1: stock's stored EAC config.** (root) `strings /onyxconfig/mmkv/onyx_config | grep -A2 com.onyx.android.note`,
or `adb shell dumpsys oec_service`. Look for `refreshModeIndex`, `updateMode`, `turbo`,
`ditherThreshold` (expect 128; 160 or 180 would confirm §7.3), `useGCForNewSurface`, and the
fallback display config's `ditherThreshold`. Also check `KEY_MAINTAIN_PREV_PKG_REFRESH_PKG_SET`
membership if visible.

**E2: stock's app settings.** Read stock's `NoteSetting` (Settings → auto full refresh / count), or
count page turns until `update_mode[1]` appears. Hypothesis: exactly every 10th page turn gives one
`waveform[2] update_mode[1]`, arriving together with a `State change to: toPause` and then `running`
about 500 ms later. Refuted if GCs appear on other counts or without the pause.

**E3: the base repaint.** In stock, open and close the pen-colour popup. Expected on **show**: a
toPause (pen state 3), then one partial update (`update_mode[0]`) whose waveform is chosen by the
kernel, with no `### EUS`. On **dismiss**: a `waveform_mode  wrong!` line followed by an
`onyx_tcon_send_cmd` for the editor rect, expected with `waveform[0]` (SF sends 0, §4.4) unless the
kernel substitutes. Refutes §4.4/§5.1 if dismiss produces a normal waveform without the "wrong"
line. Note (§5.1a) that the REGAL one-shot may be consumed by a content invalidate that happens to
run first, so the "wrong" frame may not coincide exactly with the pause.

**E4: what the REGAL_PLUS frame looks like on panel.** Same as E3, closing a popup over dense
handwriting. Record the `waveform[...]` value on the frame after `waveform_mode wrong`, and whether
the area under the popup is actually redrawn (ghost left or not). Decides whether stock's dismiss
repaint is effectively "no update" on this panel.

**E5: fast mode.** Two-finger scroll a page in stock, lift, wait 6 s. Expected: `### EUS, s 4 c 0`
at scroll start; A2-class `waveform[4]` updates during the scroll; about 5 s after the last finger
up, `### EUS, s 2 c 0` and **no** `update_mode[1]`. A GC at the end refutes the `clearTransientUpdate(false)`
reading. Variant: press the toolbar Refresh button within the 5 s window and compare its
`waveform/flags` with a Refresh press outside fast mode (tests the `[0xaca250]` branch of
`applyGCOnce`).

**E6: colour CU flag.** Draw a stroke with the pen tool, then select the eraser tool and erase, then
the lasso tool. Compare `flags[...]` on the partial updates in each tool. Hypothesis: eraser, lasso
and shape tools (and open popups) add a flag bit (bit 22 = `0x400000` in the mode word, possibly
translated to `0x200000`) that pen-tool frames lack, and `EpdcManager: CU 1` / `CU 0` lines appear
on tool switches.

**E7: where the post-lift GC comes from.** Reproduce the known flash in Tangleaf (pause session, draw
a stroke) and look for `SurfaceFlinger: hasHwTcon: 1` about 1.5 s after the lift, just before the
`waveform[2] update_mode[1]`. Present → the flash is `ExtBufferSchema::onDeactivate` (§8.2), and the
pen report should find what activates the ExtBuffer schema. Absent → look for another `### EUS, s 2`
source. Repeat the same stroke-then-erase sequence in stock with the stylus eraser end and confirm
that stock produces neither `hasHwTcon` nor `### EUS, s 4` during or after the erase.

**E8: the dither-threshold side effect.** In stock, start a pinch, then within 5 s open the
floating-ball menu or a system dialog so another component is on top when the exit timer fires.
Then `dumpsys oec_service` for the other package's `ditherThreshold`. Confirms or refutes §7.3's
"wrong app" write.

**E9: material centre.** Open and leave the stock material store. Expected: `### EUS, s 4` on
entry, and on leaving `### EUS, s 2` **plus** one `update_mode[1]` (clear flag 1 → applyGCOnce).

**E11: colour CU during an eraser-end stroke with the pen tool.** In stock, with the pen tool
selected, erase one stroke with the stylus eraser end. Expected: `EpdcManager: CU 1` at the start
of the erase, `CU 0` when the erase points are delivered, and no `### EUS`. Compare `flags[...]` on
the partial updates during the erase with those of a normal pen stroke. Refuted if no `CU` lines
appear (then the eraser end does not reach `onBeginRawErasing` in the pen handler).

**E12: system "full refresh" gesture in stock.** Trigger the system refresh key/gesture while stock
shows a note. Expected from stock's side: one pen pause (`toPause`), one partial mode-5 update of
the editor, `running` about 500 ms later. Any `update_mode[1]` in the same window comes from the
firmware side, not from stock (stock only receives the `REFRESH_SCREEN` broadcast).

**E10: resume-time EAC push.** Switch from an app pinned to A2 by EinkWise ("Speed" profile) to
stock. Expected: `### EUS, s 2` after stock resumes (its HD block clears the app-scope slot). The
flag-2 deferral means any GC comes only with the next new layer. Also confirm the turbo change:
turbo 0 if stock's stored turbo is 0, and turbo 5 again after stock's first fast mode.

---

## 12. Verification notes

Independent re-check of this report against the primary sources (static only, no device access).
Paths as in "Source shorthands".

### 12.1 Checked and confirmed

| Claim                                                                                                                                                | How it was checked                                                                                                                                                            |
| ---------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `InkRenderManager` sequence (one-shot → enableRegal → setUpdateMode → invalidate → reset), default `i = REGAL_PLUS`                                  | read `sdk/notecore/editor/display/InkRenderManager.java:26-141`                                                                                                               |
| No `config_note_noteair4c.json`; bean default `"REGAL_PLUS"`; `DisplayBundle.init()` applies it                                                      | listed `res/raw/config_note_*`; read `DeviceNoteConfig.java:38-90`, `DeviceNoteConfigBean.java:39`, `DisplayBundle.java:81-86`                                                |
| `UpdateMode` → int mapping and `ViewUpdateHelper` constants (9, 98, 2308, 524290, 5)                                                                 | read `SDMDevice.h()` `:1593-1634`, the `createDevice()` field reads, `fw/ViewUpdateHelper.java:55-100`                                                                        |
| Transaction codes (`0xff0014/15/23/26/2a/66/67`, `0x100040/51/55/75/80`) and their SF jump-table targets                                             | decimal constants in `fw/ViewUpdateHelper.java:109-199`; jump tables at `0x341444` / `0x341094` decoded with a script over the ELF                                            |
| `hwScreenRefresh` nibble table, `update_mode = bit 5`, bit 18 → `0x40000`, bit 12 → `0x8000` for REGAL                                               | disassembled `0x55b074-0x55b2a0`, decoded byte table `0x340e45`                                                                                                               |
| `ExtBufferSchema::onDeactivate`: one-shot 98 count 2 when `vendor.onyx.htcon`, log `hasHwTcon: %d`                                                   | disassembled `0x55ba5c-0x55bb00`, resolved strings `0xfa45f`, `0x106fce`                                                                                                      |
| Pause path = `leaveScribbleMode` (`ENABLE_POST(-1,1,pid)`) + pen state 3; resume = pen state 2                                                       | `PauseRawPenAction.n()`, `NoteManager.java:856-883`, `TouchHelper.java:276-306`, `SFTouchRender.java:263-285,368-407`, `EpdPenManager.java:28-39`, `SDMDevice.java:1875-1877` |
| Resume delay 500 ms on colour (`RawPenArgs.f`, `PenEvent.DELAY_ENABLE_RAW_DRAWING_MILLS`), `pauseResumeArgs()` uses it                               | `RawPenArgs.java:58-69,231-233`, `PenEvent.java:16-19`                                                                                                                        |
| Counted-GC arithmetic (10th tick after `checkIntervalChanges(10)`), the four tick sites, the two immediate sites                                     | read `common/utils/EpdDeviceManager.java:8-63`; grep for all callers                                                                                                          |
| `EpdEventHandler`: `c` never assigned (false), enter/exit sequence, 5 s timer, event wiring                                                          | read the whole class `:48-320`                                                                                                                                                |
| Pen-up reconcile gated by `isTablet()` first                                                                                                         | `OnyxSystemConfig.java:145-150`, `SystemConfigBean.java:22`, `SystemPropertiesUtil.java:10-14,49-58`                                                                          |
| `RefreshPageEvent` never constructed                                                                                                                 | grep over both decompilations; the class is empty; only `classes5.dex` references the name                                                                                    |
| `supportEAC` input-pipe filter; `TabletEACRefreshImpl.onResume` reads `getRefreshConfig` directly; `handleSFDebouncer` clears for `supportEAC=false` | read `EInkHelper.java:1127-1170`, `TabletEACRefreshImpl.java:81-150,240-250`                                                                                                  |
| `setDitherThreshold` persistence into the app config and the fallback display config, on the top component                                           | `EInkHelper.java:1570-1579`, `OECService.java:1528-1530,1908-1920`, `TabletEACDisplayImpl.java:142-150`                                                                       |
| `View.invalidate()` captures the mode synchronously via `ViewGroup.invalidateChild` → `ViewRootImpl.addUpdateEntry`                                  | `aosp/view/View.java:10808-10816`, `ViewGroup.java:4755-4768`, `ViewRootImpl.java:12232-12238`; `EditorView extends View`, app is hardware-accelerated (manifest)             |
| `Device.currentDevice()` = `SDMDevice` for `lito`; `EpdDeviceManager` backend = `EpdSDM`                                                             | `sdk/device/Device.java:90-106`, `sdk/api/device/EpdDeviceManager.java:110-131`                                                                                               |

### 12.2 Changed

| Old claim                                                                                                | New claim                                                                                                                                     |
| -------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| §5.1: `DisplayArgs.enablePost` "is false by default and none of the refresh paths in this report set it" | Erase finish (`EraseFinishAction.render()`), `ErasingShapeAction` and `displayNoteDirtyRect(z)` set it; new row P1                            |
| §5.1: `view.invalidate()` is always called                                                               | Only if `displayArgs.invalidate`; erase finish passes `false`                                                                                 |
| §1 / §3: "Stock never flashes"; "no per-stroke refresh"                                                  | Reworded to what the code requests; per-stroke plain `View.invalidate()` (row P0) and the gated 500 ms pen-up timer are now stated            |
| §4.4 / §5.1: invalid nibble "continues with no waveform set"                                             | The ioctl struct's waveform field is 0 (zeroed at `0x55b0a4`), flags 0 [code]; kernel effect unknown                                          |
| §6 and §9: popup resume 500 ms                                                                           | `POPUP_RESUME_PEN_TIME_MS` = 800 on colour (400 on mono)                                                                                      |
| §3 G4: text input start/finish gives a GC                                                                | Only when fast mode is active and the active handler does not render in fast mode; otherwise nothing                                          |
| §5.2 trigger table: pen/shape change uses `ScribbleHandler.refreshScreen()` with the default recipe      | They use their own private `refreshScreen()` with `setResumeRawDrawing(false)`; `ScribbleHandler.refreshScreen()` has no caller               |
| §5.6: CU owners are erase, selection, universal-shape handlers; pen handler never                        | Also `NormalShapeHandler`; and the pen handler toggles CU around every draw-erase stroke (stylus eraser end)                                  |
| §5.8 / §6: `isTablet` unknown but "either value leaves the reconcile off"                                | True only if the device-wide stored `SystemConfigBean.enablePenUpRefresh` is false; that value is read from `readSystemConfig` and is unknown |
| §5.8: `ColorDeviceUtils` no callers **[prior doc]**                                                      | Confirmed by grep [code]                                                                                                                      |
| §5.2: census of ~40 sites **[prior doc]**                                                                | Replaced by a grep-based census in this report                                                                                                |
| §6: `note_noteair4c.json` is loaded [code]                                                               | File content [code]; that `Build.MODEL` maps to it is [inferred]                                                                              |
| §4.1: `createDevice()` `:447-497`                                                                        | Starts at `:447`, runs to about `:710`                                                                                                        |
| §2: `DeviceNoteConfigBean` under `sdk/scribble/device/`                                                  | It is `sdk/scribble/data/config/DeviceNoteConfigBean.java`                                                                                    |

### 12.3 Added

- Rows P0 (content redraw, unbracketed invalidate) and P1 (erase-finish `ENABLE_POST`) in §3.
- §5.1a: other entry points into `InkRenderManager.invalidate`, and that a pending REGAL one-shot is
  consumed by whichever invalidate runs first [inferred].
- `TouchHelper` no-op guard and `resetPenDefaultRawDrawing()` side effect of every pause/resume.
- System `REFRESH_SCREEN` broadcast → `FullScreenRefreshEvent` → partial repaint (§5.2).
- Ordering caveat for the counted GC and the page-action raw-drawing resume (§5.3).
- Fast-mode exit timer is not re-armed when a finger is down (§5.4).
- `BaseActivity.resumeComponentsImpl` flexible-screen guard (§5.5).
- Dead: `ScribbleHandler.refreshScreen()`, `EpdDeviceManager.enterAnimationUpdate/exitAnimationUpdate`.
- Experiments E11 (CU on eraser end) and E12 (system refresh gesture).

### 12.4 Not confirmed / remaining doubts

- §8.1: the claim that vtable slot `+0x78` has exactly three callers, the `usleep` values and the
  `setFBSchema` fast rule were not re-disassembled; they stay as written (partly [prior doc]).
- §8.2 `HWEpdcManager::clearScreen` map contents and the "clear flag 1 → GC" table remain
  **[prior doc]**.
- `HWNormalSchema::output` candidate order and last-wins arbitration (§4.3) remain **[prior doc]**;
  whether a mode-9 entry actually reaches `hwScreenRefresh` unmodified is therefore still inferred.
- The `ENABLE_POST`, pen-state and `enableRegal` SF effects are cited to `08-surfaceflinger.md` and
  not re-verified here.
- MaterialCenterFragment, DefaultRefreshListener, OnyxBaseDialog, ViewDragHelper rows (§5.5) and the
  §7.2 `kcb` / `ValidateAppConfigAction` chain were not re-opened.
- Whether the stylus eraser end is delivered to the pen handler as `onBeginRawErasing` (needed for
  the new CU finding) is taken from the pen/erasing reports, not re-traced here (E11).
- `PopupChangeAction.q` = "show" is inferred from the `PopupWindowChangeEvent(true)` it posts on the
  pre-step; the constructor callers were not traced.
- Firmware match: resolved (E0). The tablet runs the dumped 4.2 build; SF and `framework.jar` hashes match.
