# The Onyx system handwriting pipeline (`screennote`) — how it is armed, and can a third-party app opt in?

> Written 2026-09-07 against the decompiled framework, the `com.onyx` system app, the Pen SDK and
> the on-device native libraries, before `/system/bin/surfaceflinger` was extracted from the firmware
> package. Statements marked as inference about the SurfaceFlinger side can now be checked against the
> binary — see [README](README.md). Absolute paths in this document refer to the scratch tree described
> in [01-sources-and-firmware.md](01-sources-and-firmware.md).

Device: BOOX Note Air 4C, `lito`, Android 13, FW 4.2-rel, Kaleido colour panel.
Target app: `dev.okhsunrog.tangleaf` (Tauri/WebView, renders its own ink under the firmware ink layer).

Sources cited by `path:line`, relative to these roots:

| tag    | root                                                                                                   |
| ------ | ------------------------------------------------------------------------------------------------------ |
| `fw/`  | `/home/okhsunrog/tmp_zfs/onyx_framework/out/sources` (framework.jar)                                   |
| `svc/` | `/home/okhsunrog/tmp_zfs/onyx_framework/out-services/sources` (services.jar)                           |
| `kcb/` | `/home/okhsunrog/tmp_zfs/onyx_framework/out-kcb/sources` (com.onyx system app + bundled `onyxsdk-eac`) |
| `sdk/` | `/home/okhsunrog/tmp_zfs/onyx_sdk_decompiled/src` (the AARs we link against)                           |
| `app/` | `/home/okhsunrog/code/rust/notes-rs`                                                                   |

Everything below is read from decompiled code. Statements marked **[inference]** are not directly
witnessed in code.

> Correction to the briefing: `services.jar` _does_ contain the Onyx hook that publishes the service —
> `svc/com/android/server/SystemServer.java:1178-1181`. The service _implementation_ lives in
> framework.jar (`fw/android/onyx/optimization/OECService.java`) but runs **inside system_server**.

---

## 0. Executive summary

- A view becomes "the draw view" purely by **string match on its resource-entry name or its fully
  qualified class name** against one field of the per-app EAC config: `noteConfig.drawViewKey`.
  There is no window flag, no view tag, no Binder registration, no SDK call.
- The pipeline is gated by four booleans, all inside the per-app EAC JSON we already read and write:
  `supportEAC`, `enable`, `globalActivityConfig.noteConfig.supportNoteConfig`,
  `globalActivityConfig.noteConfig.enable` (+ `noteConfig.globalStrokeStyle.enable` to actually _start_).
- Those note fields are **factory data**: nothing in the firmware's UI or code writes
  `supportNoteConfig` / `drawViewKey` / `styleMap`. They ship in `res/raw/eac_<Build.MODEL>.json`
  inside `kcb.apk` for exactly **six** third-party packages (OneNote, Evernote, Yinxiang, jnotes,
  Fenbi, WPS — §2.1). Two of them key off a **class name of a WebView/ink host**, which is precisely
  the shape we would need. There is no supported way to add a seventh.
- **There is no permission check on the write path.** `applyAppConfigToService` takes the target
  package name _from the caller's JSON payload_ and replaces the whole stored `EACAppConfig` verbatim.
  Our `refreshConfig` write is not a special case; every field is equally writable, for any package —
  so we can add ourselves to that table anyway.
- Opting in is therefore a config write + an app restart. **But it is very likely a net loss for us**:
  the system pipeline drives exactly the same global primitives our `TouchHelper`/`OnyxInk` path
  already drives (pen state, handwriting region limit/exclude, stroke parameters), from a different
  thread, with a whole-view repaint region and a `repaintEverything()` on every finger-down. See §3.

---

## 1. Q1 — What makes a view a "draw view" in the system's eyes?

### 1.1 The call graph, top down

`android.view.View` is patched with six hooks into `EACScreenNoteManager`:

| hook                                                        | site                                            |
| ----------------------------------------------------------- | ----------------------------------------------- |
| `dispatchTouchEvent` (only when the event was **consumed**) | `fw/android/view/View.java:7595`                |
| `dispatchVisibilityChanged`                                 | `fw/android/view/View.java:7628`                |
| `setVisibility` / visibility propagation                    | `fw/android/view/View.java:7098`                |
| `onVisibilityAggregated`                                    | `fw/android/view/View.java:12903`               |
| `onWindowFocusChanged`                                      | `fw/android/view/View.java:12952`               |
| `performClick` → `notifyEACScreenNoteManagerOnClick`        | `fw/android/view/View.java:13164`, `:5281-5283` |
| `beforeScroll` (gated by `allowApplyTransientUpdate`)       | `fw/android/view/View.java:6453`                |

`EACScreenNoteManager.getActiveHandler` (`fw/android/onyx/optimization/EACScreenNoteManager.java:124-137`)
dispatches to one of three handlers by a **per-process** `packageType`:

- `NOT_SUPPORT` → `EmptyHandler` (all methods empty, `fw/.../screennote/handler/EmptyHandler.java`)
- `BASIC_APP` → `SingleDrawViewHandler`
- `IME` → `ImeDrawViewHandler`

### 1.2 Gate A — device supports scribble

`EACScreenNoteManager.java:36`:
`DEVICE_SUPPORT_SCRIBBLE = DeviceConfig.singleton().isSupportScribble()` (`fw/android/onyx/config/DeviceConfig.java:179`).
False ⇒ `ProviderType.NONE` unconditionally (`EACScreenNoteManager.java:126-128`).
**[inference]** true on a Note Air 4C; verify with experiment E1.

### 1.3 Gate B — the package type, computed **once per process**

`EACScreenNoteManager.initPackageTypeImpl()` — `EACScreenNoteManager.java:153-162`:

