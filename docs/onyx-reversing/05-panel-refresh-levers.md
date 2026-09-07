# Panel-refresh levers reachable from an unprivileged app — Onyx BOOX Note Air 4C (`lito`, Android 13, 4.2-rel, Kaleido 3 CFA)

> Written 2026-09-07 against the decompiled framework, the `com.onyx` system app, the Pen SDK and
> the on-device native libraries, before `/system/bin/surfaceflinger` was extracted from the firmware
> package. Statements marked as inference about the SurfaceFlinger side can now be checked against the
> binary — see [README](README.md). Absolute paths in this document refer to the scratch tree described
> in [01-sources-and-firmware.md](01-sources-and-firmware.md).

Scope: what an ordinary third-party app can actually change about how this panel repaints, what each
lever does on **this** firmware and **this** panel, and what to do for handwriting, ordinary UI and
de-ghosting.

Evidence roots (all paths absolute):

- Framework: `/home/okhsunrog/tmp_zfs/onyx_framework/out/sources/android/**`
- Framework resources (the per-device JSON that decides everything): `/home/okhsunrog/tmp_zfs/onyx_framework/fwres/x/res/raw/`
- `com.onyx` system app: `/home/okhsunrog/tmp_zfs/onyx_framework/out-kcb/sources/`
- Onyx SDK: `/home/okhsunrog/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-device-1.3.5.2/sources/`
- Native: `/home/okhsunrog/tmp_zfs/onyx_framework/native/`
- Ours: `/home/okhsunrog/code/rust/notes-rs/plugins/tauri-plugin-mobile-system/android/src/main/java/dev/okhsunrog/mobile_system/`

Statements marked **[inference]** are not directly attested by code.

---

## 0. The single most important file

`ConfigLoader.load(cls, category)` builds the resource name `Build.MODEL + "_" + category`, lowercased,
and looks it up in `android`'s `res/raw`, merging it over the base config
(`/home/okhsunrog/tmp_zfs/onyx_framework/out/sources/android/onyx/config/ConfigLoader.java:34-63`).
For this device that resolves to **`noteair4c_systemui.json`**, **`noteair4c_eac.json`** and
**`noteair4c_config.json`**, all present in `/home/okhsunrog/tmp_zfs/onyx_framework/fwres/x/res/raw/`.

`/home/okhsunrog/tmp_zfs/onyx_framework/fwres/x/res/raw/noteair4c_systemui.json` (verbatim):

```json
{
  "systemUIMode": 2,
  "systemEinkUIMode": 1,
  "keyboardAttachedRefreshIndex": 3,
  "forceHideSystemDivider": true,
  "fullKeyboardLayoutConfigMode": 0,
  "supportAntiFlicker": true,
  "refreshConfigMap": {
    "refresh_mode_1": { "mode": 5, "title": "REGAL_PLUS" },
    "refresh_mode_2": { "mode": 2, "turbo": 5, "title": "NEW_SPEED" },
    "refresh_mode_4": { "mode": 0, "title": "HD" }
  },
  "refreshConfig": ["refresh_mode_1", "refresh_mode_2"],
  "refreshConfigForSystemApp": ["refresh_mode_4", "refresh_mode_2"],
  "refreshMigrateMap": { "0": 5, "1": 2, "2": 2, "3": 5, "4": 2 },
  "refreshMigrateMapForSystemApp": { "0": 0, "1": 2, "2": 2, "3": 5, "4": 2 }
}
```

`/home/okhsunrog/tmp_zfs/onyx_framework/fwres/x/res/raw/noteair4c_eac.json` (verbatim):

```json
{
  "enableXMode": false,
  "saturationMinValue": 60,
  "cfaSaturationDefault": 0,
  "minAnimationDuration": 80,
  "enhanceDelta": 90,
  "enhanceThreshold": 2,
  "epdColorMode": 1,
  "colorMode": "vivid",
  "dpiModeMap": { "eac_dpi_type_1": 0, "eac_dpi_type_2": 450, "eac_dpi_type_3": 450 }
}
```

Consequences that run through the whole report:

- **There are only three refresh profiles on this device, not five.** `refresh_mode_3` and
  `refresh_mode_5` do not exist in the map; `RefreshModeIndex` (`.../android/onyx/utils/RefreshModeIndex.java:6-12`)
  defines `NONE, REFRESH_MODE_1..REFRESH_MODE_5` but only 1, 2, 4 are populated here.
  EinkWise offers a third-party app only `refresh_mode_1` (REGAL*PLUS) and `refresh_mode_2` (NEW_SPEED);
  `refresh_mode_4` (HD, mode 0) is listed only for system apps — but nothing \_enforces* that list on
  the write path (§3.4).
- `epdColorMode: 1` ⇒ `DeviceController.isCfaDevice() == true`
  (`.../android/onyx/hardware/DeviceController.java:334-335`), and `isBwDevice()` is false (`:317`).
- `enableXMode: false` in `noteair4c_eac.json` is **inert**: that file is deserialised into
  `android.onyx.config.EACConfig`, which has no such field. `enableXMode` belongs to `SysUIConfig`
  (`config/SysUIConfig.java:26`, default **`true`**) and `noteair4c_systemui.json` does not set it; its
  only consumer is the SystemUI mirror `systemui/SystemUIConfig.java:102,244`, not the refresh apply
  path. X mode is nonetheless unreachable _through the profile_: `refreshConfigMap` has no mode-4 entry
  and `refreshMigrateMap` sends `4 → 2`. `UI_X_A2_MODE` / `UI_X_DU_MODE` remain settable by a direct
  `applyAppScopeUpdate` or `View.setDefaultUpdateMode`.
- `colorMode: "vivid"` is **not a key of `colorModeSet`** (which has `eac_color_type_1..3`), so
  `EACConfig.getCfaSaturationDefault()` / `getCfaBrightnessDefault()` / `getContrastDefault()` all fall
  through to the scalar fields (`.../android/onyx/config/EACConfig.java:74-92`): saturation default **0**,
  brightness default **0**, contrast (gamma) default **30** (`Constant.GLOBAL_CONTRAST_DEFAULT = 30`,
  `.../android/onyx/optimization/Constant.java:388`).

---

## 1. Update modes and masks as defined on this firmware

All constants are in
`/home/okhsunrog/tmp_zfs/onyx_framework/out/sources/android/onyx/ViewUpdateHelper.java:33-101`.

### 1.1 The bit layout

A "UI mode" integer is a waveform nibble OR-ed with flag bits.

| Field        | Mask                                             | Values                                                                                                                             |
| ------------ | ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| waveform     | `EINK_WAVEFORM_MODE_MASK = 15`                   | `INIT 0`, `DU 1`, `GC16 2`, `GC4 3`, `ANIM(A2) 4`, `AUTO 5`, `REAGL(REGAL) 6`, `DU4 8`, `REAGL_PLUS 9`, `GCC16 11`, `DEEP_GC16 12` |
| partial/full | `EINK_UPDATE_MODE_MASK = 32`                     | `PARTIAL 0`, `FULL 32`                                                                                                             |
| auto region  | `EINK_AUTO_MODE_MASK = 16`                       | `REGIONAL 0`, `AUTOMATIC 16`                                                                                                       |
| wait         | `EINK_WAIT_MODE_MASK = 64`                       | `NOWAIT 0`, `WAIT 64`                                                                                                              |
| combine      | `EINK_COMBINE_MODE_MASK = 128`                   | `NOCOMBINE 0`, `COMBINE 128`                                                                                                       |
| dither       | `EINK_DITHER_MODE_MASK = 256`                    | `NODITHER 0`, `DITHER 256`                                                                                                         |
| invert       | `EINK_INVERT_MODE_MASK = 512`                    | `NOINVERT 0`, `INVERT 512`                                                                                                         |
| convert      | `EINK_CONVERT_MODE_MASK = 1024`                  | `NOCONVERT 0`, `CONVERT 1024`                                                                                                      |
| dither depth | `EINK_DITHER_COLOR_MASK = 2048`                  | `Y4 0` (4-bit), `Y1 2048` (1-bit)                                                                                                  |
| REAGL-D      | `EINK_REAGL_MODE_REAGLD = 4096`                  |                                                                                                                                    |
| handwriting  | `EPDC_FLAG_HANDWRITE_GU = 524288`                |                                                                                                                                    |
| merge        | `2097152` (implicit, see `MERGE_UPDATE_MODE_*`)  |                                                                                                                                    |
| shutdown     | `EINK_FLAG_SHUTDOWN = 5242880`                   |                                                                                                                                    |
| X-dither     | `EINK_DITHER_X = EINK_ONYX_AUTO_MASK = 16777216` |                                                                                                                                    |
| mono         | `EINK_APPLY_MONO = EINK_ONYX_GC_MASK = 33554432` |                                                                                                                                    |

### 1.2 The composed modes

| Name                           | Value      | Decoded                                                                                             | Honoured on this CFA panel?                                                                  |
| ------------------------------ | ---------- | --------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `UI_DU_MODE`                   | 1          | DU, partial, no dither                                                                              | yes, but the EAC layer never emits it — see 1.4                                              |
| `UI_GU_MODE`                   | 2          | GC16, partial, nowait                                                                               | **yes** — the workhorse partial                                                              |
| `UI_GC4_MODE`                  | 3          | GC4, partial                                                                                        | yes [inference: no gating found]                                                             |
| `UI_A2_PERFORMANCE_MODE`       | 4          | ANIM (A2), partial, no dither                                                                       | yes                                                                                          |
| `UI_DEFAULT_MODE`              | 5          | AUTO — let the EPDC pick                                                                            | yes; `View.defaultUpdateMode` initialises to this (`android/view/View.java:2321,2369,13719`) |
| `UI_REGAL_MODE`                | 6          | REAGL                                                                                               | **no** — `supportRegal()` is false; see 1.5                                                  |
| `UI_REGAL_PLUS_MODE`           | 9          | REAGL_PLUS                                                                                          | **claimed by the profile map but not by the panel**; see 1.5                                 |
| `UI_GC_MODE`                   | 98         | `GC16 \| FULL \| WAIT` = full flash, blocking                                                       | **yes** — the de-ghost primitive                                                             |
| `UI_GCC_MODE`                  | 107        | `GCC16 \| FULL \| WAIT`                                                                             | CFA-specific "GC colour" [inference from name + `GCC16=11`]                                  |
| `UI_DEEP_GC_MODE`              | 108        | `DEEP_GC16 \| FULL \| WAIT`                                                                         | yes — the strongest clear                                                                    |
| `UI_DU_QUALITY_MODE`           | 2305       | `DU \| DITHER \| Y1`                                                                                | yes                                                                                          |
| `UI_A2_QUALITY_MODE`           | 2308       | `ANIM \| DITHER \| Y1`                                                                              | yes — what `applyTransientUpdate(ANIMATION_QUALITY)` sends                                   |
| `UI_DU4_MODE`                  | 2312       | `DU4 \| DITHER \| Y1`                                                                               | yes                                                                                          |
| `UI_X_DU_MODE`                 | 16777217   | `DU \| DITHER_X`                                                                                    | settable directly; unreachable via the profile (no mode-4 entry, `refreshMigrateMap 4→2`)    |
| `UI_X_A2_MODE`                 | 16777220   | `ANIM \| DITHER_X`                                                                                  | same                                                                                         |
| `UI_MONO_A2_MODE`              | 33554436   | `ANIM \| APPLY_MONO`                                                                                | reachable, but `applyMonoLevel` is skipped on CFA (§4)                                       |
| `UI_GC_SHUTDOWN_MODE`          | 5242978    | `98 \| SHUTDOWN`                                                                                    | shutdown only                                                                                |
| `UI_REGAL_SHUTDOWN_MODE`       | 5242886    | `6 \| SHUTDOWN`                                                                                     | shutdown only                                                                                |
| `MERGE_UPDATE_MODE_BY_TIMEOUT` | 2097184    | merge-flag \| FULL                                                                                  | used by `mergeDisplayUpdate`                                                                 |
| `MERGE_UPDATE_MODE_BY_COUNT`   | 2097185    | merge-flag \| FULL \| DU                                                                            | used by `mergeDisplayByCount`                                                                |
| `HAND_WRITING_REPAINT_MODE`    | **524290** | `EPDC_FLAG_HANDWRITE_GU (524288) \| GC16 (2)`                                                       | yes                                                                                          |
| `INVALID_MODE`                 | −1         | in `applyUpdateMode` this triggers `whiteClear(2, true)` (`impl/TabletEACRefreshImpl.java:106-108`) |                                                                                              |

