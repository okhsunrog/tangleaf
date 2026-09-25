# Stock Notes: finger, palm and input routing

Subsystem document 06 of the stock-Notes research (package `com.onyx.android.note`, build
`knote2_release`), written from static analysis only. It describes how the stock app separates
stylus tip, eraser end, side button, finger and palm input, which layer receives each, and what
each routing decision does to the SurfaceFlinger (SF) pen session.

## Conventions

Path prefixes used in citations:

| prefix    | location                                                                                                       |
| --------- | -------------------------------------------------------------------------------------------------------------- |
| `stock/`  | `~/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/` (jadx of the stock APK, app plus bundled SDK) |
| `rx/`     | `~/tmp_zfs/boox-notes-inspect/decoded/sources/com/android/onyxsdk/rxgesture/` (bundled gesture library)        |
| `res/`    | `~/tmp_zfs/boox-notes-inspect/decoded/resources/res/`                                                          |
| `pub/`    | `~/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-pen-1.5.4.3/sources/com/onyx/android/sdk/pen/` (public Pen SDK)     |
| `fw/`     | `~/tmp_zfs/onyx_framework/out/sources/` (framework.jar)                                                        |
| `fws/`    | `~/tmp_zfs/onyx_framework/out-services/sources/` (services.jar)                                                |
| `native/` | `~/tmp_zfs/onyx_framework/native/` (disassembled with `llvm-objdump`; addresses are file offsets)              |
| `doc08`   | `docs/onyx-reversing/08-surfaceflinger.md` (earlier SF binary analysis, not re-verified here)                  |

Line numbers are those of the jadx output files as they are on disk. Claim markers:

- **[code]** read directly in the cited source or disassembly;
- **[inferred]** reasoned from code that was read, not directly observed;
- **[unknown]** not determinable from the available artefacts.

SF pen states are the `EpdPenManager` constants (`stock/sdk/pen/EpdPenManager.java:9-13`):
0 STOP, 1 START, 2 DRAWING, 3 PAUSE, 4 ERASING. "Render off" below always means
`SFTouchRender.setDrawingRenderEnabled(false)` = `leaveScribbleMode` (`ENABLE_POST(1)`) + pen state 3;
"render on" means pen state 2 (`stock/sdk/pen/touch/SFTouchRender.java:264-285, 368-374`) [code].
What SF does on those transitions (PAUSE runs `stopHandwriting` and recomposites, etc.) is taken
from doc08 and is marked [inferred, doc08] where it matters.

---

## 1. Summary

1. **Two independent input channels, both always live.** The stylus digitizer is read twice:
   (a) by an in-process evdev reader, `libonyx_pen_touch_reader.so`, that the bundled Pen SDK starts
   inside the Notes process and that feeds `RawInputReader.onTouchPointReceived`, and (b) by
   Android's own input pipeline, which delivers the same stylus contacts as ordinary `MotionEvent`s
   (tool type 2 stylus, 4 eraser) to the `EditorView`. Fingers reach the app **only** through
   Android `MotionEvent`s; the raw reader opens only the pen digitizer. SF additionally runs its own
   reader for the live ink ([inferred, doc08]).
2. **The raw channel carries all ink and all erasing.** Tip strokes, eraser-end strokes and
   side-button strokes arrive as raw callbacks (`onBeginRawDrawing` / `onBeginRawErasing` ...) and are
   posted to the note event bus; the `MotionEvent` copy of the same stylus contact never draws or
   erases, but it is not pure bookkeeping either: besides HWR point capture, "raw point not reported"
   diagnostics and layer warnings it drives pen-hold shape recognition (`RxPenGesture` →
   `PenStatusChangeEvent` → `HWRContextLessEventHandler.onPenHolding` → `StartRecognizeDiagramAction`)
   and, as a single-pointer `TouchDownEvent`, can trigger a pen **resume** (see item 4)
   [code, see §3.4; corrected in verification].
3. **Fingers never draw in pen mode.** Finger drawing exists only in lasso-selection mode
   (`SelectionHandler.supportFingerDraw()`); every other handler returns `false`. Fingers are routed
   to a gesture stack (`RxGesture`) that does pinch-zoom, pan, single-finger swipe page-turn, taps and
   long-presses.
4. **Finger touch on the canvas does not pause the pen.** No code path in stock Notes **pauses** the
   SF session on a plain finger-down on the canvas. A canvas touch-down can, however, **resume** it:
   `PenEventHandler.onTouchDownEvent` (`stock/note/note/eventhandler/PenEventHandler.java:517-524`)
   clears the "status bar shown" flag and requests a render+input resume (500 ms on colour) when that
   flag was set; and `ScribbleFragment.g0` hides the soft keyboard on every `ACTION_DOWN`, which
   [inferred] ends a keyboard-shown pause through `PenEventHandler.onKeyboardChange` (`:376-387`).
   Touches inside the locked zoom-preview overlay do pause (§3.10). (The "PEN_PAUSE on every finger-down" rule quoted in
   earlier documents belongs to the firmware's own screen-note pipeline, `fw/android/onyx/optimization/screennote/handler/BaseHandler.java`,
   which stock Notes does not use.) The pen session is paused only when a finger gesture actually
   becomes a **scroll or pinch** (`ViewportScrollBeginEvent` / `ViewportScaleBeginEvent` →
   `PauseResumeRawPenAction`, plus the firmware "fast mode" transient update).
5. **Finger suppression while the pen is near is done in software, from hover.** Every evdev `SYN`
   from the pen digitizer (hover or contact) is reported by the native reader with state 5. Java turns
   that into `PenActiveEvent`, and 100 ms after the last one into `PenDeactivateEvent`.
   `ScribbleTouchDistributor` forwards both to `RxGesture.setPenActive`, and while "pen active" is set
   `RxGesture` drops every finger `MOVE` and all gesture detection. State 5 is not region-tested, so
   hovering anywhere over the digitizer (including over docked toolbars outside the limit rect)
   arms the filter [inferred from `RawInputReader.H()` having no region check and the native report
   path; see §3.3]. That is the only "palm rejection"
   in the app; there is no palm detector in use (`PalmDetector` is dead code), no CTP disable call and
   no `TouchHelper.enableFingerTouch` call.
6. **Toolbars and overlays are carved out geometrically, not by pausing.** The pen limit rect is the
   `EditorView` bounds; docked toolbars sit outside it. Floating overlays that lie over the canvas
   (float tool menu, zoom preview, page navigator, scroll bars, clipboard menu, link-jump menu, audio
   menu, compact float icon) register an **exclude rect** of a typed slot, and the new exclude list is
   pushed to SF and to the in-process reader with `UpdateRawDrawRectAction` **without any pen-state
   change**. Touching a floating-toolbar item, however, **does** pause the session on touch-down
   (`FloatDraggableMenuBar ... onDown → RawPenPauseEvent(pauseArgs)`).
7. **Eraser end = `BTN_TOOL_RUBBER`; side button = `BTN_STYLUS`.** The native reader switches to its
   `EraseReader` (rubber) or `SideEraseReader` (side button + tip, enabled by default in the native
   code and never changed by stock) and reports `isErasing=true`; the SDK turns that into
   `onBeginRawErasing(shortcutErasing = side-button?)`. Stock Notes never calls
   `enableSideBtnErase`/`setEnablePenSideButton`, never reads Android stylus button state, and treats
   rubber and side button identically (`ScribbleHandler.onBeginRawErasing` ignores the flag).
   No input-routing code pauses the pen for the eraser end; whether erasing runs with render on is
   decided by the eraser type (see §9 and document 03-erasing).
8. **Hover `MotionEvent`s are ignored.** No hover listener is installed; hover is known to the app
   only through the raw reader's state-5 reports.

---

## 2. Central classes and entry points

| role                                                                                                            | class / method                                                                                                                                                                                                                                     | file:line                                                                                                                                                                                                    |
| --------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| host view of the canvas                                                                                         | `EditorView` (plain `View`, `R.id.surface_view`)                                                                                                                                                                                                   | `stock/sdk/notecore/editor/display/EditorView.java:20`; `res/layout/fragment_scribble_normal_mode.xml:20-27`                                                                                                 |
| makes the host view clickable                                                                                   | data binding `surfaceView.setOnClickListener(ScribbleMenuModel::onWorkLayoutClick)`                                                                                                                                                                | `stock/note/databinding/FragmentScribbleNormalModeBindingImpl.java:419`, `:73-79`; `stock/note/note/menu/viewmodel/scribble/ScribbleMenuModel.java:461-463, 110-112`                                         |
| `MotionEvent` entry                                                                                             | `ScribbleFragment.N()` installs `OnTouchListener` → `g0()` → `ScribbleTouchDistributor.onTouchEvent`                                                                                                                                               | `stock/note/note/ui/ScribbleFragment.java:593-605`, `:806-811`                                                                                                                                               |
| second `MotionEvent` entry (full-screen floating menu)                                                          | `FragmentFloatMenu.l()/m()` forwards touches on the float-menu parent to its own `ScribbleTouchDistributor` when full-screen                                                                                                                       | `stock/note/note/menu/fragment/FragmentFloatMenu.java:170-194`                                                                                                                                               |
| fragment-root intercept (sees every touch in the fragment before children, AOSP intercept semantics [inferred]) | `ScribbleFragment.onCreateView` → `InterceptFrameLayout.setOnInterceptTouchEventListener` → `y0()`: sets `ScribbleMenuModel.touchState` (DOWN / MULTI_DOWN) and feeds `ToolbarGestureDetector` (only created when `scene.supportToolbarGesture()`) | `stock/note/note/ui/ScribbleFragment.java:1466-1474, 1191-1201, 638-641`                                                                                                                                     |
| zoom-preview touch entry                                                                                        | zoom-menu handler consumer → `ScribbleFragment.i0` → `ScribbleTouchDistributor.onZoomPreviewTouchEvent` → `ZoomPreviewGestureListener` (tap/scroll → `ScrollByViewportAction`, no pen-state call in that chain)                                    | `ScribbleFragment.java:678-684, 850-852`; `ScribbleTouchDistributor.java:401-405`; `stock/note/note/touch/ZoomPreviewGestureListener.java:79-137`; `stock/note/note/action/zoom/ScrollByViewportAction.java` |
| rich-text scene (separate `TouchHelper`)                                                                        | `RichTextBundle` `OnTouchListener` → `HandWritingHandler.onTouchEvent` → its own `TouchHelper.create(view, cb)` (mask 2 if the device reports a stylus, else 1; no `setPostInputEvent`)                                                            | `stock/note/note/RichTextBundle.java:36-61`; `stock/note/note/handler/text/HandWritingHandler.java:147-157, 465-516`; `stock/sdk/pen/TouchHelper.java:38-40` (§3.13)                                         |
| `MotionEvent` router                                                                                            | `ScribbleTouchDistributor`                                                                                                                                                                                                                         | `stock/note/note/touch/ScribbleTouchDistributor.java:67-495`                                                                                                                                                 |
| gesture engine                                                                                                  | `RxGesture`, `RxGestureContext`, `RxGestureConfig`                                                                                                                                                                                                 | `rx/RxGesture.java:317-369`, `rx/RxGestureContext.java:292-300`, `rx/RxGestureConfig.java:16-40`                                                                                                             |
| pen-hold detector for stylus `MotionEvent`s                                                                     | `RxPenGesture`                                                                                                                                                                                                                                     | `rx/pen/RxPenGesture.java:13-93`                                                                                                                                                                             |
| single-pointer touch → doc events                                                                               | `ScribbleTouchDetector`                                                                                                                                                                                                                            | `stock/note/note/touch/ScribbleTouchDetector.java`                                                                                                                                                           |
| tap / long-press                                                                                                | `SimpleGestureListener` (via `GestureDetector`)                                                                                                                                                                                                    | `stock/note/note/touch/SimpleGestureListener.java`                                                                                                                                                           |
| `MotionEvent` doc events → active handler                                                                       | `TouchEventHandler`                                                                                                                                                                                                                                | `stock/note/note/eventhandler/TouchEventHandler.java:69-306`                                                                                                                                                 |
| raw pen channel setup                                                                                           | `NoteManager.bindPenHostView` → `TouchHelper.create(view, 3, cb, false)` + `setPostInputEvent(true)`                                                                                                                                               | `stock/sdk/notecore/editor/NoteManager.java:221-255`                                                                                                                                                         |
| raw callbacks → doc events                                                                                      | `NoteManager.getRawInputCallback()`                                                                                                                                                                                                                | `stock/sdk/notecore/editor/NoteManager.java:449-512`                                                                                                                                                         |
| SDK fan-out                                                                                                     | `TouchHelper` (SF + App renders)                                                                                                                                                                                                                   | `stock/sdk/pen/TouchHelper.java:349-366`                                                                                                                                                                     |
| raw reader (Java)                                                                                               | `RawInputReader.onTouchPointReceived`                                                                                                                                                                                                              | `stock/sdk/pen/RawInputReader.java:538-571`                                                                                                                                                                  |
| raw reader (native)                                                                                             | `android::PenManager`, `BtnReader`, `DrawReader`, `EraseReader`, `SideEraseReader`                                                                                                                                                                 | `native/libonyx_pen_touch_reader.so` 0x9d34-0xb3c0                                                                                                                                                           |
| `MotionEvent` pen/finger reader                                                                                 | `AppTouchRender` / `AppTouchInputReader`                                                                                                                                                                                                           | `stock/sdk/pen/touch/AppTouchRender.java`, `stock/sdk/pen/touch/AppTouchInputReader.java`                                                                                                                    |
| resume gate                                                                                                     | `PenEventHandler.A()` (render) / `z()` (input)                                                                                                                                                                                                     | `stock/note/note/eventhandler/PenEventHandler.java:105-107, 271-273`                                                                                                                                         |
| overlay exclude rects                                                                                           | `BaseMenuHandler.addExcludeRect/removeExcludeRect` → `UpdateRawDrawRectAction`                                                                                                                                                                     | `stock/note/note/menu/view/handler/BaseMenuHandler.java:51-96`; `stock/note/note/action/pen/UpdateRawDrawRectAction.java:46-49`                                                                              |
| finger pan / zoom → pen                                                                                         | `ViewportEventHandler`                                                                                                                                                                                                                             | `stock/note/note/eventhandler/ViewportEventHandler.java:78-134`                                                                                                                                              |
| fast mode                                                                                                       | `EpdEventHandler.onApplyFastModeEvent`                                                                                                                                                                                                             | `stock/note/note/eventhandler/EpdEventHandler.java:109-116, 203-218`                                                                                                                                         |