```java
if (app != null && DEVICE_SUPPORT_SCRIBBLE && EInkHelper.getNoteConfig(app).isSupportNoteConfig()) {
    packageType = allIMEPkgs.contains(currentOpPackageName) ? IME : BASIC_APP;
}
```

It is invoked exactly once, from `initPackageAsync()` in the singleton constructor
(`EACScreenNoteManager.java:116-122`, `:139-151`, `:182-185`). There is **no re-init path and no
config-changed receiver in this class** — the only receiver it owns is `FloatButtonStatusReceiver`
for `com.onyx.floatingbutton.touch` (`:81-114`). Confirmed by search. ⇒ **flipping
`supportNoteConfig` requires the app process to be restarted.**

`EInkHelper.getNoteConfig(Context)` — `fw/android/onyx/optimization/EInkHelper.java:891-897`:

```java
if (!isAppConfigActive()) return EACNoteConfig.dummyConfig();
return new EACNoteConfig(getReadOnlyAppConfig()
        .fuzzyMatchActivityConfig(activityName(context, null)).getNoteConfig());
```

`isAppConfigActive()` — `EInkHelper.java:1156-1162` — requires **all** of:
`OECServiceUtils.isReady()` (system property `sys.oec.service.ready`, `fw/.../OECServiceUtils.java:17-23`),
the device-wide `EInkHelper.isEnable()`, `appConfig.isSupportEAC()`, `appConfig.isEnable()`.

`fuzzyMatchActivityConfig` — `fw/android/onyx/optimization/data/v2/EACAppConfig.java:146-157` — exact
key lookup in `activityConfigMap`, then a **substring** fallback (`activityName.contains(key)`), then
`globalActivityConfig`.

### 1.4 Gate C — the view itself: a _string match_

`EACNoteConfig.matchDrawView` — `fw/android/onyx/optimization/data/v2/EACNoteConfig.java:77-82`:

```java
if (view == null || TextUtils.isEmpty(this.drawViewKey)) return false;
return EACUtils.matchView(view, this.drawViewKey);
```

`EACUtils.matchView` — `fw/android/onyx/optimization/EACUtils.java:94-109`:

```java
int id = view.getId();
if (id != -1) {
    name = view.getContext().getResources().getResourceEntryName(id);   // NotFoundException swallowed
    if (!TextUtils.isEmpty(name) && name.equals(str)) return true;
}
return view.getClass().getName().equals(str);
```

So the key is either an **`android:id` entry name** (e.g. `note_draw_view`) or a **fully qualified
class name** (e.g. `dev.okhsunrog.tangleaf.generated.RustWebView`). Nothing else. There is no
`WindowManager.LayoutParams` flag, no `View` tag, no service registration, and the SDK's
`TouchHelper` does not tell the framework anything about which view is the note view.

### 1.5 The one place the _cached_ per-view flag matters

`View.mNoteDrawViewFlag` — `fw/android/view/View.java:626`, initialised to `0` in both constructors
(`:2322`, `:2370`). Setter: `View.noteViewDetect(boolean)` — `fw/android/view/View.java:11648-11650`
(`1` = NOTE_DRAW_VIEW, `2` = NOT_NOTE_DRAW_VIEW; constants at
`fw/.../screennote/EACScreenNoteConstant.java:18-21`). Readers: `isNoteDrawView()` `:11207-11209`,
`isNoteViewDetected()` `:11211-11213`.

It is populated only by `EInkHelper.noteViewDetect(View)` — `fw/.../EInkHelper.java:1287-1292`:

```java
if (view.isNoteViewDetected() || EACScreenNoteManager.sharedInstance().getPackageType() == NOT_SUPPORT) return;
view.noteViewDetect(getNoteConfig(view.getContext()).matchDrawView(view));
```

called from exactly three places in `View`:

- the `AttributeSet` constructor — `fw/android/view/View.java:4048` (i.e. **XML-inflated views only**)
- `setId(int)` — `fw/android/view/View.java:15128`
- `setLabelFor(int)` — `fw/android/view/View.java:15194`

and `isNoteDrawView()` gates **exactly one** thing — the call that actually _starts_ the pipeline:

`BaseHandler.onVisibilityChanged` — `fw/.../screennote/handler/BaseHandler.java:412-422`:

```java
if (view.isNoteDrawView()) { async(() -> onVisibilityChangedImpl(view, z)); }
```

Motion events, window-focus changes and clicks do **not** consult the cached flag; they re-run
`matchDrawView` live (`BaseHandler.java:139-146`, `:98-100`, `:305`).

**Consequence for us:** a programmatically created `RustWebView` never runs the `AttributeSet`
constructor, so unless we call `setId(...)` (or reflect `View.noteViewDetect(true)`) the pipeline is
never _started_, even with a perfect `drawViewKey`. Note also the ordering hazard: `noteViewDetect`
early-returns while `packageType` is still `NOT_SUPPORT`, and `packageType` is computed on an Rx
worker thread (`EACScreenNoteManager.java:139-151`) — so `setId` must happen after that async init.

**Answer to Q1:** the registration path is _config-driven name matching_, not an API. The only
"registration" an app performs is (a) having the matching id/class and (b) triggering
`EInkHelper.noteViewDetect` via `setId`/XML inflation, after which a visibility change to VISIBLE
arms everything.

---

## 2. Q2 — Exactly which config fields gate the pipeline

Read/written through `EACReflectUtils.sMethodGetAppConfigFromService` /
`sMethodApplyAppConfigToService`, i.e. the JSON serialisation of
`android.onyx.optimization.data.v2.EACAppConfig` (fastjson, JavaBean getters ⇒ the JSON keys are the
getter names). Paths as they appear in the blob our `MobileSystemPlugin.ensureSpeedRefreshProfile()`
already parses (`app/plugins/tauri-plugin-mobile-system/android/src/main/java/dev/okhsunrog/mobile_system/MobileSystemPlugin.kt:265-291`):