### 1.3 What `HAND_WRITING_REPAINT_MODE` really is

`524290 = 524288 | 2` = **`UI_GU_MODE` (a partial, non-blocking GC16 update) plus the
`EPDC_FLAG_HANDWRITE_GU` flag.** It is not a distinct waveform. The flag tells the EPDC/SF side that
this update is the reconcile of a region that the raw-drawing pen path has been scribbling into, so
the composited framebuffer must replace the pen overlay rather than be merged under it.
`HAND_WRITING_REPAINT_MODE` appears nowhere else in the framework — it is a constant exported purely
for apps and for `SDMDevice` to reflect (`SDMDevice.java:1042`,
`/home/okhsunrog/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-device-1.3.5.2/sources/com/onyx/android/sdk/device/SDMDevice.java`).
The system's own equivalent for third-party note apps is the separate
`ViewUpdateHelper.handwritingRepaint(view, l, t, r, b, boolean)` transaction (code `1048647`,
`ViewUpdateHelper.java:781-805`), fired 500 ms after stylus-up by
`android/onyx/optimization/screennote/handler/BaseHandler.java:268-283` over the draw view's visible
rect, with the delay coming from `EACNoteConfig.repaintLatency` (default **500**,
`optimization/data/v2/EACNoteConfig.java:21`).

### 1.4 The EAC-level modes and how they map down

`android/onyx/optimization/Constant.java:159-166`:

```
UPDATE_MODE_UNKNOWN = -1   UPDATE_MODE_NORMAL/DEFAULT = 0   UPDATE_MODE_DU = 1
UPDATE_MODE_A2 = 2         UPDATE_MODE_REGAL = 3            UPDATE_MODE_X = 4
UPDATE_MODE_REGAL_PLUS = 5
```

`android/onyx/optimization/EACUtils.java:127-142` `toEpdMode()`:

| EAC mode     | → EPD mode   | meaning                |
| ------------ | ------------ | ---------------------- |
| 0 NORMAL     | **5**        | AUTO — EPDC decides    |
| 1 DU         | **2305**     | `DU \| DITHER \| Y1`   |
| 2 A2         | **2308**     | `ANIM \| DITHER \| Y1` |
| 3 REGAL      | **6**        | REAGL                  |
| 4 X          | **16777220** | `ANIM \| DITHER_X`     |
| 5 REGAL_PLUS | **9**        | REAGL_PLUS             |

### 1.5 What is silently downgraded on this panel

Two independent remapping layers, both attested:

1. **`refreshMigrateMap` (per device, `noteair4c_systemui.json`)**, applied by
   `optimization/action/ValidateAppConfigAction.java:180-185` whenever the stored config has **no valid
   `refreshModeIndex`** and `turbo == 0`:
   `{0:5, 1:2, 2:2, 3:5, 4:2}`. So for a third-party app:
   **NORMAL(0) → REGAL_PLUS(5)**, **DU(1) → A2(2)**, A2(2) → A2(2), **REGAL(3) → REGAL_PLUS(5)**,
   **X(4) → A2(2)**. For a system/Onyx app the map is `{0:0, 1:2, 2:2, 3:5, 4:2}` — NORMAL stays NORMAL.
   _This is why setting DU on this device gets you A2, and is the most likely mechanical explanation
   for "DU while typing was worse artefacts": it was never DU._ **[inference on the causal link;
   the map itself is verbatim.]**
2. **REGAL is refused twice over.** `ViewUpdateHelper.supportRegal()` round-trips SF code `16711708`
   (`ViewUpdateHelper.java:1337-1344`) and returns false here. On top of that
   `EACBaseRefreshImpl.getConfigUpdateMode` (`impl/EACBaseRefreshImpl.java:118-132`) contains:

   ```java
   if (mode == 3) { // "override auto refresh from regal to normal,
                    //  only use regal when key/motion event up"
       mode = 0;
   }
   ```

   so even where REGAL survives validation it is used only for the debounced post-input repaint,
   never as the running mode.

3. **`refresh_mode_1` claims mode 5 = REGAL_PLUS** on this device even though the panel does not do
   REAGL. `toEpdMode(5) = 9 = EINK_WAVEFORM_MODE_REAGL_PLUS`. The EPDC will fall back to whatever
   waveform it has for 9. **[inference: with `supportRegal:false` the practical result is a GC16-class
   update; this is exactly the "our REGAL view-mode setting falls back to GU" observation.]**
4. **X mode is unreachable through the profile** — `refreshConfigMap` has no mode-4 entry and
   `refreshMigrateMap` sends `4 → 2`. (It is _not_ disabled by config: see §0 — `enableXMode:false`
   in `noteair4c_eac.json` lands in a bean that has no such field.) `UI_X_A2_MODE` (16777220) and
   `UI_X_DU_MODE` (16777217) are still settable directly.
5. **`applySysScopeUpdate` is an empty method** on this firmware —
   `ViewUpdateHelper.java:401-402`:

   ```java
   public static void applySysScopeUpdate(int i, int i2, int i3) {
   }
   ```

   Therefore `clearSysScopeUpdate()` (`:484-486`) and `setUpdateScheme(int)` (`:1298-1300`) are
   **guaranteed no-ops**. Anything routed through them does nothing at all.

6. **`inSystemFastMode()` is hardcoded `false`** (`ViewUpdateHelper.java:824-826`), so
   `toggleA2Mode()` always takes the `switchToA2Mode()` branch.

---

## 2. Every app-reachable lever

### 2.0 The three transport channels

1. **SurfaceFlinger binder** — `ViewUpdateHelper.transactData(code, Parcel, Parcel)`
   (`ViewUpdateHelper.java:1421-1438`) does `ServiceManager.getService("SurfaceFlinger").transact(code, …)`
   with interface token `android.ui.ISurfaceComposer`. **No permission check anywhere in
   `ViewUpdateHelper`**; the only gate is SELinux `binder_call(untrusted_app, surfaceflinger)`, which
   is allowed by default on AOSP. Every exception is swallowed and logged — a rejected transaction is
   indistinguishable from success (`:1432-1434`, `:1387-1389`). Callable from a normal app after
   `HiddenApiBypass` (which we already do, `VendorAccess.kt:19-23`).
2. **`oec_service` binder** — `EInkHelper` → `IOECService`
   (`optimization/EInkHelper.java:72-93`, `optimization/IOECService.java`). Nothing in `OECService.java`
   calls `checkCallingPermission` / `enforceCallingPermission` / `Binder.getCallingUid` on any refresh
   or display setter; the only uid-shaped check in the file is
   `ActivityManagerHelper.isSystemApp(pkg)` guarding `fullPMAccess` (`OECService.java:1230`). Every
   setter operates on `getCurrentTopComponent()` — i.e. **on whichever app is foreground when you
   call it**, so it is our app iff we are foreground. Most setters call `saveDeviceConfig(...)`, so the
   effect is **persisted and survives our process, our activity, and a reboot**.
3. **View fields** — `View.setDefaultUpdateMode(int)` / `getDefaultUpdateMode()`
   (`android/view/View.java:14577-14579`, `:9261-9262`) are a plain private `int` field
   (`:525`) initialised to **5** (`UI_DEFAULT_MODE`) at `:2321,2369,13719`. Process-local, view-local,
   gone when the process dies. `View.invalidate(mode)` / `invalidate(rect, mode)` /
   `postInvalidate*(…, mode)` take a per-call mode (`:10788-10820`, `:13290-13361`).

### 2.1 Lever-by-lever

Scope key: **V** = one view, **A** = our app (top component), **S** = system-wide.

