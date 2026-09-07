# E-ink panel upkeep: stay inside EinkWise, or drive the panel ourselves

Onyx BOOX Note Air 4C (`lito`, Android 13, firmware `2026-04-28_17-50_4.2-rel_04282_555977efe`,
build 8548, Kaleido 3 CFA). Written 2026-09-07. **Nothing here is implemented.** This document
exists so the choice can be made on evidence rather than on feel.

## Source shorthands

| shorthand | path                                                                                    |
| --------- | --------------------------------------------------------------------------------------- |
| `fw/`     | `/home/okhsunrog/tmp_zfs/onyx_framework/out/sources/android/onyx/`                      |
| `aosp/`   | `/home/okhsunrog/tmp_zfs/onyx_framework/out/sources/android/`                           |
| `kcb/`    | `/home/okhsunrog/tmp_zfs/onyx_framework/out-kcb/sources/`                               |
| `stock/`  | `/home/okhsunrog/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/`          |
| `sdk/`    | `/home/okhsunrog/tmp_zfs/onyx_sdk_decompiled/src/`                                      |
| `kt/`     | `plugins/tauri-plugin-mobile-system/android/src/main/java/dev/okhsunrog/mobile_system/` |
| `ts/`     | `src/`                                                                                  |

Everything is cited to `file:line` or to a symbol. Anything that is inference says **[inference]**.

---

## The premise, corrected

The framing that prompted this study — "stock is excluded from EinkWise, so the tap counter behind
Settings → Display → Full Refresh Frequency never fires for it" — is **right in its conclusion and
slightly wrong in its mechanism**. It is worth getting exact, because the exact mechanism is also
the cheapest lever.

There are two independent gates, and only one of them is the one usually named:

**Gate 1 — the in-app reporter (this is the decisive one).** Every input event in every app is
reported to the firmware by the patched `ViewRootImpl`:

```java
// aosp/view/ViewRootImpl.java:11940-11958  (called for KeyEvent DOWN/UP and MotionEvent 0/1/5/6)
EInkHelper.inputEventChangedAsync(Process.myPid(), context, action, keyCode, simpleName);
```

and that call is filtered, inside our own process, on `supportEAC`:

```java
// fw/optimization/EInkHelper.java:1127-1146
Observable.just(context)…
    .filter(obj -> EInkHelper.isAppConfigSupportEAC())      // :1131
    .doOnNext(obj -> EInkHelper.inputEventChangedImpl(...))  // :1136 → EACUtils.writePipeMsg(...)
```

```java
// fw/optimization/EInkHelper.java:1164-1168
public static boolean isAppConfigSupportEAC() {
    if (isServiceReady() && isEnable() && !TextUtils.isEmpty(getReadOnlyAppConfig().getPkgName()))
        return getReadOnlyAppConfig().isSupportEAC();
    return false;
}
```

With `supportEAC == false` the pipe message is never written, so `OECService` never sees a finger
lift for that app at all. That is why stock Notes' taps cannot be counted: **the counter is not
disabled, it is starved.**

**Gate 2 — the debouncer install.** The counter's _consumer_ is also gated, on the service side:

```java
// fw/optimization/impl/TabletEACRefreshImpl.java:143-149
private void handleSFDebouncer(EACAppConfig cfg, EACRefreshConfig cur, EACRefreshConfig fallback) {
    if (cfg.isSupportEAC()) applySFDebouncer(cfg.getPkgName(), cur);
    else                    clearSFDebouncer();          // ViewUpdateHelper.debouncer(false,0,0,0,0)
}
```

`handleSFDebouncer` runs on every resume (`:239-248`), on every config apply (`:225-236`) and on
every system-window show/hide (`:190-201`). So for a `supportEAC:false` app SurfaceFlinger is left
with **no debouncer at all** — no post-input settle repaint, no `gcInterval` counter to tick.

What is **not** gated is the tick itself. `OECService`'s pipe callback passes
`ensureAppConfig(pkg)` straight through, and `EACBaseRefreshImpl.onInputEventUpdate` reads
`eACAppConfig.getRefreshConfig(componentName)` directly rather than through the `supportEAC`-aware
`getRefreshConfigByCurrentComponent`:

```java
// fw/optimization/OECService.java:585-593        (the native /dev/onyx/listener pipe callback)
getRefreshImpl().onInputEventUpdate(pipeMessage, getCurrentTopComponent(),
    getReadOnlyDeviceConfig().ensureAppConfig(getCurrentTopComponent().getPackageName()));

// fw/optimization/impl/EACBaseRefreshImpl.java:149-151
public void onInputEventUpdate(PipeMessage m, ComponentName cn, EACAppConfig cfg) {
    handleInputEventImpl(m, cfg.getRefreshConfig(cn));      // NOT getRefreshConfigByCurrentComponent
}
```

and stock Notes' own config even leaves the refresh block **enabled** while `supportEAC` is false:

```java
// fw/optimization/data/v2/EACAppConfig.java:100-111
public static EACAppConfig createOnyxAppConfig(String str) {
    …
    eACAppConfig.supportEAC = false;                                        // :104
    eACAppConfig.globalActivityConfig.getRefreshConfig().setEnable(true);   // :108
    eACAppConfig.globalActivityConfig.getDisplayConfig().setEnable(true);   // :109
}
```

So the correct statement is: **the tap counter is closed at the source (gate 1, in our own process),
and its effect is separately neutered at the sink (gate 2, no debouncer installed).** Either alone
would remove the flash; both are flipped by the same boolean.

For completeness, the third link in the chain — what the counter _does_ when it is fed:

```java
// fw/optimization/impl/EACBaseRefreshImpl.java:25-45
if (MOTION_EVENT_TYPE && eventAction == 1) {          // ACTION_UP
    increaseRepaintCount(cfg);                        // → ViewUpdateHelper.debounceIncRefresh()
    EACUtils.applyDebouncerTransientUpdateMode(pkg, mode);
}
// :47-52 — the tick is itself gated on the RAW updateMode being in {0,3,5}
if (Constant.DEBOUNCER_UPDATE_MODE_MAP.containsKey(cfg.getUpdateMode())) ViewUpdateHelper.debounceIncRefresh();
```

`DEBOUNCER_UPDATE_MODE_MAP = {3→0, 5→0, 0→0}` (`fw/optimization/Constant.java:391-397`).
`applyDebouncerTransientUpdateMode` has an extra `i != 0` guard (`fw/optimization/EACUtils.java:25-31`),
so under **HD** (mode 0, which is what we write) the input-up path does _only_ `debounceIncRefresh()`;
the Regal-flavoured transient repaint is a `refresh_mode_1` behaviour, not ours.

The GC cadence is `gcInterval`, default **20**, `<=0` → `Integer.MAX_VALUE`
(`fw/optimization/data/v2/EACRefreshConfig.java:15`, `:76-82`), UI range 0…50 step 5
(`docs/onyx-reversing/07-einkwise.md:164`). That seekbar _is_ Settings → Display → Full Refresh
Frequency, and it is hidden unless the stored raw `updateMode ∈ {0,3}`
(`07-einkwise.md:221`, `EACViewConfigs.showByType`).

Our own commit `c89bc5f` already recorded the observed cadence as "ten taps", which is a
`gcInterval` of 10 rather than the default 20 — i.e. the user has moved that slider.

---

## 1. What stock actually does instead

### 1.0 A correction that reframes everything below

**Stock's per-stroke ink reconcile — the routine our `refreshFrame()` is modelled on — does not run
on this device.** `EpdShapeHandler.P0()` is gated:

```java
// stock/note/note/handler/common/EpdShapeHandler.java:248-263
if (!listB0.isEmpty() && OnyxSystemConfig.isUseHWUpdateForPenUpRefresh()) { … new GrayscaleRefreshAction(…) }
```

```java
// stock/sdk/data/config/system/OnyxSystemConfig.java:145-150
public static boolean isUseHWUpdateForPenUpRefresh() {
    if (SystemPropertiesUtil.isTablet()) return false;
    return a().enablePenUpRefresh;
}
// stock/sdk/data/config/system/data/SystemConfigBean.java:21-22
public int penUpRefreshTimeMs = 500;
public boolean enablePenUpRefresh = !DeviceInfoUtil.isColorDevice();     // ← false on a Kaleido 3
```

So on a Note Air 4C, `GrayscaleRefreshAction` — the single site that ever sets
`HAND_WRITING_REPAINT_MODE` in the whole app (`stock/note/note/action/render/GrayscaleRefreshAction.java:60`)
— is off by default. It is a user setting (`stock/sdk/note/ui/setting/viewmodel/SettingViewModel.java:111`,
`LoadSettingsDataAction.java:43`), so it can be turned on, but the shipping behaviour on our panel
is: **stock never repaints ink in a special mode after a stroke.**

What repaints stock's ink instead is mechanism A/B below — the full-view `View.invalidate()`
bracketed by a raw-pen render pause/resume — fired by ~50 UI events. Our comment at
`kt/OnyxInk.kt:489-495` ("Stock's `GrayscaleRefreshAction`, and nothing else") therefore describes
a code path the stock app does not execute on this hardware. **[This is the most consequential
finding in the document after §6.]**

### 1.1 The six mechanisms

| #   | Mechanism                                                       | Entry point                              | Sites                                                         |
| --- | --------------------------------------------------------------- | ---------------------------------------- | ------------------------------------------------------------- |
| A   | Full-view refresh with a raw-pen render **and input** cycle     | `InvalidateScreenAction`                 | **40** constructions                                          |
| B   | The same without the delay wrapper                              | `InvalidateViewWithPenControlAction`     | **9** constructions + 2 direct event posts + 3 REGAL variants |
| C   | Per-stroke ink reconcile in `HAND_WRITING_REPAINT_MODE`         | `GrayscaleRefreshAction`                 | **1** construction, 2 call paths — **off on this device**     |
| D   | Counted GC                                                      | `common/utils/EpdDeviceManager`          | **6** `applyGCWithInterval` + **3** `applyGCImmediately`      |
| E   | "Fast mode": `applyTransientUpdate(ANIMATION_QUALITY)` + dither | `EpdEventHandler` ← `ApplyFastModeEvent` | **36** producers, 1 consumer                                  |
| F   | One-shot `REGAL_PLUS` on the next frame                         | `InkRenderManager.enableRegalModeOnce()` | **9**                                                         |

Plus one-off primitives: `repaintEveryThing(GC)` ×2, `enablePost` ×2, `useGCForNewSurface` ×3,
`enableColorCU`/`disableColorCU` ×6, `applyAppScopeUpdate`/`clearAppScopeUpdate` ×2, `resetEpdPost` ×2,
`handwritingRepaint` ×1 (a data-binding adapter for ordinary UI views, `stock/note/common/utils/DataBindingUtils.java:69-73`),
`waitForUpdateFinished` ×1 (**dead**, `stock/note/note/request/WaitViewUpdateRequest.java:18`).

Total `EpdController` call sites: **61**, of which **27** are in `note/test/*` debug activities;
**~34 real**, of which **~16** actually move the panel.

### 1.2 What a "refresh" _is_ in stock

```java
// stock/sdk/notecore/editor/display/InkRenderManager.java:79-97
public final void invalidate(DisplayArgs a) {
    if (a.getF3819d()) enableRegalModeOnce();     // one-frame REGAL_PLUS tag
    if (a.getB())      enablePost();              // EpdController.enablePost(view, 1)
    setViewUpdateModeOnce();                      // :115-123 — View.setDefaultUpdateMode(mode)
    if (a.getC()) view.invalidate();              // default true
    resetViewUpdateMode();                        // :103-108
}
```

`DisplayArgs` defaults: `enablePost=false`, `invalidate=true`, `enableRegalModeOnce=false`
(`stock/sdk/data/note/DisplayArgs.java:19-23`). So a refresh is **literally `View.invalidate()` on
the editor view**, optionally tagged for one frame with `REGAL_PLUS` or `HAND_WRITING_REPAINT_MODE`.
Everything else in stock's policy is about _when_ to do it and how to bracket it with a raw-pen
pause/resume.

### 1.3 Mechanism A — `InvalidateScreenAction`, the workhorse

`stock/note/note/action/render/InvalidateScreenAction.java`. The constructor (`:29-39`) builds a
`RawPenArgs` with **pause render + pause input + resume render + resume input**, with
`resumeDelayTime = PenEvent.DELAY_ENABLE_RAW_DRAWING_MILLS`
(`stock/sdk/notecore/editor/data/RawPenArgs.java:68-70`: `isColorDevice() ? 500 : 150`). The pipeline
(`:103-127`) is: main thread → optional `enableRegalModeOnce()` (`:89-93`) → optional
`delay(delayRefreshTime)` (`:71-81`) → **only if a document is open** post
`InvalidateViewWithPenControlEvent(rawPenArgs)` (`:83-87`).

The event is consumed by the **active tool handler**, so the refresh is handler-gated:
`ScribbleHandler.java:1008-1017` runs it; `SelectionHandler.java:2174-2180` **overrides it and skips
the refresh entirely while a selection is non-empty.**

Builder knobs: `setCheckUseRegalMode`, `setDelayRefreshTime`, `setDelayResumePenTimeMs`,
`setPauseRawDrawingRender`, `setPauseRawInputReader`, `setResumeRawDrawing`, `clearDelayResumePenTimeMs`
(`:96-160`).

The 40 triggers, grouped by kind:

| Kind                             | Triggers                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Focus / lifecycle**            | window focus **gained or lost** (`PenEventHandler.java:334-347`); screen rotation ≥1 step (`DeviceReceiverEventHandler.java:228-234`); editor view (re)attach/resize with nothing to render (`RenderToEditorViewAction.java:51-63`)                                                                                                                                                                                                                                                                                |
| **Popups / dialogs / OS chrome** | every `BasePopWindow` show **and** dismiss (`PopupChangeAction.java:37`, `setCheckUseRegalMode(!show)`, `setResumeRawDrawing(false)`); every system toast appear/disappear (`ToastChangeAction.java:40`); the Onyx floating nav ball appearing/disappearing (`PenEventHandler.java:367`, `setDelayResumePenTimeMs(500)`)                                                                                                                                                                                           |
| **Toolbars / menus**             | edge-swipe toolbar show/hide (`SlideToolbarVisibilityToggleDetector.java:70`, `:77`); toolbar pick-up and left/right flip (`ToolbarMenuModel.java:583-585`); float-menu re-layout (`FloatToolMenuHandler.java:554-556`), move (`FragmentFloatMenu.java:213-215`), movable-state change (`MenuEventHandler.java:170-175`); audio menu, **9** sites (`AudioMenuModel.java:337-339`); "add custom pen" (`FunMenuHandler.java:53-60`); `setFloatMenuMoveEnabled` (`BaseDocMenuHandler.java:148-151`)                   |
| **Tool changes**                 | pen/shape pick, 6 adapters (`ShapeChangeAction.java:38`); float-menu pen change, 4 adapters (`FloatPenChangeAction.java:64`); pen↔eraser toggle (`TogglePenTypeAction.java:46`); enter text tool (`StartTextInputAction.java:69`); enter/leave fill (`StartFillColorAction.java:55`, `QuitFillColorAction.java:34`); enter universal shape (`StartUniversalShapeAction.java:37`); enter **and** leave rich text (`RichTextHandler.java:420`, `:468`); "erase all / by type" (`EraseByTypeAction.java:79`)          |
| **Gestures**                     | lasso pen-up, only when not moving, **`setDelayRefreshTime(300)`** (`SelectionHandler.java:2155-2161`); zoom-selection rect rejected (`ZoomSelectionHandler.java:87-90`); finger down on a draggable float widget (`CommonEventHandler.java:158-161` ← `DragGestureListener.java:106`)                                                                                                                                                                                                                             |
| **Explicit / external**          | broadcast `onyx.android.intent.action.REFRESH_SCREEN` (`EpdShapeHandler.java:359-362`); broadcast `ctp.status.change` (`ScribbleFragment.java:1986-1989`); toolbar "disable touch" (`ToggleFingerTouchAction.java:70`); the GC cadence itself (`EpdDeviceManager.java:19`)                                                                                                                                                                                                                                         |
| **Content oddities**             | writing on a hidden layer, 2 sites (`ScribbleFragment.java:1210`, `:1223`); HWR placement fragment becoming visible (`FragmentHWRPlacement.java:586-590`); HWR diagram conversion finished (`HWRDiagramConvertAction.java:711`, `setPauseRawInputReader(false)`); meeting panel show/hide (`SetEditorViewVisibilityAction.java:39`); `ScribbleHandler.refreshScreen()` (`:1254-1256`) and `onRefreshPage()` (`:1162-1169`, `doOnComplete` → `repaintEveryThing(GC)`); the same for HWR (`HWRHandler.java:151-158`) |

`ColorDeviceUtils.refreshScreen/refreshScreenDelay(600)` (`stock/note/note/utils/ColorDeviceUtils.java:13-25`)
is unreachable — no callers.

### 1.4 Mechanism B — `InvalidateViewWithPenControlAction`

`stock/note/note/action/render/InvalidateViewWithPenControlAction.java:62-86`:

```
pauseRawPen(rawPenArgs) → observeOn(mainUI) → renderManager.invalidate(displayArgs) → resumeRawPen(rawPenArgs)
```

Defaults `RawPenArgs.pauseResumeArgs()` (pause both, resume both, delay 0) and a plain `DisplayArgs`
(`:27-29`). The 9 triggers: the `InvalidateViewWithPenControlEvent` from every `InvalidateScreenAction`
(`ScribbleHandler.java:1016`); **erase finish** (`ScribbleHandler.java:982`, `EraseHandler.java:102`,
`UniversalShapeHandler.java:639`); a settings/template/background sub-fragment closing
(`EpdEventHandler.java:296`, with `DisplayArgs.enableRegalDisplayArgs()` → a **REGAL_PLUS** frame);
leaving note-link jump mode (`MenuBundle.java:85`); float-menu style switch
(`NestedListFloatMenuMode.java:234`); a fill operation completing (`FillColorAction.java:252`);
entering lasso (`StartSelectionAction.java:77`).

### 1.5 Mechanism D — the counted GC, in full

```java
// stock/note/common/utils/EpdDeviceManager.java  (63 lines; the whole file is policy)
private int a = 10;   // :10  gcInterval
private int b;        // :11  refreshCount
private void a(EditorBundle eb) {                              // :16-20
    EpdController.byPass(0);
    EpdController.applyGCOnce();
    new InvalidateScreenAction(eb).execute();
}
public void applyGCImmediately(EditorBundle eb) { this.b = this.a; applyGCWithInterval(eb); }   // :29-32
public void applyGCWithInterval(EditorBundle eb) {                                              // :34-41
    int i = this.b; this.b = i + 1;
    if (i >= this.a) { this.b = 0; a(eb); }
}
public void checkIntervalChanges(int i) { if (i - 1 == getGcInterval()) return; setGcInterval(i); }  // :43-48
public void resetRefreshCount() { this.b = 0; }                 // :54-56
public EpdDeviceManager setGcInterval(int i) { this.a = i - 1; this.b = 0; return this; }        // :58-62
```

So `setGcInterval(N)` ⇒ `a = N-1` ⇒ **a GC fires on every Nth tick**, then the counter resets.
The interval comes from the app's own setting, not from EinkWise:

```java
// stock/note/note/action/setting/LoadAndApplySettingAction.java:37-39
EpdDeviceManager.instance().checkIntervalChanges(
    noteSetting.isEnableFullRefresh() ? noteSetting.getFullRefreshCount() : Integer.MAX_VALUE);
// stock/sdk/note/ui/setting/bean/NoteSetting.java:46-47
public int fullRefreshCount = 10;
public boolean enableFullRefresh = DeviceInfoUtil.isColorDevice();     // ← TRUE on a Note Air 4C
```

**On our panel stock GCs every 10 ticks; on a mono device it never GCs at all.** Reloaded at app
start (`NoteApplication.java:214-216`), on the broadcast `note.setting.changed.action`
(`KNoteCommonReceiver.java:54-56`, `:187-189`), and from `HWRToolMenuModel.java:112`.

**What counts as a tick — six sites, and note what is absent:**

| Site                                                                | Event                       |
| ------------------------------------------------------------------- | --------------------------- |
| `stock/note/note/action/page/BasePageAction.java:124`               | **every page change**       |
| `stock/note/note/menu/SelectionMenuHandler.java:346`                | every selection-menu click  |
| `stock/note/note/menu/TextInputMenuHandler.java:258`                | every text-input-menu click |
| `stock/note/note/menu/viewmodel/scribble/FuncBarMenuModel.java:229` | rename note                 |

**There is no per-stroke, per-scroll, per-dialog or per-N-strokes GC anywhere.** Strokes never tick
the counter.

Forced GC now (`applyGCImmediately`, 3 sites): the explicit **"Refresh" toolbar button**
(`ToolbarMenuModel.java:1051-1054`); `QuitFastModeImmediatelyEvent` with flag true
(`EpdEventHandler.java:271`, from `TextInputHandler.java:270` and `TextInputHelper.java:180`).

Counter **resets** (so a cheap refresh "pays off" the debt): `EpdEventHandler.java:187` on leaving
fast mode, and `:273` on quitting fast mode without a GC.

### 1.6 Mechanism E — fast mode

`stock/note/note/eventhandler/EpdEventHandler.java` is the single owner. Enter (`a()`, `:109-116`):
cancel timers → `Device.setDitherThreshold(EPD_HIGH_CONTRAST)` → `EpdController.setEpdTurbo(deviceConfig.epdTurbo)`
→ `EpdController.applyTransientUpdate(transformUpdateModel ?: ANIMATION_QUALITY)` → post
`FastModeStateChangedEvent(true)`. Exit (`d()`, `:136-142`): `setDitherThreshold(128)`,
`clearTransientUpdate(false)`, release the wakelock.

Constants: `EPD_HIGH_CONTRAST = isColorDevice() ? 160 : 255` (`:80-82`), restored to `128`;
auto-exit timer **5000 ms** plus a **5000 ms** wakelock (`:124-128`); `DelayApplyFastModeEvent`
default delay **200 ms** (`stock/note/note/event/zoom/DelayApplyFastModeEvent.java:5`);
`deviceConfig.epdTurbo` default **5** (`stock/sdk/note/ui/config/DeviceConfig.java:82`).

Exit is **deferred, not immediate**: `k()` (`:185-190`) checks the active handler's
`renderInFastMode()`, calls `resetRefreshCount()`, then arms the 5 s timer rather than exiting;
`e()` (`:144-154`) refuses to exit while a finger is still down.

36 producers, essentially "anything that animates": pinch and scroll begin/end
(`ViewportEventHandler.java:82`, `:97`, `:116`, `:131`), float-menu and widget drags, zoom preview,
selection transform, rectangle-selection drag, the thumbnail list, the canvas seekbar, text input,
HWR placement, and the SDK's own `AutoFastModeNestedScrollView` / `AutoFastModeRecyclerViewOnScrollListener`.

### 1.7 The scroll path, specifically

`stock/note/note/eventhandler/ViewportEventHandler.java`: scale begin `:77-84` and scroll begin
`:111-118` → zoom thumbnail + `ApplyFastModeEvent(true)` + `PauseResumeRawPenAction`; scale/scroll
end `:92-98`, `:120-134` → `ApplyFastModeEvent(false)` + `UpdateRawDrawRectAction`.

**No GC, no update-mode change, no `InvalidateScreenAction` on scroll or zoom.** The only
panel-level effect of scrolling is the transient `ANIMATION_QUALITY`, released 5 s after the gesture
ends.

### 1.8 Undo / redo

`stock/note/note/action/undoredo/UndoRedoAction.java`: `pauseRawPen(RawPenArgs.pauseArgs())` (`:265`)
→ `forceRenderVisibleDirtyScreenRectToDisplay` (`:294`) → `renderManager.invalidate()` → `resumePen()`
(`:199`, `:361-363`). **No GC, no update-mode override, no `InvalidateScreenAction`.**

### 1.9 Entering and leaving a writing session

- `onWindowFocusChanged` → `ActivityFocusChangedEvent` → `PenEventHandler.java:334-347`: stores the
  focus flag, **resets `statusBarShowing`**, and runs `InvalidateScreenAction` on **both** gain and
  loss (gated on the active handler wanting the pen).
- `resumeComponentsImpl` → `ResumeScribbleComponentEvent` → `EpdEventHandler.java:299-305` re-enters
  fast mode if the handler wants it.
- `ScribbleFragment.onDestroy` (`:1482-1496`) → `EpdController.resetEpdPost()`.
- `ScribbleFragment.onResume`/`onPause`/`onStop`/`onSupportVisible`/`onSupportInvisible` contain
  **no refresh calls at all**.

### 1.10 Time- and count-based, complete

| Kind                                | Value                                                                                        | Where                                                                    |
| ----------------------------------- | -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| GC every N ticks                    | `a = fullRefreshCount - 1`; `fullRefreshCount` default **10**, enabled iff `isColorDevice()` | `EpdDeviceManager.java:10`, `:34-41`, `:58-62`; `NoteSetting.java:46-47` |
| Fast-mode auto-exit                 | **5000 ms** (+5000 ms wakelock)                                                              | `EpdEventHandler.java:124-128`                                           |
| Delayed fast-mode apply             | **200 ms**                                                                                   | `DelayApplyFastModeEvent.java:5`                                         |
| Resume raw pen after a pause        | **500** (colour) / **150** (mono)                                                            | `RawPenArgs.java:68-70`                                                  |
| Popup resume                        | **500** / **300**                                                                            | `PenEvent.java:9`, `:19-21`                                              |
| Selection resume                    | **500**                                                                                      | `PenEvent.java:10`                                                       |
| Shape-change resume                 | **100**                                                                                      | `PenEvent.java:11`                                                       |
| Lasso pen-up refresh delay          | **300 ms**                                                                                   | `SelectionHandler.java:2159`                                             |
| Erase resume delay                  | **300 ms**; pre-delay `eraserAreaDisplayDelayMillSecond` default **0**                       | `EraseFinishAction.java:158-160`; `DeviceConfig.java:89`                 |
| Page-turn resume budget             | `max(600 − pageChangeDurationMs, DELAY_ENABLE_RAW_DRAWING_MILLS)`                            | `BasePageAction.java:122`                                                |
| Eraser-tool select pause/resume     | **150 ms**                                                                                   | `EraseHandler.java:63-65`                                                |
| Material-centre app-scope fast mode | **500 ms**                                                                                   | `MaterialCenterFragment.java:116-126`                                    |

### 1.11 EinkWise from stock's side: one dead class

The only place stock touches EAC at all is
`stock/note/note/eventhandler/UpdateModeEventHandler.java:24-31`, `:51-74`, which would push the
app's EAC refresh mode to `2` via `ChangeAppUpdateModeAction`. **Its two trigger events
(`RichTextApplyFastUpdateModeEvent`, `RichTextRestoreUpdateModeEvent`) are never posted anywhere in
the APK.** `NoteApplication.java` contains no EPD call at all; the only global refresh policy set at
startup is the GC interval (`:214-216`). So stock genuinely never delegates to the automatic manager
— consistent with `supportEAC:false`.

---

## 2. The shape of the policy, separated from its call sites

### 2.1 The rules, stated without Android

Strip the views away and stock's policy is nine rules. Nothing else in the app decides a refresh.