| JSON path                                                  | type   | must be                                                   | evidence                                                                                                 |
| ---------------------------------------------------------- | ------ | --------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `supportEAC`                                               | bool   | `true`                                                    | `EInkHelper.java:1156-1162`                                                                              |
| `enable`                                                   | bool   | `true`                                                    | `EInkHelper.java:1156-1162`                                                                              |
| `globalActivityConfig.noteConfig.supportNoteConfig`        | bool   | `true`                                                    | `EACScreenNoteManager.java:155`; UI gate `kcb/com/onyx/android/sdk/eac/util/EACViewUtils.java:1083-1087` |
| `globalActivityConfig.noteConfig.enable`                   | bool   | `true`                                                    | `BaseHandler.java:99`, `:122`, `:289`, `:297`, `:305`                                                    |
| `globalActivityConfig.noteConfig.drawViewKey`              | string | the view's id-entry-name or class name                    | `EACNoteConfig.java:77-82`                                                                               |
| `globalActivityConfig.noteConfig.globalStrokeStyle.enable` | bool   | `true` (else `startEACScreenNote` is a no-op)             | `BaseHandler.java:331-342`                                                                               |
| `activityConfigMap["<activity cls>"].noteConfig.*`         | object | optional per-activity override, chosen by substring match | `EACAppConfig.java:146-157`                                                                              |

**`supportNoteConfig` is the "optimization block" you see disabled.** In the EinkWise UI the pen tab
(`EACViewConfigs.TabType.TAB_TYPE_NOTE_EAC`) is rendered only when
`eacAppConfig.isSupportNoteConfig()` (`kcb/.../eac/util/EACViewUtils.java:1083-1087`), which is
`getGlobalActivityConfig().getNoteConfig().isSupportNoteConfig()`
(`kcb/com/onyx/android/sdk/eac/data/v2/EACAppConfig.java:146-148`). That tab carries exactly three
controls — `PEN_EAC_ENABLE`, `PEN_EAC_REPAINT_LATENCY`, `PEN_EAC_STROKE_WIDTH`
(`kcb/com/onyx/android/sdk/eac/data/EACDataType.java:36-40`) — which write, respectively,
`noteConfig.enable`, `noteConfig.repaintLatency`, `noteConfig.globalStrokeStyle.strokeWidth`
(`kcb/.../eac/util/EACViewUtils.java:408-444`) on `globalActivityConfig` **and** every entry of
`activityConfigMap`.

**No code anywhere writes `supportNoteConfig`, `drawViewKey`, `styleMap` or `globalStrokeStyle`.**
An exhaustive grep for `setDrawViewKey(|setSupportNoteConfig(|setStyleMap(|setGlobalStrokeStyle(`
over `out`, `out-kcb`, `out-services`, `out-ota` matches only the beans' own copy-constructors and
setter definitions (`kcb/com/onyx/android/sdk/eac/data/v2/EACNoteConfig.java:23,59,64,74,79`;
`fw/.../data/v2/EACNoteConfig.java:29-30,89,94,104,109`), confirmed at DEX string-pool level
(`kcb.apk/classes5.dex` contains `DrawViewKey` ×2 and `SupportNoteConfig` ×2 — the accessor names
only). `repaintLatency` and `globalStrokeStyle.strokeWidth` are the _only_ note fields any UI mutates
(`kcb/.../eac/util/EACViewUtils.java:426,428,438,441`). These fields are **read-only,
factory-provisioned data**.

### 2.1 The factory preset table — the ground truth we can copy

They ship inside the APK, not in a theme download: `res/raw/eac_<Build.MODEL>.json` (falling back to
`eac_default.json`, plus `eac_force`), loaded by `ConfigLoader.load(...)` at
`kcb/com/onyx/android/sdk/eac/data/EACConstantDeviceConfig.java:16`, names at
`kcb/com/onyx/android/sdk/eac/data/EACConstant.java:25-27,101`, consumed via
`kcb/com/onyx/android/sdk/eac/data/PresetEACConfig.java:39`. 152 such files ship; the note table lives
under the `presetAppConfig` key and is present in **89 of 152** (every note-capable model; _absent_
from `eac_default.json`). `jsonVersion: 109`.

Exactly **six** third-party packages are provisioned, identically across all 89 files:

| package                        | `drawViewKey`                                               | shape                                                                                          |
| ------------------------------ | ----------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `com.microsoft.office.onenote` | `com.microsoft.office.airspace.AirspaceInkLayer`            | global; full `styleMap` (`pen_1…`, `highlighter_1…`); `strokeStyle:1, strokeWidth:6`           |
| `com.evernote`                 | `com.evernote.neutron.editorwebview.EditorWebViewManager$b` | global; no `styleMap`                                                                          |
| `com.yinxiang`                 | `renderview_id`                                             | per-activity (`…NoteActivity`, `…NewNoteActivity`); `styleMap` `stroke_one/two/three` = 5/7/9  |
| `com.jideos.jnotes`            | `draw_panel`                                                | `compatibleVersionCode:0`; `styleMap` `btn_color1..3`, `rl_size1..3`, `btn_pen`, `btn_markpen` |
| `com.fenbi.android.servant`    | `scroll_scratch`                                            | the **only** entry that sets `repaintLatency: 500` explicitly                                  |
| `cn.wps.moffice_eng`           | `text_editor`, `pdf_renderview`                             | per-activity (`…pdf.multiactivity.PDFReader`, `…writer.multiactivity.Writer`, `Writer1`)       |

Two of these are directly relevant to us: **Evernote's key is a FQCN of a WebView-manager inner
class**, and **OneNote's is a FQCN of a custom ink layer** — i.e. Onyx itself provisions the pipeline
for WebView-hosted editors by class name, exactly the shape we would need. WPS and jnotes use bare
`android:id` entry names.

