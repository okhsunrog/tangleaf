# The raw-drawing render flag on a Note Air 4C (`lito`, FW 4.2, Kaleido)

> Written 2026-09-07 against the decompiled framework, the `com.onyx` system app, the Pen SDK and
> the on-device native libraries, before `/system/bin/surfaceflinger` was extracted from the firmware
> package. Statements marked as inference about the SurfaceFlinger side can now be checked against the
> binary — see [README](README.md). Absolute paths in this document refer to the scratch tree described
> in [01-sources-and-firmware.md](01-sources-and-firmware.md).

What `TouchHelper.setRawDrawingRenderEnabled(false/true)` actually is, why cycling it heals a dead
rectangle when nothing else does, what the cycle costs, and how to use it in a live writing session.

| shorthand | root                                                                                                                       |
| --------- | -------------------------------------------------------------------------------------------------------------------------- |
| `fw/`     | `/home/okhsunrog/tmp_zfs/onyx_framework/out/sources/android/onyx/` (decompiled `framework.jar`)                            |
| `sdk/`    | `/home/okhsunrog/tmp_zfs/onyx_sdk_decompiled/src/` (onyxsdk-pen 1.5.4.3, onyxsdk-device 1.3.5.2)                           |
| `kcb/`    | `/home/okhsunrog/tmp_zfs/onyx_framework/out-kcb/sources/` (decompiled `kcb.apk`)                                           |
| `native/` | `/home/okhsunrog/tmp_zfs/onyx_framework/native/*.so`                                                                       |
| `stock/`  | `/home/okhsunrog/tmp_zfs/reversed_onyx_notes_app/src/`                                                                     |
| `ref/`    | `/home/okhsunrog/tmp_zfs/reference_notes_apps/`                                                                            |
| `ours/`   | `/home/okhsunrog/code/rust/notes-rs/plugins/tauri-plugin-mobile-system/android/src/main/java/dev/okhsunrog/mobile_system/` |

Everything marked **[inference]** is reasoning, not something read out of a file.

---

## 0. The one-paragraph answer

The render flag is not a renderer switch. It is a **SurfaceFlinger session switch**. Every EPD call in
this stack is a raw `IBinder.transact` on the `SurfaceFlinger` service with the interface token
`android.ui.ISurfaceComposer` (`fw/ViewUpdateHelper.java:1421-1440`, `:1346-1350`), and
`setRawDrawingRenderEnabled` resolves to exactly three of them: `ENABLE_POST(-1,1,pid)` ("stop
suppressing this process's composited output"), `SET_SCREEN_HANDWRITING_PEN_STATE(PEN_PAUSE)` and, on
the way back up, `SET_SCREEN_HANDWRITING_PEN_STATE(PEN_DRAWING)`. Nothing else in the SDK emits
`ENABLE_POST(1)`, and no amount of `handwritingRepaint`, `View.invalidate`, exclude-rect clearing or
input-reader cycling touches SurfaceFlinger's handwriting session state at all — three of those four
do not even reach SurfaceFlinger's pen path, and the fourth (`handwritingRepaint`) is a request
_inside_ a session that is already wedged. That is why only the render flag heals it.

---

## 1. `setRawDrawingRenderEnabled` traced to the wire

### 1.1 The transport

```java
// fw/ViewUpdateHelper.java:1346-1350
public static Parcel surfaceComposerData() {
    Parcel p = Parcel.obtain();
    p.writeInterfaceToken("android.ui.ISurfaceComposer");   // TOKEN_TAG, :83
    return p;
}
// fw/ViewUpdateHelper.java:1421-1440
public static void transactData(int code, Parcel data, Parcel reply) {
    IBinder service = ServiceManager.getService("SurfaceFlinger");   // SF_TAG, :82
    if (service == null) { Log.e(TAG, "null surfaceflinger."); }
    else if (!service.transact(code, data, reply, 0)) Log.e(TAG, "transact failed: " + code);
}
```

Two consequences. First, the receiving side is the `surfaceflinger` binary, which is not readable on
this device — so §2 is partly reconstruction. A sweep of every supplied `native/*.so` found **none** of
these transaction codes as literals, split immediates or strings; `libonyx_epd_listener.so` is a FIFO +
`eventfd` pump (`EpdListener::readEpdEventLoop`, `processEpdEvent`, `notifyFd`, log tag
`lib_epd_listener`) with no dispatch table, so the code that consumes these transactions really is in
`surfaceflinger`.

Second, **there is no `View`**. Methods that take a `View` use it only to translate coordinates
(`fw/ViewUpdateHelper.java:785-806` for `handwritingRepaint`, `:1210-1224` for the region setters). The
session is keyed by **`Process.myPid()`**, written explicitly into the parcel:

```java
// fw/ViewUpdateHelper.java:1195-1200
public static void setScreenHandWritingPenState(int i) {
    Parcel p = surfaceComposerData();
    p.writeInt(i);
    p.writeInt(Process.myPid());        // <-- the session key
    transactData(SET_SCREEN_HANDWRITING_PEN_STATE, p, null);   // 16711693
}
// fw/ViewUpdateHelper.java:602-608
public static void enablePost(int i) {
    Parcel p = surfaceComposerData();
    p.writeInt(-1);                     // layer selector: -1 == "all layers of this pid" [inference]
    p.writeInt(i);                      // 0 = suppress posting, 1 = normal posting
    p.writeInt(Process.myPid());
    transactData(ENABLE_POST, p, null); // 16711692
}
```

`SDMDevice` reaches both by reflection on `android.onyx.ViewUpdateHelper` (`sdk/onyxsdk-device-1.3.5.2/
.../SDMDevice.java:1009` picks the class, `:1106` binds `enablePost`, `:1110` binds
`setScreenHandWritingPenState`). Both framework methods are `static`, so the `View` receiver that
`SDMDevice.enablePost(View,int)` / `setScreenHandWritingPenState(View,int)` pass
(`SDMDevice.java:1765-1775`, `:1826-1834`) is discarded by `Method.invoke`. **The pen session is
per-process, not per-view.**

### 1.2 The Java call chain

```java
// sdk/onyxsdk-pen-1.5.4.3/.../TouchHelper.java:188-198
public TouchHelper setRawDrawingRenderEnabled(boolean enabled) {
    if (this.f76a && this.f77b != enabled) {          // f76a = created, f77b = render enabled
        for (TouchRender r : this.f79d) r.setDrawingRenderEnabled(enabled);
        this.f77b = enabled;
    }
    return this;
}
```

Our helper is created with `FEATURE_SF_TOUCH_RENDER | FEATURE_APP_TOUCH_RENDER`
(`ours/OnyxInk.kt:327-333`), so `f79d` holds **both** renders, in that order
(`TouchHelper.java:363-372`). Each contributes:

```java
// sdk/.../pen/touch/SFTouchRender.java:265-271
public void setDrawingRenderEnabled(boolean enabled) {
    if (enabled) m213j();      // :198-200  EpdPenManager.resumeDrawing()
    else         m212g();      // :194-197  EpdController.leaveScribbleMode(hostView) + pauseDrawing()
}
// sdk/.../pen/EpdPenManager.java:36-46
public void resumeDrawing() { EpdController.setScreenHandWritingPenState(view, 2); }  // PEN_DRAWING
public void pauseDrawing()  { EpdController.setScreenHandWritingPenState(view, 3); }  // PEN_PAUSE
// sdk/.../device/SDMDevice.java:1763-1766
public void leaveScribbleMode(View v) { enablePost(v, 1); }
// sdk/.../pen/touch/AppTouchRender.java:200-206
public void setDrawingRenderEnabled(boolean enabled) {
    this.f214c = enabled;
    if (enabled) return;                                   // <-- asymmetric on purpose
    EpdController.enablePost(getHostView(), 1);
}
```

Full wire trace of one cycle, in emission order:

| leg                | transaction                                      | code     | payload                |
| ------------------ | ------------------------------------------------ | -------- | ---------------------- |
| `render(false)` #1 | `ENABLE_POST` (SF render -> `leaveScribbleMode`) | 16711692 | `-1, 1, pid`           |
| `render(false)` #2 | `SET_SCREEN_HANDWRITING_PEN_STATE`               | 16711693 | `3 (PEN_PAUSE), pid`   |
| `render(false)` #3 | `ENABLE_POST` (App render)                       | 16711692 | `-1, 1, pid`           |
| `render(true)` #1  | `SET_SCREEN_HANDWRITING_PEN_STATE`               | 16711693 | `2 (PEN_DRAWING), pid` |

Four Binder transactions, three distinct effects, **no repaint, no region push, no stroke-style write,
no input-reader call, no sleep**. Pen-state constants are `PEN_STOP 0 / PEN_START 1 / PEN_DRAWING 2 /
PEN_PAUSE 3 / PEN_ERASING 4` in both the SDK (`EpdPenManager.java:10-14`) and the framework
(`fw/optimization/Constant.java:299-320`).

### 1.3 What `ENABLE_POST` means

Three independent framework call sites pin the semantics of the second int:

```java
// fw/ViewUpdateHelper.java:994-999 — the app is dying
public static void notifySurfaceFlingerOnAppDie() { ... writeInt(-1); writeInt(1); ... ENABLE_POST ... }
// fw/ViewUpdateHelper.java:1071-1078 — full reset of this pid's EPD state
public static void resetEpdPost() { ... writeInt(-1); writeInt(1); writeInt(myPid()); ENABLE_POST;
                                    setScreenHandWritingPenState(0); }
// fw/optimization/screennote/EACScreenNoteUtils.java:108-113 — put the app's own pixels back
public static void repaintView(View v) {
    ViewUpdateHelper.enablePost(1);
    ViewUpdateHelper.refreshScreen(v, getCurrentRefreshMode(v.getContext()));
}
```

`1` = "post normally", `0` = "do not post". SurfaceFlinger must be told `1` when the app **dies**, or
the suppression would outlive the process — that is only necessary if `0` is a _sticky, process-keyed
suppression of composited output to the panel_. And `repaintView` fixes the ordering rule: you cannot
refresh the app's own content onto the panel until posting is re-enabled. **[inference, but tightly
constrained — the three call sites only make sense under this reading.]**