| R      | Rule                                                                                                                                                                                                                                                                                                                                    | Where it lives                                                                                                                                                                                                                |
| ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **R1** | **Anything that drew over the writing surface, on both appearance and disappearance, gets: pause the pen layer → repaint the whole surface in the ordinary mode → resume the pen layer after a delay.** The delay is 500 ms on a colour panel, 150 ms otherwise; 500 ms for popups, 300 ms after an erase, 100 ms after a shape change. | `InvalidateScreenAction` / `InvalidateViewWithPenControlAction`; `RawPenArgs.java:68-70`, `PenEvent.java:8-11`                                                                                                                |
| **R2** | **The repaint is the whole surface, always.** Stock never passes a rectangle to the panel. The only region-limited work is the _software_ render into the page bitmap.                                                                                                                                                                  | `InkRenderManager.java:79-97` (bare `view.invalidate()`); `ForceRenderSliceListActionsKt`                                                                                                                                     |
| **R3** | **Count "screen changes", and do a real GC every Nth.** A screen change is a **page change** or a **menu action that mutates the document** — never a stroke, never a scroll, never a dialog. N is a user setting, default 10, and the feature is on by default only on colour panels.                                                  | `EpdDeviceManager.java:34-41`; `BasePageAction.java:124`; `SelectionMenuHandler.java:346`; `TextInputMenuHandler.java:258`; `FuncBarMenuModel.java:229`; `NoteSetting.java:46-47`                                             |
| **R4** | **A refresh that was just performed pays off the debt: reset the counter.** Leaving fast mode resets it; an explicit refresh sets it to full so the next tick fires immediately.                                                                                                                                                        | `EpdDeviceManager.java:29-32`, `:54-56`; `EpdEventHandler.java:187`, `:273`                                                                                                                                                   |
| **R5** | **An explicit user request for a clean panel refreshes now**, not on the counter.                                                                                                                                                                                                                                                       | `ToolbarMenuModel.java:1051-1054`                                                                                                                                                                                             |
| **R6** | **While something is animating, put the panel in a transient fast mode and raise contrast; leave it 5 s after the animation stops, not immediately, and never while a finger is still down.**                                                                                                                                           | `EpdEventHandler.java:109-116`, `:124-128`, `:136-154`                                                                                                                                                                        |
| **R7** | **Never enter that fast mode for a stylus stroke.** Only viewport transforms, selection transforms and list scrolling qualify; `renderInFastMode()` is false in the base handler and every writing handler inherits it.                                                                                                                 | `stock/sdk/notecore/editor/handler/BaseHandler.java:278-281`                                                                                                                                                                  |
| **R8** | **Some surfaces suppress the refresh instead of performing it.** While a selection exists, the refresh is dropped entirely rather than repainting over a live overlay.                                                                                                                                                                  | `SelectionHandler.java:2174-2180`                                                                                                                                                                                             |
| **R9** | **A page's first frame, and a frame after a settings surface closes, goes out one notch better** — a one-shot `REGAL_PLUS` tag rather than a GC.                                                                                                                                                                                        | `GotoPageImplAction.java:247`; `PageInsertDataAction.java:128`; `EpdEventHandler.java:296`; `StatusBarChangeAction.java:41`; `PopupChangeAction.java:37` (`setCheckUseRegalMode(!show)` — **dismiss** gets it, show does not) |

Two rules that are _absent_ and worth naming, because their absence is the design:

- **There is no time-based refresh.** No idle timer ever refreshes the panel. Every timer in §1.10
  is a _resume delay_ or a _fast-mode exit_, never a repaint trigger.
- **There is no stroke-driven refresh** on this device (§1.0). Ink reaches the panel because the
  firmware drew it and because the next R1 event repaints over it.

### 2.2 The counting

| Quantity                                     | Value                                                                                                                                                                                                                                                |
| -------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `EpdController` call sites                   | **61** total, **27** in `note/test/*`, **~34** real, **~16** that move the panel                                                                                                                                                                     |
| R1 call sites                                | **40** (`InvalidateScreenAction`) + **9** (`InvalidateViewWithPenControlAction`) + 5 event posts                                                                                                                                                     |
| R3/R4/R5 call sites                          | **6** `applyGCWithInterval` + **3** `applyGCImmediately` + **2** `resetRefreshCount`                                                                                                                                                                 |
| R6/R7 call sites                             | **36** producers, **1** consumer class                                                                                                                                                                                                               |
| R9 call sites                                | **9**                                                                                                                                                                                                                                                |
| **Distinct actions/classes carrying policy** | **17**                                                                                                                                                                                                                                               |
| **Policy lines** (decisions)                 | **~450–650** decompiled source lines                                                                                                                                                                                                                 |
| **Plumbing lines** (view/Rx/geometry wiring) | `ForceRenderSliceListActionsKt` 44 KB + `ComputeBoxListActionsKt` 41 KB + `DocCommonActionsKt` 22 KB + `ForceDisplayActionsKt` 19 KB + `EditorShapesActionsKt` 17 KB + `DisplayActionsKt` 16 KB ≈ **160 KB containing not one `EpdController` call** |
| **Ratio**                                    | **≈ 1 : 15** policy to plumbing, by decompiled LoC                                                                                                                                                                                                   |

The 17 policy classes: `common/utils/EpdDeviceManager` (63 L — the whole file is policy),
`eventhandler/EpdEventHandler` (317 L), `eventhandler/PenEventHandler` (570 L, two 17-flag
predicates at `:105-107` and `:271-273`), `eventhandler/DeviceReceiverEventHandler` (293 L),
`eventhandler/ViewportEventHandler` (145 L), `eventhandler/ColorModeEventHandler` (85 L),
`eventhandler/UpdateModeEventHandler` (75 L, **dead**), `action/render/InvalidateScreenAction` (164 L),
`action/render/InvalidateViewWithPenControlAction` (97 L), `action/render/GrayscaleRefreshAction`
(110 L, **inactive on this panel**), `action/render/{Display,ConcatDisplay}VisibleBoxAction` (~180 L),
`action/ui/{PopupChange,Toast,StatusBar,ScreenShot}ChangeAction` (283 L), `event/PenEvent` (91 L),
`sdk/…/RawPenArgs`, `sdk/…/InkRenderManager` (142 L), `handler/common/EpdShapeHandler` (`P0`/`B0`/`F0`,
~90 L), `utils/ColorDeviceUtils` (26 L, **dead**).

**The load-bearing observation.** The 40 + 9 R1 sites are not 49 different policies; they are one
policy applied at 49 places, and the reason there are 49 is that in an Android app _every_ surface
that can appear over the canvas is a separate class. A WebView app has **one** such surface.

### 2.3 The unstated invariant

Stock never repaints ink for its own sake. Every R1 firing is triggered by _something else_ having
covered the panel, and the ink repaint is a side effect of restoring the app's own content. That is
also — per `handwriting-logic-review.md` §0 — why stock never accumulates a stuck SurfaceFlinger
handwriting schema: each R1 does `setRawDrawingRenderEnabled(false) → invalidate → (true)`, i.e. a
`leaveScribbleMode` plus a `PEN_PAUSE`/`PEN_DRAWING` transition, ~50 times per session. **[inference,
but the strongest one in this document: it explains stock's self-healing without any explicit
"clear region" call, of which the APK has none.]**

---

## 3. How much of it applies to us

### 3.0 Our hook points, as they exist today

Before mapping stock's rules onto us, here is the inventory of places we can hang a refresh, and
what each already does.

| Hook                                              | Where                                                                                                                                                        | What runs there now                                                                                                                                                        |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Workspace reducer** — every navigation          | `ts/features/workspace/workspace-store.ts:16-24`                                                                                                             | nothing. It used to hold a `NAVIGATIONS` set (`open_target`, `go_back`, `go_forward`, `close_pane`, `show_compact_pane`) calling `noteNavigation()`; removed in `c89bc5f`. |
| **Overlay close** — dialog, popover, select, menu | `ts/components/ui/{dialog,popover,select}.tsx`, `ts/features/handwriting/{new-note-button,handwriting-note-view}.tsx`, `ts/features/pages/note-row-menu.tsx` | `refreshPanelAfterClose` → `requestFullRefreshSoon()` (`ts/app/eink-refresh.ts:53-60`)                                                                                     |
| **The settle debouncer**                          | `ts/app/eink-refresh.ts:22`, `:37-44`                                                                                                                        | `SETTLE_MS = 300`, coalescing a burst into one flash                                                                                                                       |
| **The e-ink gate**                                | `ts/app/appearance.tsx:121`                                                                                                                                  | `setEinkRefreshEnabled(resolved.display === "eink")`                                                                                                                       |
| **The refresh primitive**                         | `kt/MobileSystemPlugin.kt:462-479` (`requestFullRefresh`)                                                                                                    | `EpdController.invalidate(webView, UpdateMode.GC)`                                                                                                                         |
| **Ink session pause / resume**                    | `kt/OnyxInk.kt:592-612`, `:270-280`                                                                                                                          | tears down the SF session, invalidates the whole reconcile region, re-invalidates the WebView                                                                              |
| **Ink session close**                             | `kt/OnyxInk.kt:717-725`                                                                                                                                      | `region.reset()`, then a delayed refresh at `CLOSE_REFRESH_DELAY_MS = 300`                                                                                                 |
| **IME hide**                                      | `kt/MobileSystemPlugin.kt:102`                                                                                                                               | one GC                                                                                                                                                                     |
| **Per-stroke reconcile**                          | `kt/OnyxInk.kt:496-546` (`refreshFrame`)                                                                                                                     | one `HAND_WRITING_REPAINT_MODE`-marked frame over the stroke union                                                                                                         |
| **Display-mode stack**                            | `kt/DisplayModeStack.kt`, `kt/ViewDisplayMode.kt`                                                                                                            | layered `BASE` / `SESSION` / `TRANSIENT` view default update mode with correct restore                                                                                     |
| **Transient quality mode**                        | `kt/InkRefreshPolicy.kt` + `kt/OnyxInk.kt:503-520`                                                                                                           | `applyTransientUpdate(ANIMATION_QUALITY)` for a selection drag only, released after `QUIET_MS = 5000`                                                                      |
| **Stored EAC profile**                            | `kt/MobileSystemPlugin.kt:292-330`                                                                                                                           | writes `refresh_mode_4` / `updateMode 0` / `turbo 0` once, guarded by a SharedPreference marker                                                                            |

Two structural differences from stock that decide most of the mapping:

- **Our UI is DOM.** Menus, dialogs, toolbars, the selection popup and navigation are React
  components inside one Android view. Stock's refresh policy is spread across `BaseNoteDialog`,
  `BaseNotePopWindow`, `BaseDocPopWindow`, `ToolbarMenuModel`, `FuncBarMenuModel`,
  `SelectionMenuHandler`, `TextInputMenuHandler` and `BasePageAction` — **none of which has an
  Android-view counterpart in our app.** Their triggers exist for us; their call sites do not.
- **We have events stock does not.** Route changes inside the SPA, pane splits and closes, a
  compact-layout pane swap, virtual-list scrolling inside a page — all of which change what is on
  the panel without any Android view appearing or disappearing.

### 3.1 Rule-by-rule mapping

| Rule                                                                                    | Ports?                                                       | Our trigger                                                                                                                                                                                                                                                                                                                                                                                                                              | Our hook                                                                                                                                                                                                                           |
| --------------------------------------------------------------------------------------- | ------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **R1** pause → repaint → resume around anything covering the surface                    | **Directly, and we already have it**                         | Stock's 49 sites collapse to two of ours: an overlay opening/closing (dialog, popover, select, menu, the selection popup), and the ink session pausing for any reason.                                                                                                                                                                                                                                                                   | `kt/OnyxInk.kt:592-612` already _is_ R1 — `gate.pause()` → `webView.invalidate()` → resume after `RESUME_DELAY_MS` (500 on colour, the same constant, `:226-228`). `refreshPanelAfterClose` is the DOM half. **Nothing to build.** |
| **R2** the repaint is the whole surface                                                 | **Already true, differently**                                | `refreshFrame()` calls `webView.invalidate()` with no rect; the _reconcile region_ is what is scoped. That mirrors stock exactly: whole-view invalidate, region-limited software render.                                                                                                                                                                                                                                                 | `kt/OnyxInk.kt:544-545` and the comment there                                                                                                                                                                                      |
| **R3** count screen changes, GC every Nth                                               | **Needs a different trigger, and this is the main new code** | Stock counts page changes and document-mutating menu actions. Our equivalents: a workspace navigation (`open_target`, `go_back`, `go_forward`, `close_pane`, `show_compact_pane`), a note being created or deleted, a page being added or removed inside a note. **Not** overlay opens, **not** scrolls, **not** strokes.                                                                                                                | `ts/features/workspace/workspace-store.ts:16-24` — the exact hook `c89bc5f` removed, with the exact same action set. Plus the note/page mutation actions.                                                                          |
| **R4** a refresh pays off the debt                                                      | **Directly**                                                 | Any full refresh — including the overlay-close one — resets the counter.                                                                                                                                                                                                                                                                                                                                                                 | `ts/app/eink-refresh.ts`, one line                                                                                                                                                                                                 |
| **R5** explicit user refresh is immediate                                               | **Directly, but we have no such control**                    | We would need to add one. Stock's is a toolbar button. Ours would live in the appearance settings or the note toolbar.                                                                                                                                                                                                                                                                                                                   | `ts/app/appearance.tsx` / `ts/features/settings/`                                                                                                                                                                                  |
| **R6** transient fast mode while animating, released 5 s later, never during touch      | **Already implemented, and correctly**                       | `InkRefreshPolicy` with `QUIET_MS = 5000` — the same constant as `EpdEventHandler.java:124-128`.                                                                                                                                                                                                                                                                                                                                         | `kt/InkRefreshPolicy.kt:9`, `kt/OnyxInk.kt:503-520`                                                                                                                                                                                |
| **R7** never fast-mode a stylus stroke                                                  | **Already implemented**                                      | Narrowed to a selection drag in `34dc906`; confirmed correct from three directions in `handwriting-logic-review.md` §9.                                                                                                                                                                                                                                                                                                                  | `kt/OnyxInk.kt:775`, `:783-785`                                                                                                                                                                                                    |
| **R8** suppress the refresh while a selection overlay is live                           | **Ports, and we partly have it**                             | Stock drops R1 entirely while a selection exists. We do the analogous thing for the lasso _trace_ (`ts/features/handwriting/onyx-ink.ts:337` — `if (traceRef.current) return`) but **not** for a settled selection. A full refresh while the selection frame and floating menu are on screen would flash them.                                                                                                                           | `ts/app/eink-refresh.ts` would need to consult the handwriting session, or the ink session would need to veto. **New, small.**                                                                                                     |
| **R9** one-shot REGAL_PLUS for a page's first frame and after a settings surface closes | **Irrelevant**                                               | `supportRegal()` is false on this panel and the EAC layer rewrites mode 3 → 0 (`05-panel-refresh-levers.md` §1.5; `handwriting-logic-review.md` §9). REGAL is not a mode this device runs.                                                                                                                                                                                                                                               | —                                                                                                                                                                                                                                  |
| **no time-based refresh**                                                               | **Directly — and we already violate it mildly**              | `SETTLE_MS = 300` is a _coalescing_ delay, not a timer-driven refresh, so it is fine. But `CLOSE_REFRESH_DELAY_MS = 300` on ink-session close (`kt/OnyxInk.kt:202`, `:721-724`) is a refresh on a timer with no stock counterpart. Keep it — leaving a writing session is exactly when the panel is dirtiest — but recognise it as ours.                                                                                                 | `kt/OnyxInk.kt:717-725`                                                                                                                                                                                                            |
| **no stroke-driven refresh**                                                            | **We should reconsider**                                     | §1.0: stock does not run a per-stroke reconcile on this panel. Ours (`refreshFrame`) is the core of our ink handover and we cannot simply delete it — our canvas _is_ a WebView, and unlike stock's `EditorView` it has no page bitmap the firmware layer can be handed back to except through a submitted frame. But the _mode_ we mark it with is a stock behaviour we inherited from a code path stock does not execute. See §6.4 #0. | `kt/OnyxInk.kt:496-546`                                                                                                                                                                                                            |