| Lever                                                                                                            | Exists here?                                                                                                                                                                                                                      | Transport / code                                                                                                   | Scope                        | Lifetime                                                                                                                                   | Privileged?                                                                                                                                                                                                                                              |
| ---------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ | ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `View.setDefaultUpdateMode(int)` / `getDefaultUpdateMode()`                                                      | yes, `View.java:14577`,`9261`                                                                                                                                                                                                     | field write                                                                                                        | V                            | until view detach / process death                                                                                                          | no                                                                                                                                                                                                                                                       |
| `View.invalidate(…, mode)`                                                                                       | yes, `View.java:10788-10820`                                                                                                                                                                                                      | draw request                                                                                                       | V, one frame                 | one frame                                                                                                                                  | no                                                                                                                                                                                                                                                       |
| `setFirstDrawUpdateMode`                                                                                         | **absent from the framework** (grep over `out/sources/android`: no such symbol). `ViewUpdateHelper.firstUpdateMode` is a `public static int = 98` used only as `Constant.DEFAULT_MERGE_DISPLAY_UPDATE_MODE` (`Constant.java:379`) | —                                                                                                                  | —                            | —                                                                                                                                          | —                                                                                                                                                                                                                                                        |
| `applyTransientUpdate(int)`                                                                                      | yes, `ViewUpdateHelper.java:404-408`                                                                                                                                                                                              | SF `APPLY_TRANSIENT_UPDATE 16711782`                                                                               | A (SF-side, per calling app) | until `clearTransientUpdate`                                                                                                               | no                                                                                                                                                                                                                                                       |
| `clearTransientUpdate(int/boolean)`                                                                              | yes, `:488-496`                                                                                                                                                                                                                   | SF `16711783`                                                                                                      | A                            | —                                                                                                                                          | no. The int is a `CLEAR_FLAG_*`: `NONE 0`, `IMMEDIATELY 1`, `NEW_SURFACE 2` (`:30-32`)                                                                                                                                                                   |
| `applyAppScopeUpdate(pkgHash, enable, clearFlag, epdMode, repeatLimit)`                                          | yes, `:302-322`                                                                                                                                                                                                                   | SF `APPLY_APP_SCOPE_UPDATE 16711684`                                                                               | A                            | until cleared / app die                                                                                                                    | no                                                                                                                                                                                                                                                       |
| `clearAppScopeUpdate(int)`                                                                                       | yes, `:474-482`                                                                                                                                                                                                                   | SF `CLEAR_APP_SCOPE_UPDATE 1048640`                                                                                | A                            | —                                                                                                                                          | no                                                                                                                                                                                                                                                       |
| `applySysScopeUpdate` / `clearSysScopeUpdate` / `setUpdateScheme`                                                | **method body is empty** `:401-402`, `:484-486`, `:1298-1300`                                                                                                                                                                     | —                                                                                                                  | —                            | **guaranteed no-op**                                                                                                                       | —                                                                                                                                                                                                                                                        |
| `setGlobalUpdateMode`                                                                                            | **does not exist.** `ViewUpdateHelper.globalUpdateMode` (`:103`, `=5`) is a plain static read only by the instance helper `keyUpdateMode` (`:1515-1524`)                                                                          | —                                                                                                                  | —                            | —                                                                                                                                          | —                                                                                                                                                                                                                                                        |
| `setEpdTurbo(int)` / `getEpdTurbo()`                                                                             | yes, `:1161-1165`, `:684-691`                                                                                                                                                                                                     | SF `1048661` / `1048689`                                                                                           | S (SF global)                | until changed; **not restored on app switch except by the EAC layer**                                                                      | no. Legal range `TURBO_0..TURBO_6 = 0..6` (`Constant.java:167-173`)                                                                                                                                                                                      |
| `setGcRefreshInterval(int)`                                                                                      | yes, `:1174-1178`                                                                                                                                                                                                                 | SF `1048658`                                                                                                       | S                            | until changed                                                                                                                              | no. **Nothing in the framework ever calls it** — it is an app-only knob into SF's own GC counter                                                                                                                                                         |
| `setDisplayScheme(int)`                                                                                          | yes, `:1131-1136`                                                                                                                                                                                                                 | SF `SET_APPLICATION_DISPLAY_SCHEME 16711701`                                                                       | A                            | until changed                                                                                                                              | no. Schemes 1..5 (`:219-225`). Returns hardcoded `true`, so acceptance is unverifiable                                                                                                                                                                   |
| `applyGCOnce()`                                                                                                  | yes, `:350-352`                                                                                                                                                                                                                   | SF `APPLY_GC_ONCE 16711718`                                                                                        | S, one-shot                  | one update                                                                                                                                 | no                                                                                                                                                                                                                                                       |
| `repaintEverything()`                                                                                            | yes, `:1057-1059`                                                                                                                                                                                                                 | SF `16711700`                                                                                                      | S, one-shot                  | one update                                                                                                                                 | no                                                                                                                                                                                                                                                       |
| `repaintEverything(int mode)`                                                                                    | yes, `:1061-1065`                                                                                                                                                                                                                 | SF `REPAINT_EVERY_THING_WITH_MODE 16711715`                                                                        | S, one-shot                  | one update                                                                                                                                 | no                                                                                                                                                                                                                                                       |
| `refreshScreen(view/rect, mode)`                                                                                 | yes, `:1036-1055`                                                                                                                                                                                                                 | SF `REFRESH_SCREEN 16711681`                                                                                       | region, one-shot             | one update                                                                                                                                 | no                                                                                                                                                                                                                                                       |
| `fullRefreshScreen(View)`                                                                                        | yes, `:637-646` — `refreshScreen(rect, 98)`                                                                                                                                                                                       | region                                                                                                             | one update                   | no                                                                                                                                         |
| `whiteClear(int, boolean)`                                                                                       | yes, `:1460-1465`                                                                                                                                                                                                                 | SF `WHITE_CLEAR 1048663`                                                                                           | S                            | one-shot                                                                                                                                   | no                                                                                                                                                                                                                                                       |
| `enableScreenUpdate(boolean)`                                                                                    | yes, `:620-624`                                                                                                                                                                                                                   | SF `ENABLE_UPDATE`, declared as `Spanned.SPAN_PRIORITY` (`:106`) — there is no `disableScreenUpdate`; pass `false` | S                            | until re-enabled — **dangerous: leaves the panel frozen if we die mid-suppress**                                                           | no                                                                                                                                                                                                                                                       |
| `useGCForNewSurface(boolean)`                                                                                    | yes, `:1450-1454`                                                                                                                                                                                                                 | SF `1048657`                                                                                                       | S                            | until changed                                                                                                                              | no. Default on this device is **true** (`Constant.DEFAULT_USE_GC_FOR_NEW_SURFACE = isCfaDevice()`, `Constant.java:390`) and `TabletEACRefreshImpl.applyUpdateMode` re-asserts `true` whenever the profile asks (`impl/TabletEACRefreshImpl.java:99-101`) |
| `setUpdListSize(int)`                                                                                            | yes, `:1292-1296`                                                                                                                                                                                                                 | SF `SET_UPD_LIST_SIZE 16711716`                                                                                    | S                            | until changed                                                                                                                              | no                                                                                                                                                                                                                                                       |
| `handwritingRepaint(View,l,t,r,b[,bool])`                                                                        | yes, `:781-805`                                                                                                                                                                                                                   | SF `HANDWRITING_REPAINT 1048647`                                                                                   | region, one-shot             | one update                                                                                                                                 | no                                                                                                                                                                                                                                                       |
| `setScreenHandWritingPenState(int)`                                                                              | yes, `:1195-1200`                                                                                                                                                                                                                 | SF `16711693`, writes `Process.myPid()`                                                                            | A                            | until changed                                                                                                                              | no. States `PEN_STOP 0 / START 1 / DRAWING 2 / PAUSE 3 / ERASING 4` (`Constant.PenState`)                                                                                                                                                                |
| A2 helpers: `switchToA2Mode()` / `switchToNormalMode()` / `toggleA2Mode()`                                       | yes, `:1352-1366` — but they are thin wrappers over `EInkHelper.setAppScopeRefreshMode(2 / 0)`, i.e. the **EAC** path, not SF (§3.3). `toggleA2Mode` is broken: `inSystemFastMode()` is hardcoded false (`:824-826`)              | A                                                                                                                  | **persisted**                | no                                                                                                                                         |
| `enableA2ForSpecificView` / `disableA2ForSpecificView` (SDK)                                                     | maps to `View.setDefaultUpdateMode(UI_A2_*)` [inference from SDK shape]                                                                                                                                                           | V                                                                                                                  | view lifetime                | no                                                                                                                                         |
| `byPass(count)` / `byPassUntilLayerOff/On` / `byPassAnimation()`                                                 | yes, `:410-455`                                                                                                                                                                                                                   | SF `BYPASS 1048621`, `1048662`, `1048664`, `16711792`                                                              | S                            | count-limited                                                                                                                              | no. Ownership-guarded by a static `byPassOwner` string (`:464-472`)                                                                                                                                                                                      |
| `debouncer(enable, epdMode, shortDelay, longDelay, gcInterval)`                                                  | yes, `:532-540`                                                                                                                                                                                                                   | SF `DEBOUNCER 16711780`                                                                                            | A                            | until changed                                                                                                                              | no — **this is the firmware's post-input quality repaint**, see §3.5                                                                                                                                                                                     |
| `debounceIncRefresh()`                                                                                           | yes, `:528-530`                                                                                                                                                                                                                   | SF `16711781`                                                                                                      | A                            | one tick                                                                                                                                   | no                                                                                                                                                                                                                                                       |
| `setDebouncerTransientUpdateMode(int)`                                                                           | yes, `:1125-1129`                                                                                                                                                                                                                 | SF `16711785`                                                                                                      | A                            | until changed                                                                                                                              | no                                                                                                                                                                                                                                                       |
| `mergeDisplayUpdate(mode, timeout)` / `mergeDisplayByCount(mode, count)`                                         | yes, `:960-972`                                                                                                                                                                                                                   | SF `1048617` / `1048624`                                                                                           | S                            | until changed                                                                                                                              | no. Defaults `MERGE_UPDATE_MODE_BY_TIMEOUT`, timeout 200 ms, count 50 (`Constant.java:66-70`)                                                                                                                                                            |
| `setTrigger(int)` / `setTriggerByPenRect(…)`                                                                     | yes, `:1278-1290`                                                                                                                                                                                                                 | SF `16711713` / `1048619`                                                                                          | A                            |                                                                                                                                            | no                                                                                                                                                                                                                                                       |
| `waitForUpdateFinished()`                                                                                        | yes, `:1456-1458`                                                                                                                                                                                                                 | SF `16711703`                                                                                                      | S, blocking                  |                                                                                                                                            | no                                                                                                                                                                                                                                                       |
| `enableDither(boolean)`                                                                                          | yes, `:571-575`                                                                                                                                                                                                                   | SF `ENABLE_DITHER 1048692`                                                                                         | S                            | until changed                                                                                                                              | no                                                                                                                                                                                                                                                       |
| `setDitherThreshold(int)`                                                                                        | yes, `:1138-1145`                                                                                                                                                                                                                 | SF `SET_DITHER_THRESHOLD 1048704`                                                                                  | S                            | until changed                                                                                                                              | no. **Silently drops any value `< 128`** (`:1139-1141`)                                                                                                                                                                                                  |
| `applyDitherFilterTolerance(float)`                                                                              | yes, `:344-348`                                                                                                                                                                                                                   | SF `1048720`                                                                                                       | S                            |                                                                                                                                            | no                                                                                                                                                                                                                                                       |
| `applyNoiseStrength(float)`                                                                                      | yes, `:371-375`                                                                                                                                                                                                                   | SF `1048713`                                                                                                       | S                            |                                                                                                                                            | no                                                                                                                                                                                                                                                       |
| `applyGammaCorrection(bool, int)`                                                                                | yes, `:354-363`                                                                                                                                                                                                                   | SF `APPLY_GAMMA_CORRECTION 16711695`                                                                               | S                            | until changed                                                                                                                              | no                                                                                                                                                                                                                                                       |
| `applySaturation(int)` / `applySaturationMinValue(int)` / `applySaturationFactor(float)`                         | yes, `:383-399`                                                                                                                                                                                                                   | SF `16711776` / `1048694` / `1048712`                                                                              | S                            | until changed                                                                                                                              | no. Range 0..100 (`Constant.CFA_SATURATION_MIN/MAX_VALUE`)                                                                                                                                                                                               |
| `applyBrightness(int)`                                                                                           | yes, `:324-328`                                                                                                                                                                                                                   | SF `16711777`                                                                                                      | S                            |                                                                                                                                            | no. Range 0..5 (`Constant.CFA_BRIGHTNESS_MIN/MAX_VALUE`)                                                                                                                                                                                                 |
| `applyColorParameter(contrast, saturation, brightness)`                                                          | yes, `:336-342`                                                                                                                                                                                                                   | SF `1048708`                                                                                                       | S                            |                                                                                                                                            | no — one atomic call for the three                                                                                                                                                                                                                       |
| `applyMonoLevel(int)`                                                                                            | yes, `:365-369`, offset by `offsetMonoLevel` (`:1001-1006`, `+80`)                                                                                                                                                                | S                                                                                                                  |                              | no. **Never applied on this device** — `EACBaseDisplayImpl.applyDisplayConfig` takes the CFA branch (`impl/EACBaseDisplayImpl.java:38-45`) |
| `setGrayscaleMode(int)`                                                                                          | yes, `:1180-1184`                                                                                                                                                                                                                 | SF `16711778`                                                                                                      | S                            |                                                                                                                                            | no. `CFA_GRAY_SCALE_MODE_COLOR = 0`, `_BW_ONLY = 1` (`Constant.java:44-45`)                                                                                                                                                                              |
| `setBWMode(int)` / `enableBWMode(boolean)`                                                                       | yes, `:1101-1108`, `:545-547`                                                                                                                                                                                                     | SF `ENABLE_BW_MODE 1048696`                                                                                        | S                            | persisted in `Settings.Global["view_update_bw_mode"]` by the service (`OECService.java:224-228`)                                           | no                                                                                                                                                                                                                                                       |
| `applyColorFilter(int)`                                                                                          | yes, `:330-334`                                                                                                                                                                                                                   | SF `COLOR_FILTER 1048622`                                                                                          | S                            |                                                                                                                                            | no                                                                                                                                                                                                                                                       |
| `enableColorCU(boolean)` / `enableColorAdjust(boolean)` / `enableEnhance(boolean)` / `setEnhanceStrategy(a,b,c)` | yes, `:565-581`, `:1153-1159`                                                                                                                                                                                                     | SF `1048693` / `1048721` / `1048705` / `1048697`                                                                   | S                            |                                                                                                                                            | no                                                                                                                                                                                                                                                       |
| `antiFlicker(int)`                                                                                               | yes, `:290-294`                                                                                                                                                                                                                   | SF `SET_ANTI_FLICKER 1048691`                                                                                      | S                            |                                                                                                                                            | no. Range 0..32, default 10 (`Constant.ANTI_FLICKER_*`); `supportAntiFlicker:true` in `noteair4c_systemui.json`                                                                                                                                          |
| `getColorType()`                                                                                                 | yes, `:648-655`                                                                                                                                                                                                                   | SF `1048659`                                                                                                       | read                         |                                                                                                                                            | no                                                                                                                                                                                                                                                       |
| `supportRegal()`                                                                                                 | yes, `:1337-1344`                                                                                                                                                                                                                 | SF `16711708`                                                                                                      | read                         |                                                                                                                                            | no — **returns false here**                                                                                                                                                                                                                              |
| `enableRegal(boolean)`                                                                                           | yes, `:610-618`                                                                                                                                                                                                                   | SF `ENABLE_REGAL 16711722`                                                                                         | S                            |                                                                                                                                            | no                                                                                                                                                                                                                                                       |
| `setVCom(int, String)`                                                                                           | yes, `:1302-1307`                                                                                                                                                                                                                 | SF `16711720`                                                                                                      | S                            |                                                                                                                                            | no; `vcomPath: /sys/class/ebc/vcom_value` (`noteair4c_config.json`). **Do not touch — panel damage risk.**                                                                                                                                               |
| `DitherUtils.ditherBitmap(Bitmap, quantBits, List<RectF>)` / `ditherColorBitmap(…)`                              | yes — `android/onyx/utils/DitherUtils.java`, JNI into `libonyx_neo_dither.so`                                                                                                                                                     | our own bitmap                                                                                                     | —                            | no. `quantBits` must be 1..4                                                                                                               |