Who ever writes `0`? Exactly one place in the SDK:

```java
// sdk/.../pen/touch/AppTouchRender.java:186-191
public void openDrawing() {
    m185d();
    EpdController.enablePost(getHostView(), 0);                  // <-- enter scribble mode
    EpdController.setScreenHandWritingPenState(getHostView(), 1); // PEN_START
}
```

`SFTouchRender.openDrawing()` (`:258-262`) does **not** — it only does `setStrokeStyle(0)` +
`startRawInputReader()` + `startDrawing()` (PEN*START). So with an SF-only feature mask `enablePost` is
\_never* driven to 0 by the SDK; with our mask of 3 it is driven to 0 once, by `openRawDrawing()`.
`enterScribbleMode` has **no callers anywhere** — not in the SDK, not in `kcb.apk`, not in the
framework, not in stock Notes; only definitions (`sdk/.../EpdController.java:226-228`,
`SDMDevice.java:1758-1761`).

### 1.4 What is rebuilt on false -> true

Concretely, one thing changes on the `true` leg: the session's pen state goes
`PEN_PAUSE -> PEN_DRAWING`. What the `false` leg did to `enablePost` is _not_ undone.

**[inference]** on what that means inside SurfaceFlinger. The transaction table names the machinery:
`SAVE_PEN_ATTACHED_FB` (1049600, `fw/ViewUpdateHelper.java:216`) says there is a **framebuffer attached
to the pen session**; `SET_AUTO_SYNC_BUF_ENABLE` (1048722, `:213`) says there is a policy for whether
the app's own buffer is automatically synced into it; `USE_GC_FOR_NEW_SURFACE` (1048657) and
`POST_LAYER_FILTER_GC_FOR_NEW_SURFACE = 2` (`:229`) say the post path carries per-surface lifecycle
filters; `SET_UPD_LIST_SIZE` (16711716) says the EPD update path holds a **bounded list of update
entries**, whose Java-side mirror merges overlapping same-mode rects and otherwise appends
(`fw/ViewUpdateHelper.java:242-272`, `addToUpdateEntryList`). The framework's own screen-note code
confirms the pairing of that buffer with the pen session: it turns auto-sync **off** exactly while a
draw view is visible and **on** when it is not
(`fw/optimization/screennote/handler/BaseHandler.java:168-171`,
`ViewUpdateHelper.setAutoSyncBufEnable(!isVisible)`).