### 3.2 Events we have that stock does not

| Our event                                           | Should it refresh?                                                                                                                                                                                                                             |
| --------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **SPA route change** (`open_target`, back, forward) | **Yes — this is our R3 tick.** It is the closest thing we have to stock's page turn, and it is where ghosting is visible.                                                                                                                      |
| **Pane split / close / compact pane swap**          | **Yes**, same counter. Half the screen changing is a screen change.                                                                                                                                                                            |
| **Overlay open/close**                              | **Refresh on close only, and do not count it** — that is R1, and R4 says it resets the counter. This is what we already do (`ts/app/eink-refresh.ts:53-60`).                                                                                   |
| **Virtual-list / in-page scroll**                   | **No.** Stock does nothing on scroll but a transient fast mode (§1.7), and the firmware's WebView A2 fling is already on and is not `supportEAC`-gated (§4.3). Adding a refresh here would flash on every scroll.                              |
| **Tool change (pen ↔ eraser ↔ lasso)**              | **R1 without a count.** It already goes through `configure()` → `pause()` → `webView.invalidate()`. Once §6.4 lands, it should keep marking the whole region dirty (a tool change legitimately redraws the toolbar) but not otherwise refresh. |
| **Note saved / synced**                             | **No.** Stock counts document-mutating _menu_ actions, not writes.                                                                                                                                                                             |
| **App resume / window focus gained**                | **Yes, once.** Stock's `PenEventHandler.java:334-347` runs R1 on both focus gain and loss. We do resume the gate (`kt/OnyxInk.kt:564-568`) but not a panel refresh. Something else owned the panel while we were away.                         |
| **IME show/hide**                                   | **Already: a GC on hide** (`kt/MobileSystemPlugin.kt:102`). Correct — the keyboard is the largest opaque rectangle the panel ever holds.                                                                                                       |
| **Theme / display-profile change**                  | **Yes, immediately** — every pixel changed. Currently handled implicitly by the appearance provider re-rendering; worth an explicit refresh.                                                                                                   |

### 3.3 What does not port at all

- Stock's 40 `InvalidateScreenAction` sites for toasts, the status bar, the Onyx float button, the
  audio menu, the float tool menu, HWR placement, the meeting panel, screenshots, CTP changes. Some
  of those events still happen to us (a system toast can appear over our app), but **we have no
  hook** — `PenEventHandler`'s toast and status-bar receivers are stock's own broadcast listeners.
  Our substitute is `webView.hasWindowFocus()` in `canPresent()` (`kt/OnyxInk.kt:548-551`) plus the
  resume path, which covers the cases that matter (something took focus) and not the ones that do
  not (a toast, which does not take focus).
- R9 (REGAL) — dead on this panel.
- Stock's `EpdDeviceManager.checkIntervalChanges` reading its own `NoteSetting`. Ours would read
  `IOECService.getGcInterval()` instead, so the user's system-wide choice still governs us.

---

## 4. What we lose by setting `supportEAC: false`

There are exactly two places `supportEAC` is consulted, and they cover everything: **six getters in
our own process** guarded by `EInkHelper.isAppConfigActive()`, and **four service-side checks**.
Everything else keyed on the per-app config is reached _through_ one of those.

```java
// fw/optimization/EInkHelper.java:1156-1162
private static boolean isAppConfigActive() {
    if (!isServiceReady() || !isEnable()) return false;
    EACAppConfig c = getReadOnlyAppConfig();
    return c.isSupportEAC() && c.isEnable();
}
```

### 4.1 In-process (framework code running inside our app)

| #   | Service                   | Gate                                                                           | What it returns when off                   | Does it matter to us?                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| --- | ------------------------- | ------------------------------------------------------------------------------ | ------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | **Input-event reporting** | `EInkHelper.inputEventChangedAsync` filter `isAppConfigSupportEAC()` (`:1131`) | pipe message never written                 | **This is the point of the exercise.** Losing it is the goal: no tap counter, no counted GC, no post-input settle repaint. In exchange we own every full refresh.                                                                                                                                                                                                                                                                                                                            |
| 2   | **Refresh config**        | `getRefreshConfig` (`:948-954`)                                                | `new EACRefreshConfig().setEnable(false)`  | Consumed by `View.updateCanvasImpl` (`aosp/view/View.java:6094-6104`) which stuffs it into `canvas.getEACCanvasImpl()`. Our content is composited by Chromium into a hardware layer, not drawn through `android.graphics.Canvas` in the Java view tree, so this reaches almost nothing of ours. **[inference, but well-founded]**                                                                                                                                                            |
| 3   | **Paint config**          | `getPaintConfig` (`:916-922`)                                                  | `new EACPaintConfig().setEnable(false)`    | Same call site plus `aosp/widget/Toast.java:310`. This is the text-bolding / icon-contrast / fill-contrast / image-gamma munging. It is the thing that makes third-party _Android_ UIs legible; it does nothing for a WebView that ships its own e-ink theme. Stock is already exempt by a _different_ check — `updateCanvasImpl` returns early for `ActivityManagerHelper.isOnyxApp(pkg)` (`View.java:6095`). **No loss.**                                                                  |
| 4   | **App DPI override**      | `getApplicationDPI` (`:576-585`)                                               | `-1` (= no override)                       | Read by `aosp/content/res/ResourcesImpl.java:494` and `aosp/app/ActivityThread.java:1840`, `:4656`. **This is a real regression if the user has set a per-app DPI for us.** Our own code already accounts for BOOX per-app DPI (`kt/OnyxInk.kt:318-319`: "CSS pixels may differ from Android density because BOOX has per-app DPI settings"). Losing it means the app renders at native density; our own zoom control (`ts/app/viewport-zoom.ts`) is the replacement. **Confirm on device.** |
| 5   | **Forced rotation**       | `getRotationConfig` (`:957-963`)                                               | `new EACRotationConfig().setEnable(false)` | We are a normal rotating app. **No loss.**                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| 6   | **App extra config**      | `getAppExtraConfig` (`:514-520`)                                               | `.setEnable(false)`                        | `forceFullScreen`, `usePageKeyAsVolumeKey`, `fullPMAccess` (+ its 300 000 ms timeout), `useDialogBorder`, `allowSplashScreen` (`fw/optimization/data/v2/EACAppExtraConfig.java:7-13`). `forceFullScreen` is the only one that could be visible; we do not rely on it. **Low risk, verify.**                                                                                                                                                                                                  |
| 7   | **Note config**           | `getNoteConfig` (`:892-898`)                                                   | `EACNoteConfig.dummyConfig()`              | This is the _system screennote_ config — the pipeline we deliberately stay out of (`04-system-screennote-pipeline.md`). **No loss; arguably a gain.**                                                                                                                                                                                                                                                                                                                                        |

**Not gated, and therefore not lost:**

- **WebView CSS injection.** The gate is inverted:
  ```java
  // fw/optimization/EInkHelper.java:1043-1050
  if ((!getReadOnlyAppConfig().isSupportEAC() || getReadOnlyAppConfig().isEnable()) && (css = …) != null)
      return EACCSSUtil.getEncodedBase64CSSString(new EACCSSConfig(css));
  ```
  `!supportEAC || enable` is **true** when `supportEAC` is false, so the CSS path stays open. It is
  reached from `aosp/webkit/WebViewClient.java:60-62` (`onPageFinished` → `EACCSSUtil.injectCSS`),
  itself filtered on `URLUtil.isNetworkUrl(url) && webView.isCssInjectEnabled()`
  (`fw/utils/EACCSSUtil.java:192-194`) and then on the CSS string being non-empty (`:201-203`).
  Our `EACCSSConfig` is the default (`customCSS=""`, `fontBold=false`, `fontSize=0`,
  `07-einkwise.md:319`), so nothing is injected today and nothing changes. **No effect either way.**
- **Scroll refresh delay.** `EACScrollRefreshImpl` writes `Settings.Global.SCROLL_REFRESH_DELAY`
  from `getActivityConfig(cn).getScrollRefreshDelay()` on every resume with **no `supportEAC`
  check** (`fw/optimization/impl/EACScrollRefreshImpl.java:22-46`). Note this is device-global, not
  per-app, and stays whatever the last resumed app set it to. **No change.**
- **Keyboard / page-key mapping.** `EACKeyboardManager:221` _is_ gated
  (`(enable && supportEAC) ? keyboardConfig : new EACKeyboardConfig()`), but the Note Air 4C has no
  page keys. **No loss.**

### 4.1b In system_server (via `EACBaseSystemServerManager.isAppConfigActive`, `fw/optimization/EACBaseSystemServerManager.java:11-13`)

| #   | Service                                           | Gate                                                                                                                                                                     | Matters?                                                                                                                                                                                           |
| --- | ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 8   | **Forced rotation cache**                         | `EACRotationManager.isRotationConfigEnable` (`fw/optimization/EACRotationManager.java:30-32`, applied `:43-58`)                                                          | `pkgRotationMap[pkg] = -2`, rotation passes through untouched. **No loss**, and it also means we are never `restartPackage()`d for a rotation-config change (`EACConfigChangedImpl.java:197-199`). |
| 9   | **Splash-screen override**                        | `EACSplashScreenManager.isConfigAllowSplashScreen` (`fw/optimization/EACSplashScreenManager.java:15-17`)                                                                 | We do not use it. **No loss.**                                                                                                                                                                     |
| 10  | **Page-key remap + accessibility scroll gesture** | `EACKeyboardManager.java:221-227`                                                                                                                                        | No page keys on this device. **No loss.**                                                                                                                                                          |
| 11  | **The system ScreenNote handwriting pipeline**    | `getNoteConfig` → `EACScreenNoteManager.initPackageTypeImpl` (`fw/optimization/EACScreenNoteManager.java:153-162`): `isSupportNoteConfig()` false ⇒ `packageType = NONE` | This is the mechanism behind `04-system-screennote-pipeline.md:111`, the pipeline we deliberately stay out of. **A gain: it becomes structurally impossible to be pulled in.**                     |

### 4.2 Service-side

| #   | Service                                                                       | Gate                                                                                               | Effect when off                                                                                    | Matters?                                                                                                                                                                                                                                                                                     |
| --- | ----------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 8   | **The SF debouncer** — post-input quality repaint **and** the counted auto-GC | `TabletEACRefreshImpl.handleSFDebouncer` (`:143-149`)                                              | `ViewUpdateHelper.debouncer(false,0,0,0,0)` on every resume, config apply and system-window change | **The whole point.** We lose the firmware's ghosting cleanup and must supply our own — see §3 and the port plan.                                                                                                                                                                             |
| 9   | **Effective refresh config**                                                  | `EACBaseRefreshImpl.getRefreshConfigByCurrentComponent` (`:134-140`)                               | a bare `new EACRefreshConfig()` (`enable=false`, `updateMode=0`, `gcInterval=20`, `turbo=0`)       | Feeds `getConfigUpdateMode` (`:118-132`). But note **`onResume`/`applyUpdateMode` read `getRefreshConfig` directly, not this** (`TabletEACRefreshImpl.java:239-248`, `:94-120`), so the waveform still resolves from our stored index. **See the caveat below — this needs a device check.** |
| 10  | **Scrolling refresh mode**                                                    | `EACBaseRefreshImpl.calculateScrollingRefreshMode` (`:82-100`)                                     | falls back to the device default `extraConfig.scrollingRefreshMode`                                | Device-wide fallback rather than our A2/ANIM choice. Under HD our stored mode is 0, which already takes the `default:` branch, so **no change**.                                                                                                                                             |
| 11  | **Dither**                                                                    | `TabletEACRefreshImpl.applyDither` (`:72-79`)                                                      | gated on raw `updateMode ∈ {0,3,5}` **and** `appConfig.enable` — _not_ on `supportEAC`             | Survives the flip as long as `enable` stays true and `updateMode` stays 0. **No loss** — though it only ever mattered for bitmap content.                                                                                                                                                    |
| 12  | **Kill-on-EAC-toggle**                                                        | `OECService.eacEnableStatusChangedImpl` (`:361-384`)                                               | our uid is excluded from the set that gets `killUIDImpl`                                           | We stop being force-stopped when the user toggles EinkWise globally. **A gain.**                                                                                                                                                                                                             |
| 13  | **Handwriting debounce filter**                                               | `EpdcUpdateDebounceWithDelay:100` requires `supportEAC` **and** a matching `eacScreenNoteStateMsg` | the system-screennote debounce never engages for us                                                | We are not in the screennote pipeline. **No loss.**                                                                                                                                                                                                                                          |

### 4.3 What survives the flip — the important negatives

This is the half that makes Design B viable, and it is settled from the code rather than inferred.
**None of the following consults `supportEAC`:**