### 2.2 The `oec_service` levers (all persist, all target the foreground app)

From `optimization/EInkHelper.java` (line numbers are the `public static` declarations):

`setAppScopeRefreshMode(int)` `:1469` · `getAppScopeRefreshMode()` `:541` · `setTurbo(int)` `:1715` /
`getTurbo()` `:1016` · `setGcInterval(int)` `:1611` / `getGcInterval()` `:846` ·
`setAntiFlicker(int)` `:1458` / `getAntiFlicker()` `:477` · `setDitherThreshold(int)` `:1570` /
`getDitherThreshold()` `:793` · `setEnhance(boolean)` `:1600` / `getEnhance()` `:815` ·
`setCFAColorSaturation(int)` `:1505` / `getCFAColorSaturation()` `:651` ·
`setCFAColorBrightness(int)` `:1494` / `getCFAColorBrightness()` `:636` ·
`setColorParameter(c,s,b)` `:1538` · `setGlobalContrast(int)` `:1622` / `getGlobalContrast()` `:861` ·
`setBwMode(int)` `:1483` / `getBwMode()` `:621` · `setMonoLevel(int)` `:1648` / `getMonoLevel()` `:876` ·
`setColorMode(String)` `:1527` / `getColorMode()` `:681` ·
`setScrollingRefreshMode(int)` `:1683` / `getScrollingRefreshMode()` `:977` ·
`setIsGCAfterScrolling(boolean)` `:1637` · `setAnimationDuration(int)` `:1447` /
`getAnimationDuration()` `:462` · `getAppConfigFromService(List)` `:498` ·
`applyAppConfigToService(List, Bundle)` `:230` · `applyRefreshConfigToAll(String)` `:318` ·
`getRefreshConfig(Context)` `:947` · `getNoteConfig(Context)` `:891` ·
`getDeviceExtraConfig()` `:723` · `getSFDebounceEACRefreshConfig()` `:969`.

Every one of them is `if (isServiceReady()) { try { getService().X(); } catch { log } }` — **silent on
failure**. `OECService.setAppScopeRefreshMode` additionally has an rx `filter` that drops the call
entirely when `getCurrentTopComponent() == null` (`OECService.java:1761-1778`, `:1483-1485`).

**`setSystemRefreshMode` does not exist on this firmware.** `EACReflectUtils.sMethodSetSystemRefreshMode`
(`/home/okhsunrog/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-device-1.3.5.2/sources/com/onyx/android/sdk/api/device/eac/EACReflectUtils.java:61`)
resolves `EInkHelper.setSystemRefreshMode(int)`, which is absent from `EInkHelper`'s public API list —
so that reflective handle is `null` and the SDK's device-wide fallback is a silent no-op. Our
`applyAppRefreshProfile()` probe guard (`MobileSystemPlugin.kt:302-310`) is therefore protecting
against a hazard that cannot fire here, but it is also not the reason the call does nothing (§3.3).

---

## 3. The EAC per-app config schema on this firmware

Beans: `/home/okhsunrog/tmp_zfs/onyx_framework/out/sources/android/onyx/optimization/data/v2/`.
(`data/v1/` is the legacy schema — `LEGACY_JSON_VERSION = 99`, `UPGRADED_JSON_VERSION = 100`,
`Constant.java:107-108`. The service serves v2.)

### 3.1 Top level — `EACAppConfig` (`data/v2/EACAppConfig.java:22-40`)

| Field                                                                                      | Type                                                                                          | Default                                                                                                                                                       |
| ------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pkgName`                                                                                  | String                                                                                        | `""`                                                                                                                                                          |
| `enable`                                                                                   | boolean                                                                                       | `true` (ctor `:61`); `getDefaultAppConfig` sets `false` (`:115`)                                                                                              |
| `supportEAC`                                                                               | boolean                                                                                       | `true`                                                                                                                                                        |
| `globalActivityConfig`                                                                     | `EACActivityConfig`                                                                           | new                                                                                                                                                           |
| `activityConfigMap`                                                                        | `Map<clsName, EACActivityConfig>`                                                             | `{}` — per-activity override; `getRefreshConfig(cn)` prefers it (`:256-258`)                                                                                  |
| `extraConfig`                                                                              | `EACAppExtraConfig`                                                                           | `icon=null, forceFullScreen=true, usePageKeyAsVolumeKey=true, fullPMAccess=false, fullPMAccessTimeout=300000, useDialogBorder=false, allowSplashScreen=false` |
| `dpiConfig`                                                                                | `EACDpiConfig`                                                                                |                                                                                                                                                               |
| `rotationConfig`, `keyboardConfig`, `networkConfig`, `autoStartConfig`, `autoFreezeConfig` |                                                                                               |                                                                                                                                                               |
| `globalCSSConfig`, `cssConfigMap`                                                          | `EACCSSConfig`                                                                                | WebView CSS injection                                                                                                                                         |
| `colorMode`                                                                                | String                                                                                        | `EACConfig.getColorMode()` = `"vivid"` here                                                                                                                   |
| `colorConfigMap`                                                                           | `Map<String, EACColorConfig>`                                                                 | `EACColorConfig{gamma, saturation, brightness, monoLevel}`                                                                                                    |
| `scrollArgs`                                                                               | `EACScrollArgs{duration, sampleTime, startXPercent, startYPercent, endXPercent, endYPercent}` |                                                                                                                                                               |
| `scrollViewWhiteList`, `forceScrollRefreshClsList`                                         | `List<String>`                                                                                | `[]`                                                                                                                                                          |

### 3.2 `globalActivityConfig` — `EACActivityConfig` (`data/v2/EACActivityConfig.java:8-15`)

| Field                 | Default                                                 | Notes                                                                                       |
| --------------------- | ------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `enable`              | `true`                                                  |                                                                                             |
| `clsName`             | `""`                                                    |                                                                                             |
| `refreshConfig`       | `EACRefreshConfig`                                      | §3.3                                                                                        |
| `displayConfig`       | `EACDisplayConfig`                                      | §3.4                                                                                        |
| `paintConfig`         | `EACPaintConfig`                                        | §3.5                                                                                        |
| `noteConfig`          | `EACNoteConfig`                                         | §3.6                                                                                        |
| `scrollRefreshDelay`  | `0`, **clamped to `[1000, 5000]` on read** (`:7,49-51`) | written to `Settings.Global.SCROLL_REFRESH_DELAY` by `impl/EACScrollRefreshImpl.java:23-30` |
| `eacScrollStyle`      | `1`                                                     | `2` enables `OnyxEpdBypassManager` (`OnyxEpdBypassManager.java:188`)                        |
| `isDisableScrollAnim` | `false`                                                 |                                                                                             |

### 3.3 `refreshConfig` — `EACRefreshConfig` (`data/v2/EACRefreshConfig.java:8-17`)

| Field                | Type    | Default on this device                                                                                                                                                | Safe for a third-party app to set?                                                                              |
| -------------------- | ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `enable`             | boolean | inherited                                                                                                                                                             | **yes** — if `false`, every apply path bails (`impl/TabletEACRefreshImpl.java:27-41,95-98`)                     |
| `refreshModeIndex`   | String  | `"refresh_mode_2"` in `createDummyRefreshConfig` (`:30`)                                                                                                              | **yes, and it is the only field that matters** — see below                                                      |
| `updateMode`         | int     | `0`                                                                                                                                                                   | yes, but **ignored whenever `refreshModeIndex` resolves**                                                       |
| `turbo`              | int     | `0`                                                                                                                                                                   | yes, same caveat; 0..6                                                                                          |
| `gcInterval`         | int     | **`20`** (`:15`); `obtainLegalGcInterval()` turns `<=0` into `Integer.MAX_VALUE` (`:76-82`); UI range 0..50 (`Constant.GC_INTERVAL_MIN/MAX_VALUE`)                    | yes — feeds the SF debouncer's 5th argument                                                                     |
| `antiFlicker`        | int     | `EACConfig.getAntiFlickerDefault()` = **10**; range 0..32                                                                                                             | yes                                                                                                             |
| `animationDuration`  | int     | `0` → treated as 10 ms, or 20 ms when `updateMode==3` (`impl/TabletEACRefreshImpl.java:131-141`); floored by `EACConfig.minAnimationDuration` = **80** on this device | yes                                                                                                             |
| `animationType`      | String  | null → `"debounce"`; also `"delay" / "throttleFirst" / "throttleLast"` (`Constant.java:29-33`)                                                                        | yes                                                                                                             |
| `useGCForNewSurface` | boolean | `false` in the bean, but the device default is `true` (`Constant.java:390`) and the apply path only ever sets it _on_ (`TabletEACRefreshImpl.java:99-101`)            | yes                                                                                                             |
| `supportRegal`       | boolean | `false`                                                                                                                                                               | leave alone                                                                                                     |
| `refreshModeAlias`   | String  | null                                                                                                                                                                  | leave alone / remove — resolved through `RefreshMappingConfig` (`android/onyx/RefreshMappingConfig.java:33-36`) |

**The resolution rule.** `EACBaseRefreshImpl.caculateRefreshConfig`
(`impl/EACBaseRefreshImpl.java:67-79`, duplicated verbatim in `OnyxEpdBypassManager.java:173-185`):

```java
RefreshModeIndex idx = RefreshModeIndex.toEnum(cfg.getRefreshModeIndex());
if (idx != NONE && (data = SysUIConfig.getRefreshConfigByIndex(idx.name().toLowerCase())) != null)
    return data;                       // <-- systemui.json wins