The model that fits all of it: while a pid's pen state is `PEN_DRAWING`, SurfaceFlinger composes
firmware ink into a pen-attached buffer for that pid's layers and stops auto-syncing the app's own
buffer into it; app frames reach the panel only through the explicitly marked handwriting-repaint path.
Dropping to `PEN_PAUSE` tears that arrangement down and hands the panel back to ordinary composition;
returning to `PEN_DRAWING` re-arms it **from the current contents**. The rebuild is of the
ink-overlay/damage bookkeeping for the session, not of any Java-side state.

### 1.5 What the three failed cures actually touch

- **`setRawInputReaderEnable`** never reaches SurfaceFlinger. `SFTouchRender.setInputReaderEnable`
  (`:277-283`) -> `RawInputManager.resume/pauseRawInputReader` (`:110-120`) ->
  `RawInputReader.resume()`/`pause()` (`:326-338`), which are `nativeSetPenState(4)` and
  `nativePausePen()` in `native/libonyx_pen_touch_reader.so`. That library is a separate, process-local
  evdev reader (`android::PenManager`, `android::TouchReader`, `/dev/input/event%d`) with its own pen
  state field at `PenManager+0x30`; it shares no state with SurfaceFlinger. Cycling it cannot heal a
  display fault by construction.
- **`handwritingRepaint`** (1048647) is a _request within_ the session — a sync flag plus a screen-space
  `int[4]` (`fw/ViewUpdateHelper.java:785-806`). If the session's damage bookkeeping is wedged, asking
  it to repaint a rect is asking the wedged thing to fix itself.
- **`View.invalidate()` / `EpdController.invalidate(view, mode)`** produce an ordinary app frame. With
  the pen session live and auto-sync off, an ordinary frame is exactly what does not reach the panel.
  **[inference]**
- **Clearing exclude rects** writes `SET_SCREEN_HANDWRITING_REGION_EXCLUDE` (16711714) and
  `nativeSetExcludeRegion`; it changes _where ink is accepted_, never the session lifecycle. It also
  cannot be fully withdrawn — `EACScreenNoteUtils.resetScreenHandWritingRegionExclude()` pushes
  `{0,0,0,0}` (`:115-117`), a degenerate rect, not "none".

---

## 2. Why the cycle heals a stuck region, and what would falsify it

### 2.1 The claim

The dead rectangle is **SurfaceFlinger-side pen-session state**, and the render-flag cycle is the only
SDK operation that (a) re-enables posting for the process and (b) drives the session's pen state through
`PEN_PAUSE`. Everything else either never reaches SurfaceFlinger, or operates inside a session whose
state is the problem.

Supporting facts, not inference:

1. `ENABLE_POST(-1,1,pid)` is emitted by **exactly four** SDK paths, all of them a render-disable or a
   close: `SFTouchRender.setDrawingRenderEnabled(false)` (`:194-197`, `:265-271`) and `closeDrawing()`
   (`:191-193`, `:263-268`); `AppTouchRender.setDrawingRenderEnabled(false)` (`:200-206`) and
   `closeDrawing()` (`:193-197`). An exhaustive sweep of `kcb.apk` (30k classes) found the same four
   and nothing else. **Nothing but a render-disable or a full close re-enables posting.**
2. `PEN_PAUSE` is emitted by exactly the same paths.
3. The firmware's own write-on-any-app feature treats "pause the pen state, then force the panel to
   re-show everything" as _the_ recovery gesture, and runs it on every window-focus change, every
   finger-down, every stylus-down outside the draw view and every eraser tool type
   (`fw/optimization/screennote/handler/BaseHandler.java:311-322`, `:194-198`, `:235-242`, `:118-133`)
   — see §3.2. It is the intended cadence, not a corner case.
4. Stock Notes on a colour device has **no other** healing mechanism at all: its per-stroke reconcile is
   compiled out (§3.1), leaving `InvalidateScreenAction` as the only thing that pushes the app's own
   bitmap back onto the panel.
5. The user's observation that the dead rectangle appears in **stock Notes** at a fullscreen->windowed
   transition places the fault below the app, in shared session state.

### 2.2 The competing hypothesis, and why it is not the one at HEAD

`native/libonyx_pen_touch_reader.so` contains a genuine, permanently-latchable region state. Recovered
layout of `android::PenManager` (disassembly of `inLimitRegion` @0xa95c, `inValidRegion` @0xadf4,
`setLimitRegion` @0xaaa4, `resetReader` @0x9fb4, `init` @0x9da8):

| offset             | field                                                                                   |
| ------------------ | --------------------------------------------------------------------------------------- |
| +0x08..+0x20       | four reader objects (Draw / Erase / SideErase / Btn), each `operator new(0x18)`         |
| +0x30              | native pen state                                                                        |
| +0x58 .. +0x1058   | limit-rect array, fixed 0x1000 bytes = 256 rects, **count stored unclamped** at +0x1058 |
| +0x105c .. +0x205c | exclude-rect array, same shape, count at +0x205c                                        |
| +0x2060            | stroke width / hit-test inflation (rects grown by width/2)                              |
| +0x2064            | region mode                                                                             |
| +0x2068            | **`lastRegionHit`**, initialised to -1                                                  |

When region mode is non-zero (i.e. **single-region** mode: `setSingleRegionMode()` ->
`nativeSetRegionMode(1)`), the first rect a stroke enters is latched into `lastRegionHit`, and from then
on a point counts as in-region **only if its rect index equals the latch**. The latch is cleared by
`resetLastRegionHit`, `resetReader`, `init` and `setLimitRegion` — and by nothing else; `pausePen`
(@0xa8a0), `setPenState` (@0xa898) and `setRegionMode` (@0xa954) all leave it alone. Separately, both
region setters copy `count` floats into their fixed buffers and store `count` verbatim with **no upper
clamp**, so more than 256 rects would overflow the limit array into the exclude array.

A stuck native latch is real, but it is not this bug:

- it cannot hide app content, only ink — the reported dead zone does both;
- it is cleared by `setLimitRect`, which `RawDrawingGate.pushRects()` already calls on every resume
  (`ours/RawDrawingGate.kt:12-16`, `ours/OnyxInk.kt:113-122`);