| Behaviour                                                                                                       | Gate it actually uses                                                                                                                                                                                                                        | Citation                                                                                               |
| --------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| **Refresh profile / `refresh_mode_N` / `updateMode` / turbo, applied on every resume**                          | `refreshConfig.isEnable()`. `TabletEACRefreshImpl.onResume` reads `getAppConfigByComponentName(cn).getRefreshConfig(cn)` with **no `supportEAC` test**, and `applyUpdateMode` logs "curConfig is disable" but **does not return** (`:95-98`) | `fw/optimization/impl/TabletEACRefreshImpl.java:239-250`, `:94-121`                                    |
| **The whole display block** — gamma, brightness, saturation min, saturation, BW mode, dither threshold, enhance | `displayConfig.isEnable()`; every getter goes through `ensureAppConfig(cn)` with no `supportEAC` test                                                                                                                                        | `fw/optimization/impl/TabletEACDisplayImpl.java:18-64`, `:77-83`; `impl/EACBaseDisplayImpl.java:36-48` |
| **Panel-level dither** (`ViewUpdateHelper.enableDither`)                                                        | `updateMode ∈ {0,3,5}` **and** `appConfig.isEnable()`                                                                                                                                                                                        | `fw/optimization/impl/TabletEACRefreshImpl.java:72-79`                                                 |
| **Anti-flicker, animation duration**                                                                            | `refreshConfig`                                                                                                                                                                                                                              | `TabletEACRefreshImpl.java:203-208`, `:131-141`                                                        |
| **Scroll / fling A2 in-process**                                                                                | `EInkHelper.getEacScrollStyle()` reads `globalActivityConfig.eacScrollStyle` **ungated**; `ViewRootImpl.deliverInputEvent` switches on it                                                                                                    | `aosp/view/ViewRootImpl.java:3502-3514`; `fw/optimization/EInkHelper.java:808-813`                     |
| **WebView's own always-on A2 fling**                                                                            | `mCSSInjectEnabled = true; autoA2ForScrolling = true` in the constructor                                                                                                                                                                     | `aosp/webkit/WebView.java:342-343`, `:374-393`                                                         |
| **`Settings.Global.SCROLL_REFRESH_DELAY`**                                                                      | written on every resume from the activity config, no `supportEAC` test                                                                                                                                                                       | `fw/optimization/impl/EACScrollRefreshImpl.java:23-46`                                                 |
| **Auto-freeze**                                                                                                 | `autoFreezeConfig.supportAutoFreeze && autoFreeze`                                                                                                                                                                                           | `fw/optimization/impl/EACAutoFreezeImpl.java:37-46`                                                    |
| **`fullPMAccess` / force-app-standby**                                                                          | `extraConfig.isFullPMAccess()` on the service side (note: the _in-process_ `getAppExtraConfig` **is** gated, §4.1 #6 — the two disagree)                                                                                                     | `fw/optimization/impl/EACPMImpl.java:58-66`; `OECService.java:1226-1235`                               |

So the prediction is precise: **waveform, turbo, anti-flicker, panel dither, contrast/gamma/enhance
and scroll behaviour all unchanged; the debouncer, the tap counter, the paint munging, the DPI
override and the screennote eligibility go away.** That is a very clean split, and it is exactly
the split Onyx designed: `createOnyxAppConfig` sets `supportEAC = false` and
`paintConfig.enable = false` while **deliberately keeping** `refreshConfig.enable = true` and
`displayConfig.enable = true` (`fw/optimization/data/v2/EACAppConfig.java:100-111`). Onyx's own
intent for this flag is "keep the panel profile, drop the app-mangling".

**Corollary for the port plan:** if the goal is "behave exactly like stock Notes", the target shape
is not just `supportEAC:false` but the whole `createOnyxAppConfig` shape —
`supportEAC=false`, `paintConfig.enable=false`, `dpiConfig.enable=false`,
`autoFreezeConfig.supportAutoFreeze=false`, `refreshConfig.enable=true`, `displayConfig.enable=true`.

### 4.4 The CSS-injection trap, stated plainly

Flipping `supportEAC` does **not** disable WebView CSS injection — it _enables_ the branch, because
the guard is `(!supportEAC || enable)` (`fw/optimization/EInkHelper.java:1046`), the only reversed-
polarity `supportEAC` reference in the codebase. And the injection is not a stylesheet, it is a
`javascript:` payload run through `webView.evaluateJavascript(...)`
(`fw/utils/EACCSSUtil.java:98-113`, `:169`) that forces `color:#000000 !important`,
`font-weight:900 !important` and `font-size:N% !important` on `*`, `a`, `a:visited`, `a:hover` and
every form control (`:25-41`).

It is inert today only because of three _other_ gates:

1. `URLUtil.isNetworkUrl(url) && webView.isCssInjectEnabled()` (`fw/utils/EACCSSUtil.java:193-195`).
   `mCSSInjectEnabled` defaults **true** (`aosp/webkit/WebView.java:342`). **Whether
   `isNetworkUrl` matches our Tauri origin needs a device check** — on Android, Tauri serves from
   `http://tauri.localhost`, which _is_ an `http://` URL.
2. `EACCSSConfig.enable` defaults **false** (`fw/optimization/data/v2/EACCSSConfig.java:4-12`), and
   `getEncodedBase64CSSString` returns null when disabled or when `customCSS == Constant.DISABLE_CSS`
   (`EACCSSUtil.java:100-102`).
3. our `globalCSSConfig` is the default empty one (`07-einkwise.md:319`).

**So this is a latent hazard, not a current one, and it is orthogonal to the opt-out.** If a cloud
config or an EinkWise "font bold" toggle ever populates our CSS config, the payload would fight our
e-ink theme. The defensive fix is one line in the same JSON we already write —
`globalCSSConfig.enable = false` — or a reflective `WebView.setCssInjectEnabled(false)`
(`aosp/webkit/WebView.java:1296-1298`). Worth doing regardless of which design wins.

---

## 5. Is the opt-out even ours to make?

Short answer: **yes, the field is ours to write through the path we already use — but the write we
already use does not become effective until the next boot**, and there is a second, better-targeted
write that is effective immediately. Neither needs a permission.

### 5.1 `supportEAC` is inside the object we already round-trip

`ensureRefreshProfile()` (`kt/MobileSystemPlugin.kt:292-330`) does
`getAppConfigFromService([pkg])` → JSON → mutate `globalActivityConfig.refreshConfig` →
`applyAppConfigToService([json], bundle)`. That JSON is a serialised `EACAppConfig`, and
`supportEAC` is a plain top-level field of it with a getter and a setter:

```java
// fw/optimization/data/v2/EACAppConfig.java:40, :48, :298-299, :504-506
private boolean supportEAC;   // ctor default true (:48, :70)
public boolean isSupportEAC();
public EACAppConfig setSupportEAC(boolean z);
```

The service side deserialises the **whole** object and replaces the map entry wholesale:

```java
// fw/optimization/OECService.java:189-221
EACAppConfig appConfig = EACConfigJsonUtil.getAppConfig(it.next());
…
editableDeviceConfig.getAppConfigMap().put(pkgName, appConfig);      // :200
if (shouldSendConfigChangedBroadcast(appConfigNotNull, appConfig)) hashSet.add(pkgName);
applyConfigChangeImpl(appConfigNotNull, appConfig);                  // :203 → all five impls
…
saveDeviceConfig(editableDeviceConfig, 5, false, saveMmkvFlag);      // :209
saveConfigToLocalImpl();                                            // :221  ← unconditional persist
```

So one line — `root.put("supportEAC", false)` — added to the JSON we already build is the entire
write. `saveConfigToLocalImpl()` at `:221` runs unconditionally, so the value lands in MMKV
`eac_app_<pkg>` regardless of the `args_save_mmkv` bundle flag we do not pass
(`07-einkwise.md:483-505` for the storage layout; the file is `/onyxconfig/mmkv/onyx_config`).

**There is no permission check anywhere on this path** (`07-einkwise.md:451-474`, §4.2).

One pleasant side effect of the flip: `shouldSendConfigChangedBroadcast` (`OECService.java:906-908`)
returns `false` only when the _new_ config has `supportEAC == false` **and** nothing changed. So the
first write (true→false) broadcasts a config change — one full redraw in our process — and every
idempotent rewrite afterwards is silent. Today's profile write has the opposite property and needs
the `PROFILE_MARKER` SharedPreference (`kt/MobileSystemPlugin.kt:302-303`) to avoid re-broadcasting.

### 5.2 …but the read path does not see it until the service restarts

This is the known read/write asymmetry (`07-einkwise.md:528-559`), and it applies to `supportEAC`
exactly as it applies to `refreshModeIndex`. Every read on the enforcement path goes through the
**active theme**, not through `appConfigMap`:

```java
// fw/optimization/data/v2/EACDeviceConfig.java:30-32
public EACAppConfig ensureAppConfig(String str) {
    return EACAppThemeManager.getActiveTheme(str).getAppConfig();
}
```

and `getAppConfigFromService` — the reader our own process uses to refresh its cache — goes through
the same call (`fw/optimization/OECService.java:1089-1100`). So the round trip is:

1. we write → `appConfigMap` + MMKV `eac_app_<pkg>` now say `supportEAC:false`;
2. the broadcast reaches our process, `EACConfigChangedImpl.onDefaultViewConfigChanged`
   (`fw/optimization/EACConfigChangedImpl.java:153-176`) calls `EInkHelper.fetchAppConfig(pkg)`
   (`:158`) → `updateCacheAppConfigImpl` → `getAppConfigFromService` → **the theme** → still `true`;
3. `EInkHelper.sCachedEACAppConfigRef` (`fw/optimization/EInkHelper.java:64`, set at `:1756-1763`)
   therefore keeps `supportEAC:true`, and `isAppConfigSupportEAC()` keeps returning `true`.

The two halves are reconciled at **service start**, when `ValidateAppConfigAction` runs for every
package in `eac_app_pkg_set`:

```java
// fw/optimization/action/ValidateAppConfigAction.java:94-100
EACAppTheme activeTheme = EACAppThemeManager.getActiveTheme(this.activeAppConfig.getPkgName());
this.theme = activeTheme;
activeTheme.setAppConfig(this.activeAppConfig);    // ← our eac_app_<pkg> becomes the theme's config
// :122-125  saveConfigs(): theme.save() + theme.getAppConfig().save()
```

and it honours our stored value for a third-party package:

```java
// fw/optimization/action/ValidateAppConfigAction.java:288-293
protected boolean supportEAC(String str) {
    if (!ActivityManagerHelper.isOnyxOrSystemApp(str) || this.supportEacSystemApps.contains(str))
        return this.theme.getAppConfig().isSupportEAC();
    return false;                                   // any Onyx/system app outside {chrome, vending, browser}
}
```

**So `applyAppConfigToService` alone means: write once, reboot once, and it is then permanent.**
[inference on the exact ordering; every individual step is attested, and it is the same conclusion
`07-einkwise.md:551-559` reached for the refresh profile.]

### 5.3 The immediate write: `saveEACAppThemes`

Themes are **not cached in memory** — every read hits MMKV:

```
EACAppThemeManager.getActiveTheme(pkg)            (fw/optimization/EACAppThemeManager.java:32-34)
  → getTheme(pkg, getActiveThemeType(pkg))        (:36-38, default type 3)
  → EACThemeFactory.loadThemeOrCreate(pkg, type)  (fw/utils/EACThemeFactory.java:393-427)
  → loadThemeFromMMKV → EACAppTheme.loadByKey("eac_theme_<pkg>@theme_type_<n>")   (:387-391)
```

so a write to that key is visible to the very next enforcement read. The write exists on the
service interface and does nothing but persist:

```java
// fw/optimization/OECService.java:1699-1702
public void saveEACAppThemes(List<String> list) { new SaveEACAppThemesAction(list).execute(); }
// fw/optimization/action/SaveEACAppThemesAction.java:38-59 — parse, require a non-empty pkg, theme.save()
```

**And there _is_ a theme reader** — just not on `IOECService`, and not in `EACReflectUtils`:

```java
// fw/optimization/EInkHelper.java:1283-1285  →  EACAppThemeManager.loadThemes(pkg)  (:92-102)
public static List<String> loadThemes(String pkg);      // the three theme types, as JSON
public static void saveEACAppThemes(List<String> json); // :1421-1428
```

`EACAppTheme`'s shape is `pkg`, `themeType`, `appConfig`, `currentColorType`, `currentDpiType`,
`colorTypeConfigs`, `dipTypeConfigs` (`fw/optimization/data/v2/EACAppTheme.java:34-48`), so the full
round trip is: `loadThemes(pkg)` → pick the entry whose `themeType` equals
`eac_active_theme_<pkg>` (default 3) → set `appConfig.supportEAC = false` → `saveEACAppThemes([json])`.
Both methods must be reached by our own `Class.forName("android.onyx.optimization.EInkHelper")
.getMethod(...)`; `EACReflectUtils` binds neither.

Two caveats:

- `applyEACAppTheme(json)` is **not** a theme write. It parses a theme, requires
  `EACAppThemeManager.isActiveTheme` (`EACAppThemeManager.java:56-58`), extracts the `appConfig` and
  funnels it straight back into `applyAppConfigToService` with `args_operation_flag = 2`
  (`fw/optimization/action/ApplyThemesAction.java:98-106`; `OECService.java:278-284`, `:312-316`).
  It has exactly the durability of §5.1 and additionally rewrites `updateMode`/`turbo` from the
  index (`ApplyThemesAction.java:46-56`).
- `SaveEACAppThemesAction` validates only "theme non-null and `pkg` non-empty"
  (`fw/optimization/action/SaveEACAppThemesAction.java:26-28`), so a malformed theme is written
  verbatim. Read-modify-write, never synthesise.

Recommended shape: **write `applyAppConfigToService` first (it is the vendor's own supported path —
`kcb/com/onyx/android/sdk/device/eac/v2/EACManagerV2.java:168-171` does exactly this, mutating the
JSON key `"supportEAC"` from `EACConstants.SUPPORT_EAC_KEY`), and add the `loadThemes` /
`saveEACAppThemes` pair only if the reboot latency turns out to matter.**

### 5.4 What can re-derive or reset it

From `07-einkwise.md:576-611`, filtered to what touches `supportEAC` specifically:

| Trigger                                          | Does it clobber `supportEAC`?                                                                                                                                                                                                                                                                                          | Evidence                                                                                                                             |
| ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `eac_force.json`                                 | **No — it only names Onyx's own packages.** `EACConfigJsonUtil:125` does `ensureAppConfig(pkg).setSupportEAC(false)` for the listed set (`com.onyx`, `com.onyx.kreader`, `com.onyx.android.note`, `com.onyx.android.ksync`, `com.onyx.aiassistant`). It never sets it back to `true` for anyone, so it cannot undo us. | `fw/optimization/EACConfigJsonUtil.java:125`; `07-einkwise.md:238`                                                                   |
| **OECService start** (`ValidateAppConfigAction`) | **No — it is what makes ours stick.** It copies `eac_app_<pkg>` into the theme and re-persists.                                                                                                                                                                                                                        | `ValidateAppConfigAction.java:94-100`, `:122-125`; `LoadDeviceConfigFromLocalAction.java:44-51`                                      |
| Every `com.onyx` process start                   | **Yes, partially.** `InitApplyEACConfigAction.t()` re-pushes all `PresetEACConfig` entries; our package is not one of them, so this is a no-op for us **unless** a cloud/preset entry names it. [inference: it does not]                                                                                               | `kcb/.../InitApplyEACConfigAction.java:56-63`, `:176-184`; `07-einkwise.md:578`                                                      |
| **First boot / "reset all"**                     | **Yes.** `BuildDefaultEACConfigRequest` runs for every installed app.                                                                                                                                                                                                                                                  | `kcb/.../InitApplyEACConfigAction.java:94-107`                                                                                       |
| **`jsonVersion` upgrade (OTA)**                  | **Yes — and this is the likeliest one.** `BuildDefaultEACConfigRequest.p()` explicitly overwrites `supportEAC` even for an app that already has a config (the _refresh_ block survives a rebuild; `supportEAC` does not).                                                                                              | `kcb/.../eac/rx/request/UpgradeEACConfigRequest.java:38-55`; `BuildDefaultEACConfigRequest.p()` `:194-202`; `07-einkwise.md:593-596` |
| **EinkWise panel "reset"** for our app           | **Yes.** `EACAppConfigChangeAction` `RESET` replaces the config with `RxEACUtils.getBuiltInAppConfig(ctx, pkg)`, and attaches `args_operation_flag = 2` → our WebView is reloaded.                                                                                                                                     | `kcb/.../eac/rx/action/EACAppConfigChangeAction.java:75-77`, `:104-114`                                                              |
| **Cloud fetch**                                  | **Yes**, for all installed packages, with no confirmation, triggered by the app-list screen on resume over Wi-Fi or by the _exported_ broadcast `com.onyx.EAC_FETCH_FROM_CLOUD`.                                                                                                                                       | `kcb/.../eac/rx/request/FetchCloudEACConfigRequest.java:35-47`; `kcb/.../common/broadcast/ExternalRequestReceiver.java`              |
| App install / upgrade of **our** APK             | `EACStatusChangedReceiver` on `PACKAGE_ADDED` → `AppInstalledAction` → optional cloud fetch → `SaveDefaultThemeAction`. `SaveDefaultThemeAction.f()` replaces `refreshConfig`/`displayConfig` from the theme; `supportEAC` is not in that set, but a cloud fetch in the same chain is.                                 | `kcb/.../eac/rx/action/SaveDefaultThemeAction.java:43-63`; `07-einkwise.md:586`                                                      |
| Screen on/off, user present, boot broadcast      | **No** — none of these receivers exists under `fw/optimization/`.                                                                                                                                                                                                                                                      | `07-einkwise.md:606-609`                                                                                                             |

**Practical consequence.** The write is not a one-time act; it has to be a **guarded idempotent
reassertion**, exactly like `ensureRefreshProfile()` today — except that the existing
`PROFILE_MARKER` SharedPreference is the _wrong_ guard for it, because the marker persists across a
firmware reset that clobbered the config. The correct guard is a read: `getDisallowedEACApps()` is
an `IOECService` method that lists every package with `supportEAC == false`, **and it reads
`appConfigMap`, not the theme**:

```java
// fw/optimization/OECService.java:1204-1212
public List<String> getDisallowedEACApps() {
    for (EACAppConfig c : getReadOnlyDeviceConfig().getAppConfigMap().values())
        if (!c.isSupportEAC()) arrayList.add(c.getPkgName());
    return arrayList;
}
```

It is already surfaced by the SDK as `EACReflectUtils.sMethodGetDisallowedEACApps`
(`sdk/onyxsdk-device-1.3.5.2/sources/com/onyx/android/sdk/api/device/eac/EACReflectUtils.java:28`).
So: read that list at startup; if our package is missing, write; otherwise do nothing. That is a
correct, self-healing guard with no marker file, and it also gives us an on-device probe.

### 5.5 …and none of those reset paths actually reaches `supportEAC` for us

Correcting §5.4's table with what the `com.onyx` side actually does: **nothing on this build
re-derives `supportEAC` for a third-party package.**

- **Boot validation does not touch it.** `ValidateAppConfigAction`'s seven steps are `loadConfigs`,
  `validateEnhance`, `validateRefreshMode`, `validateColorMode`, `validateDPI`,
  `validateDitherThreshold`, `saveConfigs` (`fw/optimization/action/ValidateAppConfigAction.java:189-231`);
  `supportEAC(pkg)` (`:288-293`) is **read-only**, and its only consumer is `validateDPI` (`:320-327`).
  Boot is what makes our value authoritative, not what erases it.
- **`eac_force.json` cannot reach us.** `CleanUpOnyxAppConfigRequest` (itself `@Deprecated` at `:20`)
  iterates `presetAppConfig.keySet()` (`kcb/com/onyx/android/sdk/eac/rx/request/CleanUpOnyxAppConfigRequest.java:26-36`)
  and copies `preset.isSupportEAC()` only for those packages (`:97-103`). Our package is not one.
- **The default-config rebuild cannot reach us either.** `BuildDefaultEACConfigRequest.p()` — which
  _does_ overwrite `supportEAC` (`:194-202`) — is only called when
  `PresetEACConfig.getInstance().getPresetAppConfig(pkg) != null` (`:57-61`), and
  `eac_noteair4c.json` has 35 preset entries, none of them ours. **One exception:** `c()` (`:85-109`)
  force-sets `setSupportEAC(true)` on every third-party **IME** package (`:105`).
- **The OTA ladder cannot flip it.** `UpgradeEACAppConfigToV108` _reads_ `supportEAC` and writes
  `rotationConfig.enable`/`keyboardConfig.enable` from it
  (`kcb/com/onyx/android/sdk/eac/upgrade/UpgradeEACAppConfigToV108.java:10-15`); it never writes it.
  And even the empty-ladder fallback to `BuildDefaultEACConfigRequest` hits the preset guard above.
- **App upgrade is a literal no-op**: `EACStatusChangedReceiver` `:439-441` matches
  `PACKAGE_REPLACED` and `return`s. `PACKAGE_ADDED` skips when `EXTRA_REPLACING` (`:292`).
- **Uninstall does not clean the keys.** `ClearAppEACConfigRequest` → `removeAppConfigFromService`
  (`fw/optimization/OECService.java:773-786`) removes the map entry and rewrites `eac_app_pkg_set`,
  but never deletes `eac_theme_<pkg>@theme_type_*`, `eac_app_<pkg>` or `eac_active_theme_<pkg>`. So a
  stale `supportEAC=false` **survives uninstall and reinstall** — which is convenient for us and a
  hazard for anyone debugging. **[inference from the absence of any deletion; `BaseMMKV.removeValueForKey`
  exists (`fw/optimization/BaseMMKV.java:130-132`) and is never called with these prefixes.]**

**The realistic ways to lose it**, therefore, are only: (a) the user pressing "reset" on our app's
EinkWise panel or "reset all apps"; (b) a cloud EAC config existing for our package **and** "update
newly installed app EAC config" being on with Wi-Fi at first install — the V2 branch of
`CloudConfigToAppConfigRequest` parses the cloud `EACAppConfig` wholesale (`:28`); (c) our package
acquiring a `presetAppConfig` entry in a future `eac_noteair4c.json`; (d) becoming a system app or
an IME.

That is a much better durability story than the refresh profile has, and it means the idempotent
reassertion of §5.4 is cheap insurance rather than a necessity.

### 5.6 What the user sees in EinkWise afterwards — settled

The app is **not hidden from the app list** and **not greyed out**. What changes:

1. **The "Optimization" long-press menu item is removed.**
   `ShowAppItemPopupAction.getRemoveItemIdByAppInfo` fetches the config (`:174`) and adds
   `R.id.apps_optimization` to the _remove_ list at `:188-190`; `createAppMenu` (`:138-148`) then
   calls `menuBuilder.removeItem(...)` for each. So the entry point is simply not offered.
   (`kcb/com/onyx/common/applications/action/ShowAppItemPopupAction.java`)
2. **Any other route answers with a toast.** `ShowAppEACViewAction.l()` (`:52-64`) and
   `ShowCurrentTopAppEACViewAction.p()` (`:58-66`) both do
   `if (!cfg.isSupportEAC()) EACViewUtils.showEACNonsupportWarnToast(pkg)`, which for a third-party
   package picks `R.string.eac_optimization_third_party_warn`
   (`kcb/com/onyx/android/sdk/eac/util/EACViewUtils.java:919-928`).
3. **The "EAC enabled" badge goes off** — `RxEACUtils.updateAppDataEACInfo` sets
   `AppDataInfo.isEACEnabled = supportEAC && enable` (`kcb/com/onyx/android/sdk/eac/rx/RxEACUtils.java:75`).
4. **We are filtered out of the per-app Settings lists**: DPI, page-key mode, scroll button,
   rotation, network, auto-start, the permission panel — each has an explicit `isSupportEAC` filter
   (e.g. `kcb/com/onyx/tablet/settings/action/app/LoadAppDPISettingDataAction.java:45`,
   `LoadAppPageKeyModeSettingDataAction.java:46`, `LoadAppScrollButtonSettingDataAction.java:56`,
   `kcb/com/onyx/common/setting/model/EACRotationSettingViewModel.java:41`).
5. The scroll provider degrades to the page-key provider (`kcb/com/onyx/android/sdk/eac/util/EACUtil.java:163-176`)
   and the EAC tutorial is suppressed (`kcb/com/onyx/common/applications/action/CheckShowEACTutorialAction.java:27`).

**This is exactly what stock Notes looks like today.** So the answer to "what would the user see" is:
the same thing they already see for BOOX Notes, KReader and the launcher. That is a defensible
place for a note-taking app to be, and it removes the last real objection to Design B.

One nuance worth knowing: every one of those checks reads the config through
`CheckPackageSupportEACAction` → `getAppConfigFromService` → **the theme store**
(`kcb/com/onyx/android/sdk/eac/rx/action/CheckPackageSupportEACAction.java:19-31`). So the UI will
keep offering Optimization for us until the reboot of §5.2, and the two will then agree.

### 5.7 "Settings → Display → Full Refresh Frequency" is two different things

Worth stating because the premise conflated them:

- The **launcher's own** `GcInvalidateIntervalBundle` writes `KCBMMKVHelper.setGcInvalidateInterval(n)`
  and drives an **in-process** counter inside `com.onyx` that calls `EpdController.applyGCOnce()`
  every N events (`kcb/com/onyx/common/setting/v2/data/GcInvalidateIntervalBundle.java:70-83`;
  `kcb/com/onyx/android/sdk/kcb/common/manager/EpdDeviceManager.java:75-129`). **That one affects the
  launcher's screens only and is not `supportEAC`-gated because it never leaves `com.onyx`.**
- The **per-app** equivalent, and the one that reaches us, is the EinkWise `GC_INTERVAL` slider →
  `EACRefreshConfig.gcInterval` → `IOECService.setGcInterval` → `applyDebouncerParameter` →
  `handleSFDebouncer` → **gated**.

So opting out costs us the second, not the first — and the first was never ours anyway.

---

## 6. The eraser flash

This is separable from everything above, and it is a bug in our own code rather than a
consequence of EinkWise.

### 6.1 The stated diagnosis is not the actual cause

The working theory was: _erasing gets no pen-up callback, so the reconcile falls back to a timeout
and can end up repainting the whole writing region._ The first half is true; the second does not
follow, and the timeout is not what is happening.

`InkReconcileGate.began(erasing = true)` sets `expecting = false`
(`kt/InkReconcileGate.kt:44-48`). So when the canvas frame lands, `next()` evaluates
`!canvasReady → false`, `penUpReady → false`, `expecting → false`, and returns **`SOON`**, not
`WAIT` (`kt/InkReconcileGate.kt:70-75`). `commit()` then posts the reconcile after
`FRAME_DELAY_MS = 120` (`kt/OnyxInk.kt:465-469`, `:219`). The `PEN_UP_WAIT_MS = 750` path
(`kt/OnyxInk.kt:216`) is never taken for an erase. **The eraser reconcile is already prompt.**

### 6.2 What actually repaints the whole region

`InkReconcileRegion.take()` returns the whole limit rect whenever the `whole` flag is set
(`kt/InkReconcileRegion.kt:52-56`), and `invalidateAll()` is called from exactly two places
(`kt/OnyxInk.kt:313` and `:605`). The second is inside `pause()`:

```kotlin
// kt/OnyxInk.kt:592-612
private fun pause(releaseDisplay: Boolean = true, tearDown: Boolean = true) {
    …
    region.invalidateAll()      // :605
    reconcile.reset()           // :606
```

and `pause(releaseDisplay = false, tearDown = false)` is the first thing `configure()` does on the
non-overlay-only path (`kt/OnyxInk.kt:316`). So **every `configure()` that is not purely an overlay
move marks the whole writing region dirty.**

Now follow an erase gesture through the frontend:

1. `nativeInput` receives `kind: "begin"` with `erasing: true`. The guard is
   `event.kind === "begin" && point && (tool !== "pen" || event.erasing)`
   (`ts/features/handwriting/ink-canvas.tsx:426-428`) — so for an erase it **does** call
   `beginGesture`, which calls `setGestureAction("erase")` (`:361`).
2. `interacting: gestureAction !== null` (`ts/features/handwriting/ink-canvas.tsx:467`) flips
   `false → true`.
3. `interacting` is in the configure effect's dependency list
   (`ts/features/handwriting/onyx-ink.ts:317-331`), so `update.current?.()` fires → the plugin's
   `configure()` runs.
4. `onlyOverlaysChanged` requires `a.interaction == b.interaction` (`kt/OnyxInk.kt:390`), so it is
   **false** → the full path → `pause()` → `region.invalidateAll()`.
5. At the end of the gesture, `finishGesture` → `endInteraction()` → `setGestureAction(null)`
   (`ts/features/handwriting/ink-canvas.tsx:155-166`) → `interacting` flips back → **`configure()`
   and `invalidateAll()` a second time.**

So an erase marks the whole writing region dirty **twice**, and the reconcile 120 ms later repaints
the entire limit rect in `HAND_WRITING_REPAINT_MODE`. That full-region repaint is the flash.

A plain pen stroke does not do this: `nativeInput`'s guard is `tool !== "pen" || event.erasing`, so
with the pen tool selected and no erasing, `beginGesture` is never called, `gestureAction` stays
`null`, and no `configure()` happens. **The asymmetry between "writing does not flash" and "erasing
does" is exactly this one condition.**

There is a second, smaller contributor. When the erase does reach a stroke, the frontend's damage
is `changedInkBounds(previous.strokes, strokes)`
(`ts/features/handwriting/ink-canvas.tsx:318`), which unions `inkBounds(stroke)` over every removed
stroke (`ts/features/handwriting/ink-region.ts:47-50`). In `eraserMode: "stroke"` — the default
(`ink-canvas.tsx:92`) — erasing "a word" removes _whole strokes_, so a single dot of the eraser on
a long ligature yields that ligature's full bounding box. That is correct behaviour (the pixels
really did change) but it means the erase damage is inherently wider than a pen stroke's, and it
compounds the first problem rather than causing it.

