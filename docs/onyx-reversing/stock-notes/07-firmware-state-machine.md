# 07 — Firmware and SurfaceFlinger state machine behind the stock Notes pen path

Scope: what the firmware does with the calls that the stock Onyx Notes app (`com.onyx.android.note`,
"knote2") makes through its bundled SDK: Binder transaction, SurfaceFlinger (SF) handler, the state
each one changes, timers, and what shows up in `logcat -b all`. The special question is when SF puts
the panel into update scheme 4 (A2) and when it issues the full-screen GC16 afterwards.

Research only. Nothing here proposes a change to Tangleaf.

Tags: **[code]** = read in a binary or in decompiled source (location given); **[inferred]** =
reasoned from code but not read directly; **[unknown]** = not determinable statically, turned into an
experiment in §9.

## 0. Sources and a version caveat

| What                      | Where                                                            | Notes                                                                                                                                                                                                                                                           |
| ------------------------- | ---------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| SurfaceFlinger            | `~/tmp_zfs/onyx_framework/native/surfaceflinger`                 | Stripped, but `.gnu_debugdata` holds 22,518 local symbols (`llvm-objcopy --dump-section=.gnu_debugdata=… \| xz -d`, then `nm -C`). All `0x…` addresses below are virtual addresses in this binary. `FUN@addr` means the function whose symbol starts at `addr`. |
| Framework                 | `~/tmp_zfs/onyx_framework/out/sources` (jadx of `framework.jar`) | `android/onyx/ViewUpdateHelper.java`, `android/onyx/optimization/*`, `android/view/ViewRootImpl.java`, `android/webkit/WebView.java`, `android/view/View.java`                                                                                                  |
| Stock Notes + bundled SDK | `~/tmp_zfs/boox-notes-inspect/decoded/sources`                   | `com/onyx/android/sdk/pen/*`, `com/onyx/android/sdk/device/SDMDevice.java`, `com/onyx/android/sdk/api/device/epd/EpdController.java`                                                                                                                            |
| In-process pen reader     | `~/tmp_zfs/onyx_framework/native/libonyx_pen_touch_reader.so`    | The native side of the SDK's Java `RawInputReader` (JNI `Java_com_onyx_android_sdk_pen_RawInputReader_*`). Runs in the app process, not in SF.                                                                                                                  |

**Version.** The binaries analysed here were pulled on 2026-09-07 from build
`2026-04-28_17-50_4.2-rel_04282_555977efe` (system Android 13, vendor Android 11). Verified on the
tablet 2026-09-25 (E0 done, see [00-overview §0](00-overview.md)): it runs this build, and
`/system/bin/surfaceflinger`, `framework.jar`, `libonyx_pen_touch_reader.so` and `libneo_pen.so` are
byte-identical (sha256) to the analysed copies, so every offset below applies to the device. The
"Android 11, firmware 4.1" in the original research brief was wrong; `:11/` in Onyx fingerprints is
the vendor partition.

Reverse-engineering helpers (scratch, outside the repo): annotated disassembly of
`0x520000–0x570000` with string and symbol resolution, a vtable dumper and a transaction-table
resolver, in the session scratchpad under `sf/`.

## 1. Summary

1. **SF never picks scheme 4 because of the pen state.** SF calls `setEbcUpdateScheme(4)` from three
   places only, and all three take the scheme from the **fixed update-entry slots** (app-scope and
   transient-scope update modes): `HWEpdcManager::setFBSchema` (`0x565c78`),
   `HWEpdcManager::syncFBSchema` (`0x565f74`) and `HWEpdcManager::clearHandwritingFBSchema`
   (`0x566134`) [code]. Scheme 4 is chosen when that slot holds a "fast" mode (A2/DU family), scheme 2
   otherwise. The pen session itself only toggles **scheme 3** (enter handwriting) and back to 2 or 4
   (leave handwriting) [code].
2. **The pen-state dependency is in the Java framework, not in SF.** `ViewRootImpl.deliverInputEvent`
   feeds every MotionEvent of a non-Onyx app to `OnyxScrollRefreshManager`. For a mostly vertical move
   longer than the touch slop it calls `EInkHelper.beforeScroll` → `ScrollHelper` →
   `ViewUpdateHelper.applyTransientUpdate(2308 = UI_A2_QUALITY_MODE)`. The only exemption is
   "stylus AND SF pen state == 2 (running) at ACTION_DOWN" [code, `OnyxScrollRefreshManager.java:67-99`].
   On lift, `afterScroll` waits `Settings.Global scroll_refresh_delay_v2` (default 1000 ms) and then
   calls `clearTransientUpdate(gcAfterScrolling)` [code, `ScrollHelper.java:68-86`]. In SF that clears
   the transient slot, switches back to scheme 2 (after a 100 ms sleep), optionally queues a one-shot
   GC (mode 98), and in every case queues a **full-screen** update [code, §5.3]. This matches the
   observed "scheme 4 + A2 while writing on a paused session, back to scheme 2 with one full-screen
   update ~1.5 s after lift" [inferred; confirm with E1–E4]. A second, independent entry into the same
   `ScrollHelper` exists in `WebView.onTouchEvent` → `applyFastMode` (vertical move ≥ 5 px, **no
   pen-state gate**) [code, `WebView.java:374-392,1115-1117`].
3. **Why stock Notes does not flash.** `OnyxScrollRefreshManager.deliverMotionEvent` returns early for
   any package that `ActivityManagerHelper.isOnyxApp` accepts (contains `"com.onyx"`) [code,
   `OnyxScrollRefreshManager.java:135`]. Stock's note canvas is not a `WebView`, so the WebView entry
   does not apply either [inferred]. Tangleaf is a non-Onyx package whose editor is a WebView, so both
   entries are live for it [inferred]. (Verification note.) Stock is **not** free of transient A2
   periods: it applies `applyTransientUpdate(ANIMATION_QUALITY = 0x904)` itself for zoom, viewport
   drag, selection transform, text-box moves, list drags and more (`EpdEventHandler.a()` plus the
   `kui`/`textbox` helpers, §2.2), and every clear of those goes through the same SF lambda (§5.3)
   that queues a full-screen mode-5 update [code for the calls; the visible result is [unknown]].
   Stock's eraser also runs through SF's own eraser reader, not a paused session (§4.4, §8). So the
   difference to Tangleaf is _who_ triggers the transient A2 and _when_, not that stock never uses it.
4. **Pen state 4 is not "erasing" to SF.** The SDK names 4 `PEN_ERASING`
   (`EpdPenManager.java:10`), but stock never sends it [code, grep]. SF's
   `HWEpdcManager::setHandWritingPenState` handles only 0–3 and returns for 4 [code, `0x5666cc`].
   `handleSetHandWritingPenState` still logs `Pid N State change to: paused` (4 is `"paused"` in
   `TouchReader::ctrlStateStr`) whenever the reader is not already in state 4, so a 4 produces a
   misleading log line and no state change [code, `0x560c4c`].
5. **There is no "render off but not paused" state in SF.** The SDK's
   `setRawDrawingRenderEnabled(false)` = `ENABLE_POST(1)` + pen state 3 [code,
   `SFTouchRender.java:270-273`]. `ENABLE_POST(v)` only does "if androidDrawing ≠ v, stopHandwriting"
   [code, `0x560bf8`]. Pen state 3 pauses the SF reader: SF stops inking, `GET_PEN_STATE` returns 3 and
   then 4. Staying in state 2 while SF does not ink is possible only by switching the SF reader to a
   no-op reader: `setBrushRawDrawingEnabled(false)` for the tip, or the eraser end with
   `setEraserRawDrawingEnabled(false, …)` [code, §4.4]. In that combination `GET_PEN_STATE` stays 2,
   which is exactly what the framework's stylus exemption tests [inferred].
6. **What "pause" does inside SF:** reader state 2 → 3 → 4, the SF pen reader drops all samples, and
   SF leaves the handwriting schema (7). Leaving it calls `clearHandwritingFBSchema`, which logs
   `### EUS, s 2 c 0` (or `s 4` if an A2 slot is active) after sleeping 150 ms if the previous scheme
   was 3 [code, §4.2, §5.1].

## 2. Central objects, globals and entry points

### 2.1 SF singletons and fields

| Item                  | Address / offset                                                    | Meaning                                                                                                        | Evidence                                                                                                                                       |
| --------------------- | ------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| EPDC manager          | `*(0xac9fb8)`                                                       | `HWEpdcManager` on this device (see §7)                                                                        | [code] `setSurfaceFlingerProvider@0x55eba4` reads `property_get_bool("vendor.onyx.htcon")`, logs `hasHwTcon: %d`                               |
| TouchReader           | `*(0xac9fc8)`                                                       | SF-side pen reader thread (reads the EMR input device)                                                         | [code] `handleSetHandWritingPenState` loads `[0xac9fc8]+0x98`; `HWEpdcManager::setHandWritingPenState` calls `TouchReader::pause(*(0xac9fc8))` |
| TouchReader +0x98     | ctrl state                                                          | 0 `stop`, 1 `start`, 2 `running`, 3 `toPause`, 4 `paused`, 5 `toResume`                                        | [code] `TouchReader::ctrlStateStr@0x52c504`, `pause@0x52c7c0` writes 3, waits for 4                                                            |
| TouchReader +0x9c     | loop state                                                          | 0x100 `loop init`, 0x101 `loop ready`, 0x102 `loop running`, 0x103 `loop stop`                                 | [code] same function, `threadLoop@0x52ce34`                                                                                                    |
| PenManager (SF)       | `TouchReader+0x120`, readers at `+0x8…+0x28`, current index `+0x30` | reader 0 `BtnReader`, 1 `BaseDrawReader` (no-op), 2 `DrawReader`, 3 `BaseEraseReader` (no-op), 4 `EraseReader` | [code] `PenManager::PenManager@0x52e0c8` vtables `0xa33780/8d0/850/910/890`; no-op `onSynEvent` = `0x435dc8` (empty body)                      |
| PenManager +0x2054    | "paused" byte                                                       | set 1 on toPause→paused, 0 on toResume→running                                                                 | [code] `threadLoop` `0x52d19c–0x52d1ac`                                                                                                        |
| pen owner pid         | `0xac9f90`                                                          | written by every `SET_SCREEN_HANDWRITING_PEN_STATE`                                                            | [code] `0x560cbc`                                                                                                                              |
| watch-app pids        | `std::vector<int>` at `0xac9ed8`                                    | appended by `addToWatchApp`                                                                                    | [code] `0x562c70`                                                                                                                              |
| penTriggered          | byte `0xaca244`                                                     | 1 while SF is in a handwriting session triggered by the pen                                                    | [code] `afterStartHandwriting@0x562c08` sets, `afterStopHandwriting@0x562ec0` and the reader thread clear                                      |
| pen mutex             | `0xac9eb0`                                                          | serialises ENABLE_POST and pen-state transactions                                                              | [code]                                                                                                                                         |
| FBDev                 | `0xac9e68`                                                          | `/dev/ebc` wrapper; **`+0x30` = current update scheme (low nibble)**                                           | [code] `FBDev::setEbcUpdateScheme@0x55a970` stores `w1 & 0xf` to `+0x30`, then `ioctl(fd, 0x7006, &w1)`                                        |
| one-shot update entry | mode at `0xac9dd8`, count at `0xac9de0`                             | consumed by `EpdcManager::next@0x55908c` on the next composition                                               | [code] `applyGCOnce@0x55904c` writes 98 / count 1                                                                                              |
| SF debug flag         | byte `0xac9ea0`                                                     | gates the `### …` debug logs                                                                                   | [code] set by `applySFDebug@0x561b78` (transaction `APPLY_SF_DEBUG` 1048642)                                                                   |
| eraser raw drawing    | byte `0xac9ad8`, style int `0xa5e708`                               | `SET_ERASER_RAW_DRAWING_ENABLED`                                                                               | [code] handler `0x55f8e0`                                                                                                                      |
| brush raw drawing     | byte `0xa5e704`                                                     | `SET_BRUSH_RAW_DRAWING_ENABLED`                                                                                | [code] handler `0x55f4f4`                                                                                                                      |
| side-button erase     | byte `0xa5e700`                                                     | `SET_ENABLE_PEN_SIDE_BUTTON`                                                                                   | [code] handler `0x55faa0`                                                                                                                      |