---

## 3. The mechanism, step by step

### 3.1 Wiring at document open

1. `NoteManager.bindPenHostView(view, windowFocused)` creates the SDK helper with feature mask **3**
   (`FEATURE_SF_TOUCH_RENDER | FEATURE_APP_TOUCH_RENDER`) and `touchListenerEnabled = false`:
   `TouchHelper.create(view, 3, getRawInputCallback(), false)`, then `setPostInputEvent(true)`,
   `openRawDrawing()`, and, if the window is focused, `forceSetRawDrawingEnabled(false)`
   (`stock/sdk/notecore/editor/NoteManager.java:231-250`) [code].
2. The private constructor adds `SFTouchRender` first and `AppTouchRender` second, calls
   `setTouchListenerEnabled(false)` on both, then `bindHostView` (`stock/sdk/pen/TouchHelper.java:349-366`) [code].
   Because the listener flag is false, `AppTouchRender.bindHostView` does **not** install its own
   `OnTouchListener` on the view (`stock/sdk/pen/touch/AppTouchRender.java:187-198`) [code]; the app's
   listener (step 4) is the only one.
3. `SFTouchRender.bindHostView` → `RawInputManager.setHostView(view)` sets the reader's host view
   and immediately sends the view's local visible rect as limit rect
   (`stock/sdk/pen/RawInputManager.java:144-150`; `RawInputReader.setLimitRect(Rect)` =
   `nativeSetLimitRegion` + `EpdController.setScreenHandWritingRegionLimit`,
   `stock/sdk/pen/RawInputReader.java:622-628`) [code].
4. `ScribbleFragment.N()` creates `ScribbleTouchDistributor(docBundle).subscribe()` and sets the
   `EditorView` `OnTouchListener` to `g0()`, which hides the soft keyboard on `ACTION_DOWN` and
   forwards to the distributor (`stock/note/note/ui/ScribbleFragment.java:593-605, 806-811`) [code].
5. The distributor always returns `false` from `onTouchEvent`
   (`stock/note/note/touch/ScribbleTouchDistributor.java:398`) [code]. The gesture is nevertheless
   delivered to the view in full because the layout binding calls
   `surfaceView.setOnClickListener(...)` (`FragmentScribbleNormalModeBindingImpl.java:419`) [code],
   which makes the view clickable, so AOSP `View.onTouchEvent` consumes the `DOWN` [inferred from
   AOSP semantics]. The click handler closes open sub-menus (`ScribbleMenuModel.onWorkLayoutClick → F()`,
   `ScribbleMenuModel.java:110-112, 461-463`) [code].
6. `ScribbleTouchDistributor.subscribe()` registers itself on the SDK `TouchEventBus`
   (`TouchHelper.register(this)`) so it receives `PenActiveEvent` / `PenDeactivateEvent`, and
   configures `RxGesture` with move, two-finger move, multi-touch and scale enabled, scale span
   minimum 10 mm, and scale interval `m` (`ScribbleTouchDistributor.java:412-414`) [code].

### 3.2 The raw pen channel (in-process native reader)

**Device.** `TouchReader::findDevice` scans `/dev/input/event%d` and matches device names against the
strings `"Wacom"`, `"hanvon"`, `"onyx_emp"` (string table at 0x6437, 0x649e, 0x63c4, 0x6852;
`findDevice` @0xcd64 calls `openDevice` and `memchr`) [code: strings and call graph; the exact match
logic is inferred]. It therefore opens the EMR pen digitizer only, never the capacitive touch panel:
**finger contacts cannot reach the raw channel** [inferred].

**Event decoding** (`PenManager::onEvent` @0xa000, `onAbsEvent` @0xa11c, `onKeyEvent` @0xa358) [code]:

| evdev                      | stored at        | meaning                                                                                        |
| -------------------------- | ---------------- | ---------------------------------------------------------------------------------------------- |
| `ABS_X` (0), `ABS_Y` (1)   | `+0x34`, `+0x38` | position                                                                                       |
| `ABS_PRESSURE` (0x18)      | `+0x3c`          | pressure (0 while hovering)                                                                    |
| `ABS_TILT_X/Y` (0x1a/0x1b) | `+0x44`, `+0x48` | tilt                                                                                           |
| `ABS_DISTANCE` (0x19)      | not stored       | hover distance is ignored                                                                      |
| `BTN_TOOL_RUBBER` (0x141)  | `+0x2070`        | eraser end in proximity                                                                        |
| `BTN_TOOL_BRUSH` (0x142)   | `+0x206f`        | brush tool                                                                                     |
| `BTN_TOUCH` (0x14a)        | `+0x206d`        | tip or rubber in contact                                                                       |
| `BTN_STYLUS` (0x14b)       | `+0x206e`        | side button held                                                                               |
| key codes 0x152 / 0x153    | `+0x2074`        | vendor pen-model codes; 0x153 selects a different pressure curve (`normalizePressure` @0xa428) |

**Reader state machine** (`PenManager::init` @0x9da8 builds four readers: index 0 `BtnReader`,
1 `DrawReader`, 2 `EraseReader`, 3 `SideEraseReader`; active index at `+0x28`, initially 0) [code]:

- `BtnReader::onSynEvent` @0xb0e8 (idle): if pressure > 0 then
  rubber down (`BTN_TOOL_RUBBER && BTN_TOUCH`, `isRubberDown` @0xaf68) → `EraseReader` (2);
  else side-touch down (`sideBtnEraseEnable && BTN_STYLUS && BTN_TOUCH`, `isSideTouchDown` @0xaf84) → `SideEraseReader` (3);
  else `BTN_TOUCH` → `DrawReader` (1) [code].
- `EraseReader::onKeyEvent` @0xb1b4 returns to idle when the rubber is no longer down;
  `DrawReader` @0xb310 and `SideEraseReader` @0xb260 return to idle on `BTN_TOUCH` up [code].
  So releasing the side button mid-stroke does not end a side-erase stroke; only lifting does.
- `sideBtnEraseEnable` (`+0x206c`) is initialised to **1** in the constructor @0x9d90 and only
  changed by `enableSideBtnErase` @0xaf50 [code].
- `isErasing()` = active index 2 or 3; "shortcut erasing" = index 3 (`isErasing` @0xa4f4,
  `isShortcutErasing` @0xa508) [code].

**Reports.** Every sample goes out through `TouchReader::report(x, y, pressure, tiltX, tiltY, ts,
erasing, sideErase, state)` → `TouchConsumer::onTouchEvent` @0xb4d4 → `EventQueue::push`, which
inserts a constant `false` as the middle boolean (@0xb4ec `mov w6, wzr`) → consumer thread
`threadLoop` @0xb70c → JNI `onTouchPointReceived(FFIIIZZZIJ)` [code]. The public SDK names the
parameters `(x, y, pressure, tx, ty, isErasing, shortcutDrawing, shortcutErasing, state, ts)`
(`pub/RawInputReader.java:427`) [code]. Hence **`shortcutDrawing` is always false** and
`shortcutErasing` is true only for the side button [inferred from the two disassembled call sites].

| state | produced by                                                | meaning                                                                |
| ----- | ---------------------------------------------------------- | ---------------------------------------------------------------------- |
| 0     | `moveTo` @0xa5fc from `BaseReader::onEvent` @0xb03c        | pen down / re-entry into a valid region                                |
| 1     | `quadTo` @0xa63c                                           | move inside the valid region                                           |
| 2     | `finishQuadTo` @0xa694 on reader deactivation              | pen up                                                                 |
| 3     | `quadOutRegion` @0xa668                                    | stroke left the valid region (outside limit or inside an exclude rect) |
| 5     | `onSyncEvent` @0xa178, **after every `SYN`**               | "pen present" (hover or contact)                                       |
| 6     | `pausePen` @0xa8a0 when the last reported state was 0 or 1 | forced release                                                         |

`BaseReader::onEvent` tests `inValidRegion()` before every point; leaving the region reports 3 and
re-entering reports 0, so **a stroke that crosses an exclude rect is split into two strokes**
[code]. The native reader keeps its own limit/exclude arrays (`setLimitRegion` @0xaaa4 clears only
the limit array and the last-region latch, never the exclude array) [code].

### 3.3 The raw channel in Java

`RawInputReader.onTouchPointReceived` (`stock/sdk/pen/RawInputReader.java:538-571`) [code]:

- **state 5** → `H()` (`:193-198`) → `K()` (`:214-223`): if `setPostInputEvent(true)` was called
  (NoteManager does), post `PenActiveEvent` on the first sample and (re)arm a 100 ms timer whose expiry
  posts `PenDeactivateEvent` (`:88-102`). Then `q()` calls `onPenActive` only when the reader is
  resumed. **This runs even while the raw reader is paused**, because the state-5 branch is outside
  the `reportData` gate [code, Java level]. Whether the native reader keeps emitting state 5 after
  `nativePausePen()` was not traced in the disassembly [unknown]. `H()` maps the point but applies
  no limit/exclude test, and the native state-5 report in `onSyncEvent` @0xa178 is issued outside
  `BaseReader::onEvent`'s `inValidRegion()` test, so hover **anywhere** on the digitizer posts
  `PenActiveEvent` [inferred]. `PenActiveEvent`/`PenDeactivateEvent` go to the process-wide SDK
  `TouchEventBus` (`RawInputReader.J()` → `getEventBusHolder()`, `:207-212, 511-513`), so every
  registered distributor (canvas and full-screen float menu) receives them [code]. `NoteManager`'s callback does not override `onPenActive`
  (`stock/sdk/pen/RawInputCallback.java:14-15`, `NoteManager.java:449-512`) [code].
- **states 0-3** are processed only when `f4564d` (`reportData`) is true, i.e. after
  `RawInputReader.resume()` (`:597-602`); `pause()` (`:573-577`) calls `nativePausePen()` and clears
  the flag. States 0-3 during a pause are dropped [code].
- **state 6** (forced release on pause) is always processed and ends the stroke with
  `onEndRawDrawing/Erasing(..)` (`:564-569`) [code].
- Begin/move/end are dispatched as erasing or drawing by the `isErasing` flag (`s()` `:434-445`,
  `v()` `:472-482`, `t()` `:447-457`) [code].

`SFTouchRender.b` wraps every callback in `hostView.post(...)` (main thread) (`SFTouchRender.java:14-210`
of the jadx file) [code]; `NoteManager`'s callback posts note-bus events:
`BeginRawDrawEvent(shortcutDrawing)`, `BeginRawErasingEvent(shortcutErasing)`,
`EndRawDrawingEvent(outLimitRegion)`, `EndRawErasingEvent(outLimitRegion)`,
`RawDrawingPointsReceivedEvent`, `RawDrawingPointsMoveReceivedEvent`, `RawErasingPointsReceived`,
`RawErasingPointMoveEvent`, `PenUpRefreshEvent` (`NoteManager.java:449-512`) [code].
`TouchEventHandler` delivers them to the active handler; for a handler with
`usePenTipErase() == true` (only `EraseHandler`) raw **drawing** events are delivered as erasing
(`TouchEventHandler.java:136-157, 175-190`; `stock/note/note/handler/scribble/EraseHandler.java:128-130`) [code].

### 3.4 The `MotionEvent` channel

`ScribbleTouchDistributor.onTouchEvent(view, event)` (`ScribbleTouchDistributor.java:368-399`) [code]:

1. Returns immediately unless `noteInfo.isBindPenHostView()` and the activity has **window focus**
   (logs "filter touch for activity has not focus").
2. `TouchStateInfo.onTouchEvent` tracks down/up.
3. If the active handler `supportZoom() || supportScroll()` → `RxGesture.onTouchEvent(event)`.
4. If `supportFingerDraw()` **and** the scene `canFingerTouch()` → `NoteManager.onTouchEvent(event)`
   → `TouchHelper.onTouchEvent` → `AppTouchRender.onTouchEvent` → `AppTouchInputReader.processMotionEvent`.
5. If the active provider is the pen provider and the event is a stylus (tool type 2) →
   `RxPenGesture.onTouchEvent` (pen-hold detection).
6. Unless the floating menu is being slid (`TouchStateInfo.isFloatSliding`) → `PageGestureDetector`.
7. `ScribbleTouchDetector` (single-pointer only) posts `TouchDownEvent/TouchMoveEvent/TouchUpEvent`
   with the raw `MotionEvent` (`stock/note/note/touch/ScribbleTouchDetector.java`,
   `BaseTouchDetector.a()` = pointer count > 1) [code].
8. `GestureDetector(SimpleGestureListener)` posts `SingleTapUpEvent` (throttled to one per 500 ms)
   and `LongPressEvent` (`SimpleGestureListener.java`) [code].

Steps 5-8 are **not** behind the "pen active" filter: only `RxGesture` (step 3) applies it, so
`ScribbleTouchDetector` and the tap/long-press detector still see finger events while the pen
hovers [code].

Two further consumers of the single-pointer `TouchDownEvent` (posted by `ScribbleTouchDetector`
for finger **and** stylus/eraser `MotionEvent`s) matter for the pen session [code]:

- `PenEventHandler.onTouchDownEvent` (`PenEventHandler.java:517-524`): if the status-bar flag `e`
  is set (set by `StatusBarChangeEvent`, `:490-500`), it resets the flag and calls
  `t(this,false,false,0,7,null)` = `s(true, true, DELAY_ENABLE_RAW_DRAWING_MILLS)` → render+input
  resume through the gates after 500 ms on colour → **pen state 2**.
- `AudioMenuModel.onTouchDownEvent` (`stock/note/note/menu/viewmodel/scribble/AudioMenuModel.java:448`)
  [not traced further].