- at HEAD we push **one** limit rect and **zero** exclude rects (`ours/OnyxInk.kt:339-343`, `:176-181`),
  so index 0 is the only index there is and the latch is inert.

It is, however, a sufficient explanation for the earlier states of the app that used exclude rects, and
it is the reason a heal must re-push the limit rect rather than only cycle the flag.

### 2.3 The single observation that decides it

**Run `leaveScribbleMode` alone on a live dead rectangle.** `ours/OnyxInk.kt:404` already exposes it as
`debugRepaint("leaveScribble")` — `EpdController.leaveScribbleMode(webView)`, i.e. one
`ENABLE_POST(-1,1,pid)` and nothing else. It bypasses `TouchHelper`'s dedup guard entirely, so it fires
regardless of `f77b`.

- If the rectangle **heals**: the fault is the sticky post-suppression, the cure is one transaction, and
  the policy in §5 collapses to a single call.
- If it does **not** heal but the full cycle does: the fault is the pen-session ink/damage bookkeeping,
  and the `PEN_DRAWING -> PEN_PAUSE -> PEN_DRAWING` transition is the load-bearing part. In that case a
  bare `setScreenHandWritingPenState(view, 3)` then `(view, 2)` — also two dedup-free transactions —
  should heal it identically, and that is experiment E2.

Those two probes partition the hypothesis space cleanly, and both are one-line additions to the
existing `debugRepaint` harness.

---

## 3. The healing sequences that already exist in this firmware

### 3.1 Stock Notes: `InvalidateScreenAction`, reconstructed

Source: `stock/com/onyx/android/note/note/action/render/InvalidateScreenAction.java` (164 lines, class
name not obfuscated; fields `q` = `rawPenArgs`, `r` = `checkUseRegalMode`, `s` = `delayRefreshTime`).

Constructor (`:29-39`) builds a `RawPenArgs` with **all four** flags set — pause render, pause input,
resume render, resume input — and `resumeDelayTime = PenEvent.DELAY_ENABLE_RAW_DRAWING_MILLS`. There is
deliberately **no** `setResumeRawInputReader` setter: resume-input is always true. `create()` (`:103-127`)
is `map{setRegalMode} -> flatMap{optional delay} -> map{postEvent}`.

The full emitted sequence, default args, on a colour device:

```
[main]  (opt) InkRenderManager.enableRegalModeOnce()          // only if setCheckUseRegalMode(true)
[note]  (opt) delay(delayRefreshTime)                          // only if setDelayRefreshTime(n)
[main]  postDocEvent(InvalidateViewWithPenControlEvent(rawPenArgs))
[main]  ScribbleHandler.onInvalidateScreenEvent  (:1008-1017)  // no-op if a selection exists
          -> new InvalidateViewWithPenControlAction(...).execute()      (:62-86)
[pen ]  PauseRawPenAction.n()  (:47-58)
          both flags set -> TouchHelper.setRawDrawingEnabled(false):
            -> setRawDrawingRenderEnabled(false)
                 -> EpdController.leaveScribbleMode(view)          // ENABLE_POST(-1,1,pid)
                 -> setScreenHandWritingPenState(view, 3)          // PEN_PAUSE
            -> setRawInputReaderEnable(false)  -> RawInputReader.pause()
            -> resetPenDefaultRawDrawing(): setBrushRawDrawingEnabled(true),
                                            setEraserRawDrawingEnabled(false, 5)
[main]  InkRenderManager.invalidate(DisplayArgs)  (:79-92)
          (opt) enableRegal() + setUpdateMode(view, REGAL_PLUS)
          view.invalidate()                    // <-- a PLAIN View.invalidate(), nothing more
          resetViewUpdateMode()
[pen ]  ResumeRawPenAction  (:44-48) -> EventBus.post(PenEvent(render, input, delay))
[main]  PenEventHandler.onPenEvent (:417-421) -> veto gate A() (:105-107)
[pen ]  ResumeRawDrawingRequest  (:120-138)
          ThreadUtils.mySleep(delayResumePenTimeMs)          // 500 ms on a colour device
          o() (:107-113): setDrawLimitRect(list); setDrawExcludeRect(list);
                          setPenUpRefreshTimeMs(...); setSingleRegionMode();
                          applyDrawingArgs(...); applyPenArgs / stroke style /
                          Device.setStrokeParameters(5,{5f}); setStrokeParameters(8,{w,0.5,0.1});
                          setErasePathDrawing(...); setBrushRawDrawing(...)
          setRawInputReaderEnable(true)          // INPUT FIRST
          setRawDrawingRenderEnabled(true)       // RENDER SECOND -> PEN_DRAWING
```

Order relative to pen state, stated plainly: **the full disable (render + input + `leaveScribbleMode` +
`PEN_PAUSE`) happens strictly before `view.invalidate()`; the re-enable (input, then render, ending in
`PEN_DRAWING`) happens strictly after, separated by a blocking 150 ms mono / 500 ms colour sleep and a
complete re-push of limit rects, exclude rects, region mode, pen-up interval and every stroke
parameter.** There is no `handwritingRepaint`, no `refreshScreenRegion` and no `repaintEverything`
anywhere in the sequence.

Delay constants: `stock/com/onyx/android/sdk/notecore/editor/data/RawPenArgs.java:68-70` —
`DELAY_ENABLE_RAW_DRAWING_MILLS = isColorDevice() ? 500 : 150`;
`stock/com/onyx/android/note/note/event/PenEvent.java:8-11,20` — `POPUP_RESUME_PEN_TIME_MS =
isColorDevice() ? 500 : 300`, `SELECTION_RESUME_PEN_TIME_MS = 500`, `SHAPE_CHANGE_RESUME_PEN_TIME_MS =
100`. (The `kcb.apk` build of the same SDK carries 800/400 for the popup value — the constant is
build-specific, the shape is not.) `clearDelayResumePenTimeMs()` sets the resume delay to **0** and
exists in the API but has no in-app callers. There is **no debounce** on this path: repeated triggers
each queue a full pause -> invalidate -> sleep -> resume cycle; the only suppression is the resume-side
veto gate `PenEventHandler.A(boolean)` (`:105-107`), which blocks resume while recognizing, a popup, the
status bar, a toast, a no-focus dialog, the float button, a render dialog or a float-menu press is
active, and the ref-counted popup counter (`:353-361`, `:435-445`).