EPDC (`HWEpdcManager`) fields used below:

| Offset            | Meaning                                                                                                     | Evidence                                                                           |
| ----------------- | ----------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `+0x20`           | screen-update enable (`ENABLE_UPDATE`, code 16711680)                                                       | [code] handler `0x55fce0`                                                          |
| `+0x24`           | display power state (the fixed-slot calls return early when it is 1–3)                                      | [code] `applyTransientUpdateMode@0x559924`; meaning from `07-einkwise.md` §5.4.7   |
| `+0x28`           | `androidDrawing` (1 = SF composites Android layers normally)                                                | [code] `afterStartHandwriting` writes 0, `afterStopHandwriting` writes 1           |
| `+0x2c`           | bypass counter (`BYPASS`); `REPAINT_EVERY_THING` zeroes it                                                  | [code] `0x55f538`, `0x56007c`, `bypassUpdate@0x5591b0`                             |
| `+0x30`           | new-surface policy (`USE_GC_FOR_NEW_SURFACE` → 1)                                                           | [code] `0x55f6b0`, `HWEpdcManager::getNewLayerUpdateEntry@0x5664c0`                |
| `+0x3c`           | `timed_mutex`, taken with a 300 ms `try_lock_for` by start/stopHandwriting, fixed-slot calls                | [code]                                                                             |
| `+0x98` / `+0x9c` | `EpdcSchemaManager` / current schema id (0 Normal, 1 Debouncer, 5 PowerOff, 6 Boot, 7 Handwriting, 8 Dream) | [code] `startHandwriting@0x558568` activates 7; ids from `08-surfaceflinger.md` §3 |
| `+0xb8`           | `EpdcWrapper` list of pending extra update rectangles                                                       | [code] used by the clear-transient lambda (§5.3)                                   |
| `+0x1e8`          | `FixedUpdateEntryManager` (app-scope and transient-scope slots)                                             | [code]                                                                             |
| `+0x25c`          | enhance enable                                                                                              | [code]                                                                             |
| `+0x25d`          | ext-buffer auto-sync enable (`SET_AUTO_SYNC_BUF_ENABLE`)                                                    | [code] `setAutoSyncBufEnable@0x566878`                                             |
| `+0x260`          | `EnhanceStrategy` (180 s idle timer, only enabled under scheme 4)                                           | [code] `EnhanceStrategy::EnhanceStrategy@0x5564e4` (0x2bf20 = 180000 ms)           |
| `+0x280`          | 12-byte object, flag set to 1 when scheme 4 is entered, cleared otherwise                                   | [code] `setEbcUpdateScheme` `0x565ef4–0x565f58`; consumer [unknown]                |
| `+0x288`          | "sync FB schema" flag (`enableSyncFBSchema`/`shouldSyncFBSchema`)                                           | [code]                                                                             |
| `+0x8` (byte)     | set to 1 by `clearHandwritingFBSchema` when not in fast mode                                                | [code] `0x566254`; consumer [unknown]                                              |

`HWEpdcManager` vtable (`0xa354d0`) [code]: `+0x18 applyGCOnce`, `+0x28 inFastMode`,
`+0x58 clearScreen`, `+0x60 syncFBSchema`, `+0x68 setFBSchema`, `+0x70 clearHandwritingFBSchema`,
`+0x78 setEbcUpdateScheme`, `+0xd8 enableSyncFBSchema`, `+0xe0 shouldSyncFBSchema`,
`+0xe8 setHandWritingPenState`, `+0xf0 afterPresent`.

SF provider (the `SurfaceFlinger` object seen through its Onyx interface, `*(0xac9ea8)`, vtable
`0xa2f5a0`) [code]: `+0x10` and `+0x28` both resolve to `0x4f68a4`, which ICF gave the symbols
`SurfaceFlinger::refresh` and `SurfaceFlinger::repaint`; it sets two flags at `this+0x41a/0x41b` and
schedules a composite. It **does not** call `stopHandwriting` [code]. This corrects
`08-surfaceflinger.md` §1, which put `REPAINT_EVERYTHING` in the same group as `stopHandwriting`.

### 2.2 Binder entry points the stock app uses

On this platform (`ro.board.platform = lito`) the bundled SDK chooses `SDMDevice`
(`Device.java:74-105,118-121`, lito → `DeviceIndex.SDM`) [code]. `SDMDevice` reaches the framework by
reflection into `android.onyx.ViewUpdateHelper` (`SDMDevice.java:453-694`) [code]. `ViewUpdateHelper`
writes a Parcel and calls `transactData(code, …)` on SurfaceFlinger [code, `ViewUpdateHelper.java`].
SF dispatches in `SurfaceFlingerHelper::processOnyxRequests@0x55f260` through two jump tables
(`0x341094` for `0x10002a…`, `0x341444` for `0xff0000…`) [code]; resolved handler addresses below.

Stock call sites were counted with `grep -rhoE "EpdController\.[a-zA-Z]+\("`. The pen SDK's
`SFTouchRender`/`EpdPenManager`/`RawInputReader` account for the pen-session calls; the app itself
(`com/onyx/android/note`) adds the refresh calls.