return new RefreshModeData(cfg.getUpdateMode(), cfg.getTurbo());
```

So **`refreshModeIndex` overrides `updateMode` and `turbo` completely**. Writing
`{refreshModeIndex:"refresh_mode_2", updateMode:2, turbo:5}` as we do is self-consistent but the last
two are decorative.

**The profile table for this device** (from `noteair4c_systemui.json` — there is no five-profile table
here; that is the generic `systemui.json` shape, and even there it is four):

| index                              | `mode`            | `turbo` | `title`      | `toEpdMode`               | offered to third-party apps?                                                      |
| ---------------------------------- | ----------------- | ------- | ------------ | ------------------------- | --------------------------------------------------------------------------------- |
| `refresh_mode_1`                   | 5 (`REGAL_PLUS`)  | 0       | `REGAL_PLUS` | 9                         | yes (`refreshConfig[0]`)                                                          |
| `refresh_mode_2`                   | 2 (`A2`)          | **5**   | `NEW_SPEED`  | 2308 (`ANIM\|DITHER\|Y1`) | yes (`refreshConfig[1]`) — **what we currently write**                            |
| `refresh_mode_4`                   | 0 (`NORMAL/AUTO`) | 0       | `HD`         | 5 (`AUTO`)                | listed only under `refreshConfigForSystemApp`, **but accepted on the write path** |
| `refresh_mode_3`, `refresh_mode_5` | —                 | —       | —            | —                         | **do not exist on this device**                                                   |

`refresh_mode_4` is accepted for us because `ValidateAppConfigAction.validateRefreshConfig`
(`action/ValidateAppConfigAction.java:167-186`) only asks whether the index resolves in
`refreshConfigMap`; the `refreshConfig` / `refreshConfigForSystemApp` arrays are the **EinkWise UI
picker lists**, consulted nowhere on the apply path. The extra clamp at `:170-172` (`isOnyxOrSystemApp
&& isRegal(mode) && !supportRegal → changeToAutoMode`) applies only to Onyx/system packages, not to us.
**[inference: therefore a third-party app can select HD by writing `refresh_mode_4`. Untested on
device.]**

**Why `EpdController.setAppScopeRefreshMode(FAST)` does nothing.** It reaches
`TabletEACRefreshImpl.setAppScopeRefreshMode` (`impl/TabletEACRefreshImpl.java:263-276`), which does
`appConfig.setUpdateMode(cn, i)` — it writes **`updateMode` only, never `refreshModeIndex`** — and then
applies through `setAppRefreshModeImpl` → `caculateRefreshConfig`, which prefers the index. The
readback `getAppScopeRefreshMode` (`:210-215`) is likewise `caculateRefreshConfig(...).getMode()`. So
once any `refreshModeIndex` is stored, the runtime call is write-only-to-a-field-nobody-reads.
A readback of `NORMAL` (0) is what you get when the resolved config has no valid index and
`updateMode == 0` — e.g. the app is not `supportEAC`/`enable`, in which case
`getRefreshConfigByCurrentComponent` returns a bare `new EACRefreshConfig()`
(`impl/EACBaseRefreshImpl.java:134-140`). Note also the silent drop when `getCurrentTopComponent()`
is null (`OECService.java:1483-1485`).

**What the resolved mode then drives** (`impl/TabletEACRefreshImpl.java:43-121`):

- mode ∈ {0, 3, 5} → `clearAppScopeUpdate(flag)` — i.e. **no app-scope override; the panel runs on
  AUTO** and the SF debouncer does the quality work.
- mode ∈ {1, 2, 4} → `clearSFDebouncer()` **then** `applyAppScopeUpdate(pkg, true, flag, toEpdMode(mode), Integer.MAX_VALUE)`.

### 3.4 `displayConfig` — `EACDisplayConfig` (`data/v2/EACDisplayConfig.java:7-14`)

| Field                   | Default here                                                                                                                                                     | Applied on this (CFA) device?                                                                                                                                |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `enable`                | inherited                                                                                                                                                        |                                                                                                                                                              |
| `contrast` (gamma)      | `EACConfig.getContrastDefault()` = **30**; range 0..100                                                                                                          | **yes** — `applyGammaCorrection(v>0, v)` (`impl/EACBaseDisplayImpl.java:60-63`)                                                                              |
| `monoLevel`             | `10`; range 0..175, `+80` offset applied                                                                                                                         | **no** — CFA branch skips it (`impl/EACBaseDisplayImpl.java:38-45`)                                                                                          |
| `cfaColorSaturation`    | `EACConfig.getCfaSaturationDefault()` = **0** here (`noteair4c_eac.json`), though `Constant.CFA_COLOR_SATURATION_DEFAULT` would be 65; range 0..100              | **yes**                                                                                                                                                      |
| `cfaColorSaturationMin` | `EACConfig.getSaturationMinValue()` = **60**                                                                                                                     | **yes**                                                                                                                                                      |
| `cfaColorBrightness`    | `0`; range 0..5                                                                                                                                                  | **yes**                                                                                                                                                      |
| `ditherThreshold`       | `EACConfig.getDitherThreshold()` = **255**, but `ValidateAppConfigAction.validateDitherThreshold` (`:147-153`) **forces anything that is not 128 or 180 to 180** | **yes**, and `ViewUpdateHelper.setDitherThreshold` drops `<128`. Effective values: `DITHER_NORMAL 128` or `DITHER_HIGH_CONTRAST 180` (`Constant.java:82-84`) |
| `bwMode`                | `0` (`VIEW_UPDATE_BW_MODE_DISABLE`)                                                                                                                              | **yes** — CFA-only                                                                                                                                           |
| `enhance`               | `true`                                                                                                                                                           | **yes** — `enableEnhance`; forced `true` for Onyx/system apps (`ValidateAppConfigAction.java:155-165`)                                                       |

Apply order is fixed in `EACBaseDisplayImpl.applyDisplayConfig` (`:36-48`): gamma → (CFA: brightness,
saturationMin, saturation, bwMode) → ditherThreshold → enhance.

### 3.5 `paintConfig` — `EACPaintConfig` (`data/v2/EACPaintConfig.java:5-18`)

`textBold=false`, `textEACType=0`, `antiAlisingType=0`, `fillEAC=false`, `fillContrast=0`,
`fillBrightness=0`, `iconEAC=false`, `iconContrast=0`, `iconBrightness=0`, `iconThreshold=0f`,
`imgEAC=false`, `imgGamma=0`, **`ditherBitmap=false`**, `quantBits=3`.

`ditherBitmap` gates `ViewUpdateHelper.enableDither(...)` — but only conditionally:

```java
// impl/TabletEACRefreshImpl.java:72-79
boolean dither = appCfg.fuzzyMatchActivityConfig(cls).getPaintConfig().isDitherBitmap();
if (!Constant.DEBOUNCER_UPDATE_MODE_MAP.containsKey(appCfg.getRefreshConfig(cn).getUpdateMode())
    || !appCfg.isEnable()) dither = false;