Third, `configure()` has no idempotence check at all: `onlyOverlaysChanged` additionally requires
`a.overlayRects != b.overlayRects` (`kt/OnyxInk.kt:394`), so a `configure()` where **nothing**
changed — a scroll event that moved nothing, a resize observer firing on an unchanged box — also
takes the full path and invalidates the whole region. `configure` is bound to `scroll` (capturing),
`resize`, `visibilitychange` and a `ResizeObserver` (`ts/features/handwriting/onyx-ink.ts:293-298`),
so this fires often.

### 6.3 What stock does

Stock's erase path and its stroke path are **different routines with different update modes**, and
the difference is precisely the thing we got wrong.

**During the gesture: stock repaints nothing at all.** `EraseBeginAction` pauses the raw render only
when `!EraseUtils.isEraseTrackByRawRender(type)` — and that predicate is
`type == 0 || type == 1 || type == 2`, i.e. **true for every gesture eraser type**
(`stock/sdk/scribble/utils/EraseUtils.java:23-25`), so `PauseRawPenRenderAction` is dead code on
this build (`stock/note/note/action/erase/EraseBeginAction.java:26-55`). `ErasingShapeAction`'s
display step is dead for the same reason (`stock/note/note/action/erase/ErasingShapeAction.java:63`
takes `Observable.just(this)` unconditionally; `:68-116` is unreachable). `EraseRenderer.renderToScreen`
is an empty method (`stock/sdk/notecore/editor/render/EraseRenderer.java:74-77`). Only the document
model changes, in 10 ms (stroke) / 20 ms (move) buffered batches
(`stock/note/note/handler/scribble/ScribbleHandler.java:629-631`;
`stock/sdk/note/ui/config/DeviceConfig.java:79-80`).

