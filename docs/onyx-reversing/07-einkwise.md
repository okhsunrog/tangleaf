# EinkWise, end to end — Onyx BOOX Note Air 4C (`lito`, Android 13, 4.2-rel build 8548, Kaleido 3 CFA)

> Written 2026-09-07 against the decompiled `com.onyx` system app (`out-kcb`), `framework.jar`
> (`out`), `services.jar` (`out-services`), the `kcb.apk` and `framework-res.apk` raw resources, and
> `/system/bin/surfaceflinger`. It builds on, and does not repeat,
> `/home/okhsunrog/code/rust/notes-rs/docs/onyx-reversing/05-panel-refresh-levers.md` (the lever
> inventory) and `…/04-system-screennote-pipeline.md` (the note pipeline). Where this report
> contradicts those, the contradiction is called out explicitly.
>
> Statements marked **[inference]** are not directly attested by code.
> All paths are absolute. Line numbers are jadx output line numbers.

**Vocabulary.** "EinkWise" is the user-facing name of what the code calls **EAC** — _E-ink App
Configuration_ / _Optimization_. Nothing in the sources is named "EinkWise"; every symbol is `EAC*`
or `OEC*` (_Onyx E-ink Configuration service_). The two names denote the same feature.

---

## 1. What EinkWise is, structurally

EinkWise is **three cooperating parts in three different processes**, plus a fourth piece that runs
inside _every_ app.

| Part                                        | Process                           | Code root                                                                                 | Role                                                                                                      |
| ------------------------------------------- | --------------------------------- | ----------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| The UI (tabs, sliders, radio groups, reset) | `com.onyx` (the "KCB" system app) | `/home/okhsunrog/tmp_zfs/onyx_framework/out-kcb/sources/com/onyx/android/sdk/eac/**`      | Renders the panel, builds an `EACAppConfig` JSON, hands it to the service by reflection                   |
| The store + the enforcer                    | **`system_server`**               | `/home/okhsunrog/tmp_zfs/onyx_framework/out/sources/android/onyx/optimization/**`         | `OECService` (Binder `oec_service`) owns the config, persists it to MMKV, and pushes it to SurfaceFlinger |
| The panel                                   | **`surfaceflinger`**              | `/home/okhsunrog/tmp_zfs/onyx_framework/native/surfaceflinger`                            | Receives the EPD transactions and decides the waveform per region                                         |
| The in-app agent                            | **every app's own process**       | `android.onyx.optimization.EInkHelper`, `EACConfigChangedImpl`, `android.view.View` hooks | Caches the app's config, reloads WebViews, restarts the package, redraws on config change                 |

`OECService` is registered by `SystemServer`:

```java
// /home/okhsunrog/tmp_zfs/onyx_framework/out-services/sources/com/android/server/SystemServer.java:1178-1183
public final void startOECServices(Context context) {
    OECService oECService = new OECService(context);
    ServiceManager.addService("oec_service", oECService, true);   // allowIsolated = true
    oECService.systemReady();
}
```

It lives in `system_server` (uid 1000), it is not a `SystemService` subclass, and `allowIsolated=true`
means even isolated/sandbox processes may obtain the binder.

### 1.1 One setting, tap to panel

Take **Refresh tab → "A2 (turbo 3)"** on our app.

| #   | Hop                                                                                                                                                                                                                                                              | Evidence                                                                                                                                                                                        |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | User taps the radio button in the EinkWise panel; `EACItemRadioGroupViewModelExt` holds the selection                                                                                                                                                            | `out-kcb/.../eac/model/EACItemRadioGroupViewModelExt.java:10-27`                                                                                                                                |
| 2   | On apply, `EACViewUtils` writes into the in-memory `EACRefreshConfig`: `setUpdateMode(getUpdateModeByType(type))`, `setTurbo(getTurboByType(type))`, then forces `enable=true`. **It never writes `refreshModeIndex`.**                                          | `out-kcb/.../eac/util/EACViewUtils.java:348-362`; `.../eac/data/EACViewConfigs.java:838-889`; `EACViewUtils.java:699-704`                                                                       |
| 3   | The whole `EACAppConfig` is serialised to JSON and pushed through reflection into the framework: `EInkHelper.applyAppConfigToService(List<String>, Bundle)`                                                                                                      | `out-kcb/.../sdk/api/device/eac/EACReflectUtils.java:48` (`sMethodApplyAppConfigToService`)                                                                                                     |
| 4   | `EInkHelper` forwards over Binder                                                                                                                                                                                                                                | `out/sources/android/onyx/optimization/EInkHelper.java:230-236` → `IOECService` tx **12** (`IOECService.java:379`)                                                                              |
| 5   | `OECService.applyAppConfigToServiceImpl` parses it, **replaces** `appConfigMap[pkg]` wholesale, and fans out to every `EACBaseImpl.onApplyConfig(currentTopComponent, old, new, deviceConfig)`                                                                   | `OECService.java:190-221`, `:229-234`                                                                                                                                                           |
| 6   | `TabletEACRefreshImpl.onApplyConfig` runs **only if the config's package equals the foreground component** (`isMatch`), then `applyAppRefreshModeImpl` + `applyEpdParameter`                                                                                     | `impl/TabletEACRefreshImpl.java:229-237`; `impl/EACBaseImpl.java:52-57`                                                                                                                         |
| 7   | `caculateRefreshConfig` resolves the effective `(mode, turbo)`: **`refreshModeIndex` wins over `updateMode`/`turbo` whenever it resolves in `noteair4c_systemui.json`**                                                                                          | `impl/EACBaseRefreshImpl.java:67-79`                                                                                                                                                            |
| 8   | mode ∈ {1,2,4} → `clearSFDebouncer()` then `ViewUpdateHelper.applyAppScopeUpdate(pkg, true, flag, toEpdMode(mode), Integer.MAX_VALUE)`; mode ∈ {0,3,5} → `clearAppScopeUpdate(flag)`                                                                             | `impl/TabletEACRefreshImpl.java:43-62`, `:64-70`                                                                                                                                                |
| 9   | `ViewUpdateHelper` writes a `Parcel` and `transactData(APPLY_APP_SCOPE_UPDATE = 16711684, …)` to SurfaceFlinger. The package identity on the wire is `pkgName.hashCode()` (or `-1` for null)                                                                     | `out/sources/android/onyx/ViewUpdateHelper.java:306-317`                                                                                                                                        |
| 10  | Alongside, `applyEpdParameter` sends `setEpdTurbo`, `antiFlicker`, `enableDither`, and the SF debouncer                                                                                                                                                          | `impl/TabletEACRefreshImpl.java:81-86`; `impl/EACBaseRefreshImpl.java:57-65`, `:107-110`; `EACUtils.java:34-41`                                                                                 |
| 11  | `saveDeviceConfig(cfg, 5, false, mmkvFlag)` then `saveConfigToLocalImpl()` persists the whole device config to MMKV                                                                                                                                              | `OECService.java:210`, `:218`, `:809-811`, `:1689-1698`                                                                                                                                         |
| 12  | `sendOECConfigChanged(pkg, cls, instant, operationFlag)` broadcasts `onyx.action.oec.config.change`                                                                                                                                                              | `OECService.java:1704-1713`                                                                                                                                                                     |
| 13  | `ActivityManagerService` receives it (receiver registered with **no permission**) and calls `WindowProcessController.handleEACAppConfigChanged` → `IApplicationThread` tx 59 → `ActivityThread` → `EACConfigChangedImpl` **inside the target app's process**     | `out-services/.../am/ActivityManagerService.java:14830-14838`, `:4043`; `out-services/.../wm/WindowProcessController.java:730-731`; `out/sources/android/app/ActivityThread.java:1128`, `:1837` |
| 14  | In our process, the operation flag decides: `1` → reload every WebView; `2` → reset (default + WebView reload); `3` → conditional full redraw; `0`/default → DPI dispatch, or **`restartPackage()`** on rotation change, or `dispatchFullRedraw` on paint change | `out/sources/android/onyx/optimization/EACConfigChangedImpl.java:88-103`, `:153-215`, `:291-298`                                                                                                |

So a single EinkWise tap crosses **four processes** and ends by executing code inside the target
app — including, in two of the four branches, a WebView reload or a package restart.

---

## 2. Every control EinkWise exposes on this device

### 2.1 Two panels, and which one you are actually looking at

`kcb.apk` contains a complete EAC panel — a `WindowManager` overlay, not an Activity:
`EACViewUtils` (`out-kcb/.../eac/util/EACViewUtils.java`) is a static singleton holding
`DialogEacBinding`, a `WindowManager.LayoutParams` with `type = 2003` (`TYPE_SYSTEM_ALERT`) and
`flags = 288` (`FLAG_LAYOUT_IN_SCREEN | FLAG_NOT_TOUCH_MODAL`) (`:101-103`, `:857-871`), inflated
from `R.layout.dialog_eac` (`:742-744`) and added by `showEACView(...)` (`:930`, `wm.addView` at
`:945`). It is opened by the exported `EACService` `IntentService`
(`out-kcb/.../common/applications/service/EACService.java:43`, manifest action
`android.intent.action.EAC_SERVICE`) with extra `SERVICE_ACTION = "service_action_show_eac_dialog"`,
via `ShowCurrentTopAppEACViewAction` (`out-kcb/.../common/applications/action/ShowCurrentTopAppEACViewAction.java:45-65`),
and dismissed by `HideEACViewReceiver` on `CLOSE_SYSTEM_DIALOGS` / `onyx.action.hide.eac.dialog.action`.

**That overlay is legacy on this firmware.** Two independent facts say so:
`EACViewConfigs.getTabTypes()` hard-codes only `{DISPLAY, COLOR_EAC, OTHERS, NOTE_EAC}`
(`out-kcb/.../eac/data/EACViewConfigs.java:826-835`) — the refresh tab is not in it — and its write
path terminates in a process-local `HashMap`: `EACAppConfigChangeAction` →
`ApplyAppConfigRequest.execute()` → `EACDataSource.putAppConfigList` → `m(Collection, Bundle)`, which
only does `map.put(pkg, cfg)` (`out-kcb/.../eac/sync/EACDataSource.java:79-86`, `:132-152`); the
`KSyncConfigHelper.pushConfig` branch is skipped because `BaseSourceImpl` keeps sync disabled by
default (`out-kcb/.../sdk/sync/ksync/config/source/BaseSourceImpl.java:12-33`). It never reaches
`EInkHelper`.

The **live** EinkWise panel is in `com.onyx.floatingbutton` (the navigation-ball app), which is _not_
in this firmware dump. Its bridge into `com.onyx` is four broadcasts handled by
`out-kcb/.../common/applications/receiver/EACStatusChangedReceiver.java:362`:

| Action                                             | Handler → `OperationType`      |
| -------------------------------------------------- | ------------------------------ |
| `com.onyx.floatingbutton.setting.eac.item.changed` | `BASE_CONFIG_CHANGED` (`:263`) |
| `com.onyx.floatingbutton.setting.eac.reset`        | `RESET` (`:267`)               |
| `com.onyx.floatingbutton.setting.eac.close`        | `TOGGLE` (`:255`)              |
| `com.onyx.floatingbutton.setting.eac.cloud`        | `FETCH_REMOTE` (`:259`)        |

`EACStatusChangedReceiver.j(Intent, String, OperationType)` (`:222-234`) deserialises **an entire
`EACAppConfig` from the string extra `BroadcastHelper.ARGS_EAC_KEY`** and feeds it to
`EACAppConfigChangeAction`. These receivers carry no permission, so any app can drive them.

What _is_ device-authoritative and shared by both panels is the data below: the tab→control map, the
ranges, and the field wiring. Read the tables as "what EinkWise exposes for this device", not as a
screenshot of a particular dialog.

### 2.1b Where the tab layout comes from

The tab strip and the contents of each tab are **data**, not code: `EACViewDeviceConfig` is loaded by
`ConfigLoader.load(0, EACViewDeviceConfig.class, "eac_noteair4c", "eac_default")`
(`out-kcb/.../eac/data/EACViewDeviceConfig.java:12`), and `EACViewConfigs.getEACItemModelsByTab`
walks `getEacTabToDataMap().get(tabType)` (`out-kcb/.../eac/data/EACViewConfigs.java:660-672`).
Note this loader is `mode 0 = MODEL_CONFIG_FULL_REPLACE_MODE`
(`out-kcb/.../sdk/utils/ConfigLoader.java:27-30`, `:62-64`): the model file wholly replaces the
default file, it is **not** merged. `eac_noteair4c.json` exists, so **`eac_default.json` is never
consulted on this device**; keys absent from the model file keep the Java field initialisers.
(The _framework's_ `android.onyx.config.ConfigLoader` is a different class and _does_ merge
top-level keys — `out/sources/android/onyx/config/ConfigLoader.java:33-63`.)

`eac_noteair4c.json → eacTabToDataMap` (verbatim, from `kcb.apk!/res/raw/eac_noteair4c.json`):

```json
"TAB_TYPE_DISPLAY":       ["DPI","TXT_BOLD","ANTI_ALIASING","GC_FOR_NEW_SURFACE","CONTROL_PANEL"],
"TAB_TYPE_COLOR_EAC":     ["TXT_EAC","FILL_EAC","OTHER_COLOR"],
"TAB_TYPE_REFRESH_TURBO": ["GC_FOR_NEW_SURFACE","REFRESH_MODE_TURBO","GC_INTERVAL","ANIMATION_DURATION"],
"TAB_TYPE_OTHERS":        ["FULL_PM_ACCESS","FORCE_ROTATION","SCROLL_BUTTON","REMOVE_SPLASH_SCREEN"],
"TAB_DETAIL_APP_EAC":     ["TXT_EAC","GAMMA","MONO_LEVEL","ICON_EAC","IMG_EAC","FILL_EAC"],
"TAB_TYPE_NOTE_EAC":      ["PEN_EAC_ENABLE","PEN_EAC_STROKE_WIDTH"],
"TAB_TYPE_APP_DISPLAY":   ["GAMMA","MONO_LEVEL"],
"TAB_TYPE_OTHER_COLOR":   ["ICON_EAC","IMG_EAC","WEB_FONT_COLOR","WEB_FONT_SIZE","WEB_FONT_BOLD"]
```

Ranges, steps and defaults come from `EACConstant` (`out-kcb/.../eac/data/EACConstant.java:100-206`),
which reads them from `EACConstantDeviceConfig` (Java initialisers at `:105-215`, overridden per key
by `eac_noteair4c.json`). Control→field wiring is
`EACViewConfigs.updateModels` (read-back, `:918-1039`) and `EACViewUtils` (write,
`out-kcb/.../eac/util/EACViewUtils.java:348-362` and the sibling `A0/C0/…` dispatchers).

The label strings live in `resources.arsc` and could not be decoded here (no `apktool`/`aapt` on this
machine), so the tables below name controls by their `EACDataType`.

### 2.2 Control-by-control