**39 call sites**, the ones that matter for us:

| trigger                                          | file:line                                                                                                                          |
| ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| **window focus changed**                         | `PenEventHandler.java:345` (`onActivityFocusChangedEvent`, posted from `ScribbleActivity.onWindowFocusChanged:498`)                |
| nav-ball / float button toggled                  | `PenEventHandler.java:367`, `.setDelayResumePenTimeMs(500)`                                                                        |
| **finger drag-gesture down**                     | `CommonEventHandler.java:160` via `DragGestureListener.java:106`                                                                   |
| **screen rotation**                              | `DeviceReceiverEventHandler.java:233`                                                                                              |
| full-screen refresh broadcast                    | `EpdShapeHandler.java:361`                                                                                                         |
| **page refresh**                                 | `ScribbleHandler.java:1163`, `:1255`; `HWRHandler.java:152`                                                                        |
| **lasso stroke end**                             | `SelectionHandler.java:2159`, `.setDelayRefreshTime(300)`                                                                          |
| text tool enter/exit                             | `RichTextHandler.java:334`                                                                                                         |
| **toolbar show/hide, edge slide**                | `SlideToolbarVisibilityToggleDetector.java:70`, `:77`                                                                              |
| **dialog shown**                                 | `ScribbleFragment.java:1210`, `:1223`                                                                                              |
| **finger-touch (CTP) toggled**                   | `ScribbleFragment.java:1988`; `ToggleFingerTouchAction.java:70`                                                                    |
| fragment becomes visible                         | `FragmentHWRPlacement.java:589`                                                                                                    |
| float menu moved / re-laid-out                   | `FragmentFloatMenu.java:214`; `FloatToolMenuHandler.java:555`; `BaseDocMenuHandler.java:150`                                       |
| toolbar collapsed / side-swapped                 | `ToolbarMenuModel.java:584`                                                                                                        |
| **pen change / shape change / pen-type toggle**  | `FloatPenChangeAction.java:64`, `ShapeChangeAction.java:38`, `TogglePenTypeAction.java:46`                                         |
| enter text input / universal shape / fill colour | `StartTextInputAction.java:69`, `StartUniversalShapeAction.java:37`, `StartFillColorAction.java:55`, `QuitFillColorAction.java:34` |
| **popup show/dismiss**                           | `PopupChangeAction.java:37`, `.setCheckUseRegalMode(!show).setResumeRawDrawing(false)`                                             |
| **toast**                                        | `ToastChangeAction.java:40`                                                                                                        |
| erase-all / erase-by-type                        | `EraseByTypeAction.java:79`                                                                                                        |
| **GC cadence (page turns, menu actions)**        | `EpdDeviceManager.java:19` = `byPass(0)` + `applyGCOnce()` + `InvalidateScreenAction`                                              |

Note the recurring `.setResumeRawDrawing(false)`: many UI events pause + invalidate and deliberately
leave render **off**, relying on a later focus/popup event to resume. Not triggered by: undo, redo,
save, scroll end, or `onResume`/`onPause` directly — the resume path arrives via
`ActivityFocusChangedEvent` / `onSupportVisible`.

**And on this device, that cycle is stock's only reconcile.**
`stock/com/onyx/android/sdk/data/config/system/data/SystemConfigBean.java:22` sets
`enablePenUpRefresh = !DeviceInfoUtil.isColorDevice()`, and
`OnyxSystemConfig.isUseHWUpdateForPenUpRefresh()` (`:145-149`) is the only gate on `EpdShapeHandler.P0()`
(`:248-263`), the per-stroke `GrayscaleRefreshAction`. On a Note Air 4C `P0()` short-circuits: stock
leaves firmware ink standing after every stroke and never marks a handwriting-repaint frame while
writing.

### 3.2 The framework's own screen-note handler — the cleanest reference policy

`fw/optimization/screennote/handler/BaseHandler.java` implements "write on top of any app" with the same
primitives:

| event                                       | `:line`    | sequence emitted                                                                                                                                                                                                                                                                                                                         |
| ------------------------------------------- | ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| stylus DOWN in draw view                    | `:227-233` | `cancelPendingRedraw()`; `PEN_DRAWING`                                                                                                                                                                                                                                                                                                   |
| stylus UP/CANCEL in draw view               | `:268-283` | after `getRepaintLatency()` ms: `handwritingRepaint(view, localVisibleRect, sync=false)` — **no pen-state change**                                                                                                                                                                                                                       |
| finger DOWN anywhere                        | `:194-198` | `PEN_PAUSE`; `repaintEverything()`                                                                                                                                                                                                                                                                                                       |
| stylus DOWN outside draw view (IME hidden)  | `:235-242` | `PEN_PAUSE`; `repaintEverything()`                                                                                                                                                                                                                                                                                                       |
| tool type ERASER                            | `:118-133` | `PEN_PAUSE`                                                                                                                                                                                                                                                                                                                              |
| **window focus changed (either direction)** | `:311-322` | if losing focus `cancelPendingRedraw()`; `PEN_PAUSE`; `repaintEverything()`; if gaining focus: `resetScreenHandWritingRegionExclude()`, `setScreenHandWritingRegionLimit(view, localVisibleRect)`, then `setStrokeColor`/`setStrokeStyle`/`setStrokeWidth`/`setStrokeParameters` and finally `PEN_DRAWING` (`applyStrokeParam` `:65-77`) |
| draw view size changed / global layout      | `:157-163` | `PEN_PAUSE`; `setLimitRegion(newRect)`; `PEN_DRAWING`                                                                                                                                                                                                                                                                                    |
| draw view becomes visible                   | `:165-190` | `setAutoSyncBufEnable(false)`; reset exclude; set limit; `PEN_START`; stroke params; `PEN_DRAWING`                                                                                                                                                                                                                                       |
| draw view becomes invisible                 | `:165-171` | `setAutoSyncBufEnable(true)`; `PEN_STOP`                                                                                                                                                                                                                                                                                                 |
| menu clicked                                | `:219-224` | re-apply stroke params -> `PEN_DRAWING`                                                                                                                                                                                                                                                                                                  |

Three rules fall straight out:

1. **`repaintEverything()` is never emitted while the pen state is `PEN_DRAWING`.** Pause first, repaint
   second — the same ordering rule as `repaintView()` in §1.3.