The distributor's own `RxGesture` listener additionally posts the **SDK** `TouchDownEvent` /
`TouchUpEvent` (`com.onyx.android.sdk.notecore.editor.events.app.*`, a different class from the
app's `note.event.touch.*` ones) from `onTouchDown` / `onTouchUp`
(`ScribbleTouchDistributor.java:462, 482`); their subscribers are `ScaleGestureHandlerAction`
(`:606-616`, scale bookkeeping) and nothing pen-related [code: grep of subscribers].

Stylus `MotionEvent`s also feed `RxPenGesture` (step 5) whenever the pen provider is active; it
posts `PenStatusChangeEvent` (`rx/pen/RxPenGesture.java:59-64`), consumed by
`HWRContextLessEventHandler.onPenStatusChangeEvent` → `onPenHolding` → `StartRecognizeDiagramAction`
when shape-by-hold recognition is on (`stock/note/note/eventhandler/HWRContextLessEventHandler.java:118-150`)
[code]. The pen-session effect of `StartRecognizeDiagramAction` belongs to the writing/HWR
subsystem and was not traced here [unknown].

**Which handlers accept which pointer.** `TouchEventHandler` [code]:

- ignores events in any touch-exclude rect or outside the scribble rect
  (`g()`, `TouchEventHandler.java:104-116`, using `EditorBundlesKt.getTouchExcludeRectList`);
- drops **finger** events when `handler.filterFingerTouch()`; for scribble handlers that is
  `!scene.canFingerTouch()` (`ScribbleHandler.java:822-824`), i.e. the user's "disable finger touch";
- otherwise calls `handler.onTouchDown/Move/Up/onSingleTapUp/onLongPress` with the `MotionEvent`.
  Stylus `MotionEvent`s pass these filters for down/move/up/tap, but **long-press is finger-only**:
  `onLongPressEvent` returns unless `TouchUtils.isFingerTouch(event)` (tool type 1 or 0) and
  `!filterFingerTouch()` (`TouchEventHandler.java:205-216`) [code, corrected in verification].
- `TouchUtils` tests the tool type of the pointer at `getActionIndex()`, i.e. pointer 0 for
  `MOVE` events (`stock/sdk/utils/TouchUtils.java:7-9`) [code].

What the pen handler does with the stylus `MotionEvent` copy: `EpdShapeHandler.onTouchDown/Move/Up`
only feed `addHWRTouchPoint` and, on a stylus `UP` with no raw point seen while render is enabled,
log "raw point is not report" (`stock/note/note/handler/common/EpdShapeHandler.java:453-487`);
`ScribbleHandler.onTouchUp` posts `AddShapeOnInvisibleLayerEvent` for pen tool types on a hidden
layer; `ScribbleHandler.onSingleTapUp` ignores multi-touch and pen tool types
(`ScribbleHandler.java:1181-1207`) [code]. No handler erases or draws from `MotionEvent`s outside
selection mode [inferred from the handler set in §4 table 4.4].

**`AppTouchRender` / `AppTouchInputReader`** (the SDK's own `MotionEvent` stroke reader) [code]:

- processes events only while its input flag is set by `setRawInputReaderEnable(true)`
  (`AppTouchRender.java:240-245, 299-301`);
- skip rule `f()` (`AppTouchInputReader.java:83-88`): with `enableFingerTouch` = true (default) and
  `onlyEnableFingerTouch` = false (default) **nothing is skipped**; with `enableFingerTouch(false)`
  every non-pen event is skipped; with `onlyEnableFingerTouch(true)` every pen event is skipped;
- limit/exclude test with half the stroke width as margin (`:43-80`);
- renders its own stroke with `EpdController.moveTo/quadTo(..., UpdateMode.DU)` and `penUp()`
  only while render is enabled and only for non-pen tool types (`onInterceptTouchEvent` =
  `isPenTouchType`, `AppTouchRender.java:122-180, 235-237`).
  In stock Notes this path is reached only in lasso selection mode (step 4 above) [inferred].

### 3.5 Gesture engine behaviour relevant to routing

`RxGesture.onTouchEvent` (`rx/RxGesture.java:317-364`) [code]:

1. `filterOnHover` (default on, `RxGestureConfig.java:18,130-132`) drops events while
   `context.isHover()`. `isHover` is set only by `RxGesture.onHover()`, which stock never calls (no
   hover listener anywhere in `com.onyx`; only `rx/GestureTestActivity.java:150`) → **no effect**
   [code].
2. Pointer count > 2 is dropped (except `CANCEL`).
3. `RxGestureContext.isDisabledPenTouch(event)` (`rx/RxGestureContext.java:292-300`): with
   `enabledPenTouch = false` (default, never set by stock) it is true when **pen active** or when the
   event itself is a pen tool type. In that case the event is dropped **except** that for finger
   `DOWN/UP/CANCEL` `v()` runs: `DOWN` only resets the context (the `onTouchDown` listener is **not**
   called, so `ScribbleTouchDistributor`'s `onTouchDown` — `stopScroll`, SDK `TouchDownEvent`,
   `consumeSingleTap` — does not run), `UP`/`CANCEL` store the point and call
   `onTouchUp`/`onTouchCancel`; for all three `v()` then also feeds the scale/move detectors
   (`b()`) and the derived gesture streams (`z()`) with that single event
   (`RxGesture.java:188-200, 330-337`) [code, refined in verification]. Consequences:
   - stylus `MotionEvent`s never pan or zoom;
   - while the raw reader reports the pen in range (+100 ms), finger moves, pinches and swipes are
     ignored; a finger lift still runs `ScribbleTouchDistributor.i()` (fling/turn-page evaluation,
     §3.7) [inferred: the lift path runs but its gesture context has no movement].
4. `largeAreaDetect` (default on, threshold `getSize() > 0.8`, `RxGestureConfig.java:17,30`) only
   suppresses the derived gesture streams (`z()`); the listener callbacks and the scale/move detectors
   (`b()`) have already run for that event (`RxGesture.java:338-363`) [code]. It therefore does not
   stop pinch or two-finger pan: `onScale*` come from `RxScaleGesture` via the scale detector and
   `onTwoFingerMoving` from `RxMoveProGesture` via `RxMoveGestureDetector`, both fed in `b()`
   (`RxGesture.java:86-103, 154-164`; `rx/RxMoveProGesture.java:16`) [code, upgraded from inferred].

`ScribbleTouchDistributor`'s listener (`:414-492`) [code]:

- `onTouchDown`: stop scroll, reset scale flag, arm the render-view time guard
  (`UPDATE_MENU_INTERVAL_TIME_MS`), post `TouchDownEvent(TouchPoint)`, `consumeSingleTap()`,
  record start point, compute edge indicator.
- `onTouchMove` → `c()` (`:172-213`): decides between scroll and scale from the first two pointers:
  both moved ≥ 5 mm and intersection angle ≤ 90° (and ≠ 0) → `enableScroll`; otherwise
  `enableScale = !scene.isDisablePageZoom()`. Single-finger scroll is enabled only when the scene
  supports it (continuous page mode with the preference on, not in selection) and speed ≥ 4 cm/s.
- `onTwoFingerMoving` → `j()` (`:288-326`): `ScrollBeginEvent` / `ScrollingEvent` with direction
  locking (20° threshold, `d()` `:215-235`).
- `onScaleBegin/onScaling/onScaleEnd` → `ScaleBeginEvent/ScalingEvent/ScaleEndEvent` when
  `supportZoom && isZoomEnabled (always true, NoteMMKVHelper.java:1836-1838) && enableScale`.
- `onTouchUp/onTouchCancel` → `i()` (`:275-285`): fling if a scroll was running; single-finger
  `onSingleFling` (page turn, §3.7) when not float-sliding and there was no second pointer; reset state.

### 3.6 Finger pan and pinch zoom: effect on the pen session

Pinch (`ScaleGestureHandlerAction`, `stock/note/note/gesture/ScaleGestureHandlerAction.java`) [code]:

1. `ScaleBeginEvent` → `beforeRender` (cancel pre-render) → action 0 → `ViewportScaleBeginEvent`
   (`:569-575`, `:170-183`).
2. `ViewportEventHandler.onViewportScaleBeginEvent` (`ViewportEventHandler.java:78-84`):
   show zoom preview popup, `ZoomThumbnailAction`, **`ApplyFastModeEvent(true)`** and
   **`PauseResumeRawPenAction`**.
   - `ApplyFastModeEvent(true)` → `EpdEventHandler.a()`: dither threshold 160 (colour) / 255,
     `EpdController.setEpdTurbo(deviceConfig.epdTurbo)`,
     **`EpdController.applyTransientUpdate(transformUpdateModel ?: ANIMATION_QUALITY)`**
     (`EpdEventHandler.java:80-82, 109-116`) [code].
   - `PauseResumeRawPenAction` = `PauseRawPenAction(pauseArgs)` → `TouchHelper.setRawDrawingEnabled(false)`
     (render off → **pen state 3**, `ENABLE_POST(1)`; input off → native pause, forced release;
     `resetPenDefaultRawDrawing` = brush raw drawing on, eraser raw drawing off, style 5), then
     `ResumeRawPenAction(resumeArgs)` → `PenEvent(true, true, 500 ms on colour)` → `PenEventHandler`
     gates → `ResumeRawDrawingRequest` sleeps 500 ms, re-pushes limit/exclude/region mode/stroke
     parameters, input on, render on → **pen state 2**
     (`stock/note/note/action/pen/PauseResumeRawPenAction.java:25-78`,
     `PauseRawPenAction.java:47-58`, `ResumeRawPenAction.java:44-48`,
     `stock/sdk/notecore/editor/data/RawPenArgs.java:53-70`,
     `stock/note/note/request/pen/ResumeRawDrawingRequest.java:107-138`) [code].
3. Each `ScalingEvent` → `renderManager.displayWithMatrix(matrix)` and
   **`UpdateRawPenWidthAction(averageScale × viewportScale)`** (live stroke width follows zoom)
   (`ScaleGestureHandlerAction.java:423-440`) [code].
4. On finger up: `ViewportScaleEndEvent` → `UpdateRawDrawRectAction` (limit + exclude rects re-pushed,
   no pen-state change) (`ViewportEventHandler.java:87-90`); scale end → `UpdateRawPenWidthAction(0)`,
   `ViewportScaleFinish` → **`ApplyFastModeEvent(false)`** (`ViewportEventHandler.java:93-98`;
   `ScaleGestureHandlerAction.java:285-308`) [code].
5. `ApplyFastModeEvent(false)` with fast mode on → `k()` (skipped entirely when the active handler
   `renderInFastMode()`, `EpdEventHandler.java:185-190`; the 5 s callback `e()` also returns without
   leaving fast mode while `TouchStateInfo.penRawTouch` is set, `:145-154`) [code, added in verification]: reset the firmware refresh counter and start
   a 5 s timer (with wakelock) that later calls `clearTransientUpdate` and, if a raw pen-up happened
   during fast mode, another `PauseResumeRawPenAction`; then it posts
   **`PenEvent.resumeRawDrawingImmediately()`** (render+input, 0 ms) (`EpdEventHandler.java:118-129,
145-154, 185-190, 203-218`; `stock/note/note/event/PenEvent.java:40-42`) [code].

Nothing in this chain checks whether fingers are still down before the delayed resume of step 2 runs
(the gates in `PenEventHandler.A()/z()` do not include a scaling flag) [code]. So during a pinch that
lasts longer than ~500 ms the session is expected to return to pen state 2 while the fingers are
still zooming [inferred — see experiment E3].

Two-finger pan (and single-finger scroll in continuous mode) follows the same shape:
`ScrollBeginEvent` → `ScrollGestureHandlerAction.onScrollBeginEvent` → `ViewportScrollBeginEvent`
**only if `ScrollGesturesKt.canScroll`** (`stock/note/note/gesture/ScrollGestureHandlerAction.java:81-85, 460-474`)
→ fast mode on + `PauseResumeRawPenAction` (`ViewportEventHandler.java:112-118`); the end
(`FlingAction` posts `ViewportScrollEndEvent`, `FlingAction.java:303`) → re-collect viewport,
`ApplyFastModeEvent(false)` (→ immediate resume) and `UpdateRawDrawRectAction`
(`ViewportEventHandler.java:121-134`) [code]. A two-finger drag on a page that cannot scroll posts no
`ViewportScrollBeginEvent` and therefore does not touch the pen session [inferred].

### 3.7 Finger page turn

Single-finger swipe: `i()` → `scene.onSingleFling` → for the scribble scene (not continuous mode)
`FlingTurnPageAction` (`stock/note/note/scene/NoteScribbleScene.java:417-425`) [code]. The swipe
turns the page only if all hold (`FlingTurnPageAction.java:85-118`) [code]: not cancelled; moved;
not continuous page mode; `FINGER_TURN_PAGE` mode matches the pointer count (1 = single finger,
default 1; 0 = disabled, `NoteMMKVHelper.java:854-856, 1107-1109`); speed ≥ 4 cm/s; duration
≤ 1000 ms; not multi-touch; down point not inside a touch-exclude rect; angle ≤ 30°; distance
≥ 10 mm (`PageGestureDetector.detectDirection`); plus `PageGestureDetector.Companion.a()`: the down
point is at least the border threshold away from the left/right edges and below the status bar
(`PageGestureDetector.java:57-99`). It then posts `PrevPageGestureEvent`/`NextPageGestureEvent` →
`ScribbleHandler.onPrevGesture/onNextGesture` → page change (or add page on the last page)
(`ScribbleHandler.java:1032-1131`) [code]. The pen-session effects of the page change belong to the
page/refresh subsystem.

`PageGestureDetector.onTouchEvent` itself returns early whenever `handler.supportZoom()`
(`PageGestureDetector.java:125-129`); every pen-mode handler returns `true` there
(`EpdShapeHandler.java:493`, `EraseHandler.java:113`), so in pen mode the detector is inert and page
turning comes only from the fling path above [code].

Because the finger events are filtered while "pen active", a swipe started less than ~100 ms after
the pen left hover range is not recognised [inferred].

### 3.8 The "disable finger touch" setting

- Stored in MMKV `FINGER_TOUCH_ENABLE_KEY`, default **true** (`NoteMMKVHelper.java:1747-1749`);
  rich-text scene has its own key (`:1171-1173`) [code].
- Toggled from the toolbar (`ToolbarMenuModel.onToggleTouch` → `handlerManager.onPause()` +
  `ToggleFingerTouchAction`, `stock/note/note/menu/viewmodel/scribble/ToolbarMenuModel.java:1237-1240`) or
  the touch-control menu (`TouchControlMenuProvider.java:78`) [code].
- `ToggleFingerTouchAction.create()` = `InvalidateScreenAction.setResumeRawDrawing(false)` (pause
  render+input, invalidate, resume input only) → flip the preference unless the **system** CTP
  disable region is active (`EpdController.isCTPDisableRegion` → toast) → `resumePen()`
  (`stock/note/note/action/ui/ToggleFingerTouchAction.java:49-88`) [code].
- Effect: only `TouchEventHandler` finger filtering (§3.4), finger drawing in selection mode and the
  text box (`TextInputHelper.java:665`). It does **not** switch off pinch zoom, pan or finger page
  turning (those are separate preferences: `isDisablePageZoom`, `FINGER_TURN_PAGE`,
  `isDisabledPageTurn`, `isDisabledSideGesture`, `isDisabledBottomGesture`,
  `NoteMMKVHelper.java:1719-1736`), and it makes **no SDK or firmware call**: no
  `TouchHelper.enableFingerTouch`, no `setAppCTPDisableRegion` [code].

### 3.9 Palm handling

- `PalmDetector` (touch-major threshold from `DeviceConfig.largeObjectSize`,
  `stock/note/note/touch/PalmDetector.java`) has **no callers** [code: grep over `com/onyx`].
- `TouchAreaIgnoreAction.disableFullScreenCTP()` / `resetCTPDisableRegion()` (framework
  `InputManager.setAppCTPDisableRegion`, native `setTouchAreaIgnoreRegion`,
  `fws/com/android/server/input/InputManagerService.java:2424-2432`) have **no callers**; only
  `dumpCTPInfo()` is called from `ScribbleFragment.onDestroy` (`ScribbleFragment.java:1491`) [code].
- The only effective palm measures in the app are (a) the "pen active" finger filter of §3.5, and
  (b) the pointer-count > 2 filter; the large-area filter is largely ineffective (§3.5) [code/inferred].
- Any rejection in the touch-panel firmware, the kernel driver or a patched `libinputflinger` is
  outside the extracted artefacts [unknown].

### 3.10 Toolbars, floating menus and overlays inside the drawing area

**Geometry.** The limit rect is the scribble rect = `(0,0,EditorView.width,EditorView.height)`
(`ScribbleFragment.java:776-777`; `EditorBundlesKt.getTouchLimitRectList`), translated to screen
coordinates by the framework (`fw/android/onyx/ViewUpdateHelper.java:1219-1232`) [code]. The docked
toolbars sit in `func_layout`, above `work_layout` which holds the `EditorView`
(`res/layout/fragment_scribble_normal_mode.xml:14-19`), so they are outside the limit [inferred from
layout]. A stylus tap on a docked toolbar is therefore ink-free (SF and native reader report "out of
region") and is handled as an ordinary `MotionEvent` click by the toolbar view [inferred].

**Exclude slots** (`stock/sdk/notecore/editor/data/ExcludeRectType.java:9-21`) [code]:

| slot  | overlay                                        | registered by                                                                                |
| ----- | ---------------------------------------------- | -------------------------------------------------------------------------------------------- |
| 1     | floating audio menu                            | `FloatAudioMenuHandler.java:42, 63`                                                          |
| 2     | zoom preview / zoom-without-preview menu       | `ZoomPreviewContainerHandler.java:49, 77, 225`; `ZoomWithoutPreviewMenuHandler.java:99, 121` |
| 3     | floating tool menu (float toolbar)             | `FloatToolMenuHandler.java:422-436, 879`                                                     |
| 4     | link-note jump menu                            | `LinkNoteJumpMenuHandler.java:103, 124`                                                      |
| 5 / 6 | horizontal / vertical canvas scroll bar thumbs | `CanvasSeekbarMenuHandler.java:109-122, 340-341`                                             |
| 7     | page navigator menu                            | `NotePageNavigatorMenuHandler.java:90, 111`                                                  |
| 8     | zoom tool menu                                 | `CanvasSeekbarMenuHandler.java:127-129, 342`                                                 |
| 9     | clipboard action menu                          | `ClipboardActionMenuHandler.java:158, 179`                                                   |
| 10    | compact float-menu icon                        | `ScribbleMenuModel.java:118-124, 230`                                                        |

`BaseMenuHandler.addExcludeRect/removeExcludeRect(type, rect, updateRawDrawRect = true)` stores the
rect in `NoteDocViewInfo` and runs `UpdateRawDrawRectAction` (`BaseMenuHandler.java:51-96`) [code],
which on the pen scheduler calls `NoteManager.setDrawLimitRect(scribbleRect)` and
`setDrawExcludeRect(getTouchExcludeRectList())` (`UpdateRawDrawRectAction.java:46-49`) [code].
`getTouchExcludeRectList` = all registered slot rects **plus**, for handlers with
`touchInVisibleRect()`, the invisible parts of the scribble rect
(`stock/sdk/notecore/editor/extension/EditorBundlesKt.java:315-338`) [code]. The SDK fan-out:

- `SFTouchRender.setExcludeRect` → `RawInputReader.setExcludeRect` → `nativeSetExcludeRegion` +
  `EpdController.setScreenHandWritingRegionExclude(view, rects)` — **but only if the list is
  non-empty** (`RawInputReader.java:604-611`) [code];
- `AppTouchRender.setExcludeRect` → `AppTouchInputReader.setExcludeRectList` → returns only on
  `null`, so an **empty** list is still sent as `setScreenHandWritingRegionExclude(view, Rect[0])`
  (`AppTouchInputReader.java:239-249`) → `SDMDevice` converts to `int[0]`
  (`stock/sdk/device/SDMDevice.java:2593-2608`) → `ViewUpdateHelper` sends
  `SET_SCREEN_HANDWRITING_REGION_EXCLUDE` with view flag 1 and an empty array
  (`fw/android/onyx/ViewUpdateHelper.java:1202-1216`) [code]. doc08 states SF treats `n <= 0` as
  "clear" [inferred, doc08].

- the limit list is likewise pushed twice: `AppTouchRender.setLimitRect` → `AppTouchInputReader.setLimitRectList`
  sends `setScreenHandWritingRegionLimit(view, rects)` after the SF half has sent its own
  (`stock/sdk/pen/touch/AppTouchRender.java:303-306`; `AppTouchInputReader.java:251-254`) [code;
  added in verification]. Because the SF render is added first and the App render second
  (`TouchHelper.java:355-360`), the App-render push is the last one SF receives [code].

Net effect: the SF exclude table follows the overlays, including going empty; the in-process native
reader's exclude table is **never cleared** by an empty list and keeps the last non-empty set until
the next non-empty push or a reader restart [code + inferred]. (Verified: `PenManager::setExcludeRegion`
@0xac44 clears and refills the array, and the only other clear is `PenManager::clear` @0x9ecc,
reached from `TouchReader::~TouchReader` @0xc5d8, i.e. a reader restart; `clearExcludeArray` @0xa90c
has no direct caller in the disassembly.) None of these calls changes the pen
state; the overlay appears/disappears with the SF session left in state 2 [code].

**Pausing overlays.** Some overlay interactions do pause the session:

| interaction                                                                                                                                        | calls                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | SF effect                                                         | source                                                                                                                                                                                   |
| -------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| press on a floating-toolbar item (pen **or** finger, no tool-type filter)                                                                          | `EpdController.enableColorCU()`; `FloatMenuPressingEvent(true)`; `RawPenPauseEvent(pauseArgs)` → `setRawDrawingEnabled(false)`                                                                                                                                                                                                                                                                                                                                                                                           | pen state 3, `ENABLE_POST(1)`, `ENABLE_COLOR_CU(1)`               | `stock/note/note/menu/FloatDraggableMenuBar.java:80-84` [code]                                                                                                                           |
| release of that item                                                                                                                               | `EnableColorCUEvent(false)`, `FloatMenuPressingEvent(false)`, `MenuTouchEvent`; **no resume**                                                                                                                                                                                                                                                                                                                                                                                                                            | none by itself; the resume gate stays blocked only while pressing | `FloatDraggableMenuBar.java:56-66, 113-115` [code]                                                                                                                                       |
| cancel of that item                                                                                                                                | `PenEvent.resumeRawDrawing()`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | back to state 2 after 500 ms if gates allow                       | `FloatDraggableMenuBar.java:69-72` [code]                                                                                                                                                |
| drag of a floating-toolbar item                                                                                                                    | `ApplyFastModeEvent(true, force)` / `(false)`                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | transient update on/off                                           | `FloatDraggableMenuBar.java:87-97` [code]                                                                                                                                                |
| long-press drag of the docked toolbar into a float menu (`ToolbarGestureDetector` → `TransformDragFloatMenuEvent`)                                 | `ApplyFastModeEvent(true)` + `PausePenEvent` while dragging, `ResumePenEvent` on up                                                                                                                                                                                                                                                                                                                                                                                                                                      | state 3 during drag                                               | `stock/note/note/touch/ToolbarGestureDetector.java:151-216`; `FloatToolMenuHandler.java:887-901` [code]                                                                                  |
| entering float-menu move mode                                                                                                                      | fast mode + `PausePenEvent`; leaving: `ResumePenEvent(500 ms)`                                                                                                                                                                                                                                                                                                                                                                                                                                                           | state 3 during move                                               | `stock/note/note/menu/viewmodel/scribble/FloatMenuModel.java:586-598` [code]                                                                                                             |
| drag of a `FloatMenuLayout` / zoom preview (`DragGestureListener`, pen and finger alike: `filterPenType` is disabled because `a()` returns `true`) | `onDown` → `RefreshScreenEvent` → `InvalidateScreenAction` (pause, invalidate, resume 500 ms); scroll → fast mode on; up → fast mode off                                                                                                                                                                                                                                                                                                                                                                                 | 3 → 2 cycle                                                       | `stock/note/note/touch/DragGestureListener.java:32-34, 56-62, 101-146`; `CommonEventHandler.java:158-161` [code]                                                                         |
| touch inside the zoom-preview overlay while it is locked (`ZoomPreviewLayout.onTouchEvent`, any tool type)                                         | `DOWN` → `PenPauseEvent(pauseArgs)` → `PauseRawPenAction`; `UP/CANCEL` → `PenResumeEvent(resumeArgs)` → `ResumeRawPenAction` (500 ms on colour); the click detector's `onDown` also calls `setPreviewPopupMoveEnabled(true)` → `ZoomPreviewPopMoveEnableEvent(true)` → `PenEventHandler.onPausePenEvent(null)` (sets the pause gate), released by `setPreviewPopupMoveEnabled(false)` on the up of a move → `onResumePenEvent(null)` (clears **all** `PausePenEvent` owners, then runs a full `PauseResumeRawPenAction`) | state 3 while pressed; an extra 2 → 3 → 2 cycle on the move-up    | `stock/note/note/menu/view/ZoomPreviewLayout.java:54-64, 101-118, 177-221`; `ZoomMenuModel.java:682-685`; `PenEventHandler.java:405-433, 461-481, 526-534` [code; added in verification] |
| touch on a float-main-menu item in fixed mode (items with the `a` motion-event consumer)                                                           | `DOWN` → `PenPauseEvent(pauseArgs)`; `UP/CANCEL` → `InvalidateViewWithPenControlEvent(resumeArgs, delay 0)`                                                                                                                                                                                                                                                                                                                                                                                                              | 3 during press                                                    | `stock/note/note/menu/FloatMainMenuFixedModeProvider.java:87-100, 240` [code; added in verification; which items carry the consumer not traced]                                          |
| any `BasePopWindow` / popup shown/dismissed                                                                                                        | ref-counted `PopupWindowChangeEvent` → `PauseRawPenAction(pauseArgs)`; on last dismiss resume after `POPUP_RESUME_PEN_TIME_MS`                                                                                                                                                                                                                                                                                                                                                                                           | state 3 while any popup is open                                   | `stock/sdk/kui/dialog/BasePopWindow.java:273`; `PenEventHandler.java:436-446` [code]                                                                                                     |
| ~~edge swipe that toggles toolbar visibility (`SlideToolbarVisibilityToggleDetector`)~~                                                            | the class exists (`InvalidateScreenAction` ×2, `ToolMenuVisibilityToggleEvent`) but is **never instantiated**: a grep of the whole decompiled APK finds it only in its own file                                                                                                                                                                                                                                                                                                                                          | none                                                              | `stock/note/note/touch/SlideToolbarVisibilityToggleDetector.java` [code: grep; corrected in verification]                                                                                |
| canvas touch-down while the status-bar flag is set                                                                                                 | `PenEventHandler.onTouchDownEvent` → resume                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | 3 → 2 after 500 ms                                                | `PenEventHandler.java:517-524` [code; added in verification]                                                                                                                             |
| tap on the canvas                                                                                                                                  | `onWorkLayoutClick` → close sub-menus (→ popup dismiss → resume)                                                                                                                                                                                                                                                                                                                                                                                                                                                         | only via the popup counter                                        | `ScribbleMenuModel.java:110-112, 461-463` [code]                                                                                                                                         |

The resume gate blocks render resume while (`PenEventHandler.A()`, `:105-107`) [code]: HWR hold
conversion, keyboard shown, popup count > 0, status bar shown, toast shown, no-focus system dialog,
window not focused, system float button active, screenshot, scene not scribble, host view not bound,
`!handler.canPenRawRender()`, current layer disabled, pause set (`PausePenEvent` owners),
document opening, render dialog, **float menu pressed**. Input resume (`z()`, `:271-273`) has the same
list with `isUsePenInput()` instead of `canPenRawRender()`.

### 3.11 Side button and eraser end

- **Eraser end (rubber).** Native `EraseReader` (§3.2) → Java `isErasing=true`,
  `shortcutErasing=false` → `onBeginRawErasing(false, p)` → `BeginRawErasingEvent` →
  `TouchEventHandler.beginRawErasing` → `handler.onBeginRawErasing` (`TouchEventHandler.java:159-173`)
  [code]. For the pen handler that is `ScribbleHandler.onBeginRawErasing` → `RawPenDownEvent` +
  erase begin (`ScribbleHandler.java:931-936, 239-250`) [code]. The eraser end therefore works in any
  scribble handler **without a tool switch and without any input-routing pause**; only if the raw
  input reader is paused at that moment is the rubber stroke dropped (states 0-3 are gated, §3.3)
  [code + inferred].
- The Android `MotionEvent` copy of a rubber contact (tool type 4) goes through §3.4 and does
  nothing ink-related [inferred].
- **Side button.** With `sideBtnEraseEnable` at its native default of 1, side button + tip →
  `SideEraseReader` → `onBeginRawErasing(true, p)`; `ScribbleHandler` ignores the flag, so it erases
  exactly like the rubber [code]. Stock Notes never calls `TouchHelper.enableSideBtnErase` /
  `Device.setEnablePenSideButton` (grep over the whole decompiled APK finds only the definitions,
  `SFTouchRender.java:330-333`, `SDMDevice.java:2129-2131`) [code]. SF's own default for
  `SET_ENABLE_PEN_SIDE_BUTTON` (`fw/android/onyx/ViewUpdateHelper.java:212, 1147-1150`) is [unknown];
  the user's stylus has no side button, so this path is inert on the target device.
- Stock never reads `MotionEvent.getButtonState()` or `KEYCODE_STYLUS_*` [code: grep].
- Eraser rendering by SF during a rubber stroke is configured on every resume:
  `setErasePathDrawing(isEraseTrackByRawRender(type) || selection, moveStroke ? 8 : 5)` →
  `SET_ERASER_RAW_DRAWING_ENABLED` (`ResumeRawDrawingRequest.java:61-65`;
  `stock/sdk/scribble/utils/EraseUtils.java:23-29`) [code]; after a combined pause
  `resetPenDefaultRawDrawing()` sets eraser raw drawing off with style 5 until the next resume
  (`TouchHelper.java:182-185, 276-284`) [code].

### 3.12 Hover

- The native reader emits state 5 after every `SYN`; with `ABS_PRESSURE == 0` the `BtnReader` stays
  idle, so hovering produces **only** state-5 reports (no stroke states) [code].
- Java: `PenActiveEvent` on the first, `PenDeactivateEvent` 100 ms after the last
  (`RawInputReader.java:88-102, 214-223`) [code]. Consumers: `ScribbleTouchDistributor` (finger
  filter, §3.5) and `AutoSaveEventHandler`, which postpones auto-save while the pen is active and
  re-arms it on deactivate (`stock/note/note/eventhandler/AutoSaveEventHandler.java:92-106`) [code].
  No other subscriber in `com/onyx` [code: grep].
- Android `ACTION_HOVER_*` events are not listened to anywhere in the app [code].

### 3.13 Second `TouchHelper`: the rich-text scene (added in verification)

Everything above describes the scribble scene. The rich-text scene uses a separate pen stack that
the original version of this document did not cover [code]:

- `RichTextBundle.attachHostView` installs an `OnTouchListener` on the rich-text host view
  (`stock/note/note/RichTextBundle.java:36-43, 67-77`). `f(event)`: a **finger** event is consumed
  (returns true) only when rich-text finger touch is disabled
  (`NoteMMKVHelper.isRichTextFingerTouchEnabled`), otherwise passed on to the child views; an
  **eraser-end** event (tool type 4) is never consumed and never reaches the handwriting path; a
  stylus event goes to a `GestureDetectorCompat(SimpleGestureListener)` and to
  `HandWritingHandler.onTouchEvent`, but only while `HandWritingHandler.shouldResumeRawDrawing()`
  (`:51-61`).
- `HandWritingHandler` creates its **own** `TouchHelper.create(view, cb)`
  (`stock/note/note/handler/text/HandWritingHandler.java:147-157`), whose feature mask is 2 (SF render
  only) when `DeviceFeatureUtil.hasStylus` is true, else 1 (`stock/sdk/pen/TouchHelper.java:38-40`).
  It never calls `setPostInputEvent`, so no `PenActiveEvent`s are produced by this instance [code].
  On a Note Air 4C `hasStylus` is expected to be true, i.e. mask 2 [inferred].
- On a stylus `ACTION_DOWN` it resumes raw drawing if paused (`onTouchDownEvent` → `resumeRawDrawing()`
  → limit = host view's local visible rect, exclude list, `setRawDrawingEnabled(true)` → pen state 2)
  and forwards the event to `TouchHelper.onTouchEvent` (`:457-470, 511-516, 139-146`). It pauses
  (`setRawDrawingEnabled(false)`) on keyboard shown, popups, action mode, focus loss and fast scroll,
  and pauses+resumes around every `SingleTapUpEvent` (`:383-455`) [code].
- `RichTextHandler.onSingleTapUpEvent` also synthesises `MotionEvent`s into this path
  (`RichTextHandler.java:777-786`)
  [code; not traced further].

Which scene is active is document state; the SF session effects of this second stack were not
traced in the other subsystem documents [unknown].

---

## 4. Tables

### 4.1 Input source → layer → consumer → SF effect

| input                                           | reaches                                                            | consumer in stock                                                                                                                                            | SDK / firmware calls                                                                                        | SF-visible effect                                                                                                                   |
| ----------------------------------------------- | ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| stylus tip, contact                             | native reader (states 0-3, 5) **and** `MotionEvent` tool 2         | raw: active handler draws; `MotionEvent`: HWR capture, diagnostics, `RxPenGesture` hold detection                                                            | none per point from Java; SF draws live ink itself                                                          | pen state stays 2; ink drawn by SF [inferred, doc08]                                                                                |
| stylus tip, hover                               | native reader (state 5 only) and Android hover events (ignored)    | `PenActiveEvent` → `RxGesture.setPenActive(true)`                                                                                                            | none                                                                                                        | none                                                                                                                                |
| eraser end, contact                             | native reader (`isErasing`, not shortcut) and `MotionEvent` tool 4 | `handler.onBeginRawErasing(false)`                                                                                                                           | erase begin may pause render depending on eraser type (§9)                                                  | eraser path drawn by SF if `SET_ERASER_RAW_DRAWING_ENABLED` [inferred]                                                              |
| side button + tip                               | native reader (`isErasing`, shortcut)                              | same as eraser end                                                                                                                                           | none                                                                                                        | same as eraser end                                                                                                                  |
| finger, single, tap                             | `MotionEvent`                                                      | `SingleTapUpEvent` → handler hit test; click → close sub-menus; `TouchDownEvent` → `PenEventHandler.onTouchDownEvent`; `ACTION_DOWN` hides the soft keyboard | none                                                                                                        | none unless a popup closes (→ resume), the status-bar flag was set (→ resume, §3.4) or the keyboard was shown (→ resume [inferred]) |
| finger, single, long press                      | `MotionEvent`                                                      | `LongPressEvent` → handler hit test (dropped if finger touch disabled)                                                                                       | none                                                                                                        | none by routing                                                                                                                     |
| stylus tip, long press / hold without moving    | `MotionEvent` tool 2                                               | **not** delivered as `LongPressEvent` (finger-only filter); `RxPenGesture` hold → `StartRecognizeDiagramAction` when shape-by-hold is on                     | see writing/HWR doc                                                                                         | [unknown here]                                                                                                                      |
| finger, single, swipe                           | `MotionEvent`                                                      | `FlingTurnPageAction`                                                                                                                                        | page change actions                                                                                         | page-change refresh (other subsystem)                                                                                               |
| finger, single, scroll (continuous mode only)   | `MotionEvent`                                                      | scroll gesture                                                                                                                                               | fast mode, `PauseResumeRawPenAction`, `UpdateRawDrawRectAction`                                             | 2 → 3 → 2, transient update                                                                                                         |
| two fingers, pan                                | `MotionEvent`                                                      | scroll gesture (only if the viewport can scroll)                                                                                                             | as above                                                                                                    | as above                                                                                                                            |
| two fingers, pinch                              | `MotionEvent`                                                      | scale gesture                                                                                                                                                | fast mode, `PauseResumeRawPenAction`, per-frame `UpdateRawPenWidthAction`, `UpdateRawDrawRectAction` at end | 2 → 3 → 2 (resume ~500 ms after begin), transient update                                                                            |
| three or more fingers                           | `MotionEvent`                                                      | dropped by `RxGesture`                                                                                                                                       | none                                                                                                        | none                                                                                                                                |
| finger while pen in range (+100 ms)             | `MotionEvent`                                                      | dropped by `RxGesture` (except up/cancel bookkeeping); `ScribbleTouchDetector`/tap detector still see it                                                     | none                                                                                                        | none                                                                                                                                |
| any touch while the activity lacks window focus | `MotionEvent`                                                      | dropped by the distributor                                                                                                                                   | none                                                                                                        | none                                                                                                                                |
| finger in lasso-selection mode                  | `MotionEvent`                                                      | `NoteManager.onTouchEvent` → `AppTouchRender` → raw drawing callbacks                                                                                        | `EpdController.moveTo/quadTo(DU)/penUp` while render on                                                     | app-driven stroke via framework (see 04-selection)                                                                                  |

### 4.2 SDK input APIs: used or not

| API                                                                                                 | stock usage                                                                                                                             | evidence                                                                          |
| --------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| `TouchHelper.create(view, 3, cb, false)`                                                            | yes                                                                                                                                     | `NoteManager.java:232`                                                            |
| `TouchHelper.create(view, cb)` (mask 2/1 by `hasStylus`)                                            | yes, rich-text handwriting (§3.13)                                                                                                      | `HandWritingHandler.java:155`                                                     |
| `TouchHelper.create` in `ScribbleTestActivity`, `MultiViewTouchHelper`, `rx/GestureTestActivity`    | test / unreferenced classes (`MultiViewTouchHelper` has no user; the two activities are test screens, not checked against the manifest) | grep                                                                              |
| `setPostInputEvent(true)`                                                                           | yes (enables Pen(De)ActiveEvent)                                                                                                        | `NoteManager.java:235`                                                            |
| `enableFingerTouch`, `onlyEnableFingerTouch`, `enableFingerTouchPressure`, `setFingerTouchPressure` | never                                                                                                                                   | grep: only definitions                                                            |
| `enableSideBtnErase`, `Device.setEnablePenSideButton`                                               | never                                                                                                                                   | grep: only definitions                                                            |
| `setBrushRawDrawingEnabled`, `setEraserRawDrawingEnabled`                                           | yes, on every resume                                                                                                                    | `ResumeRawDrawingRequest.java:48-50, 61-65`                                       |
| `setSingleRegionMode`                                                                               | yes, on every resume                                                                                                                    | `ResumeRawDrawingRequest.java:110`                                                |
| `setLimitRect(list)`, `setExcludeRect(list)`                                                        | on every resume and on every overlay change                                                                                             | `ResumeRawDrawingRequest.java:115-118`; `UpdateRawDrawRectAction.java:46-49`      |
| `EpdController.setAppCTPDisableRegion` / `appResetCTPDisableRegion`                                 | never (helpers exist)                                                                                                                   | `TouchAreaIgnoreAction.java:9-19`                                                 |
| `EpdController.isCTPDisableRegion`                                                                  | read-only check before toggling finger touch                                                                                            | `ToggleFingerTouchAction.java:49-55`; `ToolbarMenuModel.java:529-535`             |
| `EpdController.enableColorCU`                                                                       | float-menu press; erase begin/end; handler activation for eraser, shape, selection handlers                                             | `FloatDraggableMenuBar.java:81`; `ScribbleHandler.java:239-250, 463-468, 908-954` |
| `RxGesture.onHover`                                                                                 | never                                                                                                                                   | grep                                                                              |

### 4.3 Preferences that affect routing (defaults)

| preference                                              | default           | effect                                     | source                                            |
| ------------------------------------------------------- | ----------------- | ------------------------------------------ | ------------------------------------------------- |
| `FINGER_TOUCH_ENABLE_KEY`                               | true              | finger events reach handlers; finger lasso | `NoteMMKVHelper.java:1747-1749`                   |
| `FINGER_TURN_PAGE`                                      | 1 (single finger) | 0 disables, 2 = two-finger mode            | `:854-856`                                        |
| zoom enabled                                            | always true       | pinch allowed                              | `:1836-1838`                                      |
| `DISABLED_PAGE_ZOOM_KEY`                                | false             | disables `enableScale`                     | `:1727-1729`; `ScribbleTouchDistributor.java:210` |
| continuous single-finger scroll                         | true              | single-finger scroll in continuous mode    | `:1707-1709`; `NoteScribbleScene.java:565-567`    |
| disabled page turn / side / bottom gesture / status bar | false             | per-scene                                  | `:1719-1736`                                      |

### 4.4 Handler capabilities that decide routing

| handler                                                                          | `supportZoom` | `supportFingerDraw` | `canPenRawRender`                 | `isUsePenInput` | `usePenTipErase` | `mainPenRawInput` |
| -------------------------------------------------------------------------------- | ------------- | ------------------- | --------------------------------- | --------------- | ---------------- | ----------------- |
| base default                                                                     | false         | false               | true                              | true            | false            | false             |
| `EpdShapeHandler` (pen)                                                          | true          | false               | true                              | true            | false            | true              |
| `EraseHandler`                                                                   | true          | false               | eraser type ∈ {0,1,2}             | true            | **true**         | true              |
| `SelectionHandler`                                                               | true          | lasso mode          | lasso, not rect, not transforming | not rect        | false            | false             |
| `NormalShapeHandler`                                                             | true          | false               | true                              | true            | false            | false             |
| `FillColorHandler`                                                               | true          | false               | false                             | true            | false            | false             |
| `ZoomSelectionHandler`                                                           | false         | false               | false                             | false           | false            | false             |
| `LayerLockHandler`, `RichTextHandler`, `ReflowHWRHandler`, `PlacementHWRHandler` | varies        | false               | false                             | false           | false            | false             |

Sources: `stock/sdk/notecore/editor/handler/BaseHandler.java:50-57, 102-109, 300-328`;
`EpdShapeHandler.java:293-300, 493`; `EraseHandler.java:35-58, 113-130`;
`SelectionHandler.java:1398-1400, 1899-1905, 1959-1976, 2882-2893`; other handlers via grep of the
`return` lines [code].

---

## 5. Device-specific branches on the Note Air 4C

| branch                                                                                                          | condition                                                                                                                                                           | value on Note Air 4C                                                                 | how determined                                                                                                                                            |
| --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| device class                                                                                                    | `ro.board.platform` → `SDMDevice` for `lito`                                                                                                                        | `SDMDevice`                                                                          | `stock/sdk/device/Device.java:74-107, 118-137` [code]; platform `lito` from `docs/onyx-reversing/README.md` [inferred: earlier document, not re-verified] |
| scale interval `ScribbleTouchDistributor.m`                                                                     | `isTablet()                                                                                                                                                         |                                                                                      | hasHTCon()` ? 0 : 30 ms                                                                                                                                   | **0** | `ScribbleTouchDistributor.java:131-140` [code]; `vendor.onyx.htcon` = true because SF selects `HWEpdcManager` and the driver log shows `HWEpdcManager` lines (doc08 §0, handoff) [inferred] |
| menu/render-view interval `ScribbleTouchDistributor.n`                                                          | `hasHTCon()` ? 0 : (`isTablet` ? 250 : 1000) ms                                                                                                                     | **0** (no throttling of zoom/scroll UI updates)                                      | same [inferred from htcon=true]                                                                                                                           |
| colour test used by all rows below                                                                              | `DeviceInfoUtil.isColorDevice()` = `Device.currentDevice().getColorType() > 0`, `SDMDevice.getColorType()` reflects into the framework `EpdController.getColorType` | true [inferred: Kaleido 3 colour panel; the framework's return value was not traced] | `stock/sdk/utils/DeviceInfoUtil.java:136-138`; `stock/sdk/device/SDMDevice.java:560, 1159-1165` [code]                                                    |
| resume delay after pauses                                                                                       | `isColorDevice()` ? 500 : 150 ms                                                                                                                                    | 500 ms [inferred from the colour row]                                                | `RawPenArgs.java:68-70` [code]                                                                                                                            |
| popup resume delay                                                                                              | `isColorDevice()` ? 500 : 300 ms                                                                                                                                    | 500 ms [inferred from the colour row]                                                | `stock/note/note/event/PenEvent.java:20` [code]                                                                                                           |
| `PenEvent.DELAY_ENABLE_RAW_DRAWING_MILLS` (default resume delay, also used by the touch-down/status-bar resume) | = `RawPenArgs.DELAY_ENABLE_RAW_DRAWING_MILLS`                                                                                                                       | 500 ms [inferred from the colour row]                                                | `PenEvent.java:8` [code]                                                                                                                                  |
| fast-mode dither threshold                                                                                      | `isColorDevice()` ? 160 : 255 (restored to 128 when fast mode ends, `EpdEventHandler.d()` `:136-142`)                                                               | 160 [inferred from the colour row]                                                   | `EpdEventHandler.java:80-82` [code]                                                                                                                       |
| rich-text `TouchHelper` mask                                                                                    | `DeviceFeatureUtil.hasStylus(context)` ? 2 : 1                                                                                                                      | 2 [inferred]                                                                         | `stock/sdk/pen/TouchHelper.java:38-40` [code]                                                                                                             |
| `vendor.onyx.tablet`                                                                                            | `SystemPropertiesUtil.isTablet()`                                                                                                                                   | [unknown] (does not matter for routing once htcon is true)                           | `stock/sdk/utils/SystemPropertiesUtil.java:8, 57-59`                                                                                                      |
| native pen digitizer name                                                                                       | `Wacom` / `hanvon` / `onyx_emp`                                                                                                                                     | [unknown] which one; E6                                                              | strings in `native/libonyx_pen_touch_reader.so`                                                                                                           |
| native side-button erase                                                                                        | default 1, never changed                                                                                                                                            | enabled but no button on the user's stylus                                           | disassembly @0x9d90 [code]                                                                                                                                |
| `ENABLE_COLOR_CU`                                                                                               | called unconditionally (not gated by colour)                                                                                                                        | SF meaning [unknown]                                                                 | `fw/android/onyx/ViewUpdateHelper.java:194, 565-569` [code]                                                                                               |
| tool-type semantics                                                                                             | Android `TOOL_TYPE_STYLUS=2`, `ERASER=4`, `FINGER=1`, `UNKNOWN=0` counted as finger                                                                                 | as AOSP                                                                              | `stock/sdk/utils/TouchUtils.java:10-29` [code]                                                                                                            |

No phone/tablet or mono/colour branch was found in the finger-routing code itself beyond the table
above [code: grep of `touch/`, `gesture/`, `eventhandler/`].

---

## 6. Interactions with other subsystems

- **Session lifecycle (01).** The routing code produces pauses through `PauseRawPenAction`,
  `PauseResumeRawPenAction`, `InvalidateScreenAction`, `PausePenEvent`/`ResumePenEvent`,
  `RawPenPauseEvent` and resumes only through `PenEvent` → `PenEventHandler` gates →
  `ResumeRawDrawingRequest`. Window focus loss makes the distributor drop all touches and
  `ActivityFocusChangedEvent` runs `InvalidateScreenAction` (`PenEventHandler.java:335-347`).
- **Writing (02).** A stroke crossing an exclude rect is split (native state 3 then 0). Pinch zoom
  changes the live stroke width (`UpdateRawPenWidthAction`). The stylus `MotionEvent` copy is used for
  HWR point capture in `EpdShapeHandler`.
- **Erasing (03).** The eraser end needs no tool switch; `EraseBeginAction` pauses render
  (`PauseRawPenRenderAction`) only when the active eraser type is not drawn by SF
  (`EraseUtils.isEraseTrackByRawRender` = types 0,1,2) (`stock/note/note/action/erase/EraseBeginAction.java:26-54`)
  [code]. With the eraser **tool** selected and a non-SF-rendered eraser type, `canPenRawRender()`
  is false, so `PenEventHandler.A()` never resumes render: pen input on, SF session in state 3 for the
  whole time the tool is selected [code + inferred]. `EraseHandler.onActivate` runs a
  `PauseResumeRawPenAction` with 150 ms resume delay (`EraseHandler.java:61-66`) [code].
- **Selection (04).** Finger lasso uses `AppTouchRender` and `EpdController.moveTo/quadTo(DU)`.
  `SelectionHandler.supportScroll/supportZoom` are true, so pinch/pan work during selection.
  `touchInVisibleRect()` decides whether the invisible parts of the page are added to the exclude list.
- **Refresh (05).** Finger pan/zoom and float-menu drags use fast mode:
  `applyTransientUpdate(ANIMATION_QUALITY or DeviceConfig.transformUpdateModel)`, dither 160,
  `setEpdTurbo`, then 5 s later `clearTransientUpdate` (doc 07 notes a free full GC hidden in
  `clearTransientUpdate` on HW-Tcon) and `resetRefreshCount`. `SystemCTPChangeEvent` (broadcast
  `ctp.status.change`) triggers `InvalidateScreenAction` (`ScribbleFragment.java:1987-1989`;
  `stock/note/common/receiver/KNoteCommonReceiver.java:151-153`).
- **Firmware.** Stock is not in the screen-note pipeline, so none of the framework's
  finger-down/pen-down/eraser tool-type rules (`fw/.../screennote/handler/BaseHandler.java`) apply to it
  [inferred, doc04]. The system CTP-disable region (`InputManager.setSystemCTPDisableRegion`) is a
  system-wide feature the app only queries.

---

## 7. Findings that contradict or refine earlier documents

1. `03-raw-drawing-render-flag.md` lists "finger drag-gesture down" (`CommonEventHandler.java:160` via
   `DragGestureListener.java:106`) as a pen pause trigger. That is correct only for dragging the
   floating menu / zoom preview layouts that use `DragGestureListener`; a finger down **on the canvas**
   triggers no pause in stock [code, §3.4-3.6].
2. `RawInputReader.pause()`/`resume()` in this bundled SDK are `nativePausePen()` / `nativeSetPenState(4)`
   (`RawInputReader.java:573-602`), consistent with doc 03; natively `setPenState` only stores the value
   at `+0x30`, which `report()` also overwrites with the last reported state (@0xa6fc), and `pausePen`
   uses it to decide whether to emit the forced release [code].
3. "An empty exclude list is discarded inside the SDK" (README) is true for the SF-render half
   (`RawInputReader.setExcludeRect`) but not for the App-render half, which stock also has; with mask 3
   an empty list does reach SF [code, §3.10].

---

## 8. Open questions as hardware experiments

All experiments: stock Notes in a single page, pen tool selected, capture
`adb logcat -b all -v time` and filter `onyx_tcon_send_cmd`, `onyx_tcon_set_upd_scheme`,
`HWEpdcManager: ### EUS`, `SurfaceFlinger: Pid .* State change`, plus the app's own
`PenLogger`/`Debug` tags (`PenEventHandler`, `ScribbleTouchDistributor`, `RxGesture`) if debug logging
is enabled via the `com.onyx.android.note.toggle.debug` broadcast (`KNoteCommonReceiver.java`).

- **E1 — finger tap on the canvas does not pause.** Tap the empty canvas with one finger 10 times.
  Expected: no `State change to: toPause` for the Notes pid, no `EUS` line, no `update_mode[1]`.
  A `toPause` would refute §3.4/§3.10 (then look for a popup/sub-menu being closed).
- **E2 — pan and pinch pause/resume timing.** (a) Zoom in once, then two-finger pan slowly for 3 s.
  (b) Pinch continuously for 3 s. Expected: one `toPause` at gesture begin, one `running` ~500 ms
  later (while fingers still move in (b)), transient-update scheme lines at begin, and ~5 s after lift
  a `clearTransientUpdate`-related update (possibly a GC). Record whether `EUS s 4` appears.
- **E3 — write immediately after a pinch.** Pinch, lift, and within 200 ms write a stroke.
  Hypothesis: `ApplyFastModeEvent(false)` resumes immediately, so the stroke is written in state 2
  (no scheme-4/A2 + flash). A `EUS s 4` followed by `update_mode[1]` ~1.5 s after the stroke would show
  the pen wrote into a paused session.
- **E4 — floating-toolbar press pauses.** Switch to floating toolbar; press and hold a tool icon with
  the pen for 2 s, release on the icon, then write immediately. Expected: `toPause` on press; `running`
  only after the resulting action/resume (≥ 500 ms). If writing starts before `running`, watch for
  `EUS s 4` and a flash after lift. Repeat with a finger press.
- **E5 — overlays change regions without pausing.** Open the page navigator or zoom preview overlay,
  write strokes that cross it, close it, write again across its former area. Expected: no `toPause` on
  open/close; strokes split at the overlay edge; after closing, SF ink appears over the former area.
  If ink in the former area is accepted by SF but the stroke is still split in the saved note, the
  stale native exclude table (§3.10) is confirmed.
- **E6 — which evdev device the reader opens and hover reporting.** With root, `getevent -lp` to list
  devices; `getevent -lt /dev/input/eventN` on the pen device while hovering: confirm `SYN_REPORT`
  frames with `ABS_PRESSURE 0`, `BTN_TOOL_PEN`/`BTN_TOOL_RUBBER`, and the key codes 0x152/0x153 at
  proximity. Confirms the state-5 hover model and the device name match.
- **E7 — finger suppression window.** Hover the pen 5 mm above the screen and try to two-finger pan
  with the other hand; then lift the pen away and pan within ~100 ms and after ~300 ms. Expected: no
  pan while hovering, pan works after ~100 ms. Check `adb logcat` for the `RxGesture` "filter by pen
  touch" verbose line if gesture logging is on.
- **E8 — eraser end with the pen tool.** With the pen tool active and the default eraser type, erase
  with the back end. Expected: no `toPause` for types 0/1/2 and no flash; then switch the eraser type
  to one outside {0,1,2} and repeat: expect `toPause` at erase begin. This separates routing from the
  eraser-type rule.
- **E9 — disable finger touch.** Toggle "disable touch" in the toolbar. Expected: one pause/resume
  cycle from the toggle (`InvalidateScreenAction` with render not resumed, then `resumePen`), and
  afterwards pinch zoom and finger page swipe still work.
- **E10 — palm.** Rest the palm on the canvas while writing (pen in contact). Observe whether any
  `toPause`, scroll or zoom occurs and whether the palm's `MotionEvent`s reach the app (enable
  `RxGesture` debug logging). Also run `getevent` on the touch panel device while the pen hovers to
  see whether the CTP stops reporting (firmware-level rejection, currently [unknown]).
- **E12 — canvas touch-down resumes a status-bar pause.** Pull down the system status bar over
  stock Notes (expect `toPause`), dismiss it, then tap the canvas with a finger; repeat without
  the tap. Expected: with the tap, a `running` ~500 ms after the tap (from
  `PenEventHandler.onTouchDownEvent`); compare the timing to the no-tap case (focus-regain path).
- **E13 — locked zoom preview press.** Open the zoom preview, lock it, press and hold inside it for
  2 s with a finger, release, then drag it. Expected: `toPause` on press, `running` ~500 ms after
  release, and an additional pause/resume cycle after the drag's lift (`onResumePenEvent(null)` runs
  a `PauseResumeRawPenAction`).
- **E14 — hover over the toolbar disables finger gestures.** Hover the pen over a docked toolbar
  (outside the canvas) and two-finger pan the canvas with the other hand. §3.3 predicts no pan,
  because state 5 is not region-tested. A working pan would refute that inference.
- **E11 — `ENABLE_COLOR_CU`.** Press and release a floating-toolbar item without writing; look for any
  `onyx_tcon` or scheme line that coincides with `enableColorCU`/`disableColorCU` (the SDK logs
  `EpdController` "enableColorCU"/"disableColorCU" at debug level). Unknown SF effect.

---

## 9. Verification notes

An independent re-check against the primary sources (jadx of the stock APK under
`~/tmp_zfs/boox-notes-inspect/decoded/sources/`, `libonyx_pen_touch_reader.so` disassembly). Static
analysis only; no device was used.

**Checked and confirmed as written** [code]:

- `NoteManager.bindPenHostView` (mask 3, listener flag false, `setPostInputEvent(true)`,
  `openRawDrawing`, `forceSetRawDrawingEnabled(false)` when focused); `TouchHelper` private
  constructor order (SF render, then App render); `AppTouchRender.bindHostView` installs no listener
  when the flag is false; `RawInputManager.setHostView` pushes the local visible rect as limit.
- `NoteManager.getRawInputCallback` event mapping; `SFTouchRender.b` posts every callback to the host
  view; `SFTouchRender.setDrawingRenderEnabled` / `g()` / `h()` bodies.
- `RawInputReader.onTouchPointReceived` state handling (5 outside the `reportData` gate, 6 always,
  0-3 gated), `pause()`/`resume()`, `setExcludeRect` early return on an empty list,
  `setLimitRect`, and the 100 ms `PenDeactivateEvent` timer.
- `ScribbleTouchDistributor.onTouchEvent` order, the focus filter, `subscribe()` configuration, the
  scroll/scale decision in `c()`, the static `m`/`n` intervals and their htcon/tablet branches.
- `RxGesture.onTouchEvent` filter order, `RxGestureConfig` defaults (`filterOnHover` on,
  `largeAreaDetect` on at 0.8, `enabledPenTouch` off), `RxGestureContext.isDisabledPenTouch`,
  `onHover` having no caller outside `GestureTestActivity`.
- `TouchEventHandler` exclude/scribble-rect filter, finger filter, `usePenTipErase` redirection;
  `ScribbleHandler.filterFingerTouch`, `onSingleTapUp`, `onTouchUp`, `onBeginRawErasing` ignoring
  the shortcut flag; `EraseHandler` capabilities and `onActivate` 150 ms pause/resume.
- `ViewportEventHandler` begin/end handlers, `ScrollGestureHandlerAction` `canScroll` gate,
  `PauseResumeRawPenAction` → `PauseRawPenAction` → `ResumeRawPenAction` → `PenEvent`,
  `ResumeRawDrawingRequest` re-push of limit/exclude and pen parameters (only when render or input
  was actually off), `PenEventHandler.A()`/`z()` gate expressions (no touch/scale flag in either).
- `FloatDraggableMenuBar` press/release/cancel/drag behaviour; `DragGestureListener` →
  `RefreshScreenEvent` → `InvalidateScreenAction`; `ToolbarGestureDetector` →
  `TransformDragFloatMenuEvent` → `FloatToolMenuHandler` pause/resume.
- `ExcludeRectType` slot numbers; `EditorBundlesKt.getTouchExcludeRectList`;
  `AppTouchInputReader` skip rule and defaults; `ToggleFingerTouchAction` flow.
- Native: `PenManager` constructor sets `+0x206c` (side-button erase) to 1 @0x9d90 and the default
  pen-model code 0x152 at `+0x2074`; `TouchConsumer::onTouchEvent` inserts `false` as the second
  boolean @0xb4ec before `EventQueue::push`; `onSyncEvent` @0xa178 dispatches to the active reader
  and reads `+0x2074 == 0x153` for the pressure curve; exclude array cleared only by
  `setExcludeRegion` (clear-and-refill) and `PenManager::clear` (from `~TouchReader`).
- Whole-APK grep: no callers of `enableFingerTouch`, `onlyEnableFingerTouch`,
  `enableFingerTouchPressure`, `setFingerTouchPressure`, `enableSideBtnErase`,
  `setEnablePenSideButton`, `setAppCTPDisableRegion`/`appResetCTPDisableRegion` (except the unused
  `TouchAreaIgnoreAction`), `PalmDetector`, `KEYCODE_STYLUS_*`; `getButtonState` only inside
  AndroidX; `setOnHoverListener` only in AndroidX/rxbinding/`GestureTestActivity`;
  `PenActiveEvent`/`PenDeactivateEvent` subscribers only `ScribbleTouchDistributor` and
  `AutoSaveEventHandler`.

**Changed**:

1. §1 item 2: "the `MotionEvent` copy is used only for bookkeeping" → it also drives pen-hold shape
   recognition and can trigger a pen resume through `TouchDownEvent`.
2. §1 item 4: "No code path changes the SF pen state on a finger-down on the canvas" → no path
   _pauses_ on a plain canvas finger-down, but `PenEventHandler.onTouchDownEvent` _resumes_ when the
   status-bar flag is set, the keyboard-hide on `ACTION_DOWN` can end a keyboard pause [inferred], and
   touches inside the locked zoom preview pause.
3. §3.4: long-press was described as reaching handlers for any pointer; it is finger-only
   (`TouchEventHandler.onLongPressEvent`).
4. §3.5 item 3: the pen-active path was described as running only the up/cancel listeners; it also
   feeds the detectors and derived streams for `DOWN/UP/CANCEL`, and does **not** call the
   `onTouchDown` listener.
5. §3.5 item 4: large-area filter not stopping pinch/two-finger pan upgraded from [inferred] to [code].
6. §3.10: the `SlideToolbarVisibilityToggleDetector` row claimed an active 3 → 2 cycle; the class is
   never instantiated (struck through). `DragGestureListener` row now states pen and finger are both
   accepted.
7. §5: popup delay line pinned to `PenEvent.java:20`; the colour branch value downgraded from
   [code, README] to [inferred] (it depends on the framework's `getColorType`, which was not
   traced); the `lito` platform source marked as an earlier document, not code.
8. §3.6 step 5: added the `renderInFastMode()` and `penRawTouch` conditions that the fast-mode exit
   depends on.

**Added**: §2 rows for the fragment-root intercept, the zoom-preview touch entry and the rich-text
`TouchHelper`; §3.3 note on hover posting `PenActiveEvent` regardless of region and on the global
`TouchEventBus`; §3.4 paragraph on the other `TouchDownEvent` consumers and on `RxPenGesture`;
§3.10 rows for locked zoom-preview presses, the float-main-menu fixed-mode consumer and the status-bar
touch-down resume, plus the double limit push by the App render; §3.13 (rich-text scene);
§4.1 stylus-hold row; §4.2 rows for the other `TouchHelper.create` sites; §5 rows for the colour
test, `DELAY_ENABLE_RAW_DRAWING_MILLS` and the rich-text mask; experiments E12-E14.

**Could not confirm** [unknown]:

- Whether the native reader still emits state 5 after `nativePausePen()` (not disassembled).
- The exact device-name match logic in `TouchReader::findDevice` (strings and call graph only).
- The framework's `getColorType()` and `DeviceFeatureUtil.hasStylus` values on the Note Air 4C, and
  `vendor.onyx.htcon` / `vendor.onyx.tablet` values (htcon=true remains an inference from
  doc 07/doc 08).
- Pen-session effects of `StartRecognizeDiagramAction`, of handler hit-test actions started from
  taps/long-presses, of `AudioMenuModel.onTouchDownEvent`, and of the rich-text stack's pauses on SF.
- Whether `InputMethodUtils.hideInputKeyboard` on a canvas `ACTION_DOWN` always produces a
  `KeyboardChangeEvent(false)` when the keyboard was visible.
- Whether `ScribbleTestActivity` / `GestureTestActivity` are reachable in the release build (manifest
  not checked).
- Everything attributed to doc08 (SF behaviour on state transitions) was not re-verified here.