| Tab                          | Control                  | Widget                                                    | Config field written                                                          | Range / step                               | Default here                                                                                                                                         |
| ---------------------------- | ------------------------ | --------------------------------------------------------- | ----------------------------------------------------------------------------- | ------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| DISPLAY                      | `DPI`                    | seekbar + "use system DPI" check                          | `dpiConfig.dpi`, `dpiConfig.enable` (inverse of the check)                    | 200…640, step **20**                       | `0` (= system); `useSystemDPI=false`                                                                                                                 |
| DISPLAY                      | `TXT_BOLD`               | checkbox                                                  | `globalActivityConfig.paintConfig.textBold`                                   | —                                          | `false`                                                                                                                                              |
| DISPLAY                      | `ANTI_ALIASING`          | checkbox                                                  | `paintConfig.antiAlisingType` (checked ⇔ `== 1` = force-disable)              | 0/1/2                                      | `0` (ignore)                                                                                                                                         |
| DISPLAY                      | `GC_FOR_NEW_SURFACE`     | checkbox                                                  | `refreshConfig.useGCForNewSurface`                                            | —                                          | `false` in JSON, but `Constant.DEFAULT_USE_GC_FOR_NEW_SURFACE = isCfaDevice() = true` at runtime                                                     |
| DISPLAY                      | `CONTROL_PANEL`          | header row: enable check + "fetch cloud config" + "reset" | `appConfig.enable`; posts `EACFetchFromServiceEvent` / `EACResetConfigsEvent` | —                                          | `enable = true`                                                                                                                                      |
| COLOR_EAC                    | `TXT_EAC`                | seekbar                                                   | `paintConfig.textEACType`                                                     | 0…3, step 1                                | `0` (disabled)                                                                                                                                       |
| COLOR_EAC                    | `FILL_EAC`               | seekbar                                                   | `paintConfig.fillContrast`                                                    | 0…255, step 1                              | `0`                                                                                                                                                  |
| COLOR_EAC                    | `OTHER_COLOR`            | detail row                                                | opens `TAB_TYPE_OTHER_COLOR`                                                  | —                                          | —                                                                                                                                                    |
| REFRESH_TURBO                | `GC_FOR_NEW_SURFACE`     | checkbox                                                  | as above                                                                      | —                                          | —                                                                                                                                                    |
| REFRESH_TURBO                | **`REFRESH_MODE_TURBO`** | radio group                                               | `refreshConfig.updateMode` **and** `refreshConfig.turbo` — see §2.3           | 5 options                                  | selection derived from stored `(updateMode, turbo)`                                                                                                  |
| REFRESH_TURBO                | `GC_INTERVAL`            | seekbar                                                   | `refreshConfig.gcInterval`                                                    | 0…50, step **5**                           | `20` — **hidden unless the stored `updateMode ∈ {0,3}`**                                                                                             |
| REFRESH_TURBO                | `ANIMATION_DURATION`     | seekbar                                                   | `refreshConfig.animationDuration`                                             | 0…3000, step **50**                        | `50` — **hidden unless `updateMode ∈ {0,3}`**                                                                                                        |
| OTHERS                       | `FULL_PM_ACCESS`         | checkbox                                                  | `extraConfig.fullPMAccess`                                                    | —                                          | `false`; the _effect_ is gated on `ActivityManagerHelper.isSystemApp(pkg)` in `OECService.java:1230`                                                 |
| OTHERS                       | `FORCE_ROTATION`         | checkbox                                                  | `rotationConfig.enable`                                                       | —                                          | `false` — **changing it restarts the target app** (`EACConfigChangedImpl.java:196-199`)                                                              |
| OTHERS                       | `SCROLL_BUTTON`          | checkbox + detail                                         | global floating-button setting + `appConfig.scrollArgs`                       | —                                          | off                                                                                                                                                  |
| OTHERS                       | `REMOVE_SPLASH_SCREEN`   | checkbox (inverted)                                       | `extraConfig.allowSplashScreen`                                               | —                                          | `allowSplashScreen = true`                                                                                                                           |
| NOTE_EAC                     | `PEN_EAC_ENABLE`         | checkbox                                                  | `globalActivityConfig.noteConfig.enable`                                      | —                                          | `false`                                                                                                                                              |
| NOTE_EAC                     | `PEN_EAC_STROKE_WIDTH`   | seekbar + live preview                                    | `noteConfig.globalStrokeStyle.strokeWidth`                                    | index into `EACConstant.strokeWidthValues` | `PenConstant.DEFAULT_STROKE_WIDTH`                                                                                                                   |
| DETAIL_APP_EAC               | `GAMMA`                  | seekbar                                                   | `displayConfig.contrast`                                                      | 0…100, step **20**                         | `30`                                                                                                                                                 |
| DETAIL_APP_EAC               | `MONO_LEVEL`             | seekbar                                                   | `displayConfig.monoLevel`                                                     | 0…175, step 1                              | `10` — **inert on this CFA panel** (`impl/EACBaseDisplayImpl.java:38-45`)                                                                            |
| DETAIL_APP_EAC / OTHER_COLOR | `ICON_EAC`               | seekbar                                                   | `paintConfig.iconContrast`                                                    | 0…255, step 1                              | `0`                                                                                                                                                  |
| DETAIL_APP_EAC / OTHER_COLOR | `IMG_EAC`                | seekbar                                                   | `paintConfig.imgGamma`                                                        | 0…150, step 1                              | `60`                                                                                                                                                 |
| OTHER_COLOR                  | `WEB_FONT_COLOR`         | seekbar                                                   | `cssConfig.fontColor`                                                         | 0…255, step **64**                         | `0`                                                                                                                                                  |
| OTHER_COLOR                  | `WEB_FONT_SIZE`          | seekbar                                                   | `cssConfig.fontSize`                                                          | 100…120, step 1                            | `100`                                                                                                                                                |
| OTHER_COLOR                  | `WEB_FONT_BOLD`          | checkbox                                                  | `cssConfig.fontBold`                                                          | —                                          | `false`                                                                                                                                              |
| APP_DISPLAY                  | `GAMMA`, `MONO_LEVEL`    | seekbars                                                  | as above                                                                      |                                            | reachable only through the `APP_DISPLAY` detail row, which this device's tab map does not contain **[inference: that sub-page is unreachable here]** |

### 2.3 The refresh-mode picker, and why it is a trap

`EACDataType.REFRESH_MODE_TURBO` builds its options from
`EACViewDeviceConfig.getRefreshModeSet()`; **`eac_noteair4c.json` does not define `refreshModeSet`**,
so the hardcoded fallback list applies (`out-kcb/.../eac/data/EACViewConfigs.java:764-777`):

| Radio option                 | `updateMode` written | `turbo` written | resolved EPD mode if the index is cleared |
| ---------------------------- | -------------------- | --------------- | ----------------------------------------- |
| `TYPE_TABLET_REFRESH_NORMAL` | 0                    | 0               | AUTO (`toEpdMode(0) = 5`)                 |
| `TYPE_TABLET_REFRESH_DU`     | 1                    | 0               | `UI_DU_QUALITY_MODE` 2305                 |
| `TYPE_TABLET_REFRESH_A2`     | 2                    | 0               | `UI_A2_QUALITY_MODE` 2308                 |
| `TYPE_TABLET_REFRESH_A2_1`   | 2                    | 2               | 2308 + turbo 2                            |
| `TYPE_TABLET_REFRESH_A2_2`   | 2                    | 3               | 2308 + turbo 3                            |

Mapping functions: `getUpdateModeByType` (`:876-889`), `getTurboByType` (`:836-842`),
`getTypeByUpdateModeAndTurbo` (`:864-869`), and the private `a(int turbo)` (`:264-269`).

**`EACDataType.REFRESH_MODE`** (the reader-style DEFAULT/REGAL/DU/A2/X group, `:739-752`) is _not_ in
this device's tab map, so the older picker is not shown.

Two consequences that matter enormously:

1. **The picker writes `updateMode`/`turbo` and never `refreshModeIndex`.** A word-boundary grep for
   `setRefreshModeIndex` across `out-kcb/.../sdk/eac/**`, `.../sdk/api/**` and `.../sdk/device/**`
   returns only the bean's own setter and the copy constructor — no caller. The only writers of
   `refreshModeIndex` are the _themes_ in `eac_noteair4c.json` and
   `ValidateAppConfigAction.resetRefreshModeIndex`/`changeToAutoMode`
   (`action/ValidateAppConfigAction.java:56-63`, `:102-120`).
   Since `caculateRefreshConfig` prefers a resolving index over `updateMode`/`turbo`
   (`impl/EACBaseRefreshImpl.java:67-79`), **on this device the EinkWise refresh picker has no
   runtime effect at all while a valid `refreshModeIndex` is stored** — which is the factory state
   (`refreshModeIndexDefault: "refresh_mode_2"`). It only changes the UI's own read-back, which comes
   from `updateMode`/`turbo`. **[inference: every step is attested in this dump, but the live panel
   lives in `com.onyx.floatingbutton`, which is not in the dump and could write the index itself.
   A word-boundary grep for `setRefreshModeIndex` across all of `out-kcb` finds exactly one caller,
   `out-kcb/.../reader/apps/action/InitApplyEACConfigAction.java:156,159`, and it only ever writes
   `REFRESH_MODE_INDEX_FOR_ONYX_OR_SYSTEM_DEFAULT` for Onyx/system apps. This is experiment #1 below.]**
   Note also `EACItemRadioGroupViewModel`'s label map has no entry for `TYPE_TABLET_REFRESH_DU`
   (`out-kcb/.../eac/model/EACItemRadioGroupViewModel.java:17-29`), so selecting DU in the legacy
   dialog would NPE at `:82` — more evidence that this picker is not the live one.
2. **`GC_INTERVAL` and `ANIMATION_DURATION` disappear** from the tab whenever the stored `updateMode`
   is not 0 or 3: `showByType` → `isEnableByType` → `refreshConfig.isNormalOrRegalMode()`
   (`out-kcb/.../eac/data/EACViewConfigs.java:900-916`, `.../eac/data/v2/EACRefreshConfig.java:60-63`).
   The same predicate gates `PEN_EAC_REPAINT_LATENCY`. This is the UI-visible shadow of
   `Constant.DEBOUNCER_UPDATE_MODE_MAP = {0→0, 3→0, 5→0}`: the firmware's debounced quality repaint,
   its counted auto-GC and its dithering are all switched off outside modes 0/3/5, so EinkWise hides
   the knobs that feed them.

### 2.4 What is hidden on this device, and what gates it

| Hidden thing                                                                                                                                                                      | Gate                                                                                                                                                                                                                                                                                                                                                                                               |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `REFRESH_MODE`, `CFA_SATURATION`, `CFA_BRIGHTNESS`, `COLOR_EAC`, `DITHER_BITMAP`, `CUSTOM_GAMMA/SATURATION/BRIGHTNESS`, `PAGE_KEY_MODE`, `PEN_EAC_REPAINT_LATENCY`, `APP_DISPLAY` | simply absent from `eacTabToDataMap` in `eac_noteair4c.json`. The colour/contrast presets are reached instead through the theme picker (`currentColorType ∈ {eac_color_type_1..3}`), not through per-value sliders                                                                                                                                                                                 |
| `TYPE_REFRESH_X` (X mode)                                                                                                                                                         | only offered in the `REFRESH_MODE` fallback list, and only `if (!DeviceInfoUtil.isColorDevice())` (`EACViewConfigs.java:748-750`) — and that list is unused here anyway                                                                                                                                                                                                                            |
| `refresh_mode_4` (HD) in the picker                                                                                                                                               | `noteair4c_systemui.json` lists it under `refreshConfigForSystemApp`, not `refreshConfig`. **But those two arrays are UI picker lists only** — `ValidateAppConfigAction.validateRefreshConfig` (`:167-186`) accepts any index that resolves in `refreshConfigMap`, and the Regal clamp at `:170-172` applies only when `isOnyxOrSystemApp`                                                         |
| `enhance` forced on                                                                                                                                                               | `ValidateAppConfigAction.validateEnhance` (`:155-165`) forces `enhance = true` **only** for Onyx/system apps                                                                                                                                                                                                                                                                                       |
| `fullPMAccess` effect                                                                                                                                                             | `OECService.getFullPMAccessPkg` filters to `ActivityManagerHelper.isSystemApp(pkg)` (`:1230`)                                                                                                                                                                                                                                                                                                      |
| The whole panel, for some apps                                                                                                                                                    | `supportEAC` — `eac_force.json` sets `supportEAC:false` for `com.onyx`, `com.onyx.kreader`, **`com.onyx.android.note`** (stock Notes), `com.onyx.android.ksync`, `com.onyx.aiassistant`. `ValidateAppConfigAction.supportEAC` (`:288-293`) additionally returns `false` for any Onyx/system app outside the allowlist `{org.chromium.chrome, com.android.vending, com.android.browser}` (`:39-45`) |

Note that last row: **the stock Notes app opts out of EinkWise entirely.** It manages the panel by
itself, exactly as we do.

---

## 3. The full stored schema

### 3.1 Where the defaults come from — three independent layers

There are **three** default sources and they disagree with each other. This is the single biggest
source of confusion about EinkWise.

| Layer                               | File / class                                                                                                                                                                                                                                                                                                                                         | Loader                                                                                                                                                                                                                                                | Consumed by                                                                                                              |
| ----------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| **A. Framework EAC config**         | `framework-res.apk!/res/raw/eac.json` + `noteair4c_eac.json` → `android.onyx.config.EACConfig`                                                                                                                                                                                                                                                       | `android/onyx/config/ConfigLoader.java:33-63` — shallow, top-level key overwrite of `<model>_<category>` over `<category>`                                                                                                                            | the field initialisers of the `data/v2` beans, i.e. what a _newly created_ config contains                               |
| **B. Framework SystemUI config**    | `framework-res.apk!/res/raw/systemui.json` + `noteair4c_systemui.json` → `android.onyx.config.SysUIConfig`                                                                                                                                                                                                                                           | same loader                                                                                                                                                                                                                                           | `refreshConfigMap` — the profile table that `refreshModeIndex` resolves against                                          |
| **C. EinkWise's own device config** | `kcb.apk!/res/raw/eac_noteair4c.json` (falling back to `eac_default.json` only if the model file is missing) → `EACConstantDeviceConfig` / `PresetEACConfig` / `EACViewDeviceConfig`; plus `eac_force.json`, loaded separately by the `@Deprecated` `CleanUpOnyxAppConfigRequest` (`out-kcb/.../eac/rx/request/CleanUpOnyxAppConfigRequest.java:55`) | `com.onyx.android.sdk.utils.ConfigLoader.load(**0 = FULL_REPLACE**, cls, "eac_noteair4c", "eac_default")` (`out-kcb/.../eac/data/EACConstantDeviceConfig.java:16`, `EACConstant.java:100`, loader at `out-kcb/.../sdk/utils/ConfigLoader.java:27-30`) | the UI's slider ranges, step sizes, initial values, the tab→control map, the three themes, and `refreshModeIndexDefault` |

Concretely, `eac_default.json` on this build contains only `jsonVersion`, `presetAppConfig`
(54 packages), `eacTabToDataMap`, `themeConfigs`, `themeConfigsForOnyxOrSystem` — and it is unused,
because the loader is full-replace and `eac_noteair4c.json` exists. Every scalar default therefore
comes from the Java field initialisers in `out-kcb/.../eac/data/EACConstantDeviceConfig.java:105-215`,
overridden per key by whatever `eac_noteair4c.json` sets (fastjson only assigns present keys).
`eac_noteair4c.json` itself carries 35 `presetAppConfig` entries.

`eac_noteair4c.json` scalars, verbatim (`/tmp/.../kcb/res/raw/eac_noteair4c.json`, from `kcb.apk`):

```
jsonVersion=109  dpiDefault=0  dpiStepSize=20
updateModeDefault=2  turboDefault=2
txtEACTypeDefault=0  fillColorDefault=0  fillEACDefault=false  txtBoldDefault=false
useGCForNewSurface=false  antiFlickerDefault=10
cfaSaturationMinValue=60  eacRefreshType="TABLET_COLOR"  cfaColorModeDefault="vivid"
contrastDefault=30  cfaSaturationDefault=50  cfaBrightnessDefault=0
refreshModeIndexDefault="refresh_mode_2"
refreshModeIndexForOnyxOrSystemDefault="refresh_mode_4"
```

Note the disagreements that matter:

- **`cfaSaturationDefault`**: EinkWise (layer C) says **50**; `noteair4c_eac.json` (layer A) says **0**.
  The bean initialiser `EACDisplayConfig.cfaColorSaturation = EACConfig.singleton().getCfaSaturationDefault()`
  (`data/v2/EACDisplayConfig.java:9`) uses layer A → a config the _framework_ creates starts at 0,
  a config EinkWise's _theme_ creates starts at 50.
- **Dither threshold**: EinkWise's constants are `DITHER_LOW = 128`, `DITHER_NORMAL = 255`,
  `DITHER_HIGH_CONTRAST = 255` (`out-kcb/.../eac/data/EACConstant.java:21-23`) and its themes store
  128 or **255**. The framework's are `DITHER_NORMAL = 128`, `DITHER_HIGH_CONTRAST = 180`
  (`Constant.java:71-72`) and `ValidateAppConfigAction.validateDitherThreshold` **rewrites anything
  that is not 128 or 180 to 180** (`action/ValidateAppConfigAction.java:147-153`). So EinkWise's
  "255" theme becomes 180 at the next boot-time validation. `ViewUpdateHelper.setDitherThreshold`
  additionally drops anything `< 128` (`ViewUpdateHelper.java:1138-1145`).
- **`refreshModeIndexDefault`** is `"refresh_mode_3"` in the Java initialiser
  (`EACConstantDeviceConfig.java:211`) and `"refresh_mode_2"` in `eac_noteair4c.json`. Note that
  `refresh_mode_3` **does not exist** in `noteair4c_systemui.json` — that is the value Onyx pins
  `com.onyx.aiassistant` to in `eac_force.json`, and it silently falls through to the raw
  `updateMode`/`turbo` pair.
- `DITHER_BITMAP_DEFAULT` in EinkWise is a copy-paste bug: `= EACConstantDeviceConfig.getInstance().isFullPMDefault()`
  (`out-kcb/.../eac/data/EACConstant.java:180`), i.e. it reads the _full-PM-access_ flag. Both are
  `false` here, so the bug is invisible.

### 3.2 `EACAppConfig` — the per-package root

`/home/okhsunrog/tmp_zfs/onyx_framework/out/sources/android/onyx/optimization/data/v2/EACAppConfig.java:19-40`
(the `com.onyx` mirror is `out-kcb/.../eac/data/v2/EACAppConfig.java`, field-for-field identical).

| Field                                              | Type                              | Default                                                                                                                                                                                               | Notes                                                                                                                                                                                               |
| -------------------------------------------------- | --------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pkgName`                                          | String                            | `""`                                                                                                                                                                                                  | must equal the requested package or `getAppConfigFromService` drops it (`OECService.java:1095`)                                                                                                     |
| `enable`                                           | boolean                           | ctor `true`; `getDefaultAppConfig` `false`                                                                                                                                                            | the CONTROL_PANEL master switch                                                                                                                                                                     |
| `supportEAC`                                       | boolean                           | `true`                                                                                                                                                                                                | when `false`, `getRefreshConfigByCurrentComponent` returns a bare config and `handleSFDebouncer` clears the debouncer (`impl/EACBaseRefreshImpl.java:134-140`, `TabletEACRefreshImpl.java:143-149`) |
| `globalActivityConfig`                             | `EACActivityConfig`               | new                                                                                                                                                                                                   | §3.3                                                                                                                                                                                                |
| `activityConfigMap`                                | `Map<clsName, EACActivityConfig>` | `{}`                                                                                                                                                                                                  | per-activity override, preferred by `obtainActivityConfig(cls)` (`:311-315`) and `fuzzyMatchActivityConfig`                                                                                         |
| `extraConfig`                                      | `EACAppExtraConfig`               | `icon=null, forceFullScreen=true, usePageKeyAsVolumeKey=true, fullPMAccess=false, fullPMAccessTimeout=300000, useDialogBorder=false, allowSplashScreen=false` (`data/v2/EACAppExtraConfig.java:7-13`) |                                                                                                                                                                                                     |
| `dpiConfig`                                        | `EACDpiConfig`                    | `dpi = Resources.getSystem().getDisplayMetrics().densityDpi` (`data/v2/EACDpiConfig.java:7`) — **not 0**                                                                                              | plus inherited `enable`                                                                                                                                                                             |
| `rotationConfig`                                   | `EACRotationConfig`               | `overrideRotation = -1` (`:5`)                                                                                                                                                                        | changing `enable` or `overrideRotation` makes the in-app agent call **`restartPackage()`** (`EACConfigChangedImpl.java:196-199`)                                                                    |
| `keyboardConfig`                                   | `EACKeyboardConfig`               | `pageKeyMode = 1` (VOLUME_KEY)                                                                                                                                                                        |                                                                                                                                                                                                     |
| `networkConfig`                                    | `EACNetworkConfig`                | `overrideRules = 0`                                                                                                                                                                                   |                                                                                                                                                                                                     |
| `autoStartConfig`                                  | `EACAutoStartConfig`              | `supportAdjust=false, autoStart=true`                                                                                                                                                                 |                                                                                                                                                                                                     |
| `autoFreezeConfig`                                 | `EACAutoFreezeConfig`             | `supportAutoFreeze=false, autoFreeze=false`                                                                                                                                                           |                                                                                                                                                                                                     |
| `globalCSSConfig`, `cssConfigMap`                  | `EACCSSConfig`                    | `clsName="", customCSS="", fontBold=false, fontColor=0, fontSize=0`                                                                                                                                   | WebView CSS injection                                                                                                                                                                               |
| `colorMode`                                        | String                            | `EACConfig.getColorMode()` = `"vivid"` here                                                                                                                                                           |                                                                                                                                                                                                     |
| `colorConfigMap`                                   | `Map<String, EACColorConfig>`     | `EACColorConfig{gamma, saturation, brightness, monoLevel}` all 0                                                                                                                                      |                                                                                                                                                                                                     |
| `scrollArgs`                                       | `EACScrollArgs`                   | `{duration, sampleTime, startXPercent, startYPercent, endXPercent, endYPercent}` all 0                                                                                                                | the scroll-button gesture                                                                                                                                                                           |
| `scrollViewWhiteList`, `forceScrollRefreshClsList` | `List<String>`                    | `[]`                                                                                                                                                                                                  |                                                                                                                                                                                                     |

### 3.3 `EACActivityConfig`

`data/v2/EACActivityConfig.java:7-15`

| Field                 | Default                                                 | Notes                                                                                                                        |
| --------------------- | ------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `enable`              | `true` (from `EACBaseConfig`)                           |                                                                                                                              |
| `clsName`             | `""`                                                    |                                                                                                                              |
| `refreshConfig`       | new `EACRefreshConfig`                                  | §3.4                                                                                                                         |
| `displayConfig`       | new `EACDisplayConfig`                                  | §3.5                                                                                                                         |
| `paintConfig`         | new `EACPaintConfig`                                    | §3.6                                                                                                                         |
| `noteConfig`          | new `EACNoteConfig`                                     | §3.7                                                                                                                         |
| `scrollRefreshDelay`  | `0`, clamped to `[1000, 5000]` on read (`:7`, `:49-51`) | written to `Settings.Global.SCROLL_REFRESH_DELAY` on **every app resume** (`impl/EACScrollRefreshImpl.java:23-30`, `:44-46`) |
| `eacScrollStyle`      | `1`                                                     | `2` enables `OnyxEpdBypassManager` (`OnyxEpdBypassManager.java:188`)                                                         |
| `isDisableScrollAnim` | `false`                                                 |                                                                                                                              |

### 3.4 `refreshConfig` — `EACRefreshConfig` (`data/v2/EACRefreshConfig.java:8-17`)

| Field                | Type    | Default on this device                                                                                                                                                    | Written by which EinkWise control                                                              |
| -------------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `enable`             | boolean | inherited `true`; EinkWise forces `true` on every apply (`EACViewUtils.java:699-704`)                                                                                     | —                                                                                              |
| `refreshModeIndex`   | String  | `"refresh_mode_2"` (from the theme, layer C)                                                                                                                              | **none — the UI never writes this field** (§2.3)                                               |
| `updateMode`         | int     | `2` (`updateModeDefault`)                                                                                                                                                 | REFRESH_MODE_TURBO radio                                                                       |
| `turbo`              | int     | `0` in the bean; `2` as `turboDefault`; `5` if resolved from `refresh_mode_2`                                                                                             | REFRESH_MODE_TURBO radio                                                                       |
| `gcInterval`         | int     | `20` (`:15`); `obtainLegalGcInterval()` turns `<=0` into `Integer.MAX_VALUE` (`:76-82`)                                                                                   | GC_INTERVAL seekbar                                                                            |
| `antiFlicker`        | int     | `EACConfig.getAntiFlickerDefault()` = `10`; range 0..32 (`Constant.java:30-33`)                                                                                           | not on this device's tabs                                                                      |
| `animationDuration`  | int     | bean `0`; UI default `50`; `0` is treated as 10 ms (20 ms for REGAL) and floored by `EACConfig.minAnimationDuration = 80` (`TabletEACRefreshImpl.java:131-141`, `:87-92`) | ANIMATION_DURATION seekbar                                                                     |
| `animationType`      | String  | `null` → `"debounce"`                                                                                                                                                     | none                                                                                           |
| `useGCForNewSurface` | boolean | bean `false`; device runtime default `true` (`Constant.java:390` = `isCfaDevice()`); the apply path only ever sets it **on** (`TabletEACRefreshImpl.java:99-101`)         | GC_FOR_NEW_SURFACE checkbox                                                                    |
| `supportRegal`       | boolean | `false`                                                                                                                                                                   | none                                                                                           |
| `refreshModeAlias`   | String  | `null`                                                                                                                                                                    | none — `res/raw/refresh_mapping.json` is `{}` on this build, so `RefreshMappingConfig` is dead |

### 3.5 `displayConfig` — `EACDisplayConfig` (`data/v2/EACDisplayConfig.java:7-14`)

| Field                   | Default                                                          | Applied on this CFA panel?                                                                           |
| ----------------------- | ---------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `enable`                | `true`                                                           | gates the whole block                                                                                |
| `contrast` (gamma)      | `30`; range 0..100 step 20                                       | **yes** — `ViewUpdateHelper.applyGammaCorrection(v>0, v)` (`impl/EACBaseDisplayImpl.java:60-63`)     |
| `monoLevel`             | `10`; range 0..175, `+80` offset                                 | **no** — the CFA branch skips it (`impl/EACBaseDisplayImpl.java:38-45`)                              |
| `cfaColorSaturation`    | layer A `0` / layer C `50`; range 0..100                         | **yes**                                                                                              |
| `cfaColorSaturationMin` | `60` (`noteair4c_eac.json`)                                      | **yes**                                                                                              |
| `cfaColorBrightness`    | `0`; range 0..5                                                  | **yes**                                                                                              |
| `ditherThreshold`       | `EACConfig.getDitherThreshold()`; validator forces to 128 or 180 | **yes**                                                                                              |
| `bwMode`                | `0`                                                              | **yes** — CFA only; mirrored to `Settings.Global["view_update_bw_mode"]` (`OECService.java:224-228`) |
| `enhance`               | `true`                                                           | **yes**; forced `true` for Onyx/system apps (`ValidateAppConfigAction.java:155-165`)                 |

Apply order is fixed: gamma → (CFA: brightness, saturationMin, saturation, bwMode) → ditherThreshold
→ enhance (`impl/EACBaseDisplayImpl.java:36-48`).

### 3.6 `paintConfig` — `EACPaintConfig` (`data/v2/EACPaintConfig.java:5-18`)

`enable`(inherited `true`), `textBold=false`, `textEACType=0`, `antiAlisingType=0`, `fillEAC=false`,
`fillContrast=0`, `fillBrightness=0`, `iconEAC=false`, `iconContrast=0`, `iconBrightness=0`,
`iconThreshold=0f`, `imgEAC=false`, `imgGamma=0`, **`ditherBitmap=false`**, `quantBits=3`.

This block is pushed into every `Canvas` of every non-Onyx app on every draw:

```java
// out/sources/android/view/View.java:6094-6104   updateCanvasImpl(Canvas)
if (ActivityManagerHelper.isOnyxApp(mContext.getPackageName())) return;
EACPaintConfig paintConfig  = EInkHelper.getPaintConfig(mContext);
EACRefreshConfig refreshCfg = EInkHelper.getRefreshConfig(mContext);
canvas.getEACCanvasImpl().setPaintConfig(paintConfig);
canvas.getEACCanvasImpl().setRefreshConfig(refreshCfg);
```

`ditherBitmap` gates `ViewUpdateHelper.enableDither(...)`, but only when the **raw** `updateMode`
field is in `DEBOUNCER_UPDATE_MODE_MAP` (`TabletEACRefreshImpl.java:72-79`).

### 3.7 `noteConfig` — `EACNoteConfig` (`data/v2/EACNoteConfig.java:18-23`)

`enable=false`, `supportNoteConfig=false`, `drawViewKey=null`, `styleMap={}`, **`repaintLatency=500`**
(UI range 500..2000 step 500, `out-kcb/.../eac/data/EACConstant.java:60-62`),
`globalStrokeStyle=EACStrokeStyle{strokeStyle=0, strokeWidth=3.0f, strokeColor=0xFF000000, strokeExtraArgs=[]}`,
`compatibleVersionCode=0`.

This is the **system-driven** screen-note path: `EACScreenNoteManager` only engages when
`DEVICE_SUPPORT_SCRIBBLE && EInkHelper.getNoteConfig(app).isSupportNoteConfig()`
(`EACScreenNoteManager.java:155`). Onyx sets `supportNoteConfig=true` only for the packages listed in
`presetAppConfig` (Evernote/`com.yinxiang`, WPS `cn.wps.moffice_eng`, …). It is **`false` for us**, so
none of it runs — see §6.

### 3.8 Device-level neighbours

`EACDeviceConfig` (`data/v2/EACDeviceConfig.java:13-17`): `jsonVersion`, `appConfigMap`, `extraConfig`.

`EACDeviceExtraConfig` (`data/v2/EACDeviceExtraConfig.java:11-32`): `accessibilityTouchEventDelay`,
`allowChildModeApps`, `allowUseRegalModePkgSet`, `appDefaultConfig` (the _fallback_ refresh/display
config), `customOOMAdjPkgSet`, `displayConfig`, `enable`, `gcAfterScrolling` (default `false`,
`Constant.GC_AFTER_SCROLLING_DEFAULT`), `preGrantPermissionPkgSet`, `refreshMode`,
`scrollingRefreshMode`, `turbo`.

`EACAppTheme` (`data/v2/EACAppTheme.java`): `pkg`, `themeType` ∈ {1,2,3}, `appConfig` (a whole
`EACAppConfig`), `colorTypeConfigs: Map<eac_color_type_1..3, EACDisplayConfig>`,
`dipTypeConfigs: Map<eac_dpi_type_1..3, Integer>`, `currentColorType`, `currentDpiType`. **The theme
is the real store** — see §4.3.

---

## 4. How it reaches the system

### 4.1 The Binder interface

`android.onyx.optimization.IOECService`, service name **`"oec_service"`**, descriptor
`"android.onyx.optimization.IOECService"` (`IOECService.java:15`), **80 transactions**, contiguous
1–80 (constants at `IOECService.java:376-455`).

The transactions that matter for the panel:

| Tx                  | Method                                                                                                                            | Effect                                                                                                               |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| 12                  | `applyAppConfigToService(List<String> jsonList, Bundle args)`                                                                     | the canonical write; `args_operation_flag` int and `BroadcastHelper.ARGS_SAVE_MMKV` boolean                          |
| 16                  | `getAppConfigFromService(List<String> pkgs) → List<String>`                                                                       | the canonical read                                                                                                   |
| 13                  | `applyRefreshConfigToAll(String refreshJson)`                                                                                     | rewrites the refresh block of **every** app plus the fallback (`OECService.java:296-309`)                            |
| 14/15               | `removeAppConfigFromService` / `removeAllAppConfigFromService`                                                                    | deletes one / all per-app configs                                                                                    |
| 25/26               | `setAppScopeRefreshMode(int, boolean)` / `getAppScopeRefreshMode()`                                                               | runtime mode on the **foreground** app; writes `updateMode` only                                                     |
| 42/43               | `setTurbo(int, boolean)` / `getTurbo()`                                                                                           | persisted turbo on the foreground app                                                                                |
| 44–49               | `setAnimationDuration`, `setGcInterval`, `setAntiFlicker` + getters                                                               | persisted, foreground app                                                                                            |
| 31–38, 50–55, 59–63 | contrast / mono / CFA saturation / CFA brightness / bwMode / ditherThreshold / enhance / colorConfig / colorMode / colorParameter | persisted, foreground app                                                                                            |
| 71/72/73/80         | `saveEACAppThemes`, `applyEACAppThemes`, `applyEACAppTheme`, `applyEACTheme(String, int)`                                         | the theme path (§4.3)                                                                                                |
| 67–70               | `saveDefaultAppConfigs`, `saveDefaultThemeConfig`, `saveAsGlobalDefaultThemeConfig[ForOnyxApp]`                                   | writes the _defaults_ used to rebuild configs                                                                        |
| 1                   | `setEnable(boolean)`                                                                                                              | global EAC master switch — **and it kills the uid of every EAC-enabled running app** (`OECService.java:351`, `:668`) |
| 64                  | `setRoleHolderAsUser(String, String, boolean, int, UserHandle)`                                                                   | grants/revokes system roles                                                                                          |

There is **no note/pen and no DPI/rotation method**: those blocks travel inside the app-config JSON.

### 4.2 Permission and signature checks: there are none

- `OECService` does **not** override `onTransact`; `IOECService.Stub.onTransact`
  (`IOECService.java:1744`) only does `enforceInterface(DESCRIPTOR)`.
- A grep of the whole `android/onyx/optimization/` tree for `checkCallingPermission`,
  `enforceCallingPermission`, `Binder.getCallingUid`, `getCallingPid`, `clearCallingIdentity`,
  `PERMISSION`, `signature` finds **zero** caller checks. The only `isSystemApp` call in
  `OECService.java` is at `:1230`, and it filters the _returned list_ of `getFullPMAccessPkg()`, not
  the caller.
- `ServiceManager.addService("oec_service", oECService, true)` — `allowIsolated = true`
  (`out-services/.../SystemServer.java:1181`).
- Two exported broadcast entry points reach the same machinery without the binder:
  `onyx.android.EAC_CONFIG_ACTION` → `setDebug`/`setEnable` (`OECService.java:141-144`, receiver
  registered `RECEIVER_EXPORTED` in `initReceiver`, `:655-666`, `android/onyx/BroadcastHelper.java:515-521`),
  and `onyx.action.oec.config.change` → `ActivityManagerService.registerOECIntentReceiver`
  (`out-services/.../am/ActivityManagerService.java:14830-14838`) with no permission argument.

The only real gates for a third-party app are (a) hidden-API access — we lift it with
`HiddenApiBypass` in `VendorAccess.kt:17-26` — and (b) SELinux. Both are empirically satisfied today:
our `ensureSpeedRefreshProfile()` already reads and writes this config.

`dumpsys oec_service` prints every app config, autostart and networking config
(`OECService.java:1547-1571`) and needs only `android.permission.DUMP`.

### 4.3 What is persisted, where, and the read/write asymmetry

**Storage.** One MMKV map:

```java
// android/onyx/optimization/SystemMMKV.java:9-25
ONYX_CONFIG = "onyx_config";
ROOT_DIR  = DeviceConfig.singleton().getSystemConfigPrefix() + "mmkv";
CACHE_DIR = DeviceConfig.singleton().getSystemConfigPrefix() + "mmkv/cache";
MMKV.initialize(ActivityThread.currentApplication(), ROOT_DIR, CACHE_DIR);
kv = MMKV.mmkvWithID(ONYX_CONFIG, 2);      // 2 = MULTI_PROCESS_MODE
```

`getSystemConfigPrefix()` defaults to `"/vendor/"` (`android/onyx/config/DeviceConfig.java:13`, `:31`)
but `DeviceConfig` is itself loaded by `ConfigLoader` from `res/raw/config.json`
(`DeviceConfig.java:51`), and that file sets **`"systemConfigPrefix": "/onyxconfig/"`**
(`/home/okhsunrog/tmp_zfs/onyx_framework/fwres/x/res/raw/config.json:3`); `noteair4c_config.json`
does not override it. So the real path is **`/onyxconfig/mmkv/onyx_config`** (+ `.crc`, cache under
`/onyxconfig/mmkv/cache`). Unencrypted (`mmkvWithID(String,int)` is the no-crypt-key overload),
multi-process, mmap'd, **survives reboot**. The legacy pre-MMKV file is `/onyxconfig/eac_config`
(`Constant.java:380`), read once by `LoadDeviceConfigFromLocalAction.migrateToMMKV()`
(`action/LoadDeviceConfigFromLocalAction.java:29-42`) when `eac_app_pkg_set` is empty.

**Keys.**

| Key                                                                          | Written by                                                                                                     | Content                                                                                                                                                                                        |
| ---------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `eac_app_<pkg>`                                                              | `EACAppConfig.save()` (`data/v2/EACAppConfig.java:328-331`), reached from `EACDeviceConfig.save()` (`:97-109`) | the `EACAppConfig` JSON                                                                                                                                                                        |
| `eac_app_pkg_set`                                                            | `EACDeviceConfig.save()` (`:107`)                                                                              | the index of packages with a stored config                                                                                                                                                     |
| `eac_device_json_version`                                                    | `EACDeviceConfig.save()` (`:99`)                                                                               |                                                                                                                                                                                                |
| **`eac_theme_<pkg>@theme_type_<1\|2\|3>`**                                   | `EACAppTheme.save()` (`data/v2/EACAppTheme.java:217`)                                                          | **the live config actually served and enforced**                                                                                                                                               |
| `eac_active_theme_<pkg>`                                                     | `EACAppThemeManager.setActiveThemeType` (`:105`); read with default **3** (`:37`)                              | which theme is active                                                                                                                                                                          |
| `default_app_theme_<pkg>@theme_type_<n>`                                     | `EACAppTheme.saveToDefaultConfig()` (`:235`)                                                                   | per-app default theme                                                                                                                                                                          |
| `default_config_eac_app_theme_<n>` / `default_config_eac_onyx_app_theme_<n>` | `:225` / `:230`                                                                                                | global default themes                                                                                                                                                                          |
| `eac_default_app_config` / `eac_default_app_config<pkg>`                     | `EACAppConfig.saveGlobalDefaultConfig()` / `saveDefaultConfig()` (`:334-341`)                                  | defaults used when rebuilding                                                                                                                                                                  |
| `eink_app_config_version_<pkg>`                                              | `EACThemeUtil.saveAppConfigVersion` (`android/onyx/utils/EACThemeUtil.java:83`)                                | upgrade bookkeeping                                                                                                                                                                            |
| device-extra keys                                                            | `EACDeviceExtraConfig` (`data/v2/EACDeviceExtraConfig.java:11-20`)                                             | `pre_grant_premission_pkg`, `allowUseRegalModePkgSet`, `customOOMAdjPkgSet`, `eac_extra_gc_after_scrolling`, `gc_after_scrolling_refresh_mode`, `eac_extra_refresh_mode`, `eac_extra_turbo`, … |

The **only** `Settings.Global` write in the whole EAC package is
`Settings.Global.putInt(SCROLL_REFRESH_DELAY, …)` (`impl/EACScrollRefreshImpl.java:29`), fired on every
app resume. `view_update_bw_mode` is read, not written, by this package (`OECService.java:226`).
System properties written: `sys.oec.service.debug`, `sys.oec.service.ready` (`OECService.java:1870`, `:902`).

**The asymmetry — this is the important part.**

```java
// data/v2/EACDeviceConfig.java:30-40
public EACAppConfig ensureAppConfig(String str) {
    return EACAppThemeManager.getActiveTheme(str).getAppConfig();   // <-- the THEME
}
public EACAppConfig getAppConfigByComponentName(ComponentName cn) {
    return ensureAppConfig(cn == null ? "" : cn.getPackageName());
}
```

- **Every read** on the enforcement path goes through `ensureAppConfig` → the _active theme_:
  `getAppConfigFromService` (`OECService.java:1090-1100`), `TabletEACRefreshImpl.onResume`
  (`:240-250`), `getRefreshConfig`, `getAppScopeRefreshMode`, `getTurbo`, every display getter.
- **Every write** through `applyAppConfigToService` goes into `appConfigMap` and is persisted to
  `eac_app_<pkg>` (`OECService.java:190-219` → `EACDeviceConfig.save()` → `EACAppConfig.save()`).

The two are reconciled in exactly two places:

1. `EACThemeFactory.loadThemeOrCreate(pkg, type)` (`android/onyx/utils/EACThemeFactory.java:393-427`)
   is a lazy `firstNonNull` chain: `loadThemeFromMMKV` (key `eac_theme_<pkg>@theme_type_<n>`) →
   `loadLowVersionAppConfigThemeOrNull` (which, for type 3, loads `eac_app_<pkg>` and stashes it as
   `overwriteAppConfig`, `:357-386`) → `loadDefaultThemeFromMMKV` → `loadGlobalOnyxAppThemeOrNull` →
   `loadGlobalDefaultThemeFromMMKV` → `createTheme`. `firstNonNull` uses `Stream…findFirst()`
   (`:158-171`) and therefore **short-circuits**: _if a theme key already exists, `eac_app_<pkg>` is
   never consulted.\_
2. `ValidateAppConfigAction.loadConfigs()` does `activeTheme.setAppConfig(this.activeAppConfig)` and
   `saveConfigs()` saves both (`action/ValidateAppConfigAction.java:94-100`, `:122-125`). It is run
   for every package in `eac_app_pkg_set` **once, at service start**
   (`action/LoadDeviceConfigFromLocalAction.java:44-51`, called from `OECService.loadConfigFromLocal`
   at `:696`).

Consequence, and it is a live hazard for us: **the first time a package's config is written there is
usually no theme key, so the write is picked up immediately; but once a theme key exists — after the
first reboot, or after the user opens EinkWise on that app — a subsequent
`applyAppConfigToService` write is applied to the panel _once_ (via `onApplyConfig`, which uses the
object we just passed) and then goes dormant, because every later `onResume` re-reads the stale
theme. It only becomes durable at the next boot, when `ValidateAppConfigAction` copies
`eac_app_<pkg>` into the theme.** [inference on the exact ordering; the individual steps are all
attested.]

### 4.4 Every path that re-pushes a stored config

The fan-out point is `eacImplMap` (`OECService.java:645-653`: `0` `EACAutoFreezeImpl`, `1` `EACPMImpl`,
`2` `TabletEACDisplayImpl`, `3` `TabletEACRefreshImpl`, `4` `EACScrollRefreshImpl`).

| Trigger                                                                         | Entry                                                                                                                                                                                 | Chain                                                                                                                                                                                                                                                                        | What is re-sent to SurfaceFlinger                                                                                                                                                                                                                                                                     |
| ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **A. Top-component change** (app switch, and any activity change inside an app) | native pipe `/dev/onyx/listener` → `EpdEventListener.Callback.onActivityStateChanged`, state `1` RESUMED / `4` TOP_RESUMED (`OECService.java:604-616`)                                | `handleActivityResumeAsync`/`handleActivityTopResumedAsync` → `handleActivityResumeImpl` (`:461-471`), gated on `OptimizationBundle.isTopComponentChanged()` (`bundle/OptimizationBundle.java:79-83`) → `EACBaseImpl.onResume(prev, cur, cfg)` on all five impls             | `useGCForNewSurface(true)`, `clearAppScopeUpdate(flag)` **or** `debouncer(false,…)` + `applyAppScopeUpdate(null,…)`, `setEpdTurbo`, `antiFlicker`, `enableDither`, `debouncer(true,…)`, then the whole display block (gamma, brightness, saturationMin, saturation, bwMode, ditherThreshold, enhance) |
| **B. A config or theme apply**                                                  | `applyAppConfigToService` / `applyEACAppTheme(s)` / `applyEACTheme` / `applyAppFreezeConfigsToService`                                                                                | `applyAppConfigToServiceImpl` (`:190`) → `applyConfigChangeImpl` (`:229-234`) → `onApplyConfig(top, old, new, cfg)`; **`TabletEACRefreshImpl.onApplyConfig` runs only if `isMatch(new, topComponent)`** (`impl/TabletEACRefreshImpl.java:229-237`)                           | same set, minus the `whiteClear`/multi-window branch                                                                                                                                                                                                                                                  |
| **C. The in-app half of B**                                                     | `onyx.action.oec.config.change` → AMS → `IApplicationThread` tx 59 → `EACConfigChangedImpl.handleEACAppConfigChanged` **inside the target app** (`EACConfigChangedImpl.java:291-298`) | flag `1` → WebView `reload()`/`loadUrl()`; `2` → default + WebView reload; `3` → `dispatchFullRedraw` if fast-mode flipped; `0` → DPI dispatch / **`restartPackage()`** on rotation change / `dispatchFullRedraw` on paint change (`:88-103`, `:153-215`, `:253-259`)        |
| **D. Boot completed**                                                           | `onyx.action.boot.animation.completed` → `OECService.handleBootCompleted` (`:501-532`)                                                                                                | `initDeviceConfigImpl` (`:562-568`) then `applyDefaultConfigImpl` (`:272-277`): `enableBWMode`, `setEnhanceStrategy`, `setDitherThreshold`                                                                                                                                   |
| **E. Service construction / config load**                                       | `new OECService(context)` (`:167-175`) → `loadConfigFromLocal` (`:696`) → `LoadDeviceConfigFromLocalAction` → **`ValidateAppConfigAction` for every stored package**                  | rewrites and re-persists `refreshModeIndex` and `ditherThreshold`; broadcasts `onyx.action.oec_service_data_changed` type 1, which every app's `EInkHelper` receiver picks up (`EInkHelper.java:67`, `:126-131`)                                                             |
| **F. System window / dialog visibility**                                        | `onyx.action.sys.window.visibility.changed`, SystemUI dialog open/close (`OECService.java:150-163`) → `configDebouncerBySysWindowChanged` (`impl/TabletEACRefreshImpl.java:190-201`)  | showing → `debouncer(false,0,0,0,0)`; hidden → `applySFDebouncer` with the stored config                                                                                                                                                                                     |
| **G. Screensaver start/stop**                                                   | `ACTION_DREAMING_STARTED/STOPPED` (`OECService.java:153-158`, `:734-770`)                                                                                                             | `TabletEACDisplayImpl.onResume` → the full display block                                                                                                                                                                                                                     |
| **H. Input event up**                                                           | native pipe `inputEventUpdate` → `EACBaseRefreshImpl.handleInputEventImpl` (`:25-45`)                                                                                                 | on every MotionEvent/KeyEvent `ACTION_UP`: `debounceIncRefresh()` and `enableRegal(true)` + `setDebouncerTransientUpdateMode(toEpdMode(mode))` — **only when the mode is in `DEBOUNCER_UPDATE_MODE_MAP = {0→0, 3→0, 5→0}`** (`Constant.java:391-397`, `EACUtils.java:26-41`) |

### 4.5 Every path that REBUILDS a stored config from defaults — i.e. clobbers ours

These live in `com.onyx`, not in the framework, and they are the ones that can silently discard what
a third-party app wrote.

| Trigger                            | Path                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | Scope                                                                                                                                                                                                                                                                    |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Every `com.onyx` process start** | `ContentBrowserApplication.java:377` → `new InitApplyEACConfigAction().execute()` (`out-kcb/.../reader/apps/action/InitApplyEACConfigAction.java:56-63`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | always runs `t()` (`:176-184`, re-pushes all `PresetEACConfig` entries via `saveDefaultAppConfigs` + `applyAppDefaultConfigToService`), `u()`/`z()` (`:186-230`, re-pushes all six default themes) and `o()` → `CleanUpOnyxAppConfigRequest` (`eac_force.json` packages) |
| **First boot / after "reset all"** | `InitApplyEACConfigAction.k()` (`:94-107`, guarded by `KCBMMKVHelper.getBoolean(FIRST_OPEN, true)`) → `BuildDefaultEACConfigRequest`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | **every installed app**, ours included                                                                                                                                                                                                                                   |
| **jsonVersion upgrade (OTA)**      | `CheckEACConfigVersionRequest` (`out-kcb/.../eac/rx/request/CheckEACConfigVersionRequest.java:10-13`, `storedVersion < 109`) → `UpgradeEACConfigRequest.b()` (`:38-55`). The upgrade ladder only has entries for 103 and 107; **for a stored version of 108 or 109 the ladder is empty and it falls through to `BuildDefaultEACAppConfig`** (`:49` → `out-kcb/.../eac/upgrade/BuildDefaultEACAppConfig.java:13-16`) → `BuildDefaultEACConfigRequest`                                                                                                                                                                                                                                                                                          | **every installed app**                                                                                                                                                                                                                                                  |
| **App install**                    | `EACStatusChangedReceiver` on `PACKAGE_ADDED` (`:302`) → `AppInstalledAction` → optional cloud fetch → `SaveDefaultThemeAction`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | that package                                                                                                                                                                                                                                                             |
| **Panel "reset"**                  | `EACItemControlPanelViewModel.onResetClick()` (`out-kcb/.../eac/model/EACItemControlPanelViewModel.java:40-43`) or the `…setting.eac.reset` broadcast → `EACAppConfigChangeAction` `RESET` → `e()` (`out-kcb/.../eac/rx/action/EACAppConfigChangeAction.java:104-114`) replaces the config with `RxEACUtils.getBuiltInAppConfig(ctx, pkg)`, and `c()` (`:75-77`) attaches `args_operation_flag = 2`                                                                                                                                                                                                                                                                                                                                           | that package; **flag 2 = `onResetConfig` in our process = WebView reload**                                                                                                                                                                                               |
| **Settings → Apps → reset all**    | `ResetAllAppEACAction` → `removeAllAppConfigFromService()` + `FIRST_OPEN = true` + `InitApplyEACConfigAction`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | everything                                                                                                                                                                                                                                                               |
| **Cloud fetch**                    | `FetchCloudEACConfigRequest` (`out-kcb/.../eac/rx/request/FetchCloudEACConfigRequest.java:35-47`) — a REST `find(Build.MODEL, "3.2")` against `clusterInfo.loadOnyxApiV2BaseUrl()` — → `CloudConfigToAppConfigRequest` (`:25-43`; the legacy branch rebases on `getBuiltInAppConfig`, losing local customisation) → `SaveDefaultThemeAction`. Triggered by the app-list screen on resume over Wi-Fi (`out-kcb/.../reader/apps/ui/AppsListFragment.java:295-311`), by a new install when `isUpdateNewlyInstalledAppEACConfig` is on, by the panel's cloud button, and by the **exported** broadcast `com.onyx.EAC_FETCH_FROM_CLOUD` (`out-kcb/.../common/broadcast/ExternalRequestReceiver.java`). No user confirmation anywhere in that chain | all installed packages by default (`FetchCloudEACConfigAction.d()`, `:35-37`)                                                                                                                                                                                            |
| **Every service start**            | framework `ValidateAppConfigAction` (§4.4 trigger E) — rewrites `refreshModeIndex`/`ditherThreshold` and **persists** via `saveConfigs()`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | every package in `eac_app_pkg_set`                                                                                                                                                                                                                                       |

Note `BuildDefaultEACConfigRequest.p()` (`:194-202`) does not overwrite the refresh block for an app
that already has a config — it overwrites `supportEAC`, `autoFreezeConfig.supportAutoFreeze`,
`autoStartConfig`, `globalActivityConfig.noteConfig` and a negative `dpiConfig`. The refresh block
survives a _rebuild_; it does not survive a **reset** (`getBuiltInAppConfig`) or a **theme apply**
(`SaveDefaultThemeAction.f()`, `out-kcb/.../eac/rx/action/SaveDefaultThemeAction.java:43-63`,
which replaces `globalActivityConfig.refreshConfig` and `.displayConfig` wholesale from the theme).

**Explicitly absent**: `ACTION_SCREEN_ON`/`OFF`, `ACTION_USER_PRESENT`, `ACTION_PACKAGE_ADDED/REPLACED`,
`android.intent.action.BOOT_COMPLETED` — none of them appear anywhere under
`android/onyx/optimization/`. Screen-on re-push happens only via the dream-stopped path (G) or the
next activity resume (A).

Note a subtlety in A vs B: `onResume` calls `applyAppScopeUpdate(**null**, mode, flag)`
(`impl/TabletEACRefreshImpl.java:112`), so the package hash on the wire is `-1`
(`ViewUpdateHelper.java:307-308`), while `onApplyConfig`/`setAppScopeRefreshMode` pass the real
package name. [inference: `-1` means "not keyed to a package"; whether SF then applies it globally is
a SurfaceFlinger question, §5.]

---

## 5. What EinkWise actually does to the panel

### 5.1 The three channels that carry an update mode

| Channel                         | API                                                                                                                                                                     | Granularity                                                                                                                       | Lifetime                                                                                        | Who uses it                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Per-frame region list**       | `ViewRootImpl.addEpdc()` → `Surface.addEpdc(int[]{l,t,r,b,mode}×n)` → `nativeAddEpdc` (`android/view/ViewRootImpl.java:3257-3270`, `android/view/Surface.java:333-338`) | one entry per invalidated region, merged when mode and rect coincide (`ViewUpdateHelper.UpdateEntry.merge`, `:257-262`)           | one frame                                                                                       | the app. The mode comes from `ViewRootImpl.getViewUpdateMode(view) = view.getDefaultUpdateMode()` (`:4354-4359`), taken from the _invalidated descendant_ on the `onDescendantInvalidated` path (`:13369-13370`) and from the _root_ view elsewhere (`:3795`, `:4700`, `:13101`, `:13119`, `:13129`). When nothing was invalidated, `createDummyEntryList(mView)` sends one full-screen entry with the root view's mode (`ViewUpdateHelper.java:514-517`) |
| **App-scope standing override** | `applyAppScopeUpdate(pkgHash, enable, clearFlag, epdMode, repeatLimit)` SF `16711684`; `clearAppScopeUpdate(flag)` SF `1048640`                                         | **one global slot**, despite the name — the package hash is stored at `UpdateEntry+0x04` and never read for any decision (§5.4.3) | until explicitly cleared; process death clears nothing, and `repeatLimit` has no consumer in SF | **EinkWise / `OECService` only.** `TabletEACRefreshImpl.applyAppScopeUpdate` passes `repeatLimit = Integer.MAX_VALUE` (`impl/TabletEACRefreshImpl.java:69`)                                                                                                                                                                                                                                                                                               |
| **Transient standing override** | `applyTransientUpdate(epdMode)` SF `16711782`; `clearTransientUpdate(clearFlag)` SF `16711783`                                                                          | **a second global slot**, also unkeyed                                                                                            | until explicitly cleared                                                                        | `ScrollHelper` (`ScrollHelper.java:57-83`) — and our app during a lasso drag                                                                                                                                                                                                                                                                                                                                                                              |

`ViewUpdateHelper.getFastModeIndex()` (SF `IS_IN_FAST_MODE = 1048656`, `ViewUpdateHelper.java:169`,
`:702-709`) reports SF's own view of this: `0` normal, `1` system fast, `2` app fast
(`:829`, `:836-842`). It is the cheapest on-device probe of who currently owns the panel;
`dumpsys SurfaceFlinger`'s `"### APP_SLOT"` / `"### TRANSIENT_SLOT"` lines
(`FixedUpdateEntryManager::dump()` @ `0x557764`) are the detailed version.

### 5.2 The three profiles on this device, resolved

`noteair4c_systemui.json` (verbatim, `/home/okhsunrog/tmp_zfs/onyx_framework/fwres/x/res/raw/`):

```json
"refreshConfigMap": {
  "refresh_mode_1": { "mode": 5, "title": "REGAL_PLUS" },
  "refresh_mode_2": { "mode": 2, "turbo": 5, "title": "NEW_SPEED" },
  "refresh_mode_4": { "mode": 0, "title": "HD" }
},
"refreshConfig":            ["refresh_mode_1", "refresh_mode_2"],
"refreshConfigForSystemApp":["refresh_mode_4", "refresh_mode_2"],
"refreshMigrateMap":            { "0": 5, "1": 2, "2": 2, "3": 5, "4": 2 },
"refreshMigrateMapForSystemApp":{ "0": 0, "1": 2, "2": 2, "3": 5, "4": 2 }
```

`EACUtils.toEpdMode` (`EACUtils.java:127-142`): `0→5 AUTO`, `1→2305 DU|DITHER|Y1`,
`2→2308 ANIM|DITHER|Y1`, `3→6 REAGL`, `4→16777220 ANIM|DITHER_X`, `5→9 REAGL_PLUS`.

| Profile                       | `mode`   | `turbo` | Branch taken in `applyUpdateMode` | Transactions actually sent, per resume                                                                                                                                                                                                                                                               |
| ----------------------------- | -------- | ------- | --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `refresh_mode_1` "REGAL_PLUS" | 5        | 0       | `{0,3,5}`                         | `useGCForNewSurface(true)`; `clearAppScopeUpdate(2)`; `setEpdTurbo(0)`; `antiFlicker(10)`; `enableDither(ditherBitmap)`; `debouncer(true, toEpdMode(0)=5, shortDelay, longDelay, gcInterval)`; on every input-up `enableRegal(true)` + `setDebouncerTransientUpdateMode(5)` + `debounceIncRefresh()` |
| `refresh_mode_2` "NEW_SPEED"  | 2 (A2)   | **5**   | `{1,2,4}`                         | `useGCForNewSurface(true)`; `debouncer(false,0,0,0,0)` (**clears the debouncer**); `applyAppScopeUpdate(null/pkg, true, flag, **2308**, MAX_VALUE)`; `setEpdTurbo(5)`; `antiFlicker(10)`; `enableDither(false)` — forced, because `2 ∉ DEBOUNCER_UPDATE_MODE_MAP`                                    |
| `refresh_mode_4` "HD"         | 0 (AUTO) | 0       | `{0,3,5}`                         | as `refresh_mode_1` but the app-scope mode is AUTO and turbo 0                                                                                                                                                                                                                                       |

`shortDelay = max(EACConfig.minAnimationDuration = 80, animationDuration || 10)`;
`longDelay = clamp(shortDelay*3, 400, 1200)` (`impl/TabletEACRefreshImpl.java:87-92`, `EACUtils.java:57-59`);
`gcInterval` from `obtainLegalGcInterval()`.

Two independent downgrades apply on this panel and are unchanged from
`docs/onyx-reversing/05-panel-refresh-levers.md` §1.5: `refreshMigrateMap` rewrites the raw
`updateMode` when no valid index is stored, and REGAL is refused twice
(`ViewUpdateHelper.supportRegal()` is false, and `EACBaseRefreshImpl.getConfigUpdateMode` maps mode 3
→ 0, `impl/EACBaseRefreshImpl.java:118-132`).

### 5.3 What the profile switches on and off besides the waveform

`Constant.DEBOUNCER_UPDATE_MODE_MAP = {0→0, 3→0, 5→0}` (`Constant.java:391-397`) is the master gate.
Everything below is **off** whenever the resolved mode is 1, 2 or 4 — i.e. under `refresh_mode_2`:

| Mechanism                                                              | Code                                                                                          | Gate                                                  |
| ---------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| SF debouncer (the post-input quality repaint)                          | `EACUtils.applySFDebouncer` (`:34-41`) → SF `DEBOUNCER 16711780`                              | resolved mode ∈ map                                   |
| Debounced transient mode on input-up                                   | `EACUtils.applyDebouncerTransientUpdateMode` (`:26-32`) → `enableRegal(true)` + SF `16711785` | resolved mode ∈ map                                   |
| Counted auto-GC                                                        | `EACBaseRefreshImpl.increaseRepaintCount` (`:47-52`) → SF `16711781`                          | **raw `updateMode`** ∈ map                            |
| Dithering                                                              | `TabletEACRefreshImpl.applyDither` (`:72-79`) → SF `ENABLE_DITHER 1048692`                    | **raw `updateMode`** ∈ map **and** `appConfig.enable` |
| `ScrollHelper` (transient A2 during a drag, GC after)                  | `ScrollHelper.java:93-95`, `EACUtils.isFastMode` (`:68-70`)                                   | skipped when the resolved mode is 1, 2 or 4           |
| `GC_INTERVAL`, `ANIMATION_DURATION`, `PEN_EAC_REPAINT_LATENCY` UI rows | `EACViewConfigs.showByType`/`isEnableByType` (`:900-916`)                                     | raw `updateMode` ∈ {0,3}                              |

Note the inconsistency, which is real and exploitable: three of these read the **raw `updateMode`
field**, while the waveform itself is decided by the **resolved `refreshModeIndex`**. Writing a
`refreshModeIndex` without also writing a matching `updateMode` leaves dithering and the counted
auto-GC keyed off the stale integer.

### 5.4 Precedence: view mode vs app-scope vs transient — decoded from the binary

This is settled, not inferred. Symbols come from `.gnu_debugdata`; addresses are file/virtual.

#### 5.4.1 The arbiter

Every EPDC schema's `output()` builds an `android::UpdateEntryDetector` (a `vector<UpdateEntry>`,
stride `0x30`), `add()`s candidate entries in a fixed order, and calls
`UpdateEntryDetector::output()` @ `0x556b90` to pick one. `add()` @ `0x556950` is a plain
`push_back` — all the logic is in `output()`:

```
best = none
for entry in insertion order:
    if entry.mode <= 0:                                   continue        # -1 / 0 ignored
    if entry.mode in {0x10, 0x20, 0x40, 0x100020}:        return entry    # accept, STOP
    if entry.mode in {0x62 (98, UI_GC_MODE), 0x6c (108)}: best = entry; break
    best = entry                                                          # otherwise LAST WINS
return best  (or the null entry, mode -1)
```

So: **last valid entry wins, except that an `AUTOMATIC`/`FULL`/`WAIT` flag entry and `UI_GC_MODE`
(98) / `UI_DEEP_GC_MODE` (108) short-circuit the scan.**

#### 5.4.2 The `add()` order, per schema

| Schema                                                                       | `add()` order                                                                             | Who wins                                                                                                    |
| ---------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| **`HWNormalSchema::output` @ `0x566cf8` — the one this HW-Tcon device uses** | `filterLayer` → `next()` → `layerChangeUpdate` → `bypassUpdate` → **`fixedUpdateEntry`**  | **the app-scope / transient slot is added LAST, so it beats the layer's own mode**                          |
| `NormalSchema::output` @ `0x55b37c` (software path)                          | `filterLayer` → `layerChangeUpdate` → `next()` → `bypassUpdate` → `fixedUpdateEntry`      | same                                                                                                        |
| `DebouncerSchema::output` @ `0x55bf18`                                       | `filterLayer` → `layerChangeUpdate` → `fixedUpdateEntry` → `next()` → `debouncerUpdate()` | the debouncer and the one-shot beat the app-scope slot                                                      |
| **`HandwritingSchema::output` @ `0x55b828`**                                 | `hasHWRepaint(list)`, then `next()` only                                                  | **`fixedUpdateEntry` is never consulted — app-scope and transient are inert inside the handwriting schema** |
| `BootSchema`, `DreamSchema`, `HWDreamSchema`                                 | no `fixedUpdateEntry`                                                                     | app-scope ignored                                                                                           |

Candidate sources: (a) the per-view/per-buffer mode arrives through
`EpdcManager::filterLayer` @ `0x5591d0` and `layerChangeUpdate` @ `0x559214` →
`HWEpdcManager::getNewLayerUpdateEntry` @ `0x5664c0`; (b)/(c) the two fixed slots via
`EpdcManager::fixedUpdateEntry` @ `0x559ab0` → `FixedUpdateEntryManager::activateEntry` @ `0x557458`;
(d) the per-call one-shot via `EpdcManager::next()` @ `0x55908c`, reading the single global
`UpdateEntry` at `0xac9dd0` and decrementing its count each frame.

**Important nuance:** the per-layer _waveforms_ also reach the panel by a second, unarbitrated route —
`EpdcManager::collectLayerEpdcList` @ `0x557d48` pulls the per-layer `hwc_epdc_llist` (rect + mode at
`+0x10`) straight out of the layer and commits it **alongside** the arbitrated entry, where
`EpdcWrapper::mergeByMode` batches by waveform. So a per-view mode is not erased; it is merged with,
and visually dominated by, whatever the fixed slot imposes.

#### 5.4.3 The two fixed slots

`android::FixedUpdateEntryManager` is embedded at `EpdcManager + 0x1e8` and holds exactly **two**
slots (ctor @ `0x556e94`), logged by `dump()` @ `0x557764` as `"### APP_SLOT"` (`0x315fbd`) and
`"### TRANSIENT_SLOT"` (`0xdedd4`):

| Slot                | Written by                                                                                                                      | Java                                                  |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| **0 — application** | `EpdcManager::applyAppScopeUpdate` @ `0x558bb4` → `FixedUpdateEntryManager::apply(**0**, hash, enable, clearFlag, mode, count)` | `ViewUpdateHelper.applyAppScopeUpdate` SF `16711684`  |
| **1 — transient**   | `EpdcManager::applyTransientUpdateMode` @ `0x559924` → `apply(**1**, 0, 1, 0, mode, INT_MAX)`                                   | `ViewUpdateHelper.applyTransientUpdate` SF `16711782` |

`activateEntry()` @ `0x557458` walks the vector **from slot 0** and returns the first entry whose mode
is _fast_; otherwise a static null entry (mode −1). **Application scope therefore beats transient
scope**, and both beat the layer entry in the normal schema.

`UpdateEntry` field map, from `apply` @ `0x5570e4` plus the `dump()` format string at `0x135777`:
`+0x00 request`, `+0x04` **package-name hash**, `+0x08` **mode**, `+0x10 count`, `+0x14..0x20 rect`,
`+0x28 repeatMode`, `+0x2c repeatLimit`.

- **The package hash at `+0x04` is stored and copied but never read for any decision** — `dump()` does
  not even print it. There is no pid, uid, layer or binder token. Two global slots, last writer wins:
  **any app can overwrite or clear another app's app-scope entry.**
- **`repeatLimit` (the 5th argument, which the EAC layer passes as `Integer.MAX_VALUE`) has no
  consumer in this binary**, and `count` is never decremented for fixed entries — so the registration
  does not expire.
- **Process death clears nothing.** No `linkToDeath` on the Onyx path; `handleAppDie` (`0xff0025`,
  which has no Java caller anyway) does not touch `FixedUpdateEntryManager`.
- **Both clears are gated on the slot already holding a fast mode**: `clearAppScopeUpdate` is a no-op
  unless `inApplicationFastMode()`, `clearTransientUpdate` unless `inTransientFastMode()` (the latter
  additionally pushes a full-screen `addEpdc(0,0,W,H,5)`).
- **Every one of these entry points is silently dropped while the panel is powering off**:
  `if ((unsigned)(EpdcManager[+0x24] - 1) < 3) return;` at the head of `applyAppScopeUpdate`
  (`0x558be0`), `clearAppScopeUpdate` (`0x558ee0`), `applyTransientUpdateMode` (`0x559944`),
  `clearTransientUpdateMode` (`0x559a08`), `handleRefreshScreen` (`0x561060`), `startDebouncer` and
  `debouncerRefresh`.

#### 5.4.4 The "fast mode" whitelist — a slot can only impose a fast waveform

`isFastMode(m)` is an exact-equality whitelist, replicated verbatim in `activateEntry`, `inFastMode`
(`0x5576b8`), `apply`, `applyDither` (`0x559304`), `HWEpdcManager::setFBSchema` (`0x565c78`) and
`syncFBSchema` (`0x565f74`):

| Value                  | Name                                                   |
| ---------------------- | ------------------------------------------------------ |
| `1`                    | `UI_DU_MODE`                                           |
| `4`                    | `UI_A2_PERFORMANCE_MODE`                               |
| `0x901` = 2305         | `UI_DU_QUALITY_MODE`                                   |
| `0x904` = **2308**     | `UI_A2_QUALITY_MODE` ← what `refresh_mode_2` registers |
| `0x908` = 2312         | `UI_DU4_MODE`                                          |
| `0x1000001` = 16777217 | `UI_X_DU_MODE`                                         |
| `0x1000004` = 16777220 | `UI_X_A2_MODE`                                         |
| `0x2000004` = 33554436 | `UI_MONO_A2_MODE`                                      |

Everything else — `UI_GU_MODE` (2), AUTO (5), REGAL (6), REGAL_PLUS (9), `UI_GC_MODE` (98),
`HAND_WRITING_REPAINT_MODE` (524290) — is **stored in the slot but never returned by
`activateEntry`**, i.e. an app-scope or transient registration with a non-fast mode is inert (and,
because the clears are gated on `inFastMode`, it also cannot be cleared). This is the SF-side twin of
`EACUtils.isFastMode(mode) = {1, 2, 4}` on the EAC side, and it is exactly why
`TabletEACRefreshImpl.applyUpdateMode` calls `clearAppScopeUpdate` for modes 0/3/5 rather than
registering them.

#### 5.4.5 What registering a fast mode _actually_ changes

`applyUpdateAction` @ `0x558ce8` returns immediately when the mode and dither class are unchanged
(`0x558d3c`) — re-registering the same profile is free. Otherwise it resolves a `clearType` from the
`CLEAR_FLAG` and calls `HWEpdcManager::setFBSchema(mode, changeDither, clearType)` @ `0x565c78`,
which:

1. `applyDither(mode)` @ `0x559304`;
2. `setEbcUpdateScheme(isFastMode(mode) ? **4** : **2**, 0)` — `/dev/ebc` ioctl `0x7006`. **This is the
   real mechanism of a "profile": a global panel update-scheme switch**;
3. `clearScreen(clearType)`;
4. `EnhanceStrategy::setEnable(enhance && isFastMode(mode))` — so enhancement is only on under a fast
   profile.

`applyDither(mode)` is the surprise: **a plain A2 or DU app-scope mode does not bypass dithering, it
inverts it.**

```
setMono(false);
if (!isFastMode(mode))  { setDither(false); return mode; }                  // GPU dither off
if (mode & 0x2000000)   { mode &= ~0x900; setDither(false); setMono(true);  return mode; }
if (mode & 0x1000000)   { mode &= ~0x900; setDither(true);                  return mode; }  // X modes: GPU dither
                          mode |=  0x900; setDither(false);                 return mode;    // plain A2/DU
```

`0x900 = EINK_DITHER_MODE_DITHER | EINK_DITHER_COLOR_Y1`. So a plain fast mode turns the
RenderEngine dither **shader** off and forces **1-bit panel-side Y1 dithering on**. Only the `X`
modes keep the GPU shader dither; only `UI_MONO_A2_MODE` switches to mono. `dump()` reports this as
`isCPUDither = (mode & 0x900) != 0`, `isGPUDither = bit24`.

This is **not** in conflict with §5.3's "`refresh_mode_2` forces `enableDither(false)`": they are two
different knobs. `ViewUpdateHelper.enableDither` (SF `1048692`) sets the global `EpdImageHandler`
shader flag, which is what `TabletEACRefreshImpl.applyDither` drives from the stored config; the
`0x900` bits above are the _panel's_ own dither, set implicitly by the mode word. Under
`refresh_mode_2` the EAC layer turns the shader off, and SF's `applyDither` turns it off again while
switching the panel to 1-bit Y1 — a plausible mechanical explanation for coarse-looking greys in A2.

#### 5.4.6 `CLEAR_FLAG` — and the free full-GC hiding in `clearTransientUpdate`

`apply()` returns a 6-field `UpdateAction {beforeMode, afterMode, beforeDither, afterDither, value,
clearFlag}`. `value` is picked from a **per-slot `ClearScreenAction` triple** by the transition, and
the triples are chosen in the ctor by `property_get_bool("vendor.onyx.htcon")` — true here:

| Transition      | app slot (HW-Tcon) | transient slot (HW-Tcon) |
| --------------- | ------------------ | ------------------------ |
| none → fast     | `1`                | `0`                      |
| fast → fast     | `1`                | `1`                      |
| **fast → none** | **`8`**            | **`8`**                  |

`applyUpdateAction` then maps `clearFlag`: `2 NEW_SURFACE` → stash `value` in `0xac9e38` and set
`EpdcManager+0x30 = 3` (applied at the next new layer); `1 IMMEDIATELY` → `clearType = value`;
`0 NONE` → `clearType = 0`, nothing happens. `HWEpdcManager::clearScreen(t)` @ `0x565bf8` maps
`t = 8` → **`applyGCOnce()` (mode 98)**, and otherwise looks `t` up in `{1→0x20 FULL, 2→0x10 AUTO,
3→0x40 WAIT, 4→0x80}`.

Two directly useful consequences for us:

- **`clearTransientUpdate(true)` fires a full GC flash; `clearTransientUpdate(false)` does not.**
  That is exactly what `ScrollHelper` exploits with `clearTransientUpdate(gcAfterScrolling)`
  (`ScrollHelper.java:68-83`), and it means we can get a de-ghosting GC for free at the end of a lasso
  drag instead of scheduling a separate `invalidate(GC)`.
- **Leaving the A2 profile costs one full GC**: the app slot's `fast → none` action is `8`. So the
  one-off switch from `refresh_mode_2` to `refresh_mode_4` will produce a single visible flash, and
  every subsequent app switch away from an A2-profiled app already does.

#### 5.4.7 Waveform acceptance on this panel, and the paths that bypass arbitration

`FBDev::hwScreenRefresh` @ `0x55b074` — the HW-Tcon colour path this device uses — switches on
`mode & 0xF` through a **6-entry** table (`0x340e45`):

| `mode & 0xF` | waveform sent to `/dev/ebc` ioctl `0x700c`                                                   |
| ------------ | -------------------------------------------------------------------------------------------- |
| 1 DU         | 1                                                                                            |
| 2 GC16       | 2                                                                                            |
| 3 GC4        | 3                                                                                            |
| 4 A2/ANIM    | 4                                                                                            |
| **5 AUTO**   | **255** — SF has no DU-vs-GC16 heuristic; the kernel EPDC driver picks                       |
| 6 REGAL      | 5 (+ flag `0x8000` when bit 12 `REAGL_D` is set)                                             |
| **0, 7…15**  | `"onyx_epdc_hwscreenRefresh(): waveform_mode wrong!"` (`0x1d161a`), **waveform forced to 0** |

The software path `FBDev::refreshScreen` @ `0x55a588` has a 13-entry table and additionally accepts
11 (GCC16) and 12 (DEEP_GC16), rejecting 7–10 with `"onyx_epdc_update_to_display(): waveform_mode wrong"`.
On **this** device that means:

| EAC mode         | `toEpdMode` | nibble | accepted?                                              |
| ---------------- | ----------- | ------ | ------------------------------------------------------ |
| 0 NORMAL/AUTO    | 5           | 5      | **yes** → waveform 255                                 |
| 1 DU             | 2305        | 1      | yes                                                    |
| 2 A2             | 2308        | 4      | yes                                                    |
| 3 REGAL          | 6           | 6      | yes (but mode 3 is rewritten to 0 before it gets here) |
| 4 X              | 16777220    | 4      | yes                                                    |
| **5 REGAL_PLUS** | **9**       | **9**  | **no — waveform forced to 0**                          |

So **`refresh_mode_1` ("REGAL*PLUS"), the \_quality* profile EinkWise offers third-party apps on this
device, asks the panel for a waveform its own colour path rejects.** The same test condemns
`UI_GCC_MODE` (107, nibble 11) and `UI_DEEP_GC_MODE` (108, nibble 12) as arguments to
`repaintEverything(mode)` here — **do not use them**. `UI_GC_MODE` (98, nibble 2) is the strongest
full flash that survives.

Finally, two paths **bypass the arbitration entirely**:

- `REFRESH_SCREEN` (`16711681`) → `FBDev::refreshScreen` directly (`0x5650a0`), i.e. an immediate
  out-of-band panel update with the caller's mode, not arbitrated against the fixed slots at all.
- `UI_GC_MODE` (98) as a _per-frame_ entry short-circuits `UpdateEntryDetector::output()` (§5.4.1),
  so **our `EpdController.invalidate(webView, UpdateMode.GC)` de-ghosting wins over EinkWise's
  app-scope pin regardless of profile.**

`HAND_WRITING_REPAINT_MODE = 0x80002` is `GC16 | (1 << 19)`; `FBDev::refreshScreen` copies bit 19 into
the ioctl flags word (`0x55a6ac`) untouched, and `UpdateEntryDetector::hasHWRepaint` @ `0x556b48`
scans the layer list for exactly that value. `EpdcManager::setHandWritingRepaintUpdate` @ `0x55828c`
writes it into the **global one-shot entry** at `0xac9dd0` with `count = 1`.

**Correction to `REPORT-surfaceflinger.md` §3:** that report reads `EpdcManager+0x24` as a panel-class
field in `canScribble()`. It is the **display power state** (`"### epdUpdate %d isStandby %d isPowerOff %d"`
@ `0x10c566`, `isPowerOff = ((state-1) < 3u)`, initialised to −1 at `0x55ec38`). `canScribble()` @
`0x55915c` is `schema == 7 && !standby && !poweringOff`. Colour-panel detection is separate
(`isColorDevice` @ `0x55952c`, `getCfaMode` ioctl `0x7021`, `isHWTconColorDevice` @ `0x559614`).

#### 5.4.8 Turbo, dither and colour are `/dev/ebc` ioctls, and all global

SF never touches sysfs; the only EPDC node is **`/dev/ebc`** (`0x1d1611`), opened once in
`FBDev::openEpdcFd` @ `0x55a0cc`. `FBDev::setEpdTurbo` @ `0x55ac10` is `ioctl(fd, **0x7024**, &v)` and
nothing else; `HWEpdcManager::setEpdTurbo` @ `0x5662fc` stores it in one atomic word at
`EpdcManager+0x258`. There is **no per-package or per-pid dimension**. Likewise
`setDitherThreshold` = ioctl `0x7211`, `setAntiFlicker` = `0x7210`, `enhance` = `0x720f`,
`setUpdListSize` = `0x7016`, `setEbcUpdateScheme` = `0x7006`, and every saturation / gamma /
brightness / mono / noise / colour-adjust transaction lands on the **single global**
`EpdImageHandler` at `0xac9fc0` as a RenderEngine shader uniform. Nothing there is keyed by package,
pid or layer.

---

## 6. Collision analysis — what our app does vs. what EinkWise does

Our package is `dev.okhsunrog.tangleaf` (`src-tauri/gen/android/app/build.gradle.kts:46,49`). It is
third-party by every test the firmware applies: not a system app, and it does not contain the
substring `"com.onyx"` that `ActivityManagerHelper.isOnyxApp` (`android/onyx/utils/ActivityManagerHelper.java:223-232`)
and `ApplicationUtil.isOnyxApp` (`out-kcb/.../sdk/utils/ApplicationUtil.java:655`) look for. So:
`supportEAC = true`, the third-party `refreshMigrateMap` `{0→5, 1→2, 2→2, 3→5, 4→2}` applies,
`refreshModeIndexDefault = "refresh_mode_2"` applies, `validateEnhance` does **not** force `enhance`,
and the Regal clamp in `validateRefreshConfig` does **not** apply to us.

| What we do                                                                                                 | Code                                                 | EinkWise's state                                                                                                                                                                                      | Verdict                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ---------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `view.set(BASE, UpdateMode.REGAL)` at startup                                                              | `MobileSystemPlugin.kt:245-251`                      | `refresh_mode_2` keeps a standing app-scope A2 pin (slot 0 = 2308)                                                                                                                                    | **overridden on ordinary frames** — in `HWNormalSchema::output` the fixed slot is `add()`ed last and `UpdateEntryDetector::output()` is last-wins (§5.4.1–2). Also `supportRegal()` is false and mode 6 is not what this panel runs. Dead weight                                                                                                                                                                                      |
| `view.set(SESSION, UpdateMode.GU)` for the ink editor                                                      | `OnyxInk.kt:572-576`                                 | same A2 pin                                                                                                                                                                                           | **overridden on ordinary frames; irrelevant during a stroke.** `HandwritingSchema::output` never consults `fixedUpdateEntry` (`0x55b828`), so once SF is in schema 7 the profile does not touch the ink path. Under `refresh_mode_4` the slot is cleared and GU wins everywhere                                                                                                                                                       |
| `view.set(TRANSIENT, HAND_WRITING_REPAINT_MODE)` per reconcile                                             | `OnyxInk.kt:135-142`                                 | same A2 pin                                                                                                                                                                                           | **orthogonal in practice.** `HAND_WRITING_REPAINT_MODE = 0x80002` is what `UpdateEntryDetector::hasHWRepaint` (`0x556b48`) matches on inside the handwriting schema, where the fixed slots are not consulted. On a frame that lands in the _normal_ schema, the slot wins                                                                                                                                                             |
| `EpdController.applyTransientUpdate(ANIMATION_QUALITY)` on lasso drag; `clearTransientUpdate(false)` after | `OnyxInk.kt:146-157`                                 | EinkWise never sets a transient update itself, and `ScrollHelper` — the only framework user — is skipped entirely under a fast profile (`ScrollHelper.java:93-95`)                                    | **redundant today, and it loses the arbitration anyway.** `activateEntry` scans slot 0 before slot 1, so EinkWise's app-scope 2308 beats our transient 2308 — and both are the same mode. It is still SF-side standing state that nothing in EinkWise clears on app switch, so the defensive clear in `OnyxInk`'s constructor (`OnyxInk.kt:243-245`) stays necessary. Note the clear is itself a no-op unless `inTransientFastMode()` |
| Drive the pen session directly (`TouchHelper`, `setScreenHandWritingPenState`, `leaveScribbleMode`)        | `OnyxInk.kt:640-650`, `RawDrawingGate.kt`            | `noteConfig.supportNoteConfig = false` for us, so `EACScreenNoteManager` never attaches (`EACScreenNoteManager.java:155`) and the system's own 500 ms post-stylus-up `handwritingRepaint` never fires | **orthogonal.** EinkWise's screen-note pipeline is inert for third-party apps that are not in `presetAppConfig`                                                                                                                                                                                                                                                                                                                       |
| `EpdController.invalidate(webView, UpdateMode.GC)` for de-ghosting                                         | `MobileSystemPlugin.kt:84`, `:348`; `OnyxInk.kt:713` | none                                                                                                                                                                                                  | **orthogonal and reliable.** `UI_GC_MODE = 98` (`0x62`) is one of the two values that **short-circuit** `UpdateEntryDetector::output()` (§5.4.1), so it beats the app-scope pin under any profile; and nibble 2 passes the HW-Tcon waveform check                                                                                                                                                                                     |
| `ensureSpeedRefreshProfile()` writes `refreshModeIndex/updateMode/turbo`                                   | `MobileSystemPlugin.kt:264-292`                      | the factory default is already exactly `refresh_mode_2` + `updateMode 2`                                                                                                                              | **no-op today.** The early return at `:279-282` fires on the first call and every call after                                                                                                                                                                                                                                                                                                                                          |
| `applyAppRefreshProfile()` → `EpdController.setAppScopeRefreshMode(FAST)`                                  | `MobileSystemPlugin.kt:294-322`                      | `caculateRefreshConfig` prefers the stored index                                                                                                                                                      | **worse than a no-op — see §6.1**                                                                                                                                                                                                                                                                                                                                                                                                     |
| Full-refresh scheduling in the frontend                                                                    | `src/app/eink-refresh.ts`                            | with `refresh_mode_2` the firmware's own counted auto-GC and post-input repaint are off, so ours is the only cleanup running                                                                          | **orthogonal, and currently load-bearing.** Under `refresh_mode_4` the firmware's `gcInterval` counter comes back and ours becomes partly redundant                                                                                                                                                                                                                                                                                   |

### 6.1 `setAppScopeRefreshMode(FAST)` is not inert — it writes device-wide state

`OECService.setAppScopeRefreshMode(int)` → `TabletEACRefreshImpl.setAppScopeRefreshMode`
(`impl/TabletEACRefreshImpl.java:263-276`) does four things, only the first of which is the one we
wanted and none of which works the way our comment assumes:

```java
EACAppConfig cfg = deviceConfig.getAppConfigByComponentName(topComponent);  // the THEME's config
cfg.setUpdateMode(topComponent, i);                                          // updateMode only
deviceConfig.getAppConfigMap().put(cfg.getPkgName(), cfg);
deviceConfig.getFallbackRefreshConfig().setUpdateMode(i);                    // <-- DEVICE-WIDE
setAppRefreshModeImpl(...);                                                  // resolves via the index
BroadcastHelper.sendRefreshModeChangeBroadcast(...);
saveDeviceConfig(deviceConfig, 2);                                           // persists both
sendOECConfigChanged(topComponent, true, 3);                                 // op flag 3 into our process
```

- `getFallbackRefreshConfig()` is `extraConfig.appDefaultConfig.globalActivityConfig.refreshConfig`
  (`data/v2/EACDeviceConfig.java:63-65`) — the **device-level fallback** used for any app without its
  own config. We are writing `updateMode = 2` into it and persisting it to
  `/onyxconfig/mmkv/onyx_config`. That is a change that outlives our process, survives a reboot, and
  is not scoped to us.
- The mode we set is then discarded by `caculateRefreshConfig`, because the stored `refreshModeIndex`
  resolves. And `getAppScopeRefreshMode()` reads back through the same resolver, so it can never
  report what we wrote.
- `sendOECConfigChanged(..., 3)` makes `EACConfigChangedImpl.onRefreshModeChanged` run **in our own
  process** (`EACConfigChangedImpl.java:183-190`), which calls `EInkHelper.fetchAppConfig` and may
  `dispatchFullRedraw`.
- `OECService.setAppScopeRefreshMode` is additionally dropped outright when
  `getCurrentTopComponent() == null` (`OECService.java:1483-1485`, `:1761-1778`), so it is silent on
  failure in both directions.

### 6.2 What a write to the EAC config does to our own process

Any successful `applyAppConfigToService` for our package makes `OECService` broadcast
`onyx.action.oec.config.change`, and `shouldSendConfigChangedBroadcast` returns `true` for us
unconditionally (it can only return `false` when the _new_ config has `supportEAC == false`,
`OECService.java:906-908`). AMS then calls into our process
(`out-services/.../am/ActivityManagerService.java:4043`), and `EACConfigChangedImpl` runs with the
`args_operation_flag` we put in the `Bundle`:

| flag                                                                                                          | Effect in our process                                                                                                                                                                                                                                                                                                               |
| ------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0` (what we send, `MobileSystemPlugin.kt:286`)                                                               | `onDefaultViewConfigChanged`: re-reads DPI / rotation / paint around a `fetchAppConfig`. Unchanged DPI+rotation → `dispatchFullRedraw(token, false)`. **Changed rotation → `mActivityThread.restartPackage(pkg)` — our app is restarted.** Changed DPI → a synthetic `Configuration` is pushed into every registered window context |
| `1`                                                                                                           | every `WebView` in the process is `reload()`ed / `loadUrl(url)`-ed                                                                                                                                                                                                                                                                  |
| `2` (what EinkWise's **reset** button sends, `out-kcb/.../eac/rx/action/EACAppConfigChangeAction.java:75-77`) | both of the above                                                                                                                                                                                                                                                                                                                   |
| `3` (what `setAppScopeRefreshMode` sends)                                                                     | `dispatchFullRedraw` if the fast-mode flag flipped                                                                                                                                                                                                                                                                                  |

We round-trip the JSON through `org.json.JSONObject`, which preserves unknown keys, so the rotation
block should survive — but `EACRotationConfig.overrideRotation` defaults to `-1` and `EACDpiConfig.dpi`
defaults to the _system density_, not `0` (`data/v2/EACDpiConfig.java:7`). If a future edit ever
constructs the config rather than round-tripping it, the failure mode is a package restart or a
resolution change, not a bad waveform.

### 6.3 Ordering and timing that matter

1. **Write the profile before the ink session, never during one.** `applyAppConfigToService` →
   `onApplyConfig` → `applyUpdateMode` can emit `whiteClear(2, true)` (on `INVALID_MODE`) and always
   emits `useGCForNewSurface(true)`, `setEpdTurbo`, `antiFlicker`, `enableDither` and a debouncer
   reconfigure. Any of those landing mid-stroke is a visible glitch. Today we call
   `ensureSpeedRefreshProfile()` from `applyDisplayProfile()` (`MobileSystemPlugin.kt:257`), i.e. at
   startup — correct, and it should stay there.
2. **`onResume` re-pushes the whole profile on every top-component change**, and the top component
   includes the class name (`OptimizationBundle.isTopComponentChanged`, `bundle/OptimizationBundle.java:79-83`).
   A single-activity Tauri app is only re-pushed when the user leaves and returns — but a dialog
   Activity, a permission prompt or the share sheet all count. Anything SF-side and transient that we
   set (turbo, transient update) must be re-asserted after such a round trip, or dropped.
3. **`configDebouncerBySysWindowChanged`** fires on every SystemUI dialog/notification-shade
   open/close (`OECService.java:150-163`) and reconfigures the SF debouncer from our stored config
   (`impl/TabletEACRefreshImpl.java:190-201`). Under a 0/3/5 profile that is a real, frequent
   re-push; under `refresh_mode_2` it is a no-op because the gate rejects the mode.
4. **Our read is from the theme, our write is to the app-config slot** (§4.3). A profile change is
   applied immediately but may not survive the next app switch until the following boot. Re-assert on
   every `setDisplayProfile` (we already do) and verify with a read-back.
5. **`applyTransientUpdate` has no owner and no timeout.** Nothing in the firmware clears it on app
   switch. Our 5 s quiet-period reset (`InkRefreshPolicy.QUIET_MS`) and the constructor's defensive
   `clearTransientUpdate(false)` are the only things that do — keep both.
6. **Never call `applySystemFastMode`, `switchToA2Mode` or `toggleA2Mode`.** They are
   `EInkHelper.setAppScopeRefreshMode(2/0)` in disguise (`ViewUpdateHelper.java:1352-1366`), i.e.
   §6.1 with a friendlier name, and the "off" route goes through `clearSysScopeUpdate()`, whose
   framework body is empty (`ViewUpdateHelper.java:401-402`, `:484-486`).

---

## 7. Recommendation

### 7.1 Should we write the stored profile at all?

**Yes — once, at startup, and to a different value than today.** The stored `refreshModeIndex` is the
only lever that changes what the _firmware_ does for us, and the value we currently write is the
factory default, so today's write is provably inert. Leaving it at `refresh_mode_2` costs us five
mechanisms at once (§5.3): the SF debouncer, the debounced post-input transient repaint, the counted
auto-GC, dithering, and `ScrollHelper` — and, if §5.4's inference holds, it also pins our whole
package to A2 in SurfaceFlinger, which overrides the per-view `GU` and `HAND_WRITING_REPAINT_MODE`
modes the ink editor carefully sets.

**Which profile.** `refresh_mode_4` (`{mode: 0, turbo: 0}`, "HD"/AUTO). Not `refresh_mode_1`:
its `mode 5` resolves to `toEpdMode = 9 = REAGL_PLUS`, and nibble `9` is outside the 6-entry table
`FBDev::hwScreenRefresh` (`0x55b074`) accepts on this HW-Tcon colour panel — it is logged as
`"waveform_mode wrong!"` and forced to `0` (§5.4.7). The _quality_ profile EinkWise offers
third-party apps here asks the panel for a waveform its own colour path rejects.

Expect **one full GC flash** when the switch takes effect: leaving a fast app-scope mode maps to
clear action `8` → `applyGCOnce()` (§5.4.6). That is a one-off, and the same flash already happens
every time you switch away from an A2-profiled app.

`refresh_mode_4` is listed under `refreshConfigForSystemApp`, not `refreshConfig`, but those two
arrays are **UI picker lists only**: `ValidateAppConfigAction.validateRefreshConfig`
(`action/ValidateAppConfigAction.java:167-175`) returns early for _any_ index that resolves in
`refreshConfigMap`, and the Regal clamp on the line above is guarded by `isOnyxOrSystemApp`, which is
false for us. **[inference: therefore a third-party app can select HD by writing `refresh_mode_4`.
This is the one place a "system-app-only" enforcement could exist somewhere I did not find, so it is
experiment #2 below.]** Note that this is a disagreement with the parallel reading that the migrate
map (`0 → 5` for third-party apps) would rewrite it: that map is only consulted in the _fall-through_
branch, i.e. when no valid index is stored.

Write `updateMode: 0` and `turbo: 0` alongside the index. They are decorative for the waveform, but
`applyDither` and `increaseRepaintCount` read the **raw `updateMode` field**
(`impl/TabletEACRefreshImpl.java:72-79`, `impl/EACBaseRefreshImpl.java:47-52`), so leaving `2` there
keeps dithering and the counted auto-GC switched off even under an AUTO profile.

### 7.2 What to leave alone

| Leave alone                                                                                                                                                                                                                                                | Why                                                                                                                                                                                 |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `EInkHelper.setAppScopeRefreshMode` / `setTurbo` / `setAntiFlicker` / `setGcInterval` / `setGlobalContrast` / `setCFAColor*` / `setDitherThreshold` / `setEnhance`                                                                                         | every one of them also writes `deviceConfig.getFallbackRefreshConfig()` or the device extra config and persists it — **device-wide state written under our name** (§6.1)            |
| `EInkHelper.applyRefreshConfigToAll(String)`                                                                                                                                                                                                               | rewrites the refresh block of **every installed app** (`OECService.java:296-309`)                                                                                                   |
| `ViewUpdateHelper.enableDither`, `setDitherThreshold`, `applySaturation*`, `applyGammaCorrection`, `applyBrightness`, `setBWMode`, `setGrayscaleMode`, `antiFlicker`, `setEnhanceStrategy`, `useGCForNewSurface`, `setGcRefreshInterval`, `setUpdListSize` | all SF-**global**, not scoped to our app, and EinkWise re-asserts its own values for whichever app is foreground on the next resume — so ours would leak until then and then vanish |
| `setVCom`                                                                                                                                                                                                                                                  | panel damage risk                                                                                                                                                                   |
| `applySysScopeUpdate` / `clearSysScopeUpdate` / `setUpdateScheme`                                                                                                                                                                                          | framework bodies are empty (`ViewUpdateHelper.java:401-402`, `:484-486`, `:1298-1300`)                                                                                              |
| `repaintEverything(107)` / `(108)`                                                                                                                                                                                                                         | nibbles 11 and 12 are rejected by `hwScreenRefresh` on this colour panel and forced to waveform `0` (§5.4.7). `UI_GC_MODE` (98) is the strongest full flash that survives           |
| `noteConfig` in our EAC config                                                                                                                                                                                                                             | leaving `supportNoteConfig = false` is what keeps `EACScreenNoteManager` out of our pen session                                                                                     |
| `rotationConfig`, `dpiConfig` in the JSON we write back                                                                                                                                                                                                    | a changed rotation block makes the framework **restart our package** in our own process (§6.2)                                                                                      |

### 7.3 What to restore on exit

Everything process-local (`View.setDefaultUpdateMode`) dies with us and needs nothing. Everything
SF-side is global or unowned and **must** be undone explicitly, because SurfaceFlinger has no binder
death recipient for the Onyx path (`REPORT-surfaceflinger.md` §2.3):

- `clearTransientUpdate(false)` — nothing else ever clears it.
- `setEpdTurbo(0)` if we raised it.
- pen state `PEN_PAUSE` (3) — this is the only call that runs `stopHandwriting` → schema restore →
  `androidDrawing = 1`.
- `setScreenHandWritingRegionExclude(null, /* empty array */)` — the exclude table is a process-global
  singleton in SF.
- one `UI_GC_MODE` (98) repaint.

The **stored EAC profile should not be restored on exit**: it is per-package, it does not affect any
other app, and rewriting it on teardown would fire another config-changed broadcast into our own
process (§6.2) at exactly the wrong moment.

---

# Collisions with our app

1. **`ensureSpeedRefreshProfile()` has never changed anything.** `eac_noteair4c.json` sets
   `refreshModeIndexDefault: "refresh_mode_2"` and all three third-party themes use it, so our write
   target equals the factory value and the early return at `MobileSystemPlugin.kt:279-282` fires on
   the first call. Not a collision — a no-op.
2. **`applyAppRefreshProfile()` → `EpdController.setAppScopeRefreshMode(FAST)` writes device-wide
   state that outlives our process.** It sets `deviceConfig.getFallbackRefreshConfig().setUpdateMode(2)`
   — the fallback used for apps with no config of their own — and persists it to
   `/onyxconfig/mmkv/onyx_config` (`impl/TabletEACRefreshImpl.java:263-276`). The mode it sets for
   _us_ is then discarded by `caculateRefreshConfig`. This is the one thing we currently do that can
   leak into other apps. Delete it.
3. **EinkWise's stored `refresh_mode_2` registers `2308` in SurfaceFlinger's application slot, and
   that slot beats our per-view modes on every ordinary frame** — it is `add()`ed last into
   `UpdateEntryDetector`, which is last-wins (`HWNormalSchema::output` @ `0x566cf8`,
   `UpdateEntryDetector::output` @ `0x556b90`). It does **not** affect the ink path: the handwriting
   schema never consults `fixedUpdateEntry` (`0x55b828`). Registering a fast mode also switches the
   panel's EBC update scheme globally (`setEbcUpdateScheme(4)`, ioctl `0x7006`) and, via
   `applyDither` (`0x559304`), turns the RenderEngine dither shader **off** while forcing 1-bit Y1
   panel dithering **on** — so "A2 bypasses dithering" is the opposite of what happens.
4. **`refresh_mode_2` switches off five firmware mechanisms we would otherwise get for free**: the SF
   debouncer, the debounced post-input transient repaint, the `gcInterval`-counted auto-GC,
   `enableDither`, and `ScrollHelper`. `Constant.DEBOUNCER_UPDATE_MODE_MAP = {0,3,5}` is the gate.
   That is a complete mechanism for the accumulated ghosting we see during ordinary UI use, and it is
   why our frontend's navigation-counted full refresh is currently the only cleanup running.
5. **Our read and our write hit different MMKV keys.** `getAppConfigFromService` returns the _active
   theme's_ config (`data/v2/EACDeviceConfig.java:30-32`), while `applyAppConfigToService` stores into
   `appConfigMap` → `eac_app_<pkg>`. They are reconciled only by `ValidateAppConfigAction` at service
   start. Once a theme key exists for our package, a profile write is applied once and then goes
   dormant until the next boot. [inference on the ordering — experiment #4]
6. **`view.set(BASE, UpdateMode.REGAL)` is dead weight.** `supportRegal()` is false, mode 6 is not a
   waveform this panel runs, and the readback verification is unsound because
   `EpdController.getViewDefaultUpdateMode` returns `GU` both when the view holds GU and when the
   reflective read fails.
7. **Any EAC write we make reaches back into our own process.** `shouldSendConfigChangedBroadcast`
   is unconditionally true for a `supportEAC` app, so every successful write broadcasts
   `onyx.action.oec.config.change` and AMS calls `EACConfigChangedImpl` inside us. With our
   `args_operation_flag = 0` that is a `dispatchFullRedraw`; with EinkWise's reset (flag 2) it is a
   **WebView reload**, which for a Tauri app means losing page state.
8. **A user opening EinkWise on our app can reload or restart us.** The reset button sends flag 2
   (WebView reload); toggling `FORCE_ROTATION` sends flag 0 with a changed rotation block, which
   makes `EACConfigChangedImpl.onRotationConfigChanged` call `restartPackage()`.
9. **EinkWise's refresh-mode picker cannot change our resolved profile** — it writes `updateMode`
   and `turbo`, never `refreshModeIndex`, and the index wins. So a user "fixing" our refresh mode in
   EinkWise will appear to work in the UI and change nothing on the panel. [inference; the live panel
   is in `com.onyx.floatingbutton`, which is not in this dump — experiment #1]
10. **EinkWise can rebuild our stored config out from under us**, without asking: on an OTA that
    bumps `jsonVersion` past the 103/107 upgrade ladder, `UpgradeEACConfigRequest` falls through to a
    full `BuildDefaultEACConfigRequest` for every installed app; the cloud fetch
    (`FetchCloudEACConfigRequest`, triggered by the app-list screen on Wi-Fi, by a new install, and by
    the **exported** broadcast `com.onyx.EAC_FETCH_FROM_CLOUD`) writes into the default-theme store
    for every matching package with no confirmation; and "Settings → Apps → reset all" wipes
    everything. Our profile write must therefore be **idempotent and re-asserted**, never
    "set once at install".
11. **The pen session is orthogonal to EinkWise and stays that way as long as `supportNoteConfig`
    is false for us.** `EACScreenNoteManager` is gated on it (`EACScreenNoteManager.java:155`), so
    the system's own 500 ms post-stylus-up `handwritingRepaint` never competes with our reconcile. Do
    not set `noteConfig.enable`/`supportNoteConfig` in anything we write.
12. **Moving to `refresh_mode_4` re-arms `ScrollHelper`, which can fire `applyGCOnce()` +
    `repaintEverything()` about a second after a finger scroll ends** (`ScrollHelper.java:68-83`, gated
    on `gcAfterScrolling`, default `false`). `repaintEverything` drops SurfaceFlinger's handwriting
    schema (`REPORT-surfaceflinger.md` §3). That is harmless between strokes and disruptive during
    one — so if we enable `setIsGCAfterScrolling(true)` we must not do it while the pen editor is
    open. (It is also a device-wide setting; see §7.2.)
13. **`applyTransientUpdate` is unowned SF state that nothing in the firmware clears**, and it is
    not keyed to us: the package hash at `UpdateEntry+0x04` is stored but never read for any decision,
    there is no pid/uid/token, and `repeatLimit` has no consumer, so the registration never expires
    and any app can overwrite or clear it. EinkWise clears the _app-scope_ override on app switch but
    never a transient one. Our defensive `clearTransientUpdate(false)` in `OnyxInk`'s constructor and
    the 5 s quiet-period reset are the only things standing between a killed session and a panel stuck
    in animation-quality mode. Note also that `clearTransientUpdate(**true**)` costs a full GC flash
    (the transient slot's `fast → none` clear action is `8` → `applyGCOnce`, §5.4.6) while
    `(false)` is silent — which is a de-ghosting primitive we are currently not using.
14. **`ViewUpdateHelper.setEpdTurbo` is SF-global, not per-app.** If we raise it for a writing
    session it applies to whatever else is on screen until we lower it or until EinkWise re-asserts
    the foreground app's profile turbo on the next resume (`impl/EACBaseRefreshImpl.java:62-65`).

---

# Recommended settings

```text
# ── once, at app start (rename ensureSpeedRefreshProfile → ensureQualityRefreshProfile) ──
#    off the UI thread, before any ink session, idempotent, re-run on every setDisplayProfile

  json  = EInkHelper.getAppConfigFromService([pkg])[0]          # reflection, as today
  r     = json.globalActivityConfig.refreshConfig
  if r.refreshModeIndex != "refresh_mode_4" or r.updateMode != 0:
      r.refreshModeIndex   = "refresh_mode_4"   # {mode:0 AUTO, turbo:0} in noteair4c_systemui.json
      r.updateMode         = 0                  # applyDither + increaseRepaintCount read THIS
      r.turbo              = 0
      r.enable             = true
      r.gcInterval         = 20                 # firmware auto-GC every 20 debounced inputs
      r.animationDuration  = 0                  # -> max(minAnimationDuration=80, 10) = 80 ms settle
      r.useGCForNewSurface = true               # device default on a CFA panel anyway
      remove r.refreshModeAlias                 # refresh_mapping.json is {} on this build
      # leave displayConfig, dpiConfig, rotationConfig, noteConfig, extraConfig EXACTLY as read
      EInkHelper.applyAppConfigToService([json], Bundle{ "args_operation_flag": 0 })
  # then read back and log: the write lands in eac_app_<pkg>, the read comes from the theme (§4.3)

# ── the ink session ────────────────────────────────────────────────────────────────────
on session start:
  1. view.set(SESSION, UpdateMode.GU)                     # UI_GU_MODE = 2, GC16 partial, nibble 2
                                                          #  process-local, cannot leak
  2. ViewUpdateHelper.setEpdTurbo(5)                      # SF 1048661 — SF-GLOBAL, must be restored
  3. do NOT touch the EAC profile here

per reconcile frame (unchanged, keep):
  4. view.set(TRANSIENT, UpdateMode.HAND_WRITING_REPAINT_MODE)   # 524290 = GC16 | 0x80000
  5. draw one ordinary frame
  6. view.clear(TRANSIENT) once that frame is on screen

lasso / selection drag only:
  7. EpdController.applyTransientUpdate(UpdateMode.ANIMATION_QUALITY)   # SF 16711782, slot 1, mode 2308
  8. Device.currentDevice().clearTransientUpdate(TRUE) on drag end, with the 5 s tail
     # (true) makes the clear itself fire a full GC (clear action 8 -> applyGCOnce, section 5.4.6),
     # which is exactly the de-ghost we currently schedule separately. (false) is silent.
     # Both are no-ops unless the transient slot still holds a fast mode.

on session end / onPause / onDestroy — all five, in this order:
  9.  Device.currentDevice().clearTransientUpdate(false)
  10. ViewUpdateHelper.setEpdTurbo(0)
  11. view.clear(SESSION)                                  # restores the raw pre-session value
  12. EpdController.setScreenHandWritingPenState(view, PEN_PAUSE)   # the only schema restore
  13. EpdController.setScreenHandWritingRegionExclude(null, intArrayOf())  # empty array, not {0,0,0,0}
  14. EpdController.invalidate(webView, UpdateMode.GC)     # UI_GC_MODE 98, nibble 2 — accepted

# ── delete ──────────────────────────────────────────────────────────────────────────────
  - applyAppRefreshProfile() and the EInkHelper.setAppScopeRefreshMode probe   (collision #2)
  - view.set(BASE, UpdateMode.REGAL) and its readback verification             (collision #6)
    -> leave the view at its initial UI_DEFAULT_MODE (5), or set GU explicitly

# ── never call ──────────────────────────────────────────────────────────────────────────
  applySystemFastMode / switchToA2Mode / toggleA2Mode        (= setAppScopeRefreshMode in disguise)
  EInkHelper.applyRefreshConfigToAll                          (rewrites every app)
  any EInkHelper.set* refresh/display setter                  (writes the device-wide fallback)
  ViewUpdateHelper.enableDither / setDitherThreshold / applySaturation* / applyGammaCorrection /
  applyBrightness / setBWMode / setGrayscaleMode / antiFlicker / useGCForNewSurface /
  setGcRefreshInterval / setUpdListSize                        (SF-global)
  repaintEverything(107) or (108)                              (nibble rejected on this panel)
  setVCom                                                      (panel damage)
```

Two optional, riskier knobs, worth an experiment each but not a default:

- `globalActivityConfig.paintConfig.ditherBitmap = true` — with `updateMode: 0` stored, `applyDither`
  will now actually call `ViewUpdateHelper.enableDither(true)` on every resume. That is SF-global
  while we are foreground and reset by the next app's resume, so it is self-limiting; the open
  question is whether firmware dithering helps or hurts our own anti-aliased ink.
- `globalActivityConfig.displayConfig.ditherThreshold = 180` (`DITHER_HIGH_CONTRAST`). Only `128` and
  `180` survive `ValidateAppConfigAction.validateDitherThreshold`, and `ViewUpdateHelper.setDitherThreshold`
  drops anything below 128.

---

# On-device experiments

Each is one testable action.

1. Open EinkWise on our app, pick a different refresh option, close it, then dump our stored config
   (`EInkHelper.getAppConfigFromService([pkg])`) — confirm whether `refreshModeIndex` changed or only
   `updateMode`/`turbo` did. This settles collision #9 and tells us whether the live
   `com.onyx.floatingbutton` panel writes the index.
2. Write `refreshModeIndex:"refresh_mode_4"` with `updateMode:0, turbo:0`, restart the activity, and
   read the config back — confirm `ValidateAppConfigAction` did **not** rewrite it to
   `refresh_mode_1`/`refresh_mode_2`. This is the one place a "system-app-only" enforcement could
   exist somewhere I did not find.
3. With `refresh_mode_2` stored, log `ViewUpdateHelper.getFastModeIndex()` (SF `1048656`, no SDK
   wrapper) at app start, then repeat after switching to `refresh_mode_4`. `2` = the application slot
   holds a fast mode; `0` = neither slot does. Cross-check with `dumpsys SurfaceFlinger` for the
   `"### APP_SLOT"` / `"### TRANSIENT_SLOT"` lines from `FixedUpdateEntryManager::dump()`, which print
   `mode`, `repeatMode`, `count`, `repeatLimit`, `isCPUDither` and `isGPUDither` — the ground truth for
   §5.4.
4. Write `refresh_mode_4`, read it back immediately (expect the new value), then switch to another
   app and back and read again **without rebooting** (the theme/app-config asymmetry predicts the old
   value re-appears on the enforcement path). Then reboot and read a third time. Settles collision #5.
5. With `refresh_mode_4` stored, `logcat -s TabletEACRefreshImpl EACBaseRefreshImpl EACUtils` while
   typing in a text field — confirm `applySFDeBouncer` and `applyDebouncerTransientUpdateMode` appear
   on key-up. They must be absent today under `refresh_mode_2`.
6. Compare 60 s of ordinary scrolling and typing under `refresh_mode_4` vs today's `refresh_mode_2`,
   photographed under identical light, and count the spontaneous full flashes.
7. Set `View.setDefaultUpdateMode(webView, 9)` (`REAGL_PLUS`) directly and watch logcat for
   `"onyx_epdc_hwscreenRefresh(): waveform_mode wrong!"` — confirms that `refresh_mode_1` asks this
   panel for a waveform it rejects, and therefore that `refresh_mode_4` is the right quality profile.
8. Call `ViewUpdateHelper.setEpdTurbo(5)` on ink-session start and `getEpdTurbo()` (SF `1048689`)
   immediately after, then switch apps and back and read again — confirms the value takes and that
   EinkWise resets it on the next resume.
9. Remove `applyAppRefreshProfile()`, then dump `deviceConfig.extraConfig.appDefaultConfig`
   (`EInkHelper.getDeviceExtraConfigString()`) on a device where the old build ran — confirms whether
   we have already written `updateMode: 2` into the device-wide fallback, and lets us reset it to 0.
10. With `refresh_mode_4` stored and the pen editor open, scroll the sheet with a finger and watch for
    a full flash ~1 s later (`ScrollHelper.exitScrollRefreshModeImpl`). Confirms whether re-arming
    `ScrollHelper` is disruptive during writing (collision #12).
11. In EinkWise, press **reset** on our app and observe whether the WebView reloads — confirms the
    `args_operation_flag = 2` → `onResetConfig` → `reloadAllWebView` path (collision #8).
12. Send `am broadcast -a com.onyx.EAC_FETCH_FROM_CLOUD --es args_action REQUEST_UPDATE_APP_CONFIG
--es args_pkg dev.okhsunrog.tangleaf` on Wi-Fi and re-read our stored config — establishes
    whether the Onyx cloud has an entry for us and whether it silently replaces what we wrote
    (collision #10).
13. Read `/onyxconfig/mmkv/onyx_config` (root, or `dumpsys oec_service`) and grep for
    `eac_app_dev.okhsunrog.tangleaf`, `eac_theme_dev.okhsunrog.tangleaf@theme_type_3` and
    `eac_active_theme_dev.okhsunrog.tangleaf` — the ground truth for §4.3, and the fastest way to see
    which of the two keys the runtime is actually obeying.
14. With `refresh_mode_4` and `paintConfig.ditherBitmap = true`, confirm `EACUtils` logs
    `applyDither, enable: true` on resume, and compare grey-text and ink rendering against today.
15. Replace the lasso-drag `clearTransientUpdate(false)` with `clearTransientUpdate(true)` and see
    whether the resulting `applyGCOnce` (mode 98) removes the drag's ghost without needing our
    separate scheduled `invalidate(GC)` — and time the flash.
16. Log `"### setDither, enable %d"` and the `isCPUDither`/`isGPUDither` columns of
    `FixedUpdateEntryManager::dump()` under `refresh_mode_2` vs `refresh_mode_4` — confirms that a
    plain A2 profile turns the RenderEngine dither shader **off** and forces panel-side Y1 dithering
    **on** (`applyDither` @ `0x559304`), which is the opposite of the usual assumption.
17. From a second app, call `ViewUpdateHelper.clearAppScopeUpdate(1)` while our app is foreground
    under `refresh_mode_2` — the fixed slots carry no pid/uid/token, so this should drop _our_
    profile until the next resume. Confirms the "any app can clear any app's slot" finding and
    quantifies how exposed the mechanism is.