2. **A pen-up repaint is a `handwritingRepaint` with no pen-state transition** — the cheap path — and it
   is _debounced_ by `getRepaintLatency()` and _cancelled by the next pen-down_.
3. **A focus regain re-pushes exclude, limit and every stroke attribute** before resuming. A mere layout
   change re-pushes only the limit rect. Nothing re-pushes the region _mode_.

Subclass refinements worth copying: `SingleDrawViewHandler.resumeEACScreenNote` refuses to resume while
the IME is visible or the view lacks window focus, and `setScreenHandWritingRegionLimit` refuses to push
a limit rect while the IME is visible (`SingleDrawViewHandler.java:26-42`).

### 3.3 The reference apps converged on the same primitive

- notable: `prepareForPartialUpdate` ends with a bare `isRawDrawingRenderEnabled = false; ... = true`
  (`ref/notable/.../editor/utils/einkHelper.kt:282-289`), run before every partial update; and
  `resetScreenFreeze` (`:333-346`) is the delayed, **coalescing** version — one cancellable job, because
  "a continuous scroll fires resetScreenFreeze on every frame, each arming a fresh 500 ms resume timer"
  (`:320-322`). It also names the failure mode: a pending resume that fires after raw drawing was turned
  off entirely "would hand the screen back to the firmware with input disabled — a frozen screen nothing
  unfreezes" (`:326-328`).
- saber: `setRawDrawingEnabled(false); ...(true)` on every layout change.
- PngNote: `withTempNoRawRendering` around every surface repaint.
- mokke: "while TouchHelper has raw drawing enabled, nothing else is allowed to refresh the panel...
  pause first, refresh, re-enable when ready".
- kcb's own pen-test onboarding page does an immediate, zero-delay false->true pair around a UI change
  (`kcb/com/onyx/reader/startuptutorial/fragments/TutorialActivePenFragment.java:305-315`).

The kcb build also exposes the generic arbiter behind stock's behaviour,
`kcb/com/onyx/android/sdk/scribble/data/pen/BasePenEventHandler.java`: every subscriber funnels into
`e(boolean)` (`:55`) and resumes with `c(render=true, input=true, DELAY_ENABLE_RAW_DRAWING_MILLS)`
(`:40-52`), for activity focus (`:112`), float button (`:130`), IME (`:138`), no-focus system dialog
(`:146`), explicit pause (`:154`), generic pen event (`:161`), ref-counted popup (`:166`), screenshot
(`:177`), status bar (`:184`) and toast (`:191`), gated on all of them being clear plus window focus
(`:223`, `:227`). Every note request also pauses the pen around itself by default
(`kcb/.../notecore/editor/request/BaseNoteRequest.java:15-16`, `:32-42`).

**We are the only implementation surveyed that never cycles the flag inside a writing session.**

---

## 4. What a cycle costs

| does the cycle...                                    | answer                                                                                                                                                                                                                                                                                                                                                                                                                            | evidence                                                                                                                                                     |
| ---------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| drop queued strokes?                                 | **No** for points already delivered — the input reader is untouched, `RawInputReader.f51d` stays true, evdev polling continues. **Yes** for the firmware ink of a stroke in flight: the false leg is `PEN_PAUSE`, so ink stops being composed mid-stroke. Never cycle during a gesture.                                                                                                                                           | `SFTouchRender.java:265-271` touches no `RawInputManager` method; `RawInputReader.pause()` is reachable only from `setInputReaderEnable(false)` (`:277-283`) |
| flash the panel?                                     | **Not by itself.** The cycle emits no update-mode change and no repaint. The visible cost is whatever the next ordinary frame does at the view's current update mode. The flash is `repaintEverything()`, which the framework _pairs_ with `PEN_PAUSE` but which the cycle does not contain.                                                                                                                                      | `fw/ViewUpdateHelper.java:1059-1066`; the four transactions of §1.2                                                                                          |
| reset stroke style / width / colour / params?        | **No.** Those are separate transactions (`SET_STROKE_STYLE` 16711688, `SET_STROKE_WIDTH` 16711687, `SET_STROKE_COLOR` 16711686, `SET_STROKE_PARAMETERS` 1049089) and the cycle emits none. **But `openDrawing()`/`closeDrawing()` do** — both force style back to `STROKE_STYLE_PENCIL` (`SFTouchRender.java:170-179`, `m208i()` -> `setStrokeStyle(0)`), so `restartRawDrawing()` and `closeRawDrawing()` are _not_ substitutes. | `SFTouchRender.java:258-271`                                                                                                                                 |
| reset the pen state machine?                         | **Yes, deliberately** — that is the mechanism: `PEN_DRAWING -> PEN_PAUSE -> PEN_DRAWING`. It does **not** pass through `PEN_STOP`, so the session is not torn down and `PEN_START` is not needed to restart it.                                                                                                                                                                                                                   | `EpdPenManager.java:36-46`                                                                                                                                   |
| lose the limit / exclude regions or the region mode? | **No.** No region transaction is emitted. The framework re-pushes the limit rect around a pause only when the _geometry_ changed (`BaseHandler.java:157-163`), not because the pause lost it. Conversely the cycle also **does not clear** the native `lastRegionHit` latch (§2.2) — only `setLimitRect` does.                                                                                                                    | `SFTouchRender.java:265-271`; native disassembly                                                                                                             |
| lose brush / eraser channel flags?                   | **No.** `SET_BRUSH_RAW_DRAWING_ENABLED` (1048834) / `SET_ERASER_RAW_DRAWING_ENABLED` (1048833) are written only by `resetPenDefaultRawDrawing()` and the explicit setters. Note that the _combined_ `setRawDrawingEnabled()` **does** call `resetPenDefaultRawDrawing()` (`TouchHelper.java:170-177`) and would clobber a custom eraser style — one more reason to use the split switch.                                          | `TouchHelper.java:348-351`                                                                                                                                   |
| leave `enablePost` where it found it?                | **No.** `false` drives post to 1; `true` never drives it back to 0. After the first cycle in a session post stays 1 until `closeRawDrawing()`/`openRawDrawing()`.                                                                                                                                                                                                                                                                 | `AppTouchRender.java:200-206` vs `:186-191`                                                                                                                  |
| get swallowed?                                       | **Yes, whenever the flag is already false.** `setRawDrawingRenderEnabled` is guarded by `f77b != enabled` (`TouchHelper.java:188-198`). With an eraser or lasso tool active our render flag is already false (`ours/OnyxInk.kt:596-598`), so `false -> true` emits nothing on the false leg and the heal silently does not happen.                                                                                                | as cited                                                                                                                                                     |