So the block is off for us simply because our package is not in a table nobody can edit through any
supported UI — and the write path has no guard (§5), which is why we can add ourselves anyway.

---

## 3. Q3 — If we enable it: what we gain, and what it fights us over

### 3.1 What the system would then do on our behalf

All in `fw/android/onyx/optimization/screennote/handler/BaseHandler.java`:

| trigger                                               | action                                                                                                                                                                                                                          | line                                                                                    |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| draw view becomes VISIBLE                             | `setAutoSyncBufEnable(false)`; reset handwriting exclude; `setScreenHandWritingRegionLimit(view, localVisibleRect)`; `setScreenHandWritingPenState(PEN_START=1)`; apply stroke params; register layout + float-button listeners | `:167-190`, `:331-342`                                                                  |
| stylus DOWN inside draw view                          | `PEN_DRAWING=2`; cancel pending repaint; report state `2` down the `/dev/onyx/listener` pipe                                                                                                                                    | `:228-235`                                                                              |
| stylus UP/CANCEL inside draw view                     | after `noteConfig.repaintLatency` ms: `ViewUpdateHelper.handwritingRepaint(view, l,t,r,b, false)` over the **whole visible view rect**, then report state `3`                                                                   | `:268-283`                                                                              |
| finger DOWN anywhere in that view tree                | `PEN_PAUSE=3` **+ `ViewUpdateHelper.repaintEverything()`**                                                                                                                                                                      | `:192-196`                                                                              |
| stylus DOWN outside the draw view                     | if IME not showing: `PEN_PAUSE` + `repaintEverything()`                                                                                                                                                                         | `:237-244`                                                                              |
| `MotionEvent` toolType `TOOL_TYPE_ERASER (4)`         | `PEN_PAUSE`                                                                                                                                                                                                                     | `:132-134`                                                                              |
| window focus lost/gained                              | cancel pending repaint; `PEN_PAUSE` + `repaintEverything()`; on gain also reset exclude, re-limit region, re-apply stroke params                                                                                                | `:302-320`                                                                              |
| layout change / IME show-hide                         | `PEN_PAUSE`, re-limit region, resume                                                                                                                                                                                            | `:159-165`, `:398-400`; `SingleDrawViewHandler.java:13-23`                              |
| draw view becomes INVISIBLE                           | `PEN_STOP=0`, `setAutoSyncBufEnable(true)`, unregister listeners                                                                                                                                                                | `:167-174`, `:344-351`                                                                  |
| click on a view whose id/class is a key of `styleMap` | swap stroke params, then `resumeEACScreenNote(null)` → `PEN_DRAWING`                                                                                                                                                            | `:221-226`, `:286-292`                                                                  |
| broadcast `com.onyx.floatingbutton.touch`             | treated as finger down/up                                                                                                                                                                                                       | `EACScreenNoteManager.java:81-99`, `BaseHandler.java:202-219`                           |
| stylus event in draw view                             | `allowApplyTransientUpdate()` returns **false** ⇒ suppresses `View.beforeScroll`'s transient update and `EACScrollRefreshManager`'s                                                                                             | `:357-365`; `fw/android/view/View.java:6453`; `fw/.../EACScrollRefreshManager.java:257` |

Pen-state constants: `fw/android/onyx/optimization/Constant.java:299-322`
(`PEN_STOP 0 / PEN_START 1 / PEN_DRAWING 2 / PEN_PAUSE 3 / PEN_ERASING 4`).
The repaint it issues in `EACScreenNoteUtils.repaintView` uses **our own refresh profile**
(`EACScreenNoteUtils.java:22-32, 109-113`) — the one we already force to Speed.

### 3.2 Where it fights an app that renders its own ink into a WebView

1. **Double ownership of the same global state.** `setScreenHandWritingPenState` and the region
   limit/exclude are process-scoped SurfaceFlinger transactions (`fw/android/onyx/ViewUpdateHelper.java:1195-1200`,
   `:1202-1217`, `:1219-1231` — note `Process.myPid()` in the pen-state parcel). The SDK we already
   use drives the _same_ calls — and that includes the `FEATURE_SF_TOUCH_RENDER` path our
   `OnyxInk.kt:327-329` selects: `sdk/onyxsdk-pen-1.5.4.3/.../touch/SFTouchRender.java:178,184,191,197,202,235`
   → `sdk/onyxsdk-pen-1.5.4.3/.../EpdPenManager.java:36-49`,
   `.../touch/AppTouchRender.java:189,196`, `.../RawInputReader.java:413,423,543`,
   `.../touch/AppTouchInputReader.java:148,156`. `BaseHandler` runs on its own single-thread pool and
   a delayed scheduler (`BaseHandler.java:26-27, 80-86`), so the two state machines interleave
   non-deterministically. Last writer wins; there is no arbitration.
2. **It overwrites our stroke parameters.** `applyStrokeParam` pushes
   `setStrokeColor/setStrokeStyle/setStrokeWidth/setStrokeParameters` from the config
   (`BaseHandler.java:65-78`) on every visibility change, every window-focus gain and every
   "menu" click. That stomps the lasso dash / fountain configuration in
   `app/.../OnyxInk.kt:348-357` at times we do not control.
3. **Its repaint region is the whole view, not our damage rect.** `getLocalVisibleRectFromView`
   (`fw/.../screennote/EACScreenNoteUtils.java:38-66`) degenerates to the **full display rect** when
   the local visible rect is empty (`:48-49`). For a full-screen WebView we would get a whole-screen
   `handwritingRepaint` 500 ms after every stroke, on top of our own
   `InkReconcileRegion`/`InkDamage` reconcile.