**The erasing you see is the firmware's.** `ResumeRawDrawingRequest` re-applies, on every resume:

```java
// stock/note/note/request/pen/ResumeRawDrawingRequest.java:52-55, :61-65
Device.currentDevice().setStrokeParameters(8, new float[]{eraserArgs.getWidth(), 0.5f, 0.1f});
noteManager.setErasePathDrawing(
    EraseUtils.isEraseTrackByRawRender(type) || provider == SCRIBBLE_SELECTION_PROVIDER,   // always true
    EraseUtils.isMoveStrokeErase(type) ? 8 : 5);
```

`8` is `StrokeStyle.SOFT_ERASER`, `5` is `DASH` (`stock/sdk/pen/style/StrokeStyle.java`). This reaches
`TouchHelper.setEraserRawDrawingEnabled(true, style)` (`stock/sdk/pen/TouchHelper.java:200-206`) →
SurfaceFlinger transaction `SET_ERASER_RAW_DRAWING_ENABLED = 1048833`
(`fw/ViewUpdateHelper.java:213`, `:1167-1172`). **[inference, strong]** the firmware whitens the
erased area in the low-latency ink layer in real time, which is why stock needs no app-side frame
during the gesture and why nothing flashes.

**At erase-finish, the mode is explicitly reset.** `EraseFinishAction`'s pipeline
(`stock/note/note/action/erase/EraseFinishAction.java:512-576`) has, as its seventh step:

```java
// stock/note/note/action/erase/EraseFinishAction.java:505-508
private final void z0() { EpdController.resetViewUpdateMode(editorView); }
```

That clears any `HAND_WRITING_REPAINT_MODE` a preceding stroke's `GrayscaleRefreshAction` left on the
editor view, so **the erase frame goes out in the view's default update mode.**

**The repaint is region-limited in software and whole-view on the panel.**

```java
// stock/note/note/action/erase/EraseFinishAction.java:343-376  (render())
if (eraseContext.getChangedShapeList().isEmpty()) return just(this);   // no change → no frame at all
DisplayArgs a = new DisplayArgs();
a.setInvalidate(false);   // :353 — no view.invalidate() here
a.setEnablePost(true);    // :354 — EpdController.enablePost(view, 1)
… ForceRenderSliceListActionsKt.forceRenderVisibleDirtyScreenRectToDisplay(this, changedShapeList, a);  // :368
```

`forceRenderVisibleDirtyScreenRectToDisplay`
(`stock/note/common/extension/ForceRenderSliceListActionsKt.java:267-288`, `:541-567`) renders only
`ShapeRectUtils.getShapeScreenRect(shapeList, boxes)` — **the union screen rect of the shapes the
erase actually removed** — per intersecting page.

Then, via `EraseFinishEvent` → `EraseHandler.onEraseFinished`
(`stock/note/note/handler/scribble/EraseHandler.java:99-103`):

```
InvalidateViewWithPenControlAction      (stock/note/note/action/render/InvalidateViewWithPenControlAction.java:62-86)
  = pauseRawPen(render only)  →  observeOn(main)  →  renderManager.invalidate(new DisplayArgs())  →  resumeRawPen
```

with `RawPenArgs.pauseResumeArgs()`, `resumeDelayTime = 300`, and **`setPauseRawInputReader(false)`**
(`EraseFinishAction.java:156-168`) — the input reader stays on, only the render flag is cycled.
`renderManager.invalidate(new DisplayArgs())` with all defaults is a plain **whole-view
`View.invalidate()`** in the default mode (`stock/sdk/notecore/editor/display/InkRenderManager.java:79-97`).
No `handwritingRepaint`, no `refreshScreenRegion`, no `Rect` overload, no GC.

**And the SDK never delivers a pen-up refresh for an erase**, which our own
`kt/InkReconcileGate.kt:15-17` already knew but is worth having the exact code for:

```java
// stock/sdk/pen/RawInputReader.java:249-265   (release handler)
if (!erasing) U(p);        // :252-254 — dirty-rect accumulation skipped while erasing
… t(p, erasing, …);        // onEndRawErasing
if (erasing) return;       // :261-263 — bail out
I();                       // :264 — only here is the 500 ms penUpRefresh timer armed
```

**Side by side:**

|             | stock stroke pen-up                                                             | stock erase finish                                                                | **ours (both)**                                                                           |
| ----------- | ------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| trigger     | 500 ms SDK timer ∧ bitmap flush                                                 | immediately, when the model work finishes                                         | `commit()`/`penUpRefresh()` rendezvous, `SOON` at 120 ms for an erase                     |
| routine     | `GrayscaleRefreshAction`                                                        | `InvalidateViewWithPenControlAction`                                              | `refreshFrame()` — **one routine for both**                                               |
| update mode | `HAND_WRITING_REPAINT_MODE`, reset in `doFinally`                               | **reset first**, then the default mode                                            | **`HAND_WRITING_REPAINT_MODE` always** (`kt/OnyxInk.kt:539`, `kt/InkRepaintMode.kt:8-12`) |
| render flag | left on across the reconcile                                                    | `false` → invalidate → `true` after 300 ms                                        | `false` at erase begin → `true` on the presented frame (`kt/InkEraserRenderGate.kt`)      |
| region      | stroke-union, or the whole `scribbleRect` when `isEnabledPenDirtyRect == false` | erased shapes' union rect                                                         | stroke union, **or the whole limit rect**                                                 |
| no-op case  | —                                                                               | **`changedShapeList.isEmpty()` → no frame at all** (`EraseFinishAction.java:344`) | we always emit a frame                                                                    |

So there are **two** independent causes of our eraser flash, not one:

- **(A)** the whole-region invalidation from §6.2, and
- **(B)** we reconcile an erase in `HAND_WRITING_REPAINT_MODE`, which is exactly the mode stock
  goes out of its way to clear before an erase frame. That mode re-drives ink pixels; it is
  _designed_ to be visible.

(B) alone would flash even over a correctly-sized region.

### 6.4 What we should do instead

Five changes, in order of value:

0. **Do not mark an erase reconcile as a handwriting repaint.** `refreshFrame()` calls
   `repaintMode.acquire()` unconditionally (`kt/OnyxInk.kt:539`). Stock's erase path calls
   `EpdController.resetViewUpdateMode(view)` instead (`EraseFinishAction.java:505-508`). Carry the
   "this gesture was an erase" bit that `InkReconcileGate.began(erasing)` already receives, and skip
   `repaintMode.acquire()` for it — the frame then goes out in the view's default mode, which is
   what the display-mode stack's `SESSION`/`BASE` layers already hold. This is the single highest-
   value line in this document: it is ~5 lines and it is what stock is doing differently.

1. **Do not put `interacting` through `configure()` for an erase.** The plugin needs
   `args.interaction` to decide firmware rendering (`kt/OnyxInk.kt:779`), but the _eraser_ case is
   already handled independently by `eraserRenderGate.begin(erasing || args.eraser)`
   (`kt/OnyxInk.kt:778`) using the callback's own `erasing` flag, which arrives before any React
   state does. The `interacting` round-trip through the DOM adds nothing for an erase and costs two
   whole-region invalidations. Either exclude `"erase"` from the `gestureAction` values that set
   `interacting`, or drop `interaction` from `onlyOverlaysChanged`'s must-match set so an
   `interaction`-only delta takes the cheap path.

2. **Separate "the pen was paused" from "the panel was drawn over".** `region.invalidateAll()` is
   correct for a dialog, a menu, a tool change or a page change — things that genuinely repainted
   the panel behind us. It is wrong for a `configure()` that only re-pushed geometry. Give `pause()`
   a parameter (it already has two) and invalidate the whole region only when the caller says the
   panel was covered.

3. **Make `configure()` idempotent.** Compare the full args and return `status()` unchanged when
   they are equal, before the `onlyOverlaysChanged` branch. This is a few lines and removes a class
   of spurious whole-region repaints on scroll.

4. **Give the erase its own damage, from the native points.** We already have it:
   `markNativePoint` feeds `addDamage` with a `max(3, strokeWidth*2)` margin around every raw point
   including erasing ones (`kt/OnyxInk.kt:682-688`, called from `preview` at `:844` and `stroke` at
   `:824`). Once (1) and (2) stop clobbering the region, the eraser reconcile will already be the
   union of the eraser path and the removed strokes' bounds — which is the minimum correct answer,
   and is what the panel needs repainted.

   Worth copying alongside it: stock emits **no frame at all** when the erase changed nothing —
   `if (eraseContext.getChangedShapeList().isEmpty()) return just(this)`
   (`stock/note/note/action/erase/EraseFinishAction.java:344`). An eraser stroke over blank paper
   should cost the panel nothing. Ours currently always reconciles.

None of this requires leaving EinkWise, and it is the part of the work that should land first.
Note however that change **0 does touch display-mode ownership** — it is precisely a change to which
update mode a submitted frame carries — so per CLAUDE.md it needs the device retest, not just a
visual check. Changes 1–3 are inside the configure/region bookkeeping and need only a visual check
plus the `status()` counters.

---

## The two designs

### Design A — stay inside EinkWise (`supportEAC: true`, HD)

**What it buys.**

- The firmware does the ghosting cleanup, on the cadence the _user_ chose in Settings → Display →
  Full Refresh Frequency (`gcInterval`, default 20). We never have to guess the number, and the
  device-wide setting means our app behaves like every other app on the device — which is what a
  BOOX owner expects.
- A post-input settle repaint in AUTO after every interaction burst (`debouncer(true, epdMode 5,
shortDelay 80, longDelay 400…1200, gcInterval)`), which is why HD removed our accumulated
  ghosting in the first place.
- Per-app DPI, forced full-screen and the rest of §4.1 keep working — including anything the user
  has already configured for us in EinkWise.
- Zero new code. Zero new state. Nothing to keep in sync with an OTA.
- The `refresh_mode_4` write we already ship stays meaningful; today it is the single lever that
  changed anything (`07-einkwise.md:1030-1064`).

**What it costs.**

- **A full-panel flash every `gcInterval` finger lifts, including during handwriting.** The counter
  does not know that a tap on our toolbar is not a page turn. At the user's current setting (~10
  per `c89bc5f`) that is frequent enough to be the complaint that started this document.
- We cannot suppress it. The tick happens in `ViewRootImpl` before any of our code runs
  (`aosp/view/ViewRootImpl.java:11958`), and the GC happens inside SurfaceFlinger's debouncer. The
  only app-side levers are the update mode (leaving `{0,3,5}` disables the counter — but that is
  the Speed/A2 profile, which also disables dither, the scroll helper and the settle repaint, i.e.
  everything that keeps ghosting away: `07-einkwise.md:663-681`) or `supportEAC`.
- The firmware's idea of "a screen changed" is "a finger came up". Ours is "the route changed". The
  two do not line up, so some flashes land mid-gesture and some navigations get none.
- We remain exposed to every EAC re-derivation path in §5.4 — a cloud fetch or an OTA can silently
  change our profile back and we would not notice.

**Who it fits.** A user who never touches the panel while writing, and who values matching the
device's global setting over never seeing an unexpected flash.

### Design B — opt out (`supportEAC: false`) and own the upkeep

**What it buys.**

- **No flash we did not ask for.** Both gates close (§"The premise, corrected"): no pipe message,
  no debouncer. Every full refresh from then on is one we issued from `requestFullRefresh`.
- Refreshes become tied to _semantic_ events — a navigation, an overlay closing, a note being
  saved, leaving a writing session — instead of to finger lifts. That is precisely what stock does,
  and stock is the app on this device that does not flash while you write.