| SDK entry (bundled)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | Framework method → code                                                                                         | SF handler                                                                                                                     | SF effect                                                                                                                                                                                                  | Driver-log / logcat evidence                                                                                              |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `EpdController.setScreenHandWritingPenState(view, s)` (via `EpdPenManager` start 1 / resume 2 / pause 3 / quit 0; `AppTouchRender.openDrawing` 1 / `closeDrawing` 0; `sdk/api/device/EpdDeviceManager.start/stopScreenHandWriting` 1/0 has **no caller** in the apk; test activities only otherwise)                                                                                                                                                                                                                                                                                                                          | `setScreenHandWritingPenState(int)` → `SET_SCREEN_HANDWRITING_PEN_STATE` 16711693 (0xff000d), parcel `(s, pid)` | `0x55ff1c` → `handleSetHandWritingPenState(s, pid)@0x560c4c`                                                                   | If `s ≠ TouchReader.state`: log, store pid, `addToWatchApp`, `HWEpdcManager::setHandWritingPenState(s)` (§4.1)                                                                                             | `SurfaceFlinger: Pid %d State change to: %s`, `SurfaceFlinger: State changed!` (not gated)                                |
| `EpdController.enablePost(view, v)` / `leaveScribbleMode` (= `enablePost(1)`) / `enterScribbleMode` (= `enablePost(0)`). Stock callers: `SFTouchRender.g()/h()` (render off, close), `AppTouchRender.openDrawing` (0) / `setDrawingRenderEnabled(false)` (1) / `closeDrawing` (1), **`InkRenderManager.invalidate(displayArgs)` when `displayArgs.b` is set** (`notecore/editor/display/InkRenderManager.java:79-97`), `EnablePostAction` (meeting view models only). `BaseRenderer.beforeUnlockCanvas` has no caller                                                                                                         | `enablePost(int)` → `ENABLE_POST` 16711692 (0xff000c), parcel `(-1, v, pid)`                                    | `0x55fef0` → `handleEnablePost(v)@0x560bf8` (only the second int is used)                                                      | If `v ≥ 0` and `androidDrawing ≠ v` → `EpdcManager::stopHandwriting`                                                                                                                                       | Indirect: `Post enabled!` and `### EUS, s 2 c 0` if a handwriting session was active                                      |
| `EpdController.resetEpdPost()` (stock: `ScribbleFragment.java:1492` on destroy, `EpdEventHandler.destroy()` `:197`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | `resetEpdPost()` = `ENABLE_POST(-1,1,pid)` then pen state 0 (`ViewUpdateHelper.java:1071-1078`)                 | as above                                                                                                                       | stopHandwriting (if active), then STOP reader                                                                                                                                                              | as above + `State change to: stop`                                                                                        |
| `EpdController.moveTo / quadTo` (app-driven SF painting). Stock callers: `EpdPenManager.moveTo/quadTo` ← `PenShape.onDown/onMove/onUp` when the static `PenShape.W` ("useRawInput") is false, and it is never set (`scribble/shape/pen/PenShape.java:185-219`, setter `:98` has no caller); reached from `NormalShapeHandler.y0()/H0()` (`note/handler/common/NormalShapeHandler.java:101-141`), i.e. the `SCRIBBLE_NORMAL_SHAPE_PROVIDER` used for non-DFB shape types (`HandlerManager.java:212`, `ShapeFactory.isDFBShape`); `AppTouchRender` `:124-178` only when its touch listener is enabled (it is not in the editor) | `MOVE_TO` / `QUAD_TO` (`ViewUpdateHelper.java:974-1040`)                                                        | lambda of `handleMoveTo` `0x564a00`                                                                                            | **if `penTriggered == 0`: logs `HandlePenTrigger, sid: -1 …` and calls `EpdcManager::startHandwriting` → schema 7, scheme 3, independent of the reader state**; then `canScribble` → SF paints the segment | `HandlePenTrigger…`, `### EUS, s 3 c 0`                                                                                   |
| `setScreenHandWritingRegionLimit / Exclude / Mode`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | 16711694 / 16711714 / 1048620                                                                                   | `handleSetHandWritingRegionLimit@0x560d0c`, `…Exclude@0x560e1c`, `…Mode@0x561ad4`                                              | global PenManager limit/exclude arrays and region mode (not keyed by pid)                                                                                                                                  | see `08-surfaceflinger.md` §2                                                                                             |
| `setStrokeWidth / Color / Style`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | 16711687 / 16711686 / 16711688                                                                                  | `handleSetStrokeWidth@0x56070c` etc.                                                                                           | SF ink painter parameters                                                                                                                                                                                  | —                                                                                                                         |
| `Device.setBrushRawDrawingEnabled(b)`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | `SET_BRUSH_RAW_DRAWING_ENABLED` 1048834                                                                         | `0x55f4f4`                                                                                                                     | byte `0xa5e704 = b` → tip selects reader 2 (ink) or 1 (no-op)                                                                                                                                              | —                                                                                                                         |
| `Device.setEraserRawDrawingEnabled(b, style)` (stock default `(false, 5)`, `TouchHelper.resetPenDefaultRawDrawing`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | `SET_ERASER_RAW_DRAWING_ENABLED` 1048833                                                                        | `0x55f8e0`                                                                                                                     | byte `0xac9ad8 = b`, int `0xa5e708 = style` → eraser end selects reader 4 (SF draws with that style) or 3 (no-op)                                                                                          | —                                                                                                                         |
| `Device.setEnablePenSideButton(b)`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | 1048832                                                                                                         | `0x55faa0`                                                                                                                     | byte `0xa5e700 = b` (side button counts as rubber)                                                                                                                                                         | —                                                                                                                         |
| `EpdController.penUp()`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | `PEN_UP` 16711784                                                                                               | `0x55fc68` → `handlePenUp@0x562130` → lambda `0x564e74`                                                                        | `TouchReader::reportUpEvent` injects BTN_TOUCH 0 / BTN_TOOL_PEN 0 / SYN into the SF pen event queue                                                                                                        | —                                                                                                                         |
| `EpdController.applyTransientUpdate(mode)` (stock: `EpdEventHandler.a()` on `ApplyFastModeEvent(true)` from zoom, viewport drag, selection transform, thumbnails, float menu drag, text input, `renderInFastMode()` handlers; also `kui/utils/ViewDragHelper:91` (dialog drag), `ItemDragHelper:481`, `OnyxLinearSnapHelper:77`, `textbox/listener/DefaultRefreshListener.startMove:73`, `FastModeController:73` (translate web view). All pass `ANIMATION_QUALITY` → `SDMDevice.h()` → `UI_A2_QUALITY_MODE` 0x904, or the `DeviceConfig.transformUpdateModel` override in `EpdEventHandler.i()`)                             | `APPLY_TRANSIENT_UPDATE` 16711782                                                                               | `0x560488` → `handleApplyTransientScopeUpdate@0x561fe8` → lambda `0x564458` → `EpdcManager::applyTransientUpdateMode@0x559924` | transient slot = mode; if fast and current scheme ≠ 3: `setFBSchema` → **scheme 4** (§5)                                                                                                                   | `HWEpdcManager: ### EUS, s 4 c 0`; with debug `### applyTransientUpdateMode, updateMode %d`, `### setFBSchema mode 0x%x…` |
| `EpdController.clearTransientUpdate(view, gc)` (stock: `EpdEventHandler.d()` with `gc = false`, reached 5 s after fast mode ends (`b(false)`, `RxTimerUtil.timer(5000)`), immediately on `QuitFastModeImmediatelyEvent`, `BeforePageChangeEvent`, `CloseDocumentEvent`; `BaseActivity.resumeComponentsImpl` `(false)` on resume of every non-Scribble `BaseActivity` (`ScribbleActivity` overrides it); `ViewDragHelper:101`, `ItemDragHelper:102`, `FastModeController:49` with `true`; `OnyxLinearSnapHelper:79` `false`; `DefaultRefreshListener.onQuitEdit:66`)                                                           | `CLEAR_TRANSIENT_UPDATE` 16711783                                                                               | `0x560308` → lambda `0x5644cc`                                                                                                 | if transient slot fast: clear → scheme 2, optional GC, **plus a full-screen mode-5 update** (§5.3)                                                                                                         | `### EUS, s 2 c 0`; debug `### clearTransientUpdateMode, clearFlag %d`                                                    |
| `EpdController.applyGCOnce()`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | `APPLY_GC_ONCE` 16711718                                                                                        | `0x560214` → `HWEpdcManager::applyGCOnce@0x566430`                                                                             | one-shot entry = 98 (GC), or `*(0xaca250)` if scheme is 4                                                                                                                                                  | next composition is a full GC                                                                                             |
| `EpdController.repaintEveryThing([mode])`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | `REPAINT_EVERY_THING` 16711700 / `…_WITH_MODE` 16711715                                                         | `0x56007c` / `0x5601a4`                                                                                                        | bypass count = 0; provider `refresh` (full recomposite). With mode: one-shot entry = mode first                                                                                                            | —                                                                                                                         |
| `EpdController.handwritingRepaint(view, rect)` (stock: only the `@BindingAdapter("repaintView")` in `note/common/utils/DataBindingUtils.java:69-74`; no generated binding call to it was found in the sources [code, grep])                                                                                                                                                                                                                                                                                                                                                                                                   | `HANDWRITING_REPAINT` 1048647                                                                                   | `0x55f5d8` → `setHandWritingRepaintUpdate@0x55828c`                                                                            | pending one-shot handwriting-repaint entry; stays in schema 7                                                                                                                                              | see `08-surfaceflinger.md`                                                                                                |
| `EpdController.useGCForNewSurface(b)` (stock: `SaveProgressPopup` and `BaseActivity.resumeComponentsImpl`, both `true`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | 1048657                                                                                                         | `0x55f6b0`                                                                                                                     | EPDC `+0x30 = b>0` → new layers get a full GC (or entry `0xaca248` under scheme 4)                                                                                                                         | debug `### full gc for new surface`                                                                                       |
| `EpdController.applyAppScopeUpdate / clearAppScopeUpdate` (stock: MaterialCenter only)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | 16711684 / 1048640                                                                                              | `applyAppScopeUpdate@0x558bb4`, `clearAppScopeUpdate@0x558ebc`                                                                 | app slot; same `applyUpdateAction` path as transient → scheme 4/2                                                                                                                                          | `### EUS, s …`                                                                                                            |
| `EpdController.setEpdTurbo(n)`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | 1048661                                                                                                         | `0x55f384`                                                                                                                     | `/dev/ebc` ioctl `0x7024`, global                                                                                                                                                                          | `HWEpdcManager: ET %d`                                                                                                    |
| `EpdController.byPass(n)`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | `BYPASS` 1048621                                                                                                | `0x55f538`                                                                                                                     | EPDC `+0x2c = n` (skip n compositions)                                                                                                                                                                     | —                                                                                                                         |
| `EpdController.enableScreenUpdate(b)`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | `ENABLE_UPDATE` 16711680                                                                                        | `0x55fce0`                                                                                                                     | EPDC `+0x20 = b`                                                                                                                                                                                           | —                                                                                                                         |
| `getPenState()` (framework-internal user: `OnyxScrollRefreshManager`, `OnyxEpdBypassManager`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | `GET_PEN_STATE` 1048643                                                                                         | `0x55f5b0`                                                                                                                     | returns `TouchReader+0x98` (0–5)                                                                                                                                                                           | —                                                                                                                         |
| `isValidPenState()`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | 1048641                                                                                                         | `0x55f584` → `TouchReader::isPenStateValid@0x52c320`                                                                           | polls the reader fd                                                                                                                                                                                        | `Check fd returns: %d`                                                                                                    |
| `applySFDebug(b)` (not used by stock; useful for experiments)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | 1048642                                                                                                         | `0x55f59c` → `0x561b78`                                                                                                        | sets debug flag, prints state                                                                                                                                                                              | `pen control state: %s, penTriggered: %d, current epdc schema: %d, androidDrawing: %d`                                    |

`TouchHelper.setRawInputReaderEnable` does not reach SF: it only drives the in-process Java reader
(`RawInputReader.pause/resume/start/quit`, `RawInputReader.java:573-677`) [code]. That reader calls
`nativeSetPenState(4)` on resume and on quit and `nativePausePen()` on pause
(`RawInputReader.java:573-601`). These go to `libonyx_pen_touch_reader.so`, which stores the value in
its **own in-process** `PenManager+0x30` (`PenManager::setPenState@0xa898`: `str w1,[x0,#0x30]`)
[code]. It is not the SF pen state, and 4 there has no SF meaning [code]. What 4 means to the
in-process reader is [unknown] (not needed for the SF side).

## 3. SDK pen-session calls → SF transactions (as the bundled SDK issues them)

**Corrected in verification.** The earlier text said stock builds only an `SFTouchRender` through
`TouchHelper.create(view, cb)`. That is true for `create(view, cb)` (`TouchHelper.java:38-40`,
`hasStylus ? 2 : 1`), which stock uses only in `note/handler/text/HandWritingHandler.java:155`. The
**note editor** session is created by `NoteManager.bindPenHostView` with
`TouchHelper.create(view, 3, rawInputCallback, false)` (`sdk/notecore/editor/NoteManager.java:231`)
[code]: feature mask 3 = `SFTouchRender` **and** `AppTouchRender`, in that order
(`TouchHelper.java:349-366`), with the app touch listener disabled. `bindPenHostView` then calls
`openRawDrawing()` and, if the window is focused, `forceSetRawDrawingEnabled(false)` (`:245-250`),
which bypasses the cached-flag check (`TouchHelper.java:115-126`). Every `TouchHelper` call below is
therefore fanned out to both renders. `AppTouchRender` adds its own transactions
(`sdk/pen/touch/AppTouchRender.java:200-274`): `openDrawing` = `SET_STROKE_STYLE(0)`, `ENABLE_POST(0)`,
`PEN_STATE(1)`; `setDrawingRenderEnabled(false)` = `ENABLE_POST(1)` (true = nothing);
`closeDrawing` = `SET_STROKE_STYLE(0)`, `ENABLE_POST(1)`, `PEN_STATE(0)`. In SF the second
`PEN_STATE(1)` / `PEN_STATE(0)` equal the live reader state and are ignored (§4.1), and the extra
`ENABLE_POST(1)` finds `androidDrawing` already 1 (§4.4) [inferred from the handlers]. The full
editor sequence is in `01-pen-session-lifecycle.md` §4. `SFTouchRender` maps the public API as
follows [code, `SFTouchRender.java:233-374`, `EpdPenManager.java:28-52`]:

| Public API                                                                                         | SDK sequence                                                                                                           | SF transactions, in order                                                                                                                                          |
| -------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `openRawDrawing()`                                                                                 | `setStrokeStyle(0)`; `RawInputReader.start`; `startDrawing`                                                            | `SET_STROKE_STYLE(0)`; `PEN_STATE(1)`                                                                                                                              |
| `setRawDrawingRenderEnabled(true)` (only when the cached flag changes, `TouchHelper.java:286-296`) | `resumeDrawing`                                                                                                        | `PEN_STATE(2)`                                                                                                                                                     |
| `setRawDrawingRenderEnabled(false)`                                                                | `leaveScribbleMode`; `pauseDrawing`                                                                                    | `ENABLE_POST(1)`; `PEN_STATE(3)`                                                                                                                                   |
| `setRawInputReaderEnable(b)`                                                                       | in-process reader pause/resume only                                                                                    | none                                                                                                                                                               |
| `setRawDrawingEnabled(b)`                                                                          | render(b) + reader(b) + `resetPenDefaultRawDrawing`                                                                    | as above, then `SET_BRUSH_RAW(1)`, `SET_ERASER_RAW(0, 5)` (on **every** call, true or false; stock re-enables the eraser trail in `ResumeRawDrawingRequest`, §4.4) |
| `forceSetRawDrawingEnabled(b)` (stock: `NoteManager.bindPenHostView`)                              | render(b) + reader(b), no flag check, no `resetPenDefaultRawDrawing`                                                   | `false`: `ENABLE_POST(1)`, `PEN_STATE(3)` (+ `AppTouchRender` `ENABLE_POST(1)`)                                                                                    |
| `closeRawDrawing()`                                                                                | `setStrokeStyle(0)`; `leaveScribbleMode`; `RawInputReader.pause`; `pauseDrawing`; `RawInputReader.quit`; `quitDrawing` | `SET_STROKE_STYLE(0)`; `ENABLE_POST(1)`; `PEN_STATE(3)`; `PEN_STATE(0)`                                                                                            |
| `setBrushRawDrawingEnabled(b)`                                                                     | `Device.setBrushRawDrawingEnabled`                                                                                     | `SET_BRUSH_RAW(b)`                                                                                                                                                 |
| `setEraserRawDrawingEnabled(b, style)`                                                             | `Device.setEraserRawDrawingEnabled`                                                                                    | `SET_ERASER_RAW(b, style)`                                                                                                                                         |
| `enableSideBtnErase(b)`                                                                            | in-process + `Device.setEnablePenSideButton`                                                                           | `SET_ENABLE_PEN_SIDE_BUTTON(b)`                                                                                                                                    |
| `setLimitRect / setExcludeRect / setSingleRegionMode / setMultiRegionMode`                         | in-process native + `EpdController`                                                                                    | region transactions (see `08-surfaceflinger.md` §2)                                                                                                                |

Note that after `openRawDrawing` the SF reader is in state 1. It forwards no samples in state 0 or 1
(`threadLoop` `0x52d178–0x52d188`: `cbz state` / `cmp state,#1`) [code], so SF does not ink until the
first `PEN_STATE(2)`.

## 4. The pen-session half of the state machine

### 4.1 `HWEpdcManager::setHandWritingPenState(s)` (`0x5666bc`, jump table `0x34152a` = `00 14 2b 38`)

| s        | Branch                     | Actions                                                                                                                                                                                                                                                                      | Returns to schema? | Logs                                                                             |
| -------- | -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------ | -------------------------------------------------------------------------------- |
| 0 STOP   | `0x5666f4`                 | `### EBAS 0`; `FBDev::enableExtBufAutoSync(0)` (ioctl `0x7212`); `TouchReader::stop` (ctrl 0, loop 0x103, waits ≤ 80×10 ms for `loop ready`); `enableCytpLoFilter(0)` (writes `/sys/onyx_misc/cytp_lo_filter`); then **tail-calls `stopHandwriting`** (branch to `0x5667e4`) | yes, if schema 7   | `HWEpdcManager: ### EBAS 0`; `Wait for state loop ready timeout` if slow         |
| 1 START  | `0x566744`                 | `### EBAS <autoSync>`; `enableExtBufAutoSync(EPDC+0x25d)`; `TouchReader::start` (ctrl 1, loop 0x101, waits ≤ 80×10 ms for `loop running`); `enableCytpLoFilter(1)`                                                                                                           | no                 | `### EBAS %d`                                                                    |
| 2 RESUME | `0x5667a0`                 | `TouchReader::ensureStart` (starts the loop if needed, logs `Going to start with current state %s`); `TouchReader::resume` (ctrl 5, waits ≤ 10×10 ms for 2); if `EPDC+0x25d`: `FBDev::syncDataToExtendedBuffer` (ioctl `0x701d`)                                             | no                 | `Control state: toResume -> running` (reader thread, not gated)                  |
| 3 PAUSE  | `0x5667d4`                 | `TouchReader::pause` (ctrl 3, waits ≤ 10×10 ms for 4); **tail-calls `EpdcManager::stopHandwriting`**                                                                                                                                                                         | yes, if schema 7   | `Control state: toPause -> paused`; `Wait for state paused timeout` after 100 ms |
| ≥ 4      | `cmp w1,#3; b.hi` → return | nothing                                                                                                                                                                                                                                                                      | no                 | the caller still logs `Pid N State change to: paused` (4) or `invalid` (≥ 6)     |

**Correction to `08-surfaceflinger.md` §3:** STOP also runs `stopHandwriting`. The STOP branch ends
with `b 0x5667e4`, the same tail as PAUSE (`mov x0,x19; …; b stopHandwriting`) [code, `0x566740`].

`handleSetHandWritingPenState(s, pid)@0x560c4c` [code]: under the pen mutex, **only if
`s ≠ TouchReader+0x98`**: log `Pid <pid> State change to: <ctrlStateStr(s)>`, `0xac9f90 = pid`,
`addToWatchApp(pid)`, call the vtable `+0xe8` above, log `State changed!`. A repeated state is
silently ignored. Because the comparison uses the reader's live state, a `PEN_STATE(3)` sent while
the reader is already `paused` (4) is **not** a repeat and runs `pause()` + `stopHandwriting` again
[code].

### 4.2 Reader thread, pen samples and the handwriting trigger

`TouchReader::threadLoop@0x52ce34` [code]:

- It polls the input fd and an eventfd. On an eventfd wake it moves `toPause (3) → paused (4)` and
  sets `PenManager+0x2054 = 1`, or `toResume (5) → running (2)` and sets it to 0. In both cases it
  clears `penTriggered` (`0x52d19c–0x52d1b0`) and logs `Control state: %s -> %s` through
  `TouchReader::dumpMessage@0x52c228`, which is not gated.
- Input events are pushed to the pen event queue in every state except 0 and 1 (`0x52d178–0x52d194`),
  so events keep flowing while paused.
- `BaseReader::onEvent@0x52dfa8` returns immediately when `PenManager+0x2054` is set. **While paused
  SF neither inks nor triggers handwriting** [code].
- `BtnReader::onSynEvent@0x52e028`, which is skipped while paused, picks the active reader: rubber down,
  or side-touch down with side-button erase enabled → reader 4 if eraser raw drawing is enabled, else
  3; brush down → reader 2 if brush raw drawing is enabled, else 1 [code].
- Readers 2 (`DrawReader`) and 4 (`EraseReader`) call `BaseReader::onEvent` → `PenManager::inValidRegion`
  → `moveTo`/`quadTo`. `EraseReader::onActivate@0x52ec64` first sets the SF painter style to
  `*(0xa5e708)` [code]. Readers 1 and 3 have empty `onSynEvent` bodies: **no ink and no trigger**
  [code].
- `PenManager::moveTo@0x52e81c` → `handlePenTriggerImpl(-1, 0)@0x562f24`. If `penTriggered == 0` it
  logs `HandlePenTrigger, sid: -1 value is: 0 pid <pen pid>` (not gated) and calls
  `EpdcManager::startHandwriting` → `EpdcSchemaManager::activate(7)` → `HandwritingSchema::onActivate`
  → `afterStartHandwriting@0x562c08`: `penTriggered = 1`, `androidDrawing = 0`, `addToWatchApp`,
  **`setEbcUpdateScheme(3, 0)`** [code].
- `moveToImpl/quadToImpl/finishQuadToImpl` (`0x562728/0x562928/0x562ae0`) draw into SF's
  `MemoryPainter` and push ink straight to the panel through `FBDev::hwScreenRefresh`, bypassing
  composition. The waveform they use is [unknown] (E7).

### 4.3 `EpdcManager::stopHandwriting@0x558630`

It takes the EPDC `timed_mutex` (300 ms try, logs `try lock for failed!` on timeout). If schema is 7:
`EpdcSchemaManager::restore` → previous schema (normally 0) → `HandwritingSchema::onDeactivate@0x55b818`
(clears `mHWRepaintPending` at `0xac9e60`) → `afterStopHandwriting@0x562ec0` [code]:

1. logs `SurfaceFlinger: Post enabled!` (INFO, not gated);
2. `penTriggered = 0`, `0xaca240 = 0`, `androidDrawing = 1`;
3. `HWEpdcManager::clearHandwritingFBSchema@0x566134`: reads the fixed update entry
   (`EpdcManager::fixedUp…@0x559ab0`), passes its mode through `EpdcManager::applyDither@0x559304`
   and tests the **return value** of that call against the fast set (`0x56616c–0x566200`; checked in
   verification). Scheme = **4 if that value is fast, else 2**. Then `setEbcUpdateScheme(scheme, 0)`, `clearScreen(0)`
   (no match in the clear-type map, so nothing) [inferred from `clearScreen@0x565bf8` and the map
   contents in `07-einkwise.md` §5.4.6], `EnhanceStrategy::setEnable(enhance && fast)`, and if
   `!inFastMode()`: `EPDC+0x8 = 1` (consumer [unknown]).

If schema is not 7 it does nothing (debug log `### not in handwriting schema.`).

### 4.4 The render flag and "render off but not paused"

- `ENABLE_POST(v)`, handler `0x560bf8`: `if (v >= 0 && androidDrawing != v) stopHandwriting()`
  [code]. So `enablePost(1)` ends an active handwriting session. `enablePost(0)`
  (`enterScribbleMode`) never has an effect: if schema 7 is active, `androidDrawing` is already 0; if
  not, `stopHandwriting` is a no-op [code].
- `setRawDrawingRenderEnabled(false)` always adds `PEN_STATE(3)`, so the reader pauses (§3) [code].
- A state where SF does not ink but `GET_PEN_STATE` stays 2 exists only through the reader choice:
  brush raw drawing off (tip → reader 1) or eraser raw drawing off (eraser end → reader 3) [code
  §4.2]. `TouchHelper.setRawDrawingEnabled` resets to brush on, eraser off with style 5
  (`TouchHelper.java:182-185`) [code].
- **Corrected in verification.** The earlier text concluded that on stock the **eraser end** selects
  the no-op reader 3. That is wrong for the editor: every resume goes through
  `ResumeRawDrawingRequest.execute()` → `o()` → `n()` → `k()` and `h()`
  (`note/note/request/pen/ResumeRawDrawingRequest.java:48-65,86-104,117-131`), which sends
  `setErasePathDrawing(isEraseTrackByRawRender(eraserType) || provider == SELECTION,
  isMoveStrokeErase(eraserType) ? 8 : 5)` and `setBrushRawDrawing(resumePenBean.i)` **before**
  re-enabling input and render. `EraseUtils.isEraseTrackByRawRender` is true for eraser types 0, 1, 2
  and `isMoveStrokeErase` for 0, 1 (`sdk/scribble/utils/EraseUtils.java:23-29`) [code]. It also sets
  `setStrokeParameters(5, {5.0})` and `(8, {width, 0.5, 0.1})` (`:52-55`). So with a stroke, move or
  area eraser selected, SF's eraser end selects **reader 4** (`EraseReader`), SF draws the eraser trail
  itself with style 8 or 5, and the first eraser sample triggers handwriting exactly like the tip:
  schema 7, `### EUS, s 3 c 0`, pen state stays 2 [code for the SDK calls; the SF reader selection
  is the §4.2 reading, not re-derived in verification; the type names stroke/move/area are from
  `01-pen-session-lifecycle.md` §1 item 7; that the eraser end uses the same `EraserArgs` type as
  the eraser tool is [inferred]]. The no-op-reader
  state applies only between a `setRawDrawingEnabled(…)` and the next `ResumeRawDrawingRequest`, or
  for eraser types outside 0–2 [inferred].
- After `ENABLE_POST(1)` without a pause, the reader keeps running. The next brush sample
  re-triggers handwriting, because `penTriggered` was cleared (§4.2) [code].

### 4.5 What the handwriting schema does to ordinary frames

`HandwritingSchema::output@0x55b828` [code]:

- if no handwriting repaint is pending and `HWEpdcManager::inFastMode()` (scheme 4 **and** a fast
  fixed entry): drop the frame (debug `### warning: sys/app is in fast mode, ignoring refresh.`);
- else if a layer carries `HAND_WRITING_REPAINT_MODE` (`hasHWRepaint`): commit those updates;
- else only the one-shot entry from `next()` is committed. Without one, the frame is dropped (debug
  `### disable refresh.`).

So while SF is in schema 7, normal app redraws do not reach the panel.

## 5. The update-scheme half

### 5.1 `HWEpdcManager::setEbcUpdateScheme(s, c)@0x565de0`

[code]:

1. **Always** logs `HWEpdcManager: ### EUS, s <s> c <c>` (priority 6 = ERROR, no gate). This is the
   line seen in the driver log.
2. `prev = FBDev+0x30` (the current scheme).
3. If `s == 3` and auto-sync is off (`EPDC+0x25d == 0`): `syncDataToExtendedBuffer` (ioctl `0x701d`),
   log `### EBAS 0`, `enableExtBufAutoSync(0)` (ioctl `0x7212`).
4. `FBDev::setEbcUpdateScheme(s | (c>0 ? c<<4 : 0))` → ioctl `0x7006`. The kernel's
   `onyx_tcon_set_upd_scheme` line comes from this ioctl [inferred].
5. **Timers:** if `s ∈ {2,5}` and `prev == 4` → `usleep(100000)`; if `s ∈ {2,5}` and `prev == 3` →
   `usleep(150000)`. The sleep runs on the calling thread [code].
6. `s == 4` → `EPDC+0x280` flag = 1; otherwise flag = 0 and word = 0.

Callers of vtable `+0x78` [code, found by scanning for `ldr x, [x, #0x78]; blr`]:

| Caller                              | Scheme it sets                                  | When                                                                    |
| ----------------------------------- | ----------------------------------------------- | ----------------------------------------------------------------------- |
| `afterStartHandwriting@0x562c08`    | **3**                                           | entering schema 7 (pen trigger or app `moveTo`)                         |
| `clearHandwritingFBSchema@0x566134` | **4** if the fixed entry is fast, else **2**    | leaving schema 7 (PAUSE, STOP, ENABLE_POST(1), dream, …)                |
| `setFBSchema@0x565c78`              | **4** if the new fixed mode is fast, else **2** | from `applyUpdateAction` when an app-scope or transient slot changes    |
| `syncFBSchema@0x565f74`             | 4/2 as above                                    | `HWNormalSchema::output` when `shouldSyncFBSchema` is set (after dream) |

No other code path in the Onyx range writes the scheme [code, within the scanned range
`0x520000–0x570000`].

### 5.2 The fixed-slot path to scheme 4

`EpdcManager::applyTransientUpdateMode(mode)@0x559924` [code]: returns if the power state is 1–3;
otherwise `FixedUpdateEntryManager::apply(slot 1 = transient, …, mode, INT_MAX)` →
`EpdcManager::applyUpdateAction@0x558ce8`:

- returns if nothing changed;
- `resetSaturation`;
- **guard:** if `FBDev+0x30 == 3` (currently in the handwriting scheme) → return without switching
  (debug `### do not switch epd Schema in handwriting mode`) [code, `0x558d90–0x558da8`]. The slot
  itself **has already been updated** by `apply()`. When handwriting later ends,
  `clearHandwritingFBSchema` sees the fast slot and selects scheme 4 [code, §4.3]. **This correction
  matters:** `08-surfaceflinger.md` read that guard as "schema == 7". It is "scheme == 3";
- otherwise resolve the clear type from the action's clear flag (verification detail,
  `0x558dd8–0x558e1c`: action `+0x14 == 1` → clear type = action `+0x10`; `== 2` → clear type 0,
  and EPDC `+0x30 = 3` plus `*(0xac9e38) = action +0x10`, logged under debug as
  `### useUpdateModeForNewLayer, enable 1, clearType %d`, i.e. the clear is deferred to the next new
  layer; anything else → 0) [code] and call
  `HWEpdcManager::setFBSchema(mode, ditherChanged, clearType)@0x565c78`: `applyDither`,
  `setEbcUpdateScheme(isFast ? 4 : 2, 0)`, `clearScreen(clearType)`, enhance on/off.

The fast set is `{1, 4, 0x901, 0x904, 0x908, 0x1000001, 0x1000004, 0x2000004}` (same comparisons at
`0x565d00–0x565d70`) [code]. `2308 = 0x904 = UI_A2_QUALITY_MODE` is fast; its low nibble 4 maps to
waveform 4 (A2) in `FBDev::hwScreenRefresh` (`07-einkwise.md` §5.4.7) [code there]. Where the
`flags[0x2000]` in the A2 driver lines comes from is [unknown] (E7).

### 5.3 Leaving A2: `CLEAR_TRANSIENT_UPDATE` and the full-screen update

Lambda `0x5644cc` (from `handleClearTransientScopeUpdate`) [code]:

1. if `FixedUpdateEntryManager::inTransientFastMode()` is false → return (nothing at all);
2. `EpdcManager::clearTransientUpdateMode(clearFlag)@0x5599e4` → `apply(slot 1, …, clearFlag, mode
   -1)` → `applyUpdateAction` → `setFBSchema(none, …, clearType)` → `setEbcUpdateScheme(2, 0)` (sleeps
   100 ms because prev = 4) → `clearScreen(clearType)`. For the transient slot on an HW-Tcon device
   the fast→none action value is 8 (`FixedUpdateEntryManager` ctor `0x556f84`: `mov w9,#0x8` in the
   `vendor.onyx.htcon` branch; table in `07-einkwise.md` §5.4.6) [code]. With `clearFlag = 1`
   (IMMEDIATELY), `clearType = 8` → `HWEpdcManager::clearScreen` → `applyGCOnce` → one-shot entry 98
   = GC. The scheme is already 2, so 98 is used rather than `*(0xaca250)` [code]. With `clearFlag = 0`,
   no GC here [code, per `07-einkwise.md` §5.4.6];
3. **unconditionally** (still inside the "was fast" branch): `EpdcWrapper::addEpdc(EPDC+0xb8, 0, 0,
   width, height, 5)` (PLT `0xa21530`, `_ZN7android11EpdcWrapper7addEpdcEiiiii`; the two size
   arguments are the returns of provider vtable `+0x20` and `+0x18`, read as width/height
   [inferred]) and provider `+0x28`
   (`refresh`) → a **full-screen update in mode 5 (AUTO)** on the next composition [code].

So each exit from a transient A2 period produces a full-screen update: GC (mode 98 → waveform 2,
GC16) if the caller passed `true`, and a full-screen AUTO update (mode 5 → waveform 255, the driver
picks) in any case. The observed `waveform[2] update_mode[1] flags[0x1000]` fits the GC variant, or
the kernel's choice for a full-screen AUTO update after A2 content [unknown; E3].

The app-scope slot has the same shape (`clearAppScopeUpdate@0x558ebc`) but no extra full-screen
`addEpdc` in its caller [code].

### 5.4 Other things that switch schemes or force full updates (for completeness)

- `HWNormalSchema::output@0x566cf8`: if `shouldSyncFBSchema` and the resolved entry is fast →
  `syncFBSchema` + `enableSyncFBSchema(0)` [code]. The only setter found is `HWDreamSchema::onDeactivate`
  [code, `0x5656f8`], so this matters after sleep/wake, not for the pen.
- `getNewLayerUpdateEntry@0x5664c0`: a new surface gets full GC / fill-white / clear type depending on
  EPDC `+0x30` [code]. Stock sets `useGCForNewSurface(true)` in `SaveProgressPopup` [code].
- The debouncer schema (1) and `EpdcManager::startDebouncer@0x5589b0` (skipped when the current schema
  is 2, 3, 5 or 8; mask `0x12c`) are driven by the framework's EAC debouncer
  (`AccessibilityHelper.handleMotionWithSFDebouncer`, `EACBaseRefreshImpl`) [code]. Reaching them needs
  `AccessibilityHelper.handleMotionEvent`, which is called from `View.handleAccessibilityInjectMotionEvent`
  (`View.java:5011-5013`), i.e. accessibility-injected events only [code]. Not on the pen path
  [inferred].

## 6. Where the observed flash comes from (mechanism, step by step)

Sources: `android/view/ViewRootImpl.java`, `android/onyx/optimization/OnyxScrollRefreshManager.java`,
`ScrollHelper.java`, `EInkHelper.java`, `EACUtils.java`, `android/webkit/WebView.java`,
`android/view/View.java`, plus the SF paths above. Each step is [code] unless marked.

**Precondition (window setup).** `ViewRootImpl` sets `topScrollBounds` when a scroll-capture search
finds a scrollable view (`ViewRootImpl.java:13271-13275`, logs
`E OnyxScrollRefreshManager: checkHasScrollCallback topScrollBounds …`) or a whitelisted child
(`:4367-4374`, logs `hadChildInWhiteList topScrollBounds …`). Whether Tangleaf's window gets one is
[unknown] (E1). A WebView is a scroll-capture candidate [inferred]. (Verification note.) The search
runs from `performTraversals` only while `topScrollBounds == null` (`ViewRootImpl.java:8423-8429`,
`checkHasScrollCallback` `:3385-3396`); once set, the bounds are kept for the life of the
`ViewRootImpl` (only the constructor nulls them, `:3048`) [code]. The guarding window-focus variable
in that jadx output (`z28`) is ambiguous, so _which_ traversal first runs the search is [unknown].

**Entry A — every MotionEvent of the window** (`ViewRootImpl.deliverInputEvent`, `:3502-3514`). If
`topScrollBounds != null` and `EInkHelper.getEacScrollStyle() == 1` (default 1,
`EACActivityConfig.java:15`), the event goes to `OnyxScrollRefreshManager.deliverMotionEvent`
(`:134-155`). That returns early if the package is Onyx (`isOnyxApp`), `android`,
`com.android.systemui`, or the device id is 2003125025 (the injector). Otherwise, on an rx thread:

(Verification notes on Entry A.) `EInkHelper.getEacScrollStyle()` returns
`getReadOnlyAppConfig().getGlobalActivityConfig().getEacScrollStyle()` when the OEC service is ready and
**0** when it is not (`android/onyx/optimization/EInkHelper.java:808-813`); it does not go through
`isAppConfigActive()`, so an app's `supportEAC`/enable flag is not consulted on this path [code]. The
rx filter also drops every event, including ACTION_DOWN and ACTION_UP, whose point is outside
`topScrollBounds` (`OnyxScrollRefreshManager.java:158-161`), so a gesture that starts outside the
bounds never reads the pen state and a lift outside the bounds does not run `afterScroll` until a
later UP/CANCEL inside them [code; consequence inferred]. The pen state is read with a Binder call
on the manager's own single-thread scheduler when the queued DOWN is processed, not synchronously
at delivery [code, `:67-75,139-154`]. Hover and other actions fall into `default: return` [code].

1. ACTION_DOWN: remember x/y; `penDrawState = ViewUpdateHelper.getPenState()` — the **SF reader state**
   (§2.2) (`:67-75`).
2. ACTION_MOVE: if the point is inside `topScrollBounds`, `|dy| > touchSlop (8 px default)` and
   `|dy| > |dx|`, and **not** (toolType ∈ {STYLUS 2, ERASER 4} and `penDrawState == 2`): log
   `D OnyxScrollRefreshManager:  beforeScroll` and call `EInkHelper.beforeScroll(null)` once per
   gesture (`:89-99`, `:131-150`).
3. ACTION_UP/CANCEL after a beforeScroll: log ` afterScroll`, `EInkHelper.afterScroll(null)`
   (`:76-83`).

**Entry B — WebView only** (`WebView.onTouchEvent` → `applyFastMode`, `:374-392`). With
`autoA2ForScrolling` (true by default, `:343`): on a move with `|y − downY| ≥ 5 px` →
`View.beforeScroll(uuid, event)`. That checks `EACScreenNoteManager.allowApplyTransientUpdate`. For a
package type that is neither IME nor single-view it goes to `EmptyHandler`, which returns true
(`EACScreenNoteManager.java:125-136`, `EmptyHandler.java:12-14`). `BaseHandler` also returns true
unless the app has an enabled note config matching the view (`BaseHandler.java:358-365`). Then
`beforeScrollImpl` → `EInkHelper.beforeScroll` if `isEnableA2Scrolling()` (`EACConfig.enableA2Scrolling`
default true). Up/cancel → `View.afterScroll`. **No pen-state check and no Onyx-package check** on
this path. It needs the stylus MotionEvent to reach `WebView.onTouchEvent` [code]. Whether Tangleaf's
input routing lets it through while paused is covered by the input-routing document [unknown here].
(Verification notes on Entry B.) `View.isEnableA2Scrolling()` caches `EACConfig.singleton()
.isEnableA2Scrolling()` per view (`View.java:5081-5083,10999-11004`); `EACConfig` is loaded by
`ConfigLoader.load(EACConfig.class, CONFIG_CATEGORY)` (`android/onyx/config/EACConfig.java:43-58`),
i.e. from a firmware config file, so the field default `true` (`:30`) is not proof of the device value
[unknown]. `autoA2ForScrolling` is a per-WebView field with public `setAutoA2ForScrolling(boolean)` /
`isAutoA2ForScrolling()` (`WebView.java:885-887,1281-1283`) [code]. The UP/CANCEL branch calls
`afterScroll` on every lift, with or without a preceding `beforeScroll`; `ScrollHelper.afterScroll`
filters on `isInScrollRefreshMode` (`ScrollHelper.java:106-110,156-158`), so a lift without an A2
period does nothing in SF [code]. `ScrollHelper` is one per process and shared by Entry A and Entry B,
so a `beforeScroll` from either one cancels the other's pending exit (`:144-153`) [code].

**Common part (`ScrollHelper`, one singleton per process).**

4. `beforeScroll`: needs `getEacScrollStyle() == 1`, cancels a pending exit, and if not already in
   scroll mode posts `enterScrollRefreshModeImpl`, filtered by `!EACUtils.isFastMode(appScopeRefreshMode)`
   (`ScrollHelper.java:93-95,144-153`). With `scrollingRefreshMode ∈ {2, 4}` (default 2,
   `EACDeviceExtraConfig.java:36`): `ViewUpdateHelper.applyTransientUpdate(EACUtils.toEpdMode(2) = 2308)`
   (`:57-66`, `EACUtils.java:127-142`).
5. SF: `APPLY_TRANSIENT_UPDATE(2308)` → §5.2. With the session **paused**, SF is not in the
   handwriting scheme (3), so the guard passes → `setFBSchema` → **`### EUS, s 4 c 0`** (+ kernel
   `onyx_tcon_set_upd_scheme`), enhance on. Following app frames compose with the A2 entry
   (`waveform[4]`).
6. `afterScroll`: `delay(getExitDelay())` where `getExitDelay()` =
   `Settings.Global.getInt("scroll_refresh_delay_v2", 1000)` (`ScrollHelper.java:85-87,97-142`). A
   wakelock is held meanwhile. Then `exitScrollRefreshModeImpl` →
   `clearTransientUpdate(isGcAfterScrolling)` (`:68-83`; `gcAfterScrolling` defaults to false in
   `EACDeviceExtraConfig` v2, `:35`; the device value is [unknown]).
7. SF: §5.3 → `### EUS, s 2 c 0` after a 100 ms sleep, optional one-shot GC, full-screen mode-5 update
   → **one full-screen update: the flash**.

**Why the running session (state 2) does not flash** [inferred from the above]: Entry A exempts the
stylus when `getPenState() == 2` at ACTION_DOWN. If a transient A2 is applied anyway (Entry B, or a
finger), the SF guard blocks the scheme switch while SF is in scheme 3. The slot then stays fast, and
the **next exit from handwriting** (`clearHandwritingFBSchema`) selects scheme 4 [code]. That delayed
variant should show up as `### EUS, s 4` right after a `PEN_STATE(3)` (E5).

**Why pausing alone does not flash, and why WebView redraws alone do not** [inferred]: neither makes
a MotionEvent with vertical travel, so neither Entry A nor Entry B fires. That matches the handoff's
negative controls.

**Timing check** [inferred]: lift → rx hop → `delay(scroll_refresh_delay_v2)` (1000 ms default) → rx
hop → Binder → SF composite-thread lambda → `usleep(100 ms)` in `setEbcUpdateScheme` → next
composition. About 1.1–1.2 s with the default. The handoff's ~1.5 s suggests either a different value
of `scroll_refresh_delay_v2` on this device or scheduling slack [unknown; E2]. (Verification note.)
The setting is not static: the OEC service's `EACScrollRefreshImpl.onResume` rewrites
`Settings.Global scroll_refresh_delay_v2` on every top-activity change to the foreground activity's
`EACActivityConfig.getScrollRefreshDelay()`, and `onApplyConfig` does the same for the matching app's
global activity config (`android/onyx/optimization/impl/EACScrollRefreshImpl.java:23-46`, registered
as impl 4 in `OECService.initImpl`, `OECService.java:631-639`, dispatched from
`handleActivityResumeImpl`, `:461-469`) [code]. The getter clamps to 1000–5000 ms
(`data/v2/EACActivityConfig.java:7,14,50`), so the default `scrollRefreshDelay = 0` yields 1000 ms,
and a per-app value can stretch the delay to 5 s [code]. Which value Tangleaf's stored config holds is
[unknown] (E2).

**Pen state ERASING (4) and "render disabled but not paused":**

- Sending `PEN_STATE(4)` changes nothing in SF (§4.1). If the reader was running, it keeps running,
  SF keeps inking with whatever reader is selected, and `GET_PEN_STATE` stays 2, so Entry A's stylus
  exemption holds. If it was paused, 4 equals the reader state and is ignored. It does not affect
  Entry B [code/inferred].
- Reader state 2 with brush raw drawing off (or the eraser end with eraser raw drawing off): no SF
  ink, no scheme 3, `GET_PEN_STATE == 2`. Entry A is exempt; Entry B is not [inferred].
- Reader state 3/4 (what `setRawDrawingRenderEnabled(false)` produces): Entry A is **not** exempt for
  the stylus [code].

## 7. Device-specific branches on the Note Air 4C

| Branch                                                                                                                                                     | Where                                                                                                                                | Which side runs here                                                                                                                                                                                                                                                                                                                                                                     | How determined                                                                                                                                                                                      |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| HW-Tcon vs software EPDC manager (`HWEpdcManager` vs `SWEpdcManager`)                                                                                      | `setSurfaceFlingerProvider@0x55eba4`, `property_get_bool("vendor.onyx.htcon")`; logs `hasHwTcon: %d` at SF start                     | **HWEpdcManager**                                                                                                                                                                                                                                                                                                                                                                        | [code] branch; the device-side confirmation is the hardware-log fact that EUS lines are tagged `HWEpdcManager` (the `SWEpdcManager` variant logs `SWEpdcManager: ### EUS, scheme %d` at `0x5672cc`) |
| Clear-screen action table (HW: 1/1/8, SW: 0x65 variants)                                                                                                   | `FixedUpdateEntryManager` ctor `0x556f4c–0x557058`                                                                                   | HW table: fast→none = 8 (applyGCOnce)                                                                                                                                                                                                                                                                                                                                                    | [code] + above                                                                                                                                                                                      |
| `SWEpdcManager::setEbcUpdateScheme` has no EBAS/auto-sync or post-A2 sleeps; `SWEpdcManager::setHandWritingPenState@0x5677b0` is a separate implementation | `0x5672a8`, `0x5677b0`                                                                                                               | not used here                                                                                                                                                                                                                                                                                                                                                                            | [code]                                                                                                                                                                                              |
| Waveform acceptance: HW path accepts nibbles 1–6 only                                                                                                      | `FBDev::hwScreenRefresh@0x55b074`                                                                                                    | yes                                                                                                                                                                                                                                                                                                                                                                                      | [code], `07-einkwise.md` §5.4.7                                                                                                                                                                     |
| `applyGCOnce` under scheme 4 uses `*(0xaca250)` instead of 98                                                                                              | `HWEpdcManager::applyGCOnce@0x566430`                                                                                                | yes                                                                                                                                                                                                                                                                                                                                                                                      | [code]; the value at `0xaca250` [unknown]                                                                                                                                                           |
| Colour-panel flags in the SDK (`DeviceInfoUtil.isColorDevice()` → high-contrast dither 160 vs 255 in stock `EpdEventHandler`)                              | `EpdEventHandler.java` static init                                                                                                   | colour → 160                                                                                                                                                                                                                                                                                                                                                                             | [code]; `isColorDevice` true on a Kaleido panel [inferred]                                                                                                                                          |
| SDK device class                                                                                                                                           | `Device.java:74-121`, `ro.board.platform`                                                                                            | `SDMDevice` (lito)                                                                                                                                                                                                                                                                                                                                                                       | [code] + `01-sources-and-firmware.md` (platform `lito`)                                                                                                                                             |
| SDK render choice                                                                                                                                          | editor: `NoteManager.java:231` `TouchHelper.create(view, 3, cb, false)`; HWR text handler: `TouchHelper.java:39` `hasStylus ? 2 : 1` | editor: `SFTouchRender` + `AppTouchRender` on every device (fixed mask, no device check); HWR handler: `SFTouchRender` if `hasStylus`                                                                                                                                                                                                                                                    | [code] for the editor (corrected in verification; the earlier row said "`SFTouchRender` only"); `hasStylus` true on the Note Air 4C [inferred]                                                      |
| Eraser trail in SF                                                                                                                                         | `ResumeRawDrawingRequest.k()`                                                                                                        | on for eraser types 0–2 (style 8 for 0/1, 5 for 2), no device check                                                                                                                                                                                                                                                                                                                      | [code] (added in verification)                                                                                                                                                                      |
| Resume delay before `ResumeRawDrawingRequest` re-enables the pen                                                                                           | `ResumePenBean.c` (colour device 500 ms per `01-pen-session-lifecycle.md` §1)                                                        | colour branch                                                                                                                                                                                                                                                                                                                                                                            | see `01-pen-session-lifecycle.md`                                                                                                                                                                   |
| Onyx-package exemptions in the framework (`isOnyxApp`: package contains `"com.onyx"`)                                                                      | `OnyxScrollRefreshManager.java:135`, `ActivityManagerHelper`                                                                         | stock exempt, Tangleaf not                                                                                                                                                                                                                                                                                                                                                               | [code]                                                                                                                                                                                              |
| EAC scroll style (1 = ScrollHelper, 2 = bypass manager)                                                                                                    | `EACActivityConfig.eacScrollStyle` default 1, per-app config                                                                         | [unknown] for Tangleaf's stored config (default 1)                                                                                                                                                                                                                                                                                                                                       | E1/E6                                                                                                                                                                                               |
| `supportEAC`                                                                                                                                               | stored config (stock false, Tangleaf true per handoff)                                                                               | none of the SF code reads it. The framework paths above do not test it directly: they test the package, scroll style, app-scope mode and scrolling refresh mode. `getEacScrollStyle()` reads the cached app config without `isAppConfigActive()` (`EInkHelper.java:808-813` vs `:1156-1159`), and `EACScrollRefreshImpl.onResume` applies the per-activity delay without an enable check | [code] for the tests present, [unknown] whether EAC service values differ by `supportEAC`                                                                                                           |
| `EACConfig.enableA2Scrolling` (Entry B gate)                                                                                                               | `ConfigLoader.load(EACConfig.class, …)`                                                                                              | [unknown] (field default true)                                                                                                                                                                                                                                                                                                                                                           | E4 would show it                                                                                                                                                                                    |
| `scroll_refresh_delay_v2`                                                                                                                                  | `EACScrollRefreshImpl`, per foreground activity, clamped 1000–5000                                                                   | [unknown] for Tangleaf                                                                                                                                                                                                                                                                                                                                                                   | E2                                                                                                                                                                                                  |
| Phone vs tablet                                                                                                                                            | nothing in SF or in the paths above branches on it                                                                                   | —                                                                                                                                                                                                                                                                                                                                                                                        | [code, absence within scanned code]                                                                                                                                                                 |

## 8. Interactions with the other subsystems

- **Session lifecycle.** Open = `PEN_STATE(1)`, and SF inks only after `PEN_STATE(2)`. Every
  render-off is `ENABLE_POST(1)` + `PEN_STATE(3)`: pause the reader, and if SF was in schema 7, leave
  it with a 150 ms sleep and `EUS s 2/4`. Close adds `PEN_STATE(0)`, which also runs
  `stopHandwriting`. The session is keyed by one global pid slot. There is no death recipient
  (`08-surfaceflinger.md` §2.3).
- **Writing.** SF enters schema 7 and scheme 3 at the first valid sample after each pause/stop or
  `ENABLE_POST(1)`, because `penTriggered` is cleared. App frames are dropped in schema 7 unless they
  carry `HAND_WRITING_REPAINT_MODE` or a one-shot entry (§4.5).
- **Erasing.** (Corrected in verification; the earlier text said stock's eraser end selects the no-op
  reader 3.) After any `ResumeRawDrawingRequest`, stock has eraser raw drawing **on** with style 8
  (stroke/move eraser) or 5 (area eraser) (§4.4) [code]. The eraser end then selects SF reader 4, SF
  draws the eraser trail itself with style `*(0xa5e708)`, and the first eraser sample triggers
  schema 7 / scheme 3 like the tip [code for SF; eraser-end-uses-tool-type [inferred]]. The session
  stays running (pen state 2) during the erase, so Entry A's stylus exemption would hold even for a
  non-Onyx package [inferred]. After the erase, `EraseFinishAction` pauses the render only and resumes
  after 300 ms (`01-pen-session-lifecycle.md` §1 item 7) → `ENABLE_POST(1)` + `PEN_STATE(3)` →
  `stopHandwriting` → `### EUS, s 2 c 0` (or `s 4` if a transient slot is fast) [inferred from §4].
  The framework treats stroke style 5 as the erasing stroke (`BaseHandler.isEarsingStroke`:
  `getStrokeStyle() == 5`, `BaseHandler.java:102-104`) [code]. What SF's painter draws for styles 5
  and 8 is [unknown] (E8).
- **App-driven SF painting.** For shape types that are not DFB (`ShapeFactory.isDFBShape`), the
  editor uses `NormalShapeHandler`, which replays raw points into a `PenShape` whose
  `onDown/onMove/onUp` send `MOVE_TO`/`QUAD_TO` (§2.2). SF's `handleMoveTo` starts handwriting
  (schema 7, scheme 3) whenever `penTriggered == 0`, whatever the reader state [code]. Which pen types
  on the Note Air 4C take this path, and whether `canScribble` lets SF paint them, is [unknown] (E13).
- **Selection / lasso / zoom.** Stock uses the transient A2 slot directly (`EpdEventHandler.a()`:
  `setEpdTurbo`, `applyTransientUpdate(ANIMATION_QUALITY or config)`, and 5 s after fast mode ends
  `clearTransientUpdate(false)`, `EpdEventHandler.java:a(),b(),d()`) [code]. SF effect: `EUS s 4` on
  apply, `EUS s 2` plus a full-screen AUTO update (§5.3) on clear [inferred from §5.3]. Stock's
  `resetTransientUpdate` field `c` is never assigned in the constructor, so it is false [code].
- **Refresh.** `applyGCOnce`, `repaintEveryThing(GC)` and `useGCForNewSurface` are one-shot or
  new-layer paths in normal schema. They do not change the scheme [code].
- **Input routing.** The flash hinges on MotionEvents reaching `ViewRootImpl` (Entry A, always, for
  non-Onyx apps with `topScrollBounds`) and `WebView.onTouchEvent` (Entry B). Nothing in the SF pen
  path consumes or blocks Android MotionEvents. SF reads the EMR device independently [code].
- **Firmware config.** `scroll_refresh_delay_v2`, `gcAfterScrolling`, `scrollingRefreshMode`,
  `eacScrollStyle` and the app-scope refresh mode (EinkWise profile) all feed the A2 path. Under an A2
  app-scope profile, `ScrollHelper` refuses to enter (filter `!isFastMode(appScopeRefreshMode)`,
  `ScrollHelper.java:93-95`) [code]. **Hypothesis:** the flash-after-erase may have appeared when
  Tangleaf switched from the "Speed" (A2) EinkWise profile to HD (`5fdf200`), because that switch
  enabled the scroll-A2 path [inferred; E6].

## 9. Open questions as hardware experiments

Observation only: logcat with `adb logcat -b all -v time`, plus the listed reads. Items marked
_(writes settings)_ change device state and are for the user to decide.

| #   | Question                                                                                                             | Procedure                                                                                                                                                                                                                                                   | Confirms if                                                                                                                                                                                                                                                                                    | Refutes if                                                                                   |
| --- | -------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| E0  | Is the device on the build analysed here? **Done 2026-09-25: yes**, `…4.2-rel_04282…`, 13, SF/framework sha256 match | `adb shell getprop ro.build.display.id`; `getprop ro.build.version.release`                                                                                                                                                                                 | `…4.2-rel_04282…`, 13                                                                                                                                                                                                                                                                          | anything else: offsets and possibly logic differ; re-pull `surfaceflinger`/`framework.jar`   |
| E1  | Is Entry A (`OnyxScrollRefreshManager`) what fires in Tangleaf?                                                      | Open Tangleaf, pause the session (debug command), draw one **vertical** stylus stroke. Grep `OnyxScrollRefreshManager`                                                                                                                                      | at window start `checkHasScrollCallback topScrollBounds …` or `hadChildInWhiteList …`; during the stroke `D OnyxScrollRefreshManager:  beforeScroll`, then `HWEpdcManager: ### EUS, s 4 c 0`; after lift ` afterScroll`, then later `### EUS, s 2 c 0` + `onyx_tcon_send_cmd … update_mode[1]` | `EUS s 4` without any `beforeScroll` line → Entry B or another caller (go to E4)             |
| E2  | What sets the ~1.5 s delay?                                                                                          | `adb shell settings get global scroll_refresh_delay_v2`; measure timestamps between ` afterScroll` (or pen lift) and `### EUS, s 2`                                                                                                                         | lift→EUS ≈ setting + ~100–200 ms                                                                                                                                                                                                                                                               | a large mismatch → another timer                                                             |
| E3  | Is the final full-screen update the GC (clearFlag 1) or the unconditional mode-5 update?                             | Enable SF debug from the app (`ViewUpdateHelper.applySFDebug(true)` via reflection, transaction 1048642), repeat E1, grep `clearTransientUpdateMode, clearFlag` and `setFBSchema mode`                                                                      | `clearFlag 1` → GC path; `clearFlag 0` and still `waveform[2] update_mode[1]` → the kernel maps full-screen AUTO after A2 to GC16                                                                                                                                                              | —                                                                                            |
| E4  | Does Entry B (WebView) fire on its own?                                                                              | Same as E1 but draw a **horizontal** stroke with a little vertical wobble (3–7 px), then a strictly horizontal one                                                                                                                                          | Entry A needs `\|dy\| > 8 px` and `\|dy\| > \|dx\|`; Entry B needs `\|dy\| ≥ 5 px`. `EUS s 4` on the wobbly horizontal stroke with **no** `beforeScroll` line → Entry B                                                                                                                        | no `EUS s 4` on horizontal strokes → only Entry A                                            |
| E5  | Does a running session store a pending A2 that surfaces at the next pause?                                           | Running session: do a finger vertical drag inside the canvas (Entry A does not exempt fingers), then draw with the stylus, then pause                                                                                                                       | finger drag alone → `EUS s 4`…`EUS s 2` + full update (finger scroll A2); if the pen triggers schema 7 during the drag window, the pause shows `Post enabled!` followed by `### EUS, s 4 c 0`                                                                                                  | pause always shows `EUS s 2`                                                                 |
| E6  | Did the EinkWise profile gate this?                                                                                  | _(writes settings)_ Temporarily select the "Speed" (A2) profile for Tangleaf, repeat E1                                                                                                                                                                     | no `beforeScroll`-triggered `EUS s 4`/flash under Speed, flash under HD                                                                                                                                                                                                                        | flash under both                                                                             |
| E7  | Waveforms and flags: SF ink updates vs app A2 frames                                                                 | During a running-session stroke and during an E1 stroke, collect `onyx_tcon_send_cmd` lines                                                                                                                                                                 | ink: waveform used by `hwScreenRefresh` for SF ink; A2 frames: `waveform[4] flags[0x2000]` → identifies what `0x2000` encodes (dither/Y1 bits from `applyDither` `0x900` are a candidate)                                                                                                      | —                                                                                            |
| E8  | What SF's eraser reader draws, and whether stock's eraser end uses it                                                | In stock Notes, select the stroke eraser (then the area eraser) in the toolbar, go back to a pen, and erase with the **eraser end** of the stylus. No developer toggle needed: stock enables `setEraserRawDrawingEnabled(true, 8/5)` on every resume (§4.4) | `HandlePenTrigger` + `EUS s 3` on eraser contact, a visible SF eraser trail, no `State change to: toPause` during the erase                                                                                                                                                                    | no trigger → eraser end not on reader 4 (eraser type differs, or a pause precedes the erase) |
| E9  | Pen state 4 is a no-op                                                                                               | Running session, send `setScreenHandWritingPenState(4)` (reflection)                                                                                                                                                                                        | log `Pid N State change to: paused` + `State changed!` with **no** `Control state: … -> …`, no `Post enabled!`, no `EUS`; stylus still inks                                                                                                                                                    | reader pauses / EUS appears                                                                  |
| E10 | Brush-raw-off keeps state 2 and avoids A2                                                                            | Running session, `setBrushRawDrawingEnabled(false)`, draw a vertical stroke                                                                                                                                                                                 | no `HandlePenTrigger`, no `EUS s 3`, no `beforeScroll` (Entry A exempt); `EUS s 4` still appearing → Entry B                                                                                                                                                                                   | `beforeScroll` appears → the pen-state read is not 2                                         |
| E11 | Stock baseline                                                                                                       | In stock Notes, erase with the eraser end and with the eraser tool, and do a vertical stylus drag on a menu or list                                                                                                                                         | no `OnyxScrollRefreshManager` lines (Onyx package); EUS lines: `s 3` on each eraser/pen contact, `s 2` after each render pause, `s 4` at stock's own fast mode (zoom/move/transform) and `s 2` ≈ 5 s later, each `s 4`→`s 2` followed by one full-screen update (§5.3)                         | `beforeScroll` lines for `com.onyx.android.note`                                             |
| E13 | Does stock's app-driven `MOVE_TO` path fire?                                                                         | In stock, draw with each pen type in turn; grep `HandlePenTrigger` and note whether it appears while `Pid N State change to: toPause/paused` was the last state line                                                                                        | `HandlePenTrigger` / `EUS s 3` while the reader is paused → `NormalShapeHandler` → `MOVE_TO` path (§2.2)                                                                                                                                                                                       | trigger only while running → reader path only                                                |
| E12 | Consumers of `EPDC+0x8` and `EPDC+0x280`                                                                             | Not observable from logcat directly. With SF debug on, look for `### full gc for new surface` / `### create epdc by update entry.` right after `Post enabled!`                                                                                              | a GC or entry right after handwriting exit would point to `EPDC+0x8`                                                                                                                                                                                                                           | —                                                                                            |

## 10. Things verified here that contradict earlier notes

- `08-surfaceflinger.md` §1 and "Question 2": `REPAINT_EVERY_THING` does not route into
  `stopHandwriting`. It only zeroes the bypass counter and calls the provider's `refresh`
  (`0x56007c`, `0x4f68a4`) [code].
- `08-surfaceflinger.md` §3: pen state 0 (STOP) does call `stopHandwriting` (tail branch at
  `0x566740`) [code].
- `08-surfaceflinger.md` §3 "`do not switch epd Schema in handwriting mode`" guard: the test is
  `FBDev scheme == 3`, not schema 7, and the slot is updated before the guard [code, `0x558d90`].
- `handleEnablePost` takes only the middle parcel int, and `enablePost(0)` is inert [code, `0x55fef0`,
  `0x560bf8`].
- The handoff's "pen state 3 … SF switches the panel to update scheme 4": SF does not do that because
  of pen state 3. Pen state 3 makes SF stop owning the stylus; the scheme-4 request comes through the
  transient slot, from the Java framework's scroll-A2 logic (§6) [code for each link; the overall
  attribution is inferred until E1].

## 11. Verification notes

An independent re-check of this document against the primary sources, done statically (no device
access). The corrections are marked "Corrected in verification" or "(Verification note.)" in the
body.

### What was re-read and confirmed

| Claim                                                                                                                                                             | Source re-read                                                                                          | Result                                                                                                                            |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `ENABLE_POST` uses only the second parcel int; `v < 0` skipped; `androidDrawing != v` → `stopHandwriting`                                                         | disasm `0x55fef0`, `0x560bf8`                                                                           | confirmed                                                                                                                         |
| `handleSetHandWritingPenState` compares with live reader state `+0x98`, logs at priority 6, stores pid, `addToWatchApp`, vtable `+0xe8`                           | disasm `0x560c4c–0x560d08`                                                                              | confirmed                                                                                                                         |
| `setHandWritingPenState` jump table 0/1/2/3, `s > 3` returns, STOP and PAUSE both tail into `stopHandwriting`                                                     | disasm `0x5666bc–0x566810`, table bytes `00 14 2b 38`                                                   | confirmed                                                                                                                         |
| Transient-slot guard is `FBDev scheme (0xac9e98) == 3`, not schema 7                                                                                              | disasm `0x558d90–0x558da8`                                                                              | confirmed                                                                                                                         |
| Clear-transient lambda: return unless transient fast; else clear + unconditional full-screen `addEpdc(…, 5)` + provider refresh                                   | disasm `0x5644cc–0x564580`                                                                              | confirmed                                                                                                                         |
| `setEbcUpdateScheme`: always logs `### EUS`, scheme-3 auto-sync branch, 100 ms after 4, 150 ms after 3 (for s ∈ {2,5})                                            | disasm `0x565de0–0x565ef0`                                                                              | confirmed                                                                                                                         |
| `afterStartHandwriting` sets `penTriggered`, `androidDrawing = 0`, scheme 3                                                                                       | disasm `0x562c08`                                                                                       | confirmed                                                                                                                         |
| `startHandwriting` callers                                                                                                                                        | disasm                                                                                                  | two: `handlePenTriggerImpl` (reader) and the `handleMoveTo` lambda (app `MOVE_TO`); the second was not in the table and was added |
| Fast-mode set in `clearHandwritingFBSchema`                                                                                                                       | disasm `0x566184–0x566200`                                                                              | confirmed; the tested value is the return of `applyDither`, added                                                                 |
| `OnyxScrollRefreshManager` gates (Onyx package, scroll style, slop 8, vertical dominance, stylus/eraser + pen state 2)                                            | `OnyxScrollRefreshManager.java` whole file                                                              | confirmed                                                                                                                         |
| `ScrollHelper` enter/exit, `SCROLL_REFRESH_DELAY` = `"scroll_refresh_delay_v2"`, default 1000, `gcAfterScrolling` default false, `scrollingRefreshMode` default 2 | `ScrollHelper.java`, `Settings.java:1878`, `EACDeviceExtraConfig.java:28-51`                            | confirmed                                                                                                                         |
| `ViewRootImpl.deliverInputEvent` hook and `topScrollBounds` sources                                                                                               | `ViewRootImpl.java:3502-3514,4368-4375,13271-13275`                                                     | confirmed                                                                                                                         |
| `WebView.applyFastMode` (≥ 5 px, no pen-state check) → `View.beforeScroll` → `EACScreenNoteManager` → `EInkHelper.beforeScroll`                                   | `WebView.java:67,343,374-392,1115-1117`; `View.java:4236-4240,6448-6456`                                | confirmed                                                                                                                         |
| Bundled SDK: `SFTouchRender` sequences, `EpdPenManager` constants, `SDMDevice` reflection into `android.onyx.ViewUpdateHelper`, `lito → SDM`                      | `SFTouchRender.java:233-455`, `EpdPenManager.java`, `SDMDevice.java:453-694,2230`, `Device.java:74-105` | confirmed                                                                                                                         |
| `ViewUpdateHelper` parcels for pen state, `ENABLE_POST`, `resetEpdPost`, transient update, eraser/brush raw drawing                                               | `ViewUpdateHelper.java:404-496,602-608,720-727,974-1078,1110-1200`                                      | confirmed                                                                                                                         |
| Stock never sends pen state 4                                                                                                                                     | grep of all `setScreenHandWritingPenState` callers                                                      | confirmed (values 0–3 only)                                                                                                       |
| `ANIMATION_QUALITY` → `UI_A2_QUALITY_MODE` in the bundled `SDMDevice`                                                                                             | `SDMDevice.java:392,477,870-871,1593-1633`                                                              | confirmed (fast mode 0x904)                                                                                                       |

### What was changed

1. §3: the editor session is `TouchHelper.create(view, 3, cb, false)` (`SFTouchRender` +
   `AppTouchRender`) from `NoteManager.bindPenHostView`, not `create(view, cb)` (SF only). Added the
   `AppTouchRender` transactions and `forceSetRawDrawingEnabled`.
2. §4.4 and §8 "Erasing": stock does **not** leave the eraser end on the no-op reader.
   `ResumeRawDrawingRequest` enables SF eraser raw drawing (style 8 or 5) for eraser types 0–2 on
   every resume, so the eraser end inks in SF and triggers schema 7 / scheme 3 with pen state 2.
3. §2.2: added the `MOVE_TO`/`QUAD_TO` row (app-driven SF painting, a second way into scheme 3 that
   ignores the reader state), all stock `applyTransientUpdate`/`clearTransientUpdate` callers (the
   earlier text named only `EpdEventHandler`), the `ENABLE_POST(1)` in
   `InkRenderManager.invalidate`, the `resetEpdPost` callers, `BaseActivity.resumeComponentsImpl`, and
   marked `sdk/api/device/EpdDeviceManager` pen-state helpers and `handwritingRepaint` as having no
   (found) caller.
4. §1 item 3: stock does use transient A2 periods (and so the full-screen update on each exit); the
   "no flash" difference is about who triggers A2 and when.
5. §6: `scroll_refresh_delay_v2` is rewritten per foreground activity by `EACScrollRefreshImpl` and
   clamped to 1000–5000 ms; `getEacScrollStyle` returns 0 when the OEC service is not ready and does
   not check `isAppConfigActive`; `enableA2Scrolling` comes from a firmware config file; the Entry A
   bounds filter also applies to DOWN/UP; the Entry A pen-state read is asynchronous.
6. §5.2: the third clear-flag value (2 = defer clear to the next new layer).
7. §0: the "Android 11 vs 13" discrepancy is the fingerprint format (vendor partition); since
   verified on the tablet.
8. §7: corrected the SDK render-choice row; added rows for the SF eraser trail, the resume delay,
   `enableA2Scrolling` and `scroll_refresh_delay_v2`.
9. §9: E8 and E11 rewritten for the corrected eraser behaviour; E13 added.

### What could not be confirmed

- Everything in §2.1 that was not listed above (field offsets such as `EPDC+0x8`, `+0x280`,
  `PenManager+0x2054`, reader numbering and `BtnReader::onSynEvent` selection) was not re-derived; it
  is carried over from the original author's disassembly.
- `HandwritingSchema::output` (§4.5), `HWNormalSchema::output`, the debouncer mask and the
  `FixedUpdateEntryManager` HW-Tcon clear table (value 8) were not re-read.
- Whether the eraser end of the stylus uses the eraser-tool `EraserArgs` type, which decides between
  SF reader 4 and 3 on stock (E8).
- Which pen types reach `NormalShapeHandler` and therefore `MOVE_TO` on this device (E13).
- The device values of `enableA2Scrolling`, `eacScrollStyle`, `scrollingRefreshMode`,
  `gcAfterScrolling` and Tangleaf's `scrollRefreshDelay`.
- Whether stock's own transient-A2 exits (zoom, drag) show as a visible flash; the SF code queues a
  full-screen mode-5 update for each of them, and the kernel waveform choice for it is [unknown] (E3).