4. **`repaintEverything()` on every finger-down and every focus change** (`:195`, `:243`, `:311`).
   A WebView app is touched constantly (scroll, toolbar, menus) — that is a full-panel refresh per tap,
   and it is exactly the behaviour our `InkPauseRegistry`/`InkRefreshPolicy` currently avoids.
5. **Its own erase gate.** "Erasing" is decided by `strokeStyle == 5` in the _config_
   (`BaseHandler.java:102-104`), and while erasing it deliberately does **not** resume the pen state
   (`:230-232`). Our `InkEraserRenderGate` decides this from the actual tool; the two disagree.
6. **`setAutoSyncBufEnable(false)` for the whole time the draw view is visible** (`:170`) — a
   process-wide framebuffer-sync change we do not currently make.
7. **IME deadlock risk.** `SingleDrawViewHandler` silently refuses to resume the pen or set the region
   limit while the IME is visible (`SingleDrawViewHandler.java:26-41`). With a WebView text field, the
   pen stays paused until the IME hides.
8. **Menus give us nothing.** `isMenuView` matches Android child views by id/class
   (`EACScreenNoteUtils.java:83-91`); our menus are DOM inside the WebView, so `styleMap` can never
   fire. Harmless, but the whole style-swap half of the feature is unusable for us.
9. **It cannot see our gesture semantics.** It has no concept of our lasso trace, our frame fence
   (`InkFrameFence.kt`), our mid-gesture reconcile suppression (`InkReconcileGate.kt`) or the
   display-mode stack (`DisplayModeStack.kt`). It will pause/resume and repaint on raw
   `dispatchTouchEvent` outcomes.

**Net:** the only thing it gives us that we hand-roll is the _stylus-up → delayed
`handwritingRepaint`_ timer and the pause-on-finger/pause-on-focus-loss rules — the cheapest parts —
while taking over three globals we must own to keep the current behaviour. **[inference]** Enabling
it without also disabling our own pen-state/region writes will produce sporadic lost-ink and stuck
fast-mode frames rather than an improvement.

---

## 4. Q4 — `EACNoteConfig`: full tunable list

`fw/android/onyx/optimization/data/v2/EACNoteConfig.java` (JSON keys = bean property names):

| JSON key                | type                             | default on this firmware                                                                                                                                                         | writable by us? | notes                                                                                                                                                                                            |
| ----------------------- | -------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `enable`                | bool                             | `false` (`EACBaseConfig.enable`, `fw/.../data/v2/EACBaseConfig.java:8`)                                                                                                          | yes             | master per-app note switch; EinkWise `PEN_EAC_ENABLE`                                                                                                                                            |
| `supportNoteConfig`     | bool                             | `false` (`EACNoteConfig.java:19`)                                                                                                                                                | yes             | the "optimization block"; also gates the EinkWise pen tab                                                                                                                                        |
| `drawViewKey`           | string                           | `null` (`:18`)                                                                                                                                                                   | yes             | id-entry-name **or** FQCN                                                                                                                                                                        |
| `repaintLatency`        | int (ms)                         | `500` (`:21`); SDK default comes from a per-model JSON, `kcb/com/onyx/android/sdk/eac/data/EACConstant.java:108` → `EACConstantDeviceConfig`                                     | yes             | UI range 500…2000, step 500 (`EACConstant.java:61-63`; `kcb/.../eac/data/EACViewConfigs.java:447-450`)                                                                                           |
| `globalStrokeStyle`     | object                           | `EACStrokeStyle{enable=true, strokeStyle=0, strokeWidth=3.0f, strokeColor=0xFF000000, strokeExtraArgs=[]}` (`EACNoteConfig.java:22`; `fw/.../data/v2/EACStrokeStyle.java:14-17`) | yes             | `enable` must be true or `startEACScreenNote` no-ops; `strokeStyle==5` means "eraser"                                                                                                            |
| `styleMap`              | `Map<String, Set<{type,value}>>` | `{}` (`:20`)                                                                                                                                                                     | yes             | key = menu view id/class; `type` 0 = strokeStyle int, 2 = strokeColor int, 3 = a whole `EACStrokeStyle` JSON string (`EACStrokeStyle.java:79-99`; `fw/.../data/v2/EACStrokeStyleValuePair.java`) |
| `compatibleVersionCode` | long                             | `0` (`:23`)                                                                                                                                                                      | yes             | **dead on this firmware** — no reader anywhere in framework or kcb                                                                                                                               |

`strokeWidth` bounds used by the UI: 1…25, default 3, step 1 (`kcb/.../eac/data/EACConstant.java:47-50`).

Every one of these is just a field of the JSON blob that `applyAppConfigToService` stores verbatim
(§5), so **all of them are writable by an unprivileged app through the path we already use.**

Serialisation hazard: `EACStrokeStyle` exposes both `getStrokeExtraArgs():float[]` and
`getStrokeParams():List<Float>` but only `setStrokeExtraArgs(List<Float>)`
(`EACStrokeStyle.java:34-49, 64-67`). Round-trip the object as-is; do not hand-build it.

---

## 5. Q5 — What guards the apply path? Nothing.

- **Hosting:** `svc/com/android/server/SystemServer.java:1178-1181` —
  `new OECService(context)` then `ServiceManager.addService("oec_service", oECService, true)`
  (`allowIsolated = true`). Service name literal `oec_service` at
  `fw/android/onyx/optimization/Constant.java:117`; readiness property `sys.oec.service.ready` at
  `:116`; debug property `sys.oec.service.debug` at `:115`. Client lookup:
  `fw/.../EInkHelper.java:76-80` (`ServiceManager.getServiceOrThrow`).