- We stop being force-stopped when the user toggles EinkWise (`OECService.java:375`).
- **The panel profile survives.** Waveform, turbo, anti-flicker, panel dither, contrast, gamma,
  saturation, enhance and the scroll/fling A2 are all keyed on `refreshConfig.enable` /
  `displayConfig.enable` / `eacScrollStyle`, none of which is `supportEAC`
  (§4.3). Onyx designed this flag as "keep the panel profile, drop the app-mangling" —
  `createOnyxAppConfig` sets `supportEAC=false` alongside `refreshConfig.enable=true`
  (`fw/optimization/data/v2/EACAppConfig.java:100-111`).
- **It sticks.** Nothing on this build re-derives `supportEAC` for a third-party package — not boot
  validation, not `eac_force.json`, not the OTA jsonVersion ladder, not the default-config rebuild,
  not app upgrade, not even uninstall (§5.5).
- **It becomes structurally impossible** to be pulled into the system ScreenNote handwriting pipeline
  we deliberately stay out of: `getNoteConfig` returns a dummy, so `packageType` stays `NONE`
  (§4.1b #11).
- We stop being force-stopped when the user toggles EinkWise globally (`OECService.java:375`).

**What it costs.**

- **We must write and maintain a refresh policy** — a counter and a decision at each hook point in
  §3.0. Sized in the port plan at **~120–150 lines**, against stock's 450–650, because our UI has
  one surface where stock has forty-nine.
- **The user's Full Refresh Frequency setting stops applying to us** (the per-app half of it, §5.7).
  Either we ignore it — and we are then the one app on the device that does not honour a system
  setting — or we read it back with `IOECService.getGcInterval()` and use it as our own interval.
  The second is right and cheap; it is port-plan step 4.
- **Per-app DPI stops applying** (§4.1 #4). If the user has set one for us, the app visibly
  re-lays-out. **This is the one concrete regression risk**, and it is checkable in one minute (E2).
- **It needs a reboot to take effect** (§5.2), unless the `saveEACAppThemes` route works (E7).
- **The EinkWise panel stops offering "Optimization" for our app** and the EAC badge goes off
  (§5.6) — the same state stock Notes, KReader and the launcher are in today. Defensible, but it is
  a visible change the user should be told about rather than discover.
- More of the panel's behaviour becomes our bug surface. Today a ghosting complaint is arguably the
  firmware's; afterwards it is unambiguously ours.
- **Paint munging goes** (§4.1 #3) — irrelevant for a WebView, whose content Chromium composites
  into a hardware layer rather than drawing through the Java `Canvas`, and which ships its own
  e-ink theme. Stock is exempt from it by a separate check anyway (`View.java:6095`).

**Who it fits.** Us, if the eraser fix (§6) and a policy (§2 → §3) land together — because the flash
complaint is _two_ problems and only one of them is EinkWise's.

### The honest comparison

Design A's cost is a single, well-understood, unfixable behaviour. Design B's cost is a pile of
small, individually-cheap obligations, one measurable regression risk (DPI) and one visible UI
change (§5.6). The unknowns that made B look risky when this study started have mostly closed:

| Was uncertain                          | Now                                                                                              |
| -------------------------------------- | ------------------------------------------------------------------------------------------------ |
| Does the HD profile survive the flip?  | Almost certainly yes — none of the profile/display/scroll path reads `supportEAC` (§4.3). **E1** |
| Can we even write the flag?            | Yes, in the JSON we already round-trip; no permission; it is the vendor's own path (§5.1)        |
| Will an OTA undo it?                   | No, for a third-party package (§5.5)                                                             |
| What does the user see in EinkWise?    | The same thing they see for BOOX Notes (§5.6)                                                    |
| How much policy would we owe?          | ~120–150 lines (port plan)                                                                       |
| Does it actually remove the tap flash? | Yes, by two independent mechanisms (§"The premise, corrected"). **E3**                           |

**But the decisive question is not "which design" — it is "is the flash actually EinkWise's fault".**

Two findings say it is at most partly:

1. **§6.** The flash after erasing is entirely ours: a whole-region invalidation caused by an
   `interacting` round-trip through React, repainted in `HAND_WRITING_REPAINT_MODE`. It would
   survive the opt-out unchanged.
2. **§1.0.** Our per-stroke reconcile is modelled on `GrayscaleRefreshAction`, which **does not run
   on this device** — `enablePenUpRefresh = !isColorDevice()`. Every stroke we write marks a frame in
   a mode stock reserves for a code path it never executes here. That is a second, systemic source
   of visible repaints that has nothing to do with EinkWise.

**Recommended order: land port-plan steps 0a–0c, run E5 and E3, and only then choose.** If what
remains is a counted GC at the user's chosen interval, take Design B — everything in §4 and §5 says
it is safe and durable. If what remains is small, Design A costs nothing and B buys a policy we
would then have to maintain forever.

---

## Port plan

If Design B wins, this is the order. **Steps 0a–0c are independent of the decision and should land
first regardless** — they are the §6 fix, and they are what tells us whether the rest is needed.

| #      | Change                                                                                                                                                                                                                                                                                                                                                                       | File                                                                             | Rule                              | ~LoC | On-device verification                                                                                                                                         |
| ------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- | --------------------------------- | ---- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **0a** | Do not acquire `HAND_WRITING_REPAINT_MODE` for an erase reconcile. Thread the erase flag from `InkReconcileGate.began(erasing)` to `refreshFrame()` and skip `repaintMode.acquire()`.                                                                                                                                                                                        | `kt/OnyxInk.kt` (`refreshFrame`, `begin`), `kt/InkReconcileGate.kt`              | stock `EraseFinishAction:505-508` | ~10  | Erase a word. No flash. `status()` should show `repaintCount` rising with `lastRepaintModeRaw` unchanged.                                                      |
| **0b** | Stop an erase gesture from round-tripping `interacting` through `configure()`: drop `interaction` from `onlyOverlaysChanged`'s must-match set, **or** keep `gestureAction === "erase"` out of `interacting`.                                                                                                                                                                 | `kt/OnyxInk.kt:385-394` **or** `ts/features/handwriting/ink-canvas.tsx:467`      | §6.2                              | ~3   | Erase with `wholeRegionPending` visible in `status()`: it must stay `false` across the gesture. Repainted area shrinks from the limit rect to the erased bbox. |
| **0c** | Make `configure()` idempotent: compare all args and return `status()` early when nothing changed, before the `onlyOverlaysChanged` branch.                                                                                                                                                                                                                                   | `kt/OnyxInk.kt:306-316`                                                          | §6.2                              | ~8   | Scroll the sheet with the pen down-then-up; `repaintCount` should not rise per scroll frame.                                                                   |
| **1**  | Add `getDisallowedEACApps()` to the reflection surface and read it at startup as the guard for step 2.                                                                                                                                                                                                                                                                       | `kt/MobileSystemPlugin.kt` (near `ensureRefreshProfile`)                         | §5.4                              | ~15  | Log line naming the returned list. Must contain the five Onyx packages before we write anything.                                                               |
| **2**  | In the same JSON `ensureRefreshProfile()` already round-trips, additionally write the `createOnyxAppConfig` shape: `supportEAC=false`, `globalActivityConfig.paintConfig.enable=false`, `dpiConfig.enable=false`, `globalCSSConfig.enable=false`; keep `refreshConfig.enable=true`, `displayConfig.enable=true`. Guard on step 1's read, not on the SharedPreference marker. | `kt/MobileSystemPlugin.kt:292-330`                                               | §4.3 corollary, §5.1              | ~20  | Write, `adb reboot`, relaunch. `getDisallowedEACApps()` contains us; EinkWise's long-press menu no longer offers Optimization (§5.6).                          |
| **3**  | Confirm nothing regressed: waveform, dither, contrast, scrolling. If per-app DPI was set, decide whether to compensate in `viewport-zoom`.                                                                                                                                                                                                                                   | —                                                                                | §4.3, §4.1 #4                     | 0    | Experiment E2 below.                                                                                                                                           |
| **4**  | Reinstate the navigation counter, reading its interval from the firmware rather than hard-coding 6: `IOECService.getGcInterval()` (default 20; the user's Full Refresh Frequency).                                                                                                                                                                                           | `ts/app/eink-refresh.ts`, `kt/MobileSystemPlugin.kt` (a `getGcInterval` command) | R3, R4                            | ~40  | Navigate N-1 times: no flash. Navigate once more: one flash. Change the setting in EinkWise and re-check.                                                      |
| **5**  | Hook the counter to the workspace reducer, with the same action set `c89bc5f` removed, plus note/page create-delete.                                                                                                                                                                                                                                                         | `ts/features/workspace/workspace-store.ts`                                       | R3                                | ~15  | Open five notes; the flash lands on the Nth, not mid-gesture.                                                                                                  |
| **6**  | Refresh once on window-focus regain.                                                                                                                                                                                                                                                                                                                                         | `kt/OnyxInk.kt:562-570` or `kt/MobileSystemPlugin.kt`                            | R1 (`PenEventHandler:334-347`)    | ~8   | Switch to the launcher and back; the panel comes back clean rather than showing the launcher's ghost.                                                          |
| **7**  | Suppress a scheduled refresh while a selection frame and its floating menu are on screen; run it when the selection clears.                                                                                                                                                                                                                                                  | `ts/app/eink-refresh.ts` + the handwriting session                               | R8 (`SelectionHandler:2174-2180`) | ~20  | Lasso a word, then navigate; the selection frame must not flash while it is live.                                                                              |
| **8**  | Add an explicit "Refresh screen" control that refreshes now and resets the counter.                                                                                                                                                                                                                                                                                          | `ts/features/settings/` or the note toolbar                                      | R5                                | ~20  | Tap it: one flash, immediately.                                                                                                                                |
| **9**  | Re-run the full handwriting hardware retest (frame fence, eraser gate, display-mode ownership) — CLAUDE.md requires it because step 0a touches the reconcile mode.                                                                                                                                                                                                           | —                                                                                | —                                 | 0    | The existing device checklist.                                                                                                                                 |

**Total new policy: roughly 120–150 lines**, against stock's 450–650 — because our UI has one
surface where stock has forty-nine.

**What must NOT be done:** do not delete `refreshFrame()` on the strength of §1.0. Stock can live
without a per-stroke reconcile because its `EditorView` owns a page bitmap that `View.invalidate()`
puts back on the panel at the next R1; our canvas is a WebView whose only route to the panel is a
submitted compositor frame, and the reconcile is what submits it.

---

## On-device experiments

Each is one testable action. The first three answer the decision; the rest are confirmations.

**E0 — the cheapest reversible opt-out.** Add one debug command to the plugin, next to the existing
`debug_ink_repaint`: `@Command fun debugSetSupportEac(enabled: Boolean)` that reuses
`ensureRefreshProfile()`'s reflection verbatim — `sMethodGetAppConfigFromService` → `JSONObject` →
`root.put("supportEAC", enabled)` → `sMethodApplyAppConfigToService` — with no marker guard. That is
~15 lines and no new machinery. Then:

```
# via the Tauri MCP bridge or a dev-build button
debugSetSupportEac(false)
adb reboot
```

To revert: `debugSetSupportEac(true)` and reboot again. Nothing else is touched, and §5.5 establishes
that nothing on this build will re-derive the value either way — so the experiment is exactly as
reversible as the command that set it. **The whole cost is one debug command and two reboots.**
(Direct `service call onyx_optimization ...` from a root shell would also work — the binder
transaction is `applyAppConfigToService`, tx **12**, `fw/optimization/IOECService.java:379` — but
marshalling a `List<String>` plus a `Bundle` by hand is more error-prone than the debug command.)

**E1 — does HD's waveform survive the flip?** _(the load-bearing inference of §4.3)_
After E0 and the reboot, in a writing session, read `ViewUpdateHelper.getFastModeIndex()` and
`dumpsys SurfaceFlinger | grep -E 'APP_SLOT|TRANSIENT_SLOT'` (`07-einkwise.md:620-626`), and compare
against the same reading taken before E0. **Expected: identical.** If the app-scope slot changes,
Design B costs the profile as well as the counter and the calculus changes.

**E2 — does anything visibly regress?** After E0, walk the app: check the rendered density against
a screenshot taken before (per-app DPI, §4.1 #4), check that scrolling still flings in A2, check
that the e-ink theme still looks the same (paint munging, §4.1 #3). **Expected: only density can
change, and only if a per-app DPI was set.**

**E3 — is the flash actually gone?** After E0, tap the toolbar 30 times during a writing session at
the user's current Full Refresh Frequency. **Expected: zero unrequested full refreshes.** Before E0
the same test should produce three at a `gcInterval` of 10. **Run this one before and after — it is
the measurement the whole document is for.**

**E4 — what does EinkWise show?** After E0 and the reboot, long-press our app in the launcher.
**Expected (§5.6): no "Optimization" item in the context menu, the EAC badge off, and the app absent
from Settings → per-app DPI / page-key / scroll-button lists.** If Optimization is still offered,
the theme copy did not happen and §5.2's reboot reasoning is wrong.

**E5 — the eraser, before touching EinkWise at all.** Apply port-plan steps 0a–0c, then erase a word
and watch. **Expected: no flash, and `status()` reporting `wholeRegionPending:false` with
`lastRepaint` equal to the erased bounding box rather than the limit rect.** If the flash persists
after 0a+0b, there is a third cause and §6 is incomplete.

**E6 — is the WebView CSS payload reachable?** With the app running, set
`globalCSSConfig.enable=true, customCSS="body{color:red}"` via the same write path, then reload.
**If red text appears, `URLUtil.isNetworkUrl` matches `http://tauri.localhost` and §4.4's hazard is
live** — set `globalCSSConfig.enable=false` permanently. If nothing happens, the hazard is theoretical.

**E7 — does the immediate theme write work?** Only if E0's reboot latency turns out to matter:
reflectively call `EInkHelper.loadThemes(pkg)`, flip `appConfig.supportEAC` in the entry whose
`themeType` matches `eac_active_theme_<pkg>`, call `EInkHelper.saveEACAppThemes([json])`, then
re-read via `getAppConfigFromService` **without rebooting**. **Expected: the new value comes back**
(themes are re-read from MMKV on every access, §5.3). If it does, the reboot in E0 is unnecessary.

**E8 — confirm stock's own numbers on device.** In stock Notes, turn ten pages and count the
flashes. **Expected: exactly one** (`fullRefreshCount = 10`, enabled because this is a colour panel,
§1.5). This validates the whole reading of stock's policy in one minute and is worth doing first.