ViewUpdateHelper.enableDither(dither);
```

`DEBOUNCER_UPDATE_MODE_MAP` is `{3→0, 5→0, 0→0}` (`Constant.java:391-397`). Note this test uses the
**raw `updateMode` field**, not the resolved index — a genuine inconsistency with everything else.

### 3.6 `noteConfig` — `EACNoteConfig` (`data/v2/EACNoteConfig.java:18-23`)

`enable=false`, `supportNoteConfig=false`, `drawViewKey=null`, `styleMap={}`,
**`repaintLatency=500`**, `globalStrokeStyle=EACStrokeStyle(enable=true)`, `compatibleVersionCode=0`.
This is the _system-driven_ screen-note path (`optimization/screennote/handler/BaseHandler.java`),
which we do not use — we drive the pen ourselves through the SDK's `TouchHelper`.

### 3.7 The SF debouncer — the firmware's post-input quality repaint

This is the piece that actually governs "caret, finger touches, scrolling" quality, and it is
**switched off by the profile we currently write**.

- `EACBaseRefreshImpl.handleInputEventImpl` (`:25-45`): on every `MotionEvent`/`KeyEvent`
  **action UP**, calls `increaseRepaintCount(cfg)` then
  `EACUtils.applyDebouncerTransientUpdateMode(pkg, resolvedMode)`.
- `EACUtils.applyDebouncerTransientUpdateMode` (`EACUtils.java:26-32`): **only fires if the resolved
  mode is a key of `DEBOUNCER_UPDATE_MODE_MAP = {3,5,0}`**; then `enableRegal(true)` +
  `setDebouncerTransientUpdateMode(toEpdMode(mode))`.
- `EACUtils.applySFDebouncer` (`:34-41`): same gate; calls
  `ViewUpdateHelper.debouncer(enable, toEpdMode(map.get(mode)), shortDelay, longDelay, gcInterval)` —
  and `map.get(mode)` is **always 0**, so the debounced repaint mode is always `toEpdMode(0) = 5` (AUTO).
  `shortDelay = max(minAnimationDuration=80, animationDuration||10)`,
  `longDelay = clamp(shortDelay*3, 400, 1200)` (`EACUtils.java:57-59`), `gcInterval` from the config.
- `increaseRepaintCount` (`impl/EACBaseRefreshImpl.java:47-52`) → `ViewUpdateHelper.debounceIncRefresh()`,
  again only for {0,3,5}: this is the counter that trips a full GC every `gcInterval` inputs.

**Consequence:** with `refresh_mode_2` (resolved mode 2 = A2), _all four_ of these are inert, and
`TabletEACRefreshImpl.applyAppRefreshModeImpl` explicitly calls `clearSFDebouncer()` before applying
the A2 app-scope override (`:51-56`). We have traded away the firmware's automatic
`gcInterval`-counted GC and its AUTO-mode settle repaint, and `enableDither` is forced false.
That is a complete mechanism for "accumulated ghosting during ordinary UI use".

### 3.8 Scrolling

`android/webkit/WebView.java:388` calls `beforeScroll(getViewUUID(), motionEvent)` →
`View.beforeScrollImpl` (`android/view/View.java:4236-4238`) → `EInkHelper.beforeScroll` →
`ScrollHelper`. So a WebView's scrolling already goes through this path with no work from us.

`ScrollHelper` (`optimization/ScrollHelper.java`):

- Entry is **skipped entirely** when `EACUtils.isFastMode(EInkHelper.getAppScopeRefreshMode())` — i.e.
  when the resolved mode is 1, 2 or 4 (`:93-95`, `EACUtils.java:68-70`). With `refresh_mode_2` we are
  always "already fast", so the scroll helper never runs.
- Otherwise, on scroll start: `scrollingRefreshMode ∈ {2,4}` → `applyTransientUpdate(toEpdMode(mode))` (`:57-66`).
- On scroll end (after `Settings.Global.SCROLL_REFRESH_DELAY`, default 1000 ms, `:85-87`):
  `scrollingRefreshMode == 0` **and `gcAfterScrolling`** → `applyGCOnce()` + `repaintEverything()`;
  `∈ {2,4}` → `clearTransientUpdate(gcAfterScrolling)` (`:68-83`).
- `gcAfterScrolling` default is **false** (`Constant.GC_AFTER_SCROLLING_DEFAULT`) and is settable via
  `EInkHelper.setIsGCAfterScrolling(boolean)`; `scrollingRefreshMode` via
  `EInkHelper.setScrollingRefreshMode(int)`. Both persist in `EACDeviceExtraConfig`
  (`data/v2/EACDeviceExtraConfig.java:28,31`).

### 3.9 The EinkWise (`com.onyx`) side, and why our profile write changes nothing

EinkWise's own defaults live in a **second** asset family, inside `kcb.apk`, not in `framework-res.apk`:
`kcb.apk!/res/raw/eac_<model>.json` → `EACConstantDeviceConfig` / `PresetEACConfig` / `EACViewDeviceConfig`
(`/home/okhsunrog/tmp_zfs/onyx_framework/out-kcb/sources/com/onyx/android/sdk/eac/data/EACConstantDeviceConfig.java:16,118-213`,
`.../PresetEACConfig.java:39`), with `eac_default.json` as fallback and an unconditional `eac_force.json` overlay.

`kcb.apk!/res/raw/eac_noteair4c.json`, top-level scalars (verbatim):

```
jsonVersion=109, dpiDefault=0, dpiStepSize=20,
updateModeDefault=2, turboDefault=2,
useGCForNewSurface=false, antiFlickerDefault=10,
eacRefreshType="TABLET_COLOR", cfaColorModeDefault="vivid",
contrastDefault=30, cfaSaturationDefault=50, cfaSaturationMinValue=60, cfaBrightnessDefault=0,
refreshModeIndexDefault="refresh_mode_2",
refreshModeIndexForOnyxOrSystemDefault="refresh_mode_4"
```

**`refreshModeIndexDefault` for a third-party app on this device is already `refresh_mode_2`.**
All three entries of `themeConfigs` (the third-party themes) use `refreshModeIndex: "refresh_mode_2"`;
`themeConfigsForOnyxOrSystem` uses `refresh_mode_4 / refresh_mode_2 / refresh_mode_4`. So our
`ensureSpeedRefreshProfile()` almost certainly writes the value that was already there — it is not a
change, it is a re-assertion of the factory default. Whatever the panel is doing today, "Speed" is what
it does for _every_ third-party app out of the box.

Two further mechanics from this side:

- **`ApplyThemesAction` reconciles the two fields the other way.**
  `/home/okhsunrog/tmp_zfs/onyx_framework/out/sources/android/onyx/optimization/action/ApplyThemesAction.java:46-56`
  does `cfg.setUpdateMode(byIndex.getMode()); cfg.setTurbo(byIndex.getTurbo());` — so when a theme is
  applied, `updateMode`/`turbo` are overwritten _from_ the index. Our decorative `updateMode:2/turbo:5`
  is therefore also what the system would have written; but if we switch the index we must switch these
  two as well, or the next `applyDither` (which reads the raw field) stays wrong until a theme apply.
- **`res/raw/refresh_mapping.json` is `{}`** — so `RefreshMappingConfig` / `refreshModeAlias`
  (`android/onyx/RefreshMappingConfig.java`) is a dead mechanism on this build. Removing
  `refreshModeAlias` from our JSON, as we do, is harmless but also unnecessary.

Per-package presets shipped in `eac_noteair4c.json` (35 packages), representative entries verbatim:

```json
"com.onyx.gallery":     { "globalActivityConfig": { "refreshConfig": {
                            "enable": true, "supportRegal": true,
                            "refreshModeIndex": "refresh_mode_1" } } }
"org.chromium.chrome":  { "globalActivityConfig": {
                            "paintConfig":   { "enable": false },
                            "refreshConfig": { "animationDuration": 50, "updateMode": 2 } } }
"com.tencent.weread.eink": { "globalActivityConfig": {
                            "refreshConfig": { "updateMode": 0, "turbo": 0 },
                            "displayConfig": { "cfaColorSaturationMin": 0 } } }
"com.bilibili.comic":   { "refreshConfig": { "updateMode": 3, "turbo": 0 } }   // REGAL -> migrated to 5
"com.onyx.aiassistant": { "refreshModeIndex": "refresh_mode_3" }               // absent on this device
```

The last one is instructive: Onyx ships a package pinned to `refresh_mode_3`, which **does not exist in
`noteair4c_systemui.json`**, so `getRefreshConfigByIndex` returns null and it silently falls through to
the raw `updateMode`/`turbo` pair. That is the failure mode for any index we invent.

**Privilege on the apply path: none found.** EinkWise itself uses exactly the reflection route we use —
`out-kcb/.../api/device/eac/EACReflectUtils.java:44-75` caches `EInkHelper.getAppConfigFromService(List)`
and `applyAppConfigToService(List, Bundle)`, and `out-kcb/.../eac/rx/RxEACManager.java:74,90,201` invokes
those handles; `ChangeAppUpdateModeAction` (`out-kcb/.../eac/rx/action/ChangeAppUpdateModeAction.java:27-46`)
is the canonical read-modify-write. `oec_service` is registered with `allowIsolated = true`
(`/home/okhsunrog/tmp_zfs/onyx_framework/out-services/sources/com/android/server/SystemServer.java:1180-1181`)
and `OECService.applyAppConfigToService` / `applyRefreshConfigToAll` / `getAppConfigFromService` contain no
caller check. The only gates are hidden-API access and SELinux — and our app already reads and writes the
config successfully, which settles it empirically for this device.

---

## 4. Colour-panel specifics

### 4.1 `libonyx_cfa.so` is not what its name suggests

`/home/okhsunrog/tmp_zfs/onyx_framework/native/libonyx_cfa.so` (41 KB, stripped) exports exactly **one**
JNI entry point:

```
Java_com_onyx_android_libsetting_util_QRCodeUtil_toRgbwBitmap
```

— owned by `com.onyx.android.libsetting.util.QRCodeUtil`, i.e. the Settings app. It takes two
`RGBA_8888` bitmaps and expands each source pixel into a **2×2 RGBW quad** in the panel's
colour-filter-array subpixel domain:

```
dst[y][x] = gray(R)    dst[y][x+1] = gray(G)
dst[y+1][x] = gray(W)  dst[y+1][x+1] = gray(B)
   where W = (299*r + 587*g + 114*b) / 1000     // BT.601 luma