- **No permission gate anywhere on the path.** `OECService.java` contains zero occurrences of
  `checkCallingPermission` / `enforceCallingPermission` / `getCallingUid` / `getCallingPid` /
  `getCallingPackage`. `IOECService.Stub.onTransact` (`fw/.../IOECService.java:1742-1744`) only does
  `parcel.enforceInterface(DESCRIPTOR)` — an interface token, not a permission. Transaction 12 is
  `applyAppConfigToService`, transaction 16 is `getAppConfigFromService`
  (`fw/.../IOECService.java:1571-1576`). The caller-side facade
  `EInkHelper.applyAppConfigToService` (`fw/.../EInkHelper.java:230-236`) only checks
  `isServiceReady()`. No `onyx.permission.*` string exists near the EAC code.
- **The target package comes from the payload, not from the caller.**
  `OECService.applyAppConfigToServiceImpl` — `fw/.../OECService.java:190-221`:
  `EACConfigJsonUtil.getAppConfig(json)` is a bare
  `FastJSONUtils.parseObject(str, EACAppConfig.class)` (`fw/.../EACConfigJsonUtil.java:35`), then
  `pkgName = appConfig.getPkgName()` (`:197`) and
  `editableDeviceConfig.getAppConfigMap().put(pkgName, appConfig)` (`:200`) — **the entire stored
  `EACAppConfig` for that package is replaced verbatim**, with no field whitelist, no validation and
  no per-field sanitisation. `ValidateAppConfigAction` is _not_ called on this path (it runs only from
  `LoadDeviceConfigFromLocalAction:45-51`, i.e. at boot/config-load).
- **So:** our existing `refreshConfig` write is **not** a special case. Any field, including
  `noteConfig.supportNoteConfig`, is equally writable — and so is any _other_ package's config.
- Residual barrier not observable here: SELinux rules for `find`/`call` on the `oec_service` binder
  (no `service_contexts`/sepolicy in this dump). Empirically our app already calls both methods, so
  the domain is permitted. **[inference]**

### 5.1 The legitimate write path, for reference (a working example end-to-end)

Every EinkWise control funnels into the same reflected call we already use:

- Dialog controller `kcb/com/onyx/android/sdk/eac/util/EACViewUtils.java:98` (layout `dialog_eac`,
  tabs enumerated at `kcb/.../eac/data/EACViewConfigs.java:807-823`:
  `TAB_TYPE_DISPLAY / COLOR_EAC / REFRESH / REFRESH_TURBO / OTHERS / NOTE_EAC`).
- Each control mutates one field of the in-memory `EACAppConfig` and returns an `OperationType`, then
  `EACViewUtils.java:604` → `kcb/com/onyx/android/sdk/eac/rx/action/EACAppConfigChangeAction.java`.
  `:69-86` maps the operation to the `args_operation_flag` bundle int (`RESET`→2, `CSS_CONFIG_CHANGED`→1,
  `APPLY_ALL`→2, default **0** — which is what we already pass). `:104` `TOGGLE` flips
  `appConfig.setEnable(!isEnable())` — that is the per-app master switch.
- `kcb/com/onyx/android/sdk/eac/rx/RxEACManager.java:147-161` serialises each config with fastjson2,
  batches 50, and `:89-95` reflect-invokes `EACReflectUtils.sMethodApplyAppConfigToService`
  (handle table `kcb/com/onyx/android/sdk/api/device/eac/EACReflectUtils.java:44-77`, target class
  `"android.onyx.optimization.EInkHelper"` at `:10`).
- Settings-list writes take the same road: `kcb/com/onyx/common/setting/model/EACRotationSettingViewModel.java:67-90`
  → `kcb/com/onyx/common/setting/action/ApplyAppEACItemDataAction.java:44`
  → `kcb/com/onyx/common/setting/request/ApplyAppEACItemDataRequest.java:25`. Note they always
  **round-trip the whole `EACAppConfig`**, exactly as our plugin does.
- The legacy in-app path `SimpleEACManage` edits raw JSON strings instead of beans and dispatches on
  `jsonVersion` (`kcb/com/onyx/android/sdk/api/device/eac/SimpleEACManage.java:59-67`: ≥107 → V3,
  ≥100 → V2). The shipped `jsonVersion` is **109**, so V3
  (`kcb/com/onyx/android/sdk/device/eac/v3/EACManagerV3.java:21-25`, the Bundle-passing variant).
  Key strings at `kcb/com/onyx/android/sdk/device/eac/EACConstants.java:5-16`.

Two red herrings worth naming so nobody chases them:

- **`EACAppTheme.isSupportOptimization` is dead.** Declared at
  `kcb/com/onyx/android/sdk/eac/data/EACAppTheme.java:71` (getter `:188-190`, setter `:239-241`) and
  `fw/.../data/v2/EACAppTheme.java:42,192,284`, it has **zero call sites** in framework.jar,
  services.jar or the whole com.onyx app — verified at DEX string-pool level (`SupportOptimization`
  appears exactly twice in `classes5.dex`, as the accessor names). It is not the block you see; that
  is `supportEAC` (refusal toast at `kcb/com/onyx/common/applications/action/ShowAppEACViewAction.java:52`)
  and `enable`, plus `noteConfig.supportNoteConfig` for the pen tab.
- **The cloud path does not carry note config either.** `EACCloudService`
  (`kcb/com/onyx/android/sdk/data/v2/EACCloudService.java`) returns an opaque `config` string
  (`kcb/.../data/v2/EACCloudConfig.java:14-24`) parsed wholesale into an `EACAppConfig`
  (`kcb/.../eac/rx/request/CloudConfigToAppConfigRequest.java:26-45`); the magic-code path
  (`kcb/com/onyx/common/setting/action/QueryMagicCodeAction.java:54-102`) sets only
  `appConfig, source, pkg, versionName, updateTime, tags, code`. Whatever note config exists comes
  from `presetAppConfig` in `res/raw` (§2.1) or from whatever the server chose to embed.

### Propagation after a write