**What must be re-pushed afterwards.** Strictly: nothing — a bare cycle is self-consistent. In practice
follow stock's `ResumeRawDrawingRequest.o()` (`:107-113`) and the framework's focus-regain branch
(`BaseHandler.java:311-322`) on any event where geometry or ownership may have changed underneath:

1. `setSingleRegionMode()` + `setLimitRect(limit, excludes)` — also the **only** clear of the native
   `lastRegionHit` latch, so it is cheap insurance even when the geometry did not move.
2. `resetPenDefaultRawDrawing()` (brush on, hardware eraser off) — these flags are per **device**, not
   per session, so another app can have left them wrong.
3. stroke style / width / colour / `setStrokeParameters` for the current tool.
4. `setPenUpRefreshTimeMs()` — stock re-applies it on every resume (`ResumeRawDrawingRequest.java:109`).

That is exactly what `RawDrawingGate.resume()` already does (`ours/RawDrawingGate.kt:31-42`,
`ours/OnyxInk.kt:113-123`), minus the stroke attributes, which `configure()` pushes separately.

**Cost in time.** Four Binder transactions down, one up, all with a `null` reply parcel and flags `0`
(`fw/ViewUpdateHelper.java:1421-1440`) — sub-millisecond. The real cost is the _policy's_ resume delay:
500 ms on a colour device, during which firmware ink is off and the pen writes nothing. That is why the
cycle must never run per stroke, and why `PenEvent.noDelayResumeRawDrawing()` (delay 0) matters.

---

## 5. Recommended policy

`cycle()` = the bare four-transaction sequence of §1.2. `heal()` = `cycle()` plus the re-push list of §4.

### 5.0 Primitive to add

```kotlin
/** Dedup-proof: bypasses TouchHelper.f77b, so it fires with any tool active. */
private fun forceCycleRender() {
    EpdController.leaveScribbleMode(webView)                       // ENABLE_POST(-1,1,pid)
    EpdController.setScreenHandWritingPenState(webView, 3)         // PEN_PAUSE
    EpdController.setScreenHandWritingPenState(webView, if (toolRendering()) 2 else 3)
}
```

`TouchHelper.setRawDrawingRenderEnabled(false)` is a no-op whenever the render flag is already false —
which at HEAD is every eraser and lasso gesture (`ours/OnyxInk.kt:596-598`). A heal that silently does
nothing on the eraser tool is worse than none. Drive the three transactions directly and leave
`TouchHelper`'s cached `f77b` alone so the gate's bookkeeping stays coherent. If experiment E1 shows
`leaveScribbleMode` alone suffices, drop the two pen-state lines.

### 5.1 Events -> calls

1. **Session open** (`configure()` creating the helper). Unchanged, plus: push an **off-panel** exclude
   rect once (e.g. `Rect(-100,-100,-99,-99)`) before the first `setLimitRect`, to displace any exclude
   rect latched by an earlier build — `{0,0,0,0}` provably does not, because the probe point is inflated
   by half the stroke width and the degenerate rect still swallows the top-left corner.
2. **Window focus gained** -> `heal()` after the resume delay, mirroring
   `BaseHandler.onWindowFocusChangedImpl` (`:311-322`): pause, put the app's pixels back, re-push
   exclude -> limit -> stroke attributes, then resume. Route it through the same delayed `resumeGate`
   `pauseStateChanged()` uses instead of calling `resume()` synchronously — `ours/OnyxInk.kt:555-557` is
   the current divergence.
3. **Window focus lost / `onPause`** -> `gate.pause()` as today (render off, then input off), and cancel
   any pending resume (`einkHelper.kt:326-328` is the warning).
4. **Any named pause reason released** (`InkPauseRegistry` empties: DOM dialog, menu, keyboard, toolbar
   popup, tool change) -> `heal()` after the delay. Use **500 ms after a popup/dialog** and 500 ms
   otherwise on this colour device (`RawPenArgs.java:69`, `PenEvent.java:20`); stock uses two distinct
   constants and so should we, even if they currently coincide.
5. **Limit rect / overlay geometry changed** (sheet resize, band scroll, orientation) -> `PEN_PAUSE` ->
   `setSingleRegionMode()` + `setLimitRect(...)` -> `PEN_DRAWING`. That is
   `BaseHandler.onDrawViewSizeChangedImpl` (`:157-163`) verbatim, and it is the cheapest correct heal
   because `setLimitRect` also clears the native latch.
6. **Finger down inside the writing region** -> `PEN_PAUSE` immediately. Stock does this
   (`DragGestureListener.java:106` -> `CommonEventHandler.java:160`) and so does the framework
   (`BaseHandler.java:194-198`). Do **not** copy the framework's paired `repaintEverything()` — we own
   the whole screen; use the existing whole-region reconcile. Resume on finger up plus the delay.
7. **Tool change / eraser gate** -> keep `InkEraserRenderGate`, but route its resume through
   `forceCycleRender()` so a transition into a render-enabled tool always emits a real `ENABLE_POST(1)`
   even though `f77b` was already false.
8. **Periodic in-session heal — the piece we are missing entirely.** After every **N** completed
   gestures **or** every **T** seconds of continuous writing, whichever comes first, and only in the
   idle window after a pen-up (never mid-gesture, never before the reconcile frame has landed), run
   `heal()` with a **0 ms** resume — `PenEvent.noDelayResumeRawDrawing()` and
   `InvalidateScreenAction.clearDelayResumePenTimeMs()` prove the firmware supports an immediate re-arm.
   Start at N = 20 gestures / T = 30 s and tune with E4. Coalesce it exactly as notable does
   (`einkHelper.kt:333-346`): one cancellable job, re-armed rather than stacked.
9. **Session close** -> `closeRawDrawing()` then `EpdController.resetEpd(context)`
   (`sdk/.../EpdController.java:493-498` = `byPass(0)` + `resetEpdPost()` + `clearTransientUpdate(false)`
   - `appResetCTPDisableRegion`), so a crash or kill cannot leave `enablePost` at 0 for a dead pid.