```

It exists so QR codes render scannably on the Kaleido filter array. **There is no saturation,
brightness, gamma, grayscale-mode or colour-matrix API in it, and it is not on the display path.**
All the colour knobs are `ViewUpdateHelper` SurfaceFlinger transactions (§2.1); the actual CFA
processing lives inside SurfaceFlinger, which is not in this dump.

### 4.2 `libonyx_neo_dither.so` is an app-callable software dither

JNI entry points `Java_android_onyx_utils_DitherUtils_dither` and `…_ditherColor`, owned by
**`android.onyx.utils.DitherUtils`** (`/home/okhsunrog/tmp_zfs/onyx_framework/out/sources/android/onyx/utils/DitherUtils.java`):

```java
public static boolean ditherBitmap(Bitmap b, int quantBits, List<RectF> regions)
public static boolean ditherColorBitmap(Bitmap b, int quantBits, List<RectF> regions)
```

- `quantBits` must be **1..4**; `quant_step = 255 / ((1 << quantBits) - 1)` → 255 / 85 / 36 / 17.
- `ditherColor` is a real per-channel **Floyd–Steinberg** (7/16, 5/16, 3/16, 1/16), with two guards:
  no diffusion when a channel is within 6 of 0 or 255, and **the pixel is skipped entirely when the
  source is near-neutral (`max−min ≤ 5`) but the quantised triple is not (`max−min ≥ 6`)** — an explicit
  colour-fringing suppressor for grey content on a CFA panel.
- `dither` (mono) is _not_ Floyd–Steinberg: gray = `(11R + 16G + 5B) >> 5`, a 2-tap half/half
  diffusion, and the error term is zeroed unless gray ∈ [238, 249] — it is a near-white background
  cleaner, not a general dither.
- Formats: `RGBA_8888` and `RGB_565` only.

This is in-process, needs no service, and is a real option for pre-dithering ink/UI bitmaps ourselves.

### 4.3 The app-reachable colour-vs-mono knobs

| Knob                                                                                   | Reach                                                                            | Effect on this panel                                                                                                                                                                                   |
| -------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `setGrayscaleMode(int)` (SF `16711778`)                                                | `ViewUpdateHelper` only, **no SDK wrapper**                                      | `CFA_GRAY_SCALE_MODE_COLOR = 0`, `CFA_GRAY_SCALE_MODE_BW_ONLY = 1` (`Constant.java:44-45`). The literal "render colour or mono" switch                                                                 |
| `setBWMode(int)` / `enableBWMode(bool)` (SF `1048696`)                                 | `ViewUpdateHelper` (framework-only) or persisted via `EInkHelper.setBwMode(int)` | `ViewUpdateHelper.setBWMode` is **gated on `getColorType() > 0`** (`:1101-1108`) — passes here. The persisted form is mirrored to `Settings.Global["view_update_bw_mode"]` (`OECService.java:224-228`) |
| `applySaturation(0..100)` / `applySaturationMinValue` / `applySaturationFactor(float)` | SF, and persisted via `EInkHelper.setCFAColorSaturation`                         | Defaults here: saturation **0**, min **60** (`noteair4c_eac.json`)                                                                                                                                     |
| `applyBrightness(0..5)`                                                                | SF / `EInkHelper.setCFAColorBrightness`                                          | default 0                                                                                                                                                                                              |
| `applyGammaCorrection(v>0, v)` a.k.a. contrast                                         | SF / `EInkHelper.setGlobalContrast`                                              | default 30                                                                                                                                                                                             |
| `applyMonoLevel(int)`                                                                  | SF                                                                               | **inert on this device** — `applyDisplayConfig` never calls it on a CFA panel (`impl/EACBaseDisplayImpl.java:38-45`)                                                                                   |
| `enableColorCU` / `enableColorAdjust` / `enableEnhance` / `setEnhanceStrategy`         | SF                                                                               | `enhanceDelta:90`, `enhanceThreshold:2` on this device                                                                                                                                                 |
| `enableDither(bool)` / `setDitherThreshold(128\|180)`                                  | SF                                                                               | see §3.5 — EinkWise re-forces dither **off** for A2/DU/X profiles on every resume                                                                                                                      |

### 4.4 What colour costs

No refresh-time table exists in any of the shipped `.so` files (a word-boundary scan for `GC16 GL16
GLR16 GLD16 GCC16 DU4 A2 REGAL HLC mxcfb EPDC waveform kaleido turbo` across all nine libraries in
`/home/okhsunrog/tmp_zfs/onyx_framework/native/` returned nothing). The concrete cost evidence is:

- **`Constant.java:390`: `DEFAULT_USE_GC_FOR_NEW_SURFACE = DeviceController.isCfaDevice();`** — colour
  devices default to a **full GC flash on every new surface**; mono devices do not. That is the single
  most explicit "colour costs more" statement in the whole dump, and it is why dialogs, sheets and
  popups flash on this device.
- The colour dither path is ~3× the memory traffic and ~4× the arithmetic per pixel of the mono path
  (3 error planes vs 1, 4-tap vs 2-tap, plus a per-pixel chroma-spread test), scalar, no NEON.
- The CFA subpixel domain is 4 physical cells per logical pixel.
- `UI_GCC_MODE = 107` (`GCC16 | FULL | WAIT`) exists alongside `UI_GC_MODE = 98` (`GC16 | FULL | WAIT`)
  — a colour-specific full flash. **[inference: GCC16 is the Kaleido colour full-update waveform and
  is slower than GC16; untested.]**

---

## 5. What we currently do that the evidence says is pointless

Referring to
`/home/okhsunrog/code/rust/notes-rs/plugins/tauri-plugin-mobile-system/android/src/main/java/dev/okhsunrog/mobile_system/`.

0. **`ensureSpeedRefreshProfile()` writes the value that was already there.**
   `kcb.apk!/res/raw/eac_noteair4c.json` sets `refreshModeIndexDefault: "refresh_mode_2"` and all three
   third-party themes use `refresh_mode_2` (§3.9). Speed is this device's _factory_ profile for every
   non-Onyx app. The write is idempotent — our own log line even says "already Speed" — so it has never
   changed anything. It becomes useful only if we point it at a _different_ index.
1. **`applyAppRefreshProfile()` → `EpdController.setAppScopeRefreshMode(UpdateOption.FAST)`**
   (`MobileSystemPlugin.kt:294-322`) — dead. `UpdateOption.FAST` → EAC mode 2, which
   `TabletEACRefreshImpl.setAppScopeRefreshMode` stores in `updateMode`, which
   `caculateRefreshConfig` then discards because we also store `refreshModeIndex`
   (`impl/EACBaseRefreshImpl.java:67-78`). The readback `getAppScopeRefreshMode()` goes through the
   same resolver, so it can never report what we wrote. Delete it, or delete the `refreshModeIndex`
   write — not both.
2. **The `Class.forName("android.onyx.optimization.EInkHelper").getMethod("setAppScopeRefreshMode", int)`
   probe** (`MobileSystemPlugin.kt:303-310`) — the hazard it guards against cannot occur here.
   `EInkHelper.setAppScopeRefreshMode(int)` exists (`optimization/EInkHelper.java:1469`), so the probe
   always passes; and the SDK's fallback target `EInkHelper.setSystemRefreshMode(int)`
   **does not exist on this firmware**, so `EACReflectUtils.sMethodSetSystemRefreshMode` is `null` and
   the fallback would be a silent no-op rather than a device-wide change.
3. **`view.set(BASE, UpdateMode.REGAL)` with readback verification** (`MobileSystemPlugin.kt:245-251`).
   `supportRegal()` is false (SF `16711708`), `UI_REGAL_MODE = 6` is a waveform the panel does not run,
   and the verification is unsound anyway: `EpdController.getViewDefaultUpdateMode` returns
   `UpdateMode.GU` when the _reflective read fails_, so "accepted" and "could not read" are the same
   answer. Either drop the BASE layer entirely (leaving `View.defaultUpdateMode` at its initial
   `5 = AUTO`, `android/view/View.java:2321`) or set `GU` (2) straight away.
4. **Writing `updateMode:2` and `turbo:5` next to `refreshModeIndex:"refresh_mode_2"`**
   (`MobileSystemPlugin.kt:283-285`). The index wins for every consumer _except_ one: the raw
   `updateMode` field is what `TabletEACRefreshImpl.applyDither` tests
   (`impl/TabletEACRefreshImpl.java:72-79`), and `2 ∉ {0,3,5}` forces `enableDither(false)` on every
   resume. So the decorative field is the field that turns off dithering.
5. **The comment "Speed: partial GU updates with turbo"** (`MobileSystemPlugin.kt:461`) is wrong on
   this device: `refresh_mode_2` here is `{mode: 2, turbo: 5}` = `UPDATE_MODE_A2` → `UI_A2_QUALITY_MODE`
   (2308, `ANIM | DITHER | Y1`), not GU. The GU profile is `refresh_mode_4` / mode 0 (AUTO).
6. **The DU experiment could not have tested DU.** With no valid `refreshModeIndex`,
   `refreshMigrateMap` on this device maps `1 → 2` (`noteair4c_systemui.json`), i.e. **DU becomes A2**;
   with a valid index, `updateMode` is ignored entirely. The only way to actually get
   `UI_DU_QUALITY_MODE` (2305) is a direct `ViewUpdateHelper.applyAppScopeUpdate(pkg, true, 1, 2305, …)`
   or `View.setDefaultUpdateMode(2305)`.
7. **Trusting SDK return values.** `EpdController.setViewDefaultUpdateMode`, `resetViewUpdateMode`,
   `setDisplayScheme`, `setAppScopeRefreshMode`, `applyAppScopeUpdate` all return a hardcoded `true`;
   `SDMDevice`'s booleans mean "reflection dispatched", never "the panel did something". Worst case:
   `applySysScopeUpdate` returns `true` from a framework body that is literally `{ }`.
8. **`applyMonoLevel` / `monoLevel` are inert** — `EACBaseDisplayImpl.applyDisplayConfig` takes the CFA
   branch on this panel and never calls it (`impl/EACBaseDisplayImpl.java:38-45`). Same for the
   `monoLevel` field in the EAC display config.
9. **Never call `applySystemFastMode(true)`, `switchToA2Mode()` or `toggleA2Mode()`.** They are
   `EInkHelper.setAppScopeRefreshMode(2)` in disguise (`ViewUpdateHelper.java:1352-1366`) — a
   **persisted** change to our app's EinkWise profile — and the "off" path
   (`clearSysScopeUpdate`) is the empty method, so there is no undo.

---

# Recommendations

Two structural facts drive everything below.

- **The stored `refreshModeIndex` is the master switch.** On this device it selects one of exactly
  three entries in `noteair4c_systemui.json`, and its resolved mode decides whether the firmware's
  post-input quality repaint, its counted auto-GC, its dithering and its scroll handling run **at all**
  (`Constant.DEBOUNCER_UPDATE_MODE_MAP = {0,3,5}`). `refresh_mode_2` (A2) switches all four off.
- **Everything on the SurfaceFlinger channel is transient and gets overwritten by EinkWise on the
  next activity resume**; everything on the `oec_service` channel is persisted per package and
  survives our process and a reboot. Use SF for "for the next few seconds", EAC for "how this app
  behaves".

## (a) A handwriting session

The panel work during a stroke is done by the firmware pen path, not by us; our job is the
hand-over and the reconcile. Keep what already matches stock, and stop fighting the profile.

```
on session start (pen editor opened):
  1. view.set(SESSION, UpdateMode.GU)                    // UI_GU_MODE = 2, GC16 partial
                                                          //  process-local, cannot leak
  2. ViewUpdateHelper.setEpdTurbo(5)                      // SF 1048661, transient; gives the
                                                          //  writing speed the Speed profile
                                                          //  used to give, without changing the
                                                          //  stored profile. EinkWise restores
                                                          //  the profile's turbo on next resume.
  3. do NOT touch the EAC profile here — it is persisted and app-wide.

per reconcile frame (already correct, keep):
  4. view.set(TRANSIENT, UpdateMode.HAND_WRITING_REPAINT_MODE)   // 524290 = UI_GU_MODE | 524288
  5. draw one ordinary frame
  6. view.clear(TRANSIENT) once that frame is on screen

selection / lasso drag only (already correct, keep):
  7. EpdController.applyTransientUpdate(ANIMATION_QUALITY)       // SF 16711782, mode 2308
  8. clearTransientUpdate(false) on drag end, with a 5 s tail

on session end:
  9. ViewUpdateHelper.setEpdTurbo(0)
 10. view.clear(SESSION)
 11. one counted GC — see (c)