- `applyAppConfigToServiceImpl` calls `saveDeviceConfig(cfg, 5, /*broadcastDataChange=*/false, …)`
  (`OECService.java:209`) then `saveConfigToLocalImpl()` (`:220`) — persisted regardless of the
  `args_save_mmkv` bundle flag.
- `shouldSendConfigChangedBroadcast` (`OECService.java:906-908`) returns **true** whenever the new
  config has `supportEAC == true`, so `onyx.action.oec.config.change`
  (`fw/android/onyx/BroadcastHelper.java:176`) _is_ sent for our package (`OECService.java:1704-1713`).
- In the app process, `Application.onCreate` registers `EInkHelper`'s data-changed receiver
  (`fw/android/app/Application.java:377`), and `EACConfigChangedImpl` re-fetches the config
  (`fw/.../EACConfigChangedImpl.java:154-176`) → `sCachedEACAppConfigRef` is refreshed live.
- **But `EACScreenNoteManager.packageType` is computed once and never re-read** (§1.3) ⇒
  a `supportNoteConfig` flip needs a process restart.
- Beware: `EACConfigChangedImpl.onRotationConfigChanged` calls
  `ActivityThread.restartPackage(pkg)` (`fw/.../EACConfigChangedImpl.java:196-198`) — never touch
  `rotationConfig` or `dpiConfig` in a write you want to be silent.

---

## Opt-in recipe

Prerequisites: the hidden-API bypass already installed by the SDK's `ReflectUtil`, and the two
reflected handles we already hold (`EACReflectUtils.sMethodGetAppConfigFromService`,
`sMethodApplyAppConfigToService`).

### Step 1 — read-modify-write the app config (read/modify/write the _whole_ blob; the service replaces it verbatim)

```jsonc
// root = JSON returned by getAppConfigFromService(["dev.okhsunrog.tangleaf"])[0]
{
  "pkgName": "dev.okhsunrog.tangleaf",
  "supportEAC": true, // must already be true
  "enable": true, // must already be true
  "globalActivityConfig": {
    "refreshConfig": {
      /* leave our Speed profile exactly as-is */
    },
    "noteConfig": {
      "enable": true, // + was false
      "supportNoteConfig": true, // + was false  <-- the "optimization block"
      "drawViewKey": "dev.okhsunrog.tangleaf.generated.RustWebView", // + was null/absent
      // FQCN form, same as Evernote/OneNote presets (§2.1)
      "repaintLatency": 500, // 500..2000, step 500
      "globalStrokeStyle": { "enable": true }, // keep the rest of the object as read
      // styleMap: leave {} — our menus are DOM, they can never match
    },
  },
  // do NOT touch dpiConfig or rotationConfig (rotation changes force restartPackage)
}
```

Apply with the call we already make:
`applyAppConfigToService(listOf(root.toString()), Bundle().apply { putInt("args_operation_flag", 0); putBoolean("args_save_mmkv", true) })`
(`fw/.../EInkHelper.java:230-236`; bundle keys `fw/android/onyx/BroadcastHelper.java:92`, `:101`).

If any `activityConfigMap` entry exists whose key is a substring of our activity name, mirror the same
`noteConfig` into it — `fuzzyMatchActivityConfig` prefers it over `globalActivityConfig`
(`EACAppConfig.java:146-157`). EinkWise itself mirrors note fields into every activity entry
(`kcb/.../eac/util/EACViewUtils.java:408-444`).

### Step 2 — restart the app process

`EACScreenNoteManager.packageType` is one-shot (§1.3). Nothing short of a process restart re-arms it.

### Step 3 — make the WebView detectable as the draw view

The `RustWebView` is created in code, so the `AttributeSet` constructor hook never runs. Either:

- **(a)** after the Rx init of `EACScreenNoteManager` has settled, call `webView.setId(View.generateViewId())`
  — `setId` invokes `EInkHelper.noteViewDetect(this)` (`fw/android/view/View.java:15128`);
  `getResourceEntryName` throws and is swallowed, so the FQCN branch of `matchView` decides; **or**
- **(b)** force it: reflect the public-but-hidden `View.noteViewDetect(boolean)`
  (`fw/android/view/View.java:11648-11650`) and call `webView.noteViewDetect(true)`. This bypasses
  the `packageType` race entirely and is the deterministic option.

Then a visibility transition to `VISIBLE` on that view runs `startEACScreenNote`
(`BaseHandler.java:412-422` → `:167-190` → `:331-342`).

### Step 4 — surrender the primitives we currently own, or do not opt in