### 5.2 What replaces the per-stroke reconcile

Keep it, but demote it. It exists for a reason stock does not have: our page lives in a WebView _under_
the firmware ink layer, so ink never handed back is ink no scroll, zoom or undo can repaint. Stock skips
it because its `EditorView` _is_ the writing surface, and on this device its per-stroke path is compiled
out entirely (§3.1). So:

- **Keep** the mode-marked frame as the handover: `setViewDefaultUpdateMode(view,
HAND_WRITING_REPAINT_MODE)` -> `invalidate()` -> reset in the frame-commit callback
  (`ours/OnyxInk.kt:491-537`). That is `GrayscaleRefreshAction`'s shape and it is correct.
- **Keep** the two-sided rendezvous (`InkReconcileGate`) and the stroke-union region from
  `onPenUpRefresh` (`ours/InkReconcileRegion.kt:48-64`) — the SDK's union with the previous stroke
  (`RawInputReader.java:744-759`) is what guarantees consecutive repaints overlap.
- **Add the debounce the framework uses**, which we lack: schedule the reconcile on a short delay and
  **cancel it on the next pen-down** (`BaseHandler.java:268-275`, `:227-233`, `cancelPendingRedraw`
  `:88-96`). During fast continuous writing that collapses a run of strokes into one repaint. We
  currently attempt one per gesture. Strictly better, costs nothing.
- **Add** the render-flag heal as a _separate, much rarer_ track (item 8). Do not fold it into the
  per-stroke path: at a 500 ms colour resume latency a per-stroke cycle makes writing unusable, and at
  0 ms it is unproven.
- **Do not** re-introduce `handwritingRepaint` into the per-stroke path. Zero callers in stock's ink
  path, zero in `kcb.apk`; `34dc906` removed ours for that reason. Keep it in the debug harness only.
- **Do not** use `repaintEverything()` here. Its only two callers in `kcb.apk` are a settings toggle and
  a library refresh button, both `UpdateMode.GC` — a full-screen flash, appropriate for a focus change
  at most.

### 5.3 Invariants

- Never emit `repaintEverything()` or a full refresh while the pen state is `PEN_DRAWING`; pause first
  (`BaseHandler.java:194-198`, `:311-322`; `EACScreenNoteUtils.repaintView:108-113`).
- Never resume render while input is off, and never let a resume land after a real disable
  (`einkHelper.kt:326-328`).
- Resume order stays input-then-render, after the delay (`ResumeRawDrawingRequest.java:126-137`,
  `ours/RawDrawingGate.kt:31-42`).
- Any heal that is not purely a display heal must re-push the limit rect — the only clear of the native
  `lastRegionHit` latch (§2.2).
- Keep pushing exactly one limit rect and zero exclude rects. With one rect the native latch is inert,
  and an exclude rect can never be fully withdrawn.
- Never cycle mid-gesture: the false leg is `PEN_PAUSE` and kills firmware ink for the stroke in flight.

---

## 6. On-device experiments

Each is one testable action. Run with a dead rectangle visible; `ours/OnyxInk.kt:386-409`
(`debugRepaint`) is the existing harness and already carries most of them.

- **E1.** With a dead rectangle present, invoke `debugRepaint("leaveScribble")` — `leaveScribbleMode`
  alone, one `ENABLE_POST(-1,1,pid)`. -> If it heals, the fault is sticky post-suppression and the whole
  policy collapses to one transaction.
- **E2.** With a dead rectangle present, invoke a new `debugRepaint("penState")` emitting
  `setScreenHandWritingPenState(view, 3)` then `(view, 2)` and nothing else. -> If it heals and E1 did
  not, the fault is the pen-session ink/damage bookkeeping.
- **E3.** With a dead rectangle present and the **eraser** tool active (render flag already false),
  invoke `debugRepaint("toggleRaw")`. -> It should _not_ heal, proving the dedup guard swallows the
  cycle; repeating with `forceCycleRender()` should heal. Confirms §4's dedup finding on hardware.
- **E4.** Enable the periodic heal (§5.1 item 8) at N = 20 gestures with a 0 ms resume and write
  continuously for ten minutes. -> If no dead rectangle appears and no stroke latency is perceptible,
  the 0 ms re-arm is safe; if ink stutters at each heal, step the delay 0 -> 150 -> 500 ms and re-run.
- **E5.** Add `EpdController.getPenState()` and `isValidPenState()` (`GET_PEN_STATE` 1048643,
  `IS_PEN_STATE_VALID` 1048641) to `status()` and log them through a writing session. -> If the pen
  state is something other than `PEN_DRAWING` when the rectangle goes dead, the fault is a _lost_
  transition rather than a wedged buffer, and the cure becomes "re-assert `PEN_DRAWING` periodically",
  one transaction.
- **E6.** Log `RawInputReader.isFdValid()` (`nativeIsValid`) alongside E5. -> Distinguishes a dead evdev
  fd from a display-side fault; they need different cures and look identical from outside.
- **E7.** Reproduce the dead rectangle in **stock Notes** at a fullscreen->windowed transition, then open
  our app and check whether the same screen region is dead there. -> If it survives the app switch, the
  wedged state is global to SurfaceFlinger rather than per-pid, and no app-side policy can prevent it —
  only heal it.
- **E8.** Call `ViewUpdateHelper.setAutoSyncBufEnable(false)` at session start and `(true)` at close,
  mirroring `BaseHandler.onDrawViewVisibilityChangedImpl:168-171`, and write for ten minutes. -> If dead
  rectangles become more frequent, the pen-attached-buffer model of §1.4 is confirmed and
  `setAutoSyncBufEnable` becomes a second lever.
- **E9.** Push an off-panel exclude rect (`Rect(-100,-100,-99,-99)`) once at session creation, then clear
  it, on a device where the dead zone reproduces. -> Tests whether an exclude rect latched by an earlier
  build is contributing; a `{0,0,0,0}` clear provably cannot do this.
- **E10.** Replace our per-gesture reconcile scheduling with the framework's cancel-on-pen-down debounce
  (§5.2) and count `repaintCount` over a fixed 200-stroke writing sample. -> Confirms the reduction in
  repaints and whether any ink is left unhanded-back at the end of a burst.