```

Reasons. `HAND_WRITING_REPAINT_MODE` is `EPDC_FLAG_HANDWRITE_GU | GC16`, i.e. exactly the partial
GC16 that replaces the pen overlay; it is the same primitive the system's own screen-note handler
fires 500 ms after stylus-up (`optimization/screennote/handler/BaseHandler.java:268-283`). Turbo is
SF-global and transient, so it is the one speed knob that does not persist. Nothing here needs the
EAC profile to be Speed.

**Also adopt the render-flag cycle** described in
`/home/okhsunrog/code/rust/notes-rs/docs/planning/handwriting-pen-stack-comparison.md` §11 — stock,
notable, saber, PngNote and mokke all tear down and rebuild the firmware scribble state around every
menu/tool/refresh event, and we only do it on a named pause. That is a pen-stack change, not a
refresh lever, but it is the single most-attested de-ghosting behaviour in the corpus.

## (b) Ordinary UI use — scrolling, typing, dialogs

Change the stored profile away from Speed. This is the one recommendation with real leverage.

```
once, in ensureSpeedRefreshProfile() (rename it):
  globalActivityConfig.refreshConfig = {
      enable:            true,
      refreshModeIndex:  "refresh_mode_4",   // {mode:0 NORMAL/AUTO, turbo:0} on this device
      updateMode:        0,                  // keep consistent — applyDither reads THIS field
      turbo:             0,
      gcInterval:        20,                 // firmware auto-GC every 20 debounced inputs
      antiFlicker:       10,                 // device default; supportAntiFlicker:true here
      animationDuration: 0,                  // -> max(minAnimationDuration=80, 10) = 80 ms settle
      useGCForNewSurface: true               // device default on a CFA panel anyway
  }
  globalActivityConfig.paintConfig.ditherBitmap = true   // enableDither now actually sticks
  remove "refreshModeAlias"

optionally, via EInkHelper (persisted, foreground-scoped):
  EInkHelper.setScrollingRefreshMode(2)     // A2 while a scroll is in flight
  EInkHelper.setIsGCAfterScrolling(true)    // clearTransientUpdate(true) -> GC when it ends
  // scrollRefreshDelay is clamped to [1000, 5000] ms (EACActivityConfig.java:7,49-51)
```

What this buys, mechanically:

- `caculateRefreshConfig` now resolves to **mode 0**, which is a key of `DEBOUNCER_UPDATE_MODE_MAP`.
  That re-enables, on every touch-up and key-up:
  `debouncer(true, toEpdMode(0)=5 /*AUTO*/, 80 ms, 400 ms, gcInterval)` and
  `setDebouncerTransientUpdateMode(5)` and `enableRegal(true)` and `debounceIncRefresh()`
  (`EACUtils.java:26-41`, `impl/EACBaseRefreshImpl.java:25-52`). That _is_ the firmware's caret /
  finger / scroll cleanup, and it is currently off.
- `applyAppRefreshModeImpl` takes the `{0,3,5}` branch → `clearAppScopeUpdate()` instead of pinning
  the app to A2 (`impl/TabletEACRefreshImpl.java:45-60`), so the EPDC picks per-region waveforms
  again (AUTO).
- `applyDither` no longer forces `enableDither(false)` (`impl/TabletEACRefreshImpl.java:72-79`).
- `ScrollHelper` stops being skipped: it is gated on `!isFastMode(getAppScopeRefreshMode())`
  (`ScrollHelper.java:93-95`, `EACUtils.java:68-70`), and mode 0 is not fast.

**Durability caveat.** `refresh_mode_2` is the device's factory default for third-party apps (§3.9), so
our stored override is fighting a default that the theme machinery re-asserts: `ApplyThemesAction`
(`optimization/action/ApplyThemesAction.java:46-56`) rewrites `updateMode`/`turbo` from the index, and
`EACAppThemeManager.loadDefaultTheme(...)` supplies `refresh_mode_2` when a config is rebuilt from a
theme. Re-assert our profile on every `setDisplayProfile` (we already do) and verify it after any visit
to EinkWise.

`refresh_mode_1` (`{mode:5, turbo:0}`, REGAL*PLUS) is the other candidate — it is also in
`DEBOUNCER_UPDATE_MODE_MAP`, and it is what EinkWise offers third-party apps as the \_quality* choice.
On a `supportRegal:false` panel it asks the EPDC for waveform 9, whose behaviour here is not attested.
**Prefer `refresh_mode_4` (AUTO); test `refresh_mode_1` as the alternative.** If either turns out to be
too flashy in the editor, the right response is a _transient_ SF override during the editor
(`applyTransientUpdate` / `View.setDefaultUpdateMode`), not a return to the Speed profile.

Also apply, once, on the display side (persisted, `EInkHelper.*`, or as part of the same JSON write to
`globalActivityConfig.displayConfig`):

```
ditherThreshold: 180        // DITHER_HIGH_CONTRAST; anything not 128 or 180 is rewritten to 180
                            //  by ValidateAppConfigAction:147-153, and <128 is dropped by
                            //  ViewUpdateHelper:1139
enhance:         true
contrast:        30         // device default gamma
```

## (c) Recovering the panel from accumulated ghosting

Three tiers, cheapest first. All are stateless one-shots on the SF channel and need no privilege.

```
tier 1 — our window only, what we already do:
    EpdController.invalidate(webView, UpdateMode.GC)      // UI_GC_MODE 98 = GC16|FULL|WAIT

tier 2 — whole screen, what the firmware itself does after a scroll:
    ViewUpdateHelper.applyGCOnce()                        // SF 16711718
    ViewUpdateHelper.repaintEverything()                  // SF 16711700
    // exactly ScrollHelper.exitScrollRefreshModeImpl():71-76 when gcAfterScrolling is set

tier 3 — strongest, for a visibly stained panel:
    ViewUpdateHelper.repaintEverything(108)               // SF 16711715, UI_DEEP_GC_MODE
    // or 107 (UI_GCC_MODE, the colour full flash) — untested here
    // or ViewUpdateHelper.whiteClear(2, true)            // SF 1048663; what the EAC layer uses
    //                                                    //  for INVALID_MODE (TabletEACRefreshImpl:106)
```

And make it periodic rather than only on demand. Two independent counters exist:

- **Firmware-side:** `refreshConfig.gcInterval` (default 20), consumed by
  `debouncer(..., gcInterval)` and ticked by `debounceIncRefresh()` — but **only when the resolved
  mode is 0/3/5**, so it is dead today and comes back for free with recommendation (b).
- **App-side:** `EpdDeviceManager.setGcInterval(n)` + `applyWithGCInterval(view, false)`
  (`/home/okhsunrog/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-device-1.3.5.2/sources/com/onyx/android/sdk/api/device/EpdDeviceManager.java:96-118,157-177`).
  Pure SDK bookkeeping; since `supportRegal()` is false it takes the `WithoutRegal` branch —
  `resetUpdate(view)` for n−1 calls, then `applyGCUpdate(view)`. This is stock's
  `applyGCWithInterval`, bound to page turns and menu actions. Wire it to page turns, sheet closes and
  navigation — **not** to strokes.

There is also `ViewUpdateHelper.setGcRefreshInterval(int)` (SF `1048658`): a SurfaceFlinger-side GC
counter that **nothing in the framework ever calls**. It is app-only, undocumented, and worth one
experiment.

---

# On-device experiments

Each is one testable action.

1. Write `refreshModeIndex:"refresh_mode_4"` (with `updateMode:0, turbo:0`) into our app's EAC config,
   restart the activity, and read the config back — confirm `ValidateAppConfigAction` did **not**
   rewrite it to `refresh_mode_1`/`refresh_mode_2` (this is the one place the "system-app-only" UI list
   could turn out to be enforced somewhere I did not find).
2. With `refresh_mode_4` stored, `logcat -s TabletEACRefreshImpl EACBaseRefreshImpl EACUtils` while
   typing in a text field — confirm `applySFDeBouncer` and `applyDebouncerTransientUpdateMode` now
   appear on key-up (they must be absent today with `refresh_mode_2`).
3. With `refresh_mode_4` stored, confirm `EACUtils` logs `applyDither, enable: true` on resume after
   also setting `paintConfig.ditherBitmap = true`, and compare grey-text rendering against today.
4. Compare 60 s of ordinary scrolling+typing ghosting under `refresh_mode_4` vs `refresh_mode_1` vs
   today's `refresh_mode_2`, photographed under identical light.
5. Call `ViewUpdateHelper.setEpdTurbo(5)` on ink-session start and `setEpdTurbo(0)` on end, with the
   profile left at `refresh_mode_4`, and read back with `ViewUpdateHelper.getEpdTurbo()` (SF `1048689` —
   there is no SDK wrapper) to confirm the value takes and that EinkWise resets it on the next resume.
6. Set `View.setDefaultUpdateMode(webView, 2305)` (`UI_DU_QUALITY_MODE`) directly — the only way to
   actually reach DU on this device — and see whether real DU is better or worse than the A2 that the
   earlier "DU" experiment silently produced.
7. Drop the `BASE = UpdateMode.REGAL` layer entirely and compare against `BASE = UpdateMode.GU`
   and against no BASE layer at all; log `ViewDisplayMode.readRaw()` in each case to see what the view
   actually holds (`getViewDefaultUpdateMode` cannot tell GU from a failed read).
8. Fire `repaintEverything(107)` (`UI_GCC_MODE`) and `repaintEverything(108)` (`UI_DEEP_GC_MODE`) on a
   deliberately ghosted screen and time both — establishes whether the colour full flash is worth its
   cost on this panel.
9. Call `EInkHelper.setScrollingRefreshMode(2)` + `setIsGCAfterScrolling(true)` with the profile at
   `refresh_mode_4`, then scroll a long page — confirm `ScrollHelper` enters (A2 during the drag) and
   that a GC lands ~1 s after the finger lifts.
10. Sweep `ViewUpdateHelper.setGcRefreshInterval(n)` for n ∈ {0, 5, 20} and watch for any change in
    spontaneous full flashes — the only way to learn what this otherwise-unreferenced knob does.
11. Set `refreshConfig.antiFlicker` to 0 and to 32 (range 0..32, `supportAntiFlicker:true` here) and
    compare A2/AUTO artefacts.
12. Confirm `oec_service` is genuinely reachable from our uid by logging whether
    `EInkHelper.getAppConfigFromService(listOf(pkg))` returns non-empty (it does today, per our own
    `ensureSpeedRefreshProfile`) — this rules out the "every `EInkHelper` call is a silent SELinux
    no-op" explanation for a `NORMAL` readback and leaves only the `refreshModeIndex` one.
13. Pre-dither an ink bitmap with `android.onyx.utils.DitherUtils.ditherColorBitmap(bmp, 2, rects)`
    (Floyd–Steinberg, quantBits 1..4) and compare against the firmware's `enableDither` path.
14. Add the stock render-flag cycle (`setRawDrawingRenderEnabled(false); …(true)`) around menu/tool
    changes during a long writing session and see whether the "rectangle stops taking ink" failure
    disappears.
15. Open EinkWise, change our app's profile by hand and change it back, then re-read our stored
    `refreshModeIndex` — establishes whether a theme apply resets us to the `refresh_mode_2` default
    and therefore whether the profile write must be re-asserted on every resume.
16. Read our app's stored EAC JSON _before_ the first `ensureSpeedRefreshProfile()` call on a fresh
    install — confirms the "already Speed" prediction from `eac_noteair4c.json`
    (`refreshModeIndexDefault: "refresh_mode_2"`).