If we proceed, our code must stop writing what `BaseHandler` now owns:
per-process pen state (`OnyxInk`'s `TouchHelper`/`RawInputReader` path), the handwriting region
limit/exclude, the stroke parameters, and the stylus-up reconcile timer. Running both is the failure
mode described in §3.2.

### Reverting

Write the same JSON back with `noteConfig.supportNoteConfig=false`, `noteConfig.enable=false`,
`drawViewKey` removed, and restart the process.

---

## Risks

1. **System-wide config corruption.** `applyAppConfigToServiceImpl` replaces the _whole_ stored
   `EACAppConfig` for the package named in our payload (`OECService.java:196-201`). A malformed or
   partial blob silently wipes that package's EinkWise settings, and the pkgName comes from _our_
   JSON — a bug there can clobber another app's config.
2. **Boot-time re-validation.** `LoadDeviceConfigFromLocalAction` runs `ValidateAppConfigAction` over
   every stored app config at load (`fw/.../action/LoadDeviceConfigFromLocalAction.java:45-51`),
   which rewrites `refreshModeIndex` / `updateMode` / `turbo`
   (`fw/.../action/ValidateAppConfigAction.java:102-186`). It does **not** touch `noteConfig`, but it
   _can_ undo our Speed profile after a reboot.
3. **Restart storms.** Touching `rotationConfig` triggers `ActivityThread.restartPackage`
   (`fw/.../EACConfigChangedImpl.java:196-198`); touching `dpiConfig` triggers a configuration
   dispatch (`:149-151`). Keep both untouched.
4. **`repaintEverything()` is global, not app-scoped** (`fw/android/onyx/ViewUpdateHelper.java:1057-1059`).
   Once armed, every finger-down in our view tree and every window-focus change full-flashes the
   panel — visible to the user as ghost-clearing flicker while typing, scrolling, or pulling the
   notification shade.
5. **Ink loss from state-machine races.** Two writers of the per-pid pen state on different threads
   (§3.2 #1) can leave the panel in `PEN_PAUSE` while our renderer believes it is drawing — strokes
   render into the buffer but never reach the panel until the next full refresh.
6. **IME lockout.** `SingleDrawViewHandler.resumeEACScreenNote` refuses to resume while the IME is
   visible (`SingleDrawViewHandler.java:26-32`); a WebView-hosted keyboard could leave the pen paused.
7. **`setAutoSyncBufEnable(false)`** for the whole visible lifetime of the draw view (`BaseHandler.java:170`)
   changes framebuffer semantics for the entire process, including any non-ink surface we draw.
8. **The theme layer can silently revert us.** The per-app config also lives inside an `EACAppTheme`
   (`fw/.../data/v2/EACAppTheme.java:35, 117-119, 216-223`). `ApplyThemesAction` takes the _stored
   theme's_ `appConfig` and pushes it through `applyAppConfigToService`
   (`fw/.../action/ApplyThemesAction.java:99-106`, `OECService.java:307-316`), i.e. a full replacement
   from a copy that never saw our write. Our direct `applyAppConfigToService` call does **not** update
   the stored theme (EinkWise saves both — `ValidateAppConfigAction.java:122-125`). So the next time
   EinkWise or a theme sync applies a theme for our package, `noteConfig` reverts. Re-assert the
   config on every app start; do not assume it sticks.
9. **No permission check cuts both ways.** Nothing stops another app (or an OTA'd EinkWise) from
   overwriting our config.
10. **This is undocumented vendor behaviour on a hand-verified path.** Per `app/CLAUDE.md`, the BOOX
    handwriting path must not change its frame fence, eraser gate or display-mode ownership without a
    device retest — this change touches all three by construction.

---

## On-device experiments

Each is a single testable action. Prefix for all pen work: `adb logcat -c && adb logcat` in the
background, and `setprop persist.sys.oec.service.debug true` / `sys.oec.service.debug` to widen
`EACDebug` output (`fw/.../OECServiceUtils.java:9-15`); the screennote tag is
`EACDebugConstant.ScreenNoteDebugConfig.TAG_SCREEN_NOTE`.

1. **E1 — confirm the device gate.** Reflect
   `android.onyx.config.DeviceConfig.singleton().isSupportScribble()` from the app and log it; expect
   `true`.
2. **E2 — confirm the current package type.** Reflect
   `EACScreenNoteManager.sharedInstance().getPackageType()` and log it; expect `NOT_SUPPORT` today.
3. **E3 — read the live blob.** Dump `getAppConfigFromService(["dev.okhsunrog.tangleaf"])[0]` verbatim
   to a file and confirm the exact spelling of `globalActivityConfig.noteConfig.*` on this firmware.
4. **E4 — read a package that already has it on.** Dump the same blob for one of the six factory
   packages (§2.1) — `com.microsoft.office.onenote` and `com.evernote` are the two whose
   `drawViewKey` is a FQCN, i.e. the closest analogue to our `RustWebView`. Compare their live
   `noteConfig` against `res/raw/eac_<Build.MODEL>.json` inside `kcb.apk` to confirm the preset
   survived onto the device unmodified.
5. **E5 — flip only `supportNoteConfig`, restart, re-read E2.** Confirms the one-shot package-type
   path without arming anything else (`enable` still false ⇒ `BaseHandler` short-circuits at `:122`).
6. **E6 — check the EinkWise UI.** After E5, open EinkWise for our app and confirm the pen tab
   appears — that proves `supportNoteConfig` is the block you observed.
7. **E7 — arm fully on a scratch build**, with our own `TouchHelper` pen-state and region writes
   disabled, and log every `TAG_SCREEN_NOTE` line for one stroke; verify the sequence
   `PEN_START → PEN_DRAWING → (500 ms) → handwritingRepaint(rect)`.
8. **E8 — measure the repaint rect.** In E7, log the rect passed to `handwritingRepaint` and confirm
   whether it is the WebView bounds or the full display rect (the empty-`localVisibleRect` fallback,
   `EACScreenNoteUtils.java:48-49`).
9. **E9 — quantify the finger cost.** With the pipeline armed, tap once with a finger and count
   full-panel refreshes; that is the `repaintEverything()` on `onFingerDownImpl`.
10. **E10 — `noteViewDetect` reflection.** Call `webView.noteViewDetect(true)` by reflection and
    verify `webView.isNoteDrawView()` returns true, proving Step 3(b) works without the `setId` race.
11. **E11 — IME interaction.** With the pipeline armed, focus a text input in the WebView and confirm
    whether the pen resumes after the keyboard hides (`SingleDrawViewHandler.java:26-32`).
12. **E12 — reboot durability.** After a full opt-in, reboot and re-read the blob: confirm
    `noteConfig` survives and check whether `ValidateAppConfigAction` rewrote `refreshConfig`.
13. **E13 — validate against a live preset.** Install OneNote or Evernote (both factory-provisioned,
    §2.1), write a scribble in each, and log `TAG_SCREEN_NOTE`. This shows the intended pipeline
    behaviour on this exact firmware with a config we did not author — the reference to compare our
    own armed build against, and the cheapest way to judge §3 before touching our own config.
