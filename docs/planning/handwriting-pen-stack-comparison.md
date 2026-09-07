# The BOOX pen stack: our app, stock Notes, and five open-source apps

A reference comparison, written while chasing a firmware-level "dead rectangle" — an area of the
panel that stops taking new ink and stops showing app content, and that only a render-flag cycle
brings back. Everything here is evidence, not recollection: every claim carries a file and line.

## Source roots and shorthands

| shorthand | root                                                                                                    |
| --------- | ------------------------------------------------------------------------------------------------------- |
| `ours/`   | `/home/okhsunrog/code/rust/notes-rs/`                                                                   |
| `kt/`     | `ours/plugins/tauri-plugin-mobile-system/android/src/main/java/dev/okhsunrog/mobile_system/`            |
| `stock/`  | `/home/okhsunrog/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/`                          |
| `pen/`    | `/home/okhsunrog/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-pen-1.5.4.3/sources/com/onyx/android/sdk/pen/` |
| `ref/`    | `/home/okhsunrog/tmp_zfs/reference_notes_apps/`                                                         |

Two earlier reports are cited by name rather than re-derived:
`/home/okhsunrog/tmp_zfs/reversed_onyx_notes_app/REPORT.md` (stock app, two sections),
`/home/okhsunrog/tmp_zfs/reference_notes_apps/REPORT.md` (the five apps),
`/home/okhsunrog/tmp_zfs/onyx_sdk_decompiled/REPORT.md` (SDK).

### The states compared

| id     | what                                      | commit                                                                  |
| ------ | ----------------------------------------- | ----------------------------------------------------------------------- |
| **S1** | the original prototype adapter            | `c8657b9` "Add a BOOX native fast ink bridge for the handwriting sheet" |
| **S2** | after the Track D display-mode stack      | `ad02159`, `b9e3342`                                                    |
| **S3** | after Track F input separation            | `5b61507` "pause the firmware pen by named reason"                      |
| **S4** | after the lasso / selection-menu work     | `95e1b58`, `4c465bb`, `f74ace4`, `b5c9382`, `25f92c3`, `3e0036e`        |
| **S5** | current HEAD, the stock-aligned reconcile | `34dc906`                                                               |

Reference apps: **notate**, **notable**, **PngNote**, **saber** (`packages/onyxsdk_pen`), **mokke**
(`mokke-ink-notes`). "Stock" is `com.onyx.android.note` versionCode 45326.

---

## 0. Orientation: what the layers are

Five layers sit between the pen tip and a pixel. Almost every bug in this area is a layer being
driven as if it were one of its neighbours.

1. **The digitizer.** A Wacom-class EMR panel behind the screen, read as an evdev device by
   `libonyx_pen_touch_reader.so`. It is not an Android input device: no `MotionEvent`, no view
   hierarchy, no window z-order. The capacitive touch panel (CTP) that reports fingers is a
   _separate_ device.

2. **`RawInputReader`.** The SDK's JNI wrapper around that fd (`pen/RawInputReader.java:297`
   loads the library). It owns a native thread, maps raw coordinates to screen coordinates through
   `EpdController.getRawTouchPointToScreenMatrix()` (`pen/RawInputReader.java:269`), decides which
   points fall inside the drawable region, and calls back into `RawInputCallback`. It also owns two
   pieces of state that matter enormously: the **limit/exclude regions**
   (`pen/RawInputReader.java:410-425`, `:538-545`) and the **pen-up refresh timer**
   (`:277-283`, default 500 ms at `:131`, enabled by default at `:134`).

3. **The firmware ink layer.** When raw drawing is _rendered_, the EPD controller paints the stroke
   into the panel itself, inside the limit region minus the exclude regions, **above every window
   and outside the Android compositor entirely**. `setRawDrawingRenderEnabled` is not a Java-side
   flag: it becomes `EpdController.setScreenHandWritingPenState(view, PEN_DRAWING=2 | PEN_PAUSE=3)`
   plus `leaveScribbleMode` (SDK REPORT §3; stock REPORT line 203). This ink is **transient** — it
   lives in the panel's scribble state, not in any buffer the app can read, and the app is expected
   to take it over.

4. **EPD update modes.** Every panel write happens under a mode (`DU`, `GU`, `GC`, `REGAL`, `A2`
   family, `HAND_WRITING_REPAINT_MODE`, …). Modes exist at three scopes that override each other:
   per-view (`View.setDefaultUpdateMode`), transient/app-wide
   (`applyTransientUpdate`/`clearTransientUpdate`), and the persisted per-app EinkWise profile
   (`EInkHelper.setAppScopeRefreshMode`, or the stored JSON). SDK REPORT §1 tabulates all three.

5. **App rendering.** An ordinary Android draw pass. For stock this is `EditorView.onDraw` blitting
   a cached page bitmap; for us it is Chromium compositing a `<canvas>` into the WebView's surface.

**The handover problem.** Layer 3 draws instantly and layer 5 draws correctly. Something has to
replace the one with the other, and while raw drawing is enabled the firmware owns that screen
region: an ordinary `invalidate` does not necessarily reach it. Stock's answer is to mark the
_submitted frame_ with `HAND_WRITING_REPAINT_MODE` (§6). If the handover never happens for some
sub-rectangle, that rectangle keeps whatever the panel last held — which is exactly the "dead
rectangle".

---

## 1. `TouchHelper.create` — features, touch listener, callback thread

|             | feature mask                                       | `touchListenerEnabled`     | effective renderer                 | callback thread                                  |
| ----------- | -------------------------------------------------- | -------------------------- | ---------------------------------- | ------------------------------------------------ |
| **S1–S3**   | `SF` only (2)                                      | `false`                    | `SFTouchRender`                    | **main** (SDK posts)                             |
| **S4–S5**   | `SF｜APP` (3)                                      | `false`                    | `SFTouchRender` + `AppTouchRender` | **main**                                         |
| **stock**   | `SF｜APP` (3)                                      | `false`                    | both                               | main (SDK post, then EventBus `ThreadMode.MAIN`) |
| **notate**  | `create(view, true, cb)` → `SF`                    | (3-arg, implicitly `true`) | `SFTouchRender`                    | main                                             |
| **notable** | `create(view, cb)` 2-arg → `SF` on a stylus device | implicitly `true`          | `SFTouchRender`                    | main                                             |
| **PngNote** | `create(view, cb)` 2-arg → `SF`                    | implicitly `true`          | `SFTouchRender`                    | main                                             |
| **saber**   | `create(view, cb)` 2-arg → `SF`                    | implicitly `true`          | `SFTouchRender`                    | main                                             |
| **mokke**   | `create(view, cb)` 2-arg → `SF`                    | implicitly `true`          | `SFTouchRender`                    | main                                             |

Notes.

- The 4-argument overload is `TouchHelper(View, int feature, RawInputCallback, boolean
enableTouchListener)` (`pen/TouchHelper.java:358`, public factory `:404`). The 2-argument
  `create(view, cb)` routes through the private constructor at `pen/TouchHelper.java:44-46`, which
  passes `enableTouchListener = true` and picks the feature itself:
  `DeviceFeatureUtil.hasStylus(ctx) ? 2 : 1`. `hasStylus` matches the input-device names `onyx_emp`,
  `Wacom I2C Digitizer`, `hanvon_tp`
  (`/home/okhsunrog/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-base-1.8.5.4/sources/com/onyx/android/sdk/utils/DeviceFeatureUtil.java:20,54-64`).
  **So all five reference apps get `SFTouchRender` on this hardware, whichever overload they use** —
  the reference `REPORT.md:16,24` claim that the 2-argument form selects `AppTouchRender` is wrong on
  a real BOOX. Their `enableTouchListener = true` is inert, because
  `SFTouchRender.setTouchListenerEnabled` is an empty body (`pen/touch/SFTouchRender.java:371`).
- Stock: `TouchHelper.create(view, 3, getRawInputCallback(), false)` at
  `stock/sdk/notecore/editor/NoteManager.java:220-255`. Passing `false` is what keeps
  `MotionEvent`s flowing to the app (stock REPORT §1); with `true` the `AppTouchRender` half installs
  an `OnTouchListener` on the host view and JS pointer events break.
- We moved from `SF` to `SF｜APP` in `e1a9da7` on the hypothesis that a finger touch inside the
  region was triggering a firmware refresh under `SF` alone; the code says so at `kt/OnyxInk.kt:324-332`
  and marks it as an experiment to revert if pen rendering regresses. **This is still unverified on
  hardware.**
- **Callback threading: the SDK already hands us the main thread.** `RawInputReader` reads the
  digitizer on its own single-thread executor (`pen/RawInputReader.java:296-301`), but `SFTouchRender`
  wraps the app's callback in `C0015b` and `getHostView().post(...)` for **every** method —
  `onBeginRawDrawing` (`pen/touch/SFTouchRender.java:39-47`), the move and point-list callbacks
  (`:59-77`), the erasing pair (`:79-117`), `onPenActive` (`:119-127`) and `onPenUpRefresh`
  (`:129-137`). Every callback body in every app here therefore runs on the host view's handler
  thread.

  Two things follow for us. First, our `kt/OnyxInk.kt:803-808` re-post of `onPenUpRefresh` and its
  comment ("Delivered off the UI thread by an RxTimer") are **wrong**: the rect is already on the
  main thread, and the extra `post` only costs a message-loop turn of latency before the reconcile
  starts. Second, the `@Volatile`/`post` defensiveness elsewhere in the class buys nothing.

- Stock adds a second hop on top: the callback does nothing but `post` to an EventBus
  (`stock/sdk/notecore/editor/NoteManager.java:449-507`) and `TouchEventHandler` re-dispatches with
  `@Subscribe(threadMode = ThreadMode.MAIN)` — belt and braces for the same thread it was already on.
- **A landmine in that wrapper.** Nine of the ten methods guard with `m203a()` =
  `callback == null || getHostView() == null` (`pen/touch/SFTouchRender.java:376-378`);
  `onPenUpRefresh` checks only for a null callback and then dereferences `getHostView().post(...)`
  (`:129-137`). The host view is held weakly (`:217`), so a collected view means an NPE on the
  `"raw_input"` thread, which the reader's Runnable catches by calling `nativeRawClose()`
  (`pen/RawInputReader.java:143-162`) — killing the pen for the whole process. Since `34dc906` we
  depend on that callback, so this is now on our path; it was not before.
- The feature mask is a bitmask and `TouchHelper` fans every method out over one `TouchRender` per
  bit (`pen/TouchHelper.java:358-375`, and e.g. `:136-142`). With mask 3 both `SFTouchRender` and
  `AppTouchRender` receive every region call. Callbacks are not duplicated for us or for stock,
  because `AppTouchRender` only produces them from `MotionEvent`s it receives through an
  `OnTouchListener` that `touchListenerEnabled = false` never installs
  (`pen/touch/AppTouchRender.java:104-113`) — but the region calls very much are duplicated, and
  the two halves do not behave identically (§2a).

---

## 2. Region setup — limit rect, exclude rects, region mode

|                          | limit rect                                              | exclude rects                                                                                | region mode                       | re-applied when                               |
| ------------------------ | ------------------------------------------------------- | -------------------------------------------------------------------------------------------- | --------------------------------- | --------------------------------------------- |
| **S1**                   | single `Rect` from JS CSS geometry, clipped to viewport | `emptyList()` (no-op, see below)                                                             | never set → SDK default **multi** | `configure()` only                            |
| **S2**                   | same                                                    | `emptyList()`                                                                                | never set → **multi**             | `configure()` only                            |
| **S3**                   | same                                                    | `emptyList()`                                                                                | never set → **multi**             | `configure()` + every `RawDrawingGate.resume` |
| **S4 early** (`93c5724`) | same                                                    | **selection-menu box pushed as a real exclude rect**                                         | **multi**                         | `configure()` + resume                        |
| **S4 late** (`95e1b58`+) | same                                                    | back to `emptyList()`; menu handled by gesture swallow — clears the firmware half only (§2a) | **multi**                         | `configure()` + resume                        |
| **S5**                   | same                                                    | `NO_EXCLUDES` (empty); same asymmetry                                                        | **`setSingleRegionMode()`**       | `configure()` + every resume                  |
| **stock**                | `List<Rect>` = `scribbleRect`                           | 10 typed overlay slots **+ page gaps**, always pushed with the limit rect                    | `setSingleRegionMode()`           | every resume, every rect change               |
| **notate**               | `getLocalVisibleRect`                                   | toolbar + open sidebar                                                                       | `setSingleRegionMode()`           | `surfaceCreated`/`surfaceChanged`, re-enable  |
| **notable**              | view minus a 40 dp toolbar strip                        | the toolbar strip                                                                            | none                              | `surfaceCreated`, size change, toolbar toggle |
| **PngNote**              | `getLocalVisibleRect` offset `(0,-40)`                  | none                                                                                         | none                              | layout, `surfaceCreated`, resume, focus       |
| **saber**                | `getLocalVisibleRect`                                   | none                                                                                         | none                              | layout-change listener only                   |
| **mokke**                | `getLocalVisibleRect` of the SurfaceView                | none                                                                                         | none                              | debounced 500 ms detach + reattach            |

### 2a. `setExcludeRect` cannot be withdrawn with an empty list — on one half of the pair

A region call reaches **two independent sinks that have to agree**: the evdev point filter
(`nativeSetExcludeRegion`, in raw digitizer coordinates) and the firmware ink layer
(`EpdController.setScreenHandWritingRegionExclude`). The SF path writes both, and guards both:

```java
// pen/RawInputReader.java:418-425
public void setExcludeRect(List<Rect> rectList) {
    if (rectList == null || rectList.size() <= 0) {
        return;                                    // <-- silent early return
    }
    nativeSetExcludeRegion(m17a(rectList));
    EpdController.setScreenHandWritingRegionExclude(getHostView(), rectList.toArray(new Rect[0]));
```

`TouchHelper.setLimitRect(Rect, List<Rect>)` (`pen/TouchHelper.java:136-142`) fans out to
`RawInputManager.setLimitRect(Rect, List)` (`pen/RawInputManager.java:149-153`), which is literally
`setLimitRect(rect); setExcludeRect(excludeRectList);`. So on the SF path
`setLimitRect(limit, emptyList())` pushes the limit rect and **leaves any previously pushed exclusion
latched, on both sinks**. `setLimitRect(List<Rect>)` has the same guard
(`pen/RawInputReader.java:538-541`).

**The `AppTouchRender` half has no such guard**, and this is the part that changes the story:

```java
// pen/touch/AppTouchInputReader.java:151-157
public void setExcludeRectList(View hostView, List<Rect> excludeRectList) {
    if (excludeRectList == null) { return; }                    // null only — not empty
    CollectionUtils.safeAddAll(this.f203b, excludeRectList, true);
    EpdController.setScreenHandWritingRegionExclude(hostView, excludeRectList.toArray(new Rect[0]));
}
```

reached from `pen/touch/AppTouchRender.java:180-183`, and `SDMDevice` flattens a zero-length array
without complaint (`.../onyxsdk-device-1.3.5.2/sources/com/onyx/android/sdk/device/SDMDevice.java:3438-3454`).

So with our mask of 3 (`SF｜APP`, from `e1a9da7` onward), pushing `emptyList()` **clears the firmware
exclude region and leaves the native evdev filter armed.** The two halves disagree: the firmware
happily paints ink in that band, and the reader silently drops every point in it, so the app never
learns the stroke exists and its canvas never reproduces it. The next reconcile then composites app
content over the firmware ink and the stroke vanishes. That is a very precise description of a dead
rectangle. Under mask 2 (`SF` alone, S1–S3) the empty list did nothing at all on either sink.

**How persistent is a latched exclusion?** The native state is process-global: every JNI entry point
operates on one static `TouchReader`, whose `PenManager` holds the limit and exclude arrays.
`closeRawDrawing()` → `RawInputReader.quit()` (`pen/RawInputReader.java:341-353`) →
`nativeRawClose()` only stops the read loop; it does not touch those arrays, and the only code that
wipes them is `TouchReader`'s destructor — in practice, process exit. So a latched exclusion survives
closing the ink session, navigating away and reopening the editor, **but not restarting the app.**

**Clearing one properly.** `kt/OnyxInk.kt:183-189` has the right idea — push a list that passes the
size check but excludes nothing — but the rectangle it keeps is wrong:
`EMPTY_EXCLUDE = listOf(Rect(0, 0, 0, 0))`. The native `inExcludeRegion` test expands the probe point
by `strokeWidth / 2` before the AABB comparison, so a zero-area rect at the origin still excludes the
top-left corner of the panel. An off-panel rectangle such as `Rect(-10, -10, -9, -9)` is what is
wanted. As it stands, the constant is only reachable from `debugRepaint("clearExcludesHard")`
(`kt/OnyxInk.kt:399-403`) anyway.

**Consequence for the dead rectangle.** Between `93c5724` and `95e1b58` we pushed the floating
selection menu's box as an exclude rect, on a build already using mask 3. On any device that ran one
of those builds, the native filter has an exclusion latched in the band just above wherever a
selection last was, and every later build's `emptyList()` cleared only the firmware half of it.
Restarting the process is the only thing that has been clearing it.

### 2b. Two more region traps

- **`bindHostView` overwrites the limit rect.** `RawInputManager.setHostView` ends with
  `view.getLocalVisibleRect(rect); setLimitRect(rect)` (`pen/RawInputManager.java:141-147`), and the
  `TouchHelper` constructor calls `bindHostView` (`pen/TouchHelper.java:374`). So constructing a
  helper — or re-binding — silently replaces the limit rect with the view's visible rect, which is
  `(0,0,0,0)` before layout. The single-`Rect` overload accepts a zero-area rect and pushes it
  through with no warning (`pen/RawInputReader.java:410-416`), which makes the pen dead everywhere.
  We are safe by ordering: `configure()` calls `TouchHelper.create` first and `setLimitRect(...)`
  after (`kt/OnyxInk.kt:327-340`).
- **Region arrays are fixed-size and unlocked.** The native `PenManager` holds 4 KB (256 rects) for
  each of limit and exclude with no bounds check, and `setLimitRegion` memsets and refills the array
  with no synchronisation against the reader thread that is concurrently testing it. Changing regions
  mid-stroke is a genuine data race — which is one more reason `4c465bb`'s deferral of a reconfigure
  until pen-up (§8) was the right shape, quite apart from the truncated trace it was fixing.

### 2c. Region mode

`setSingleRegionMode()` is `nativeSetRegionMode(1)` + `EpdController.setScreenHandWritingRegionMode(view, 1)`
(`pen/RawInputReader.java:394-398`); `setMultiRegionMode()` is the same with `0` (`:400-404`).
`TouchHelper` calls neither at construction, so a helper that never asks is in mode 0.

Stock re-applies single-region mode on **every** resume, inside `ResumeRawDrawingRequest.o()`:

```java
// stock/note/note/request/pen/ResumeRawDrawingRequest.java:107-113
private final void o() {
    p();                                                    // limit + exclude rects
    getNoteManager().setPenUpRefreshTimeMs(OnyxSystemConfig.getPenUpRefreshTimeMs());
    getNoteManager().setSingleRegionMode();
    getNoteManager().applyDrawingArgs(...);
```

and `p()` at `:115-118` pushes limit and exclude rects together. notate also sets it once
(`ref/notate/app/src/main/java/com/alexdremov/notate/ui/OnyxCanvasView.kt:893`). No other reference
app does, and neither did we before `34dc906`.

`34dc906` added it both at creation (`kt/OnyxInk.kt:339`) and in `RawDrawingGate.pushRects()`
(`kt/OnyxInk.kt:113-122`).

**What the mode actually does, on each half.** On the native side it is not "one region" in the sense
the name suggests. `PenManager::inLimitRegion` expands the probe point by `strokeWidth / 2` and
AABB-tests it against each limit rect; in mode 0 any hit passes, while in mode 1 the _index_ of the
first rect a stroke enters is latched and only that rect passes for the rest of the stroke (the latch
is reset by `setLimitRegion`). With a single limit rect, as in our case and stock's, that is
behaviourally a no-op. The interesting half is the other one: `setSingleRegionMode` also calls
`EpdController.setScreenHandWritingRegionMode(view, 1)` (`pen/RawInputReader.java:394-398`), and on
`SDMDevice` that resolves to `ViewUpdateHelper.setScreenHandWritingRegionMode(int)` — a static taking
only the int, so the `View` argument is silently discarded. **What the firmware does with mode 1 is
not visible in any of these sources.** The hypothesis that multi-region mode makes the firmware
accumulate one transient scribble region per stroke, until the table fills and rectangles stop taking
ink, is consistent with everything observed and with stock re-applying mode 1 on every resume — but
it is a hypothesis about firmware behaviour, not something the decompilation proves.

The code comment at `kt/OnyxInk.kt:114-118` states it more confidently than the evidence supports;
worth softening if this is ever revisited.

### 2d. Stock's exclude list is not only for overlays

`getTouchExcludeRectList` returns the registered menu rects **plus**, when the active handler's
`touchInVisibleRect()` is true, every _invisible_ sub-rect of the limit rect — the gaps between page
boxes — whose shorter side is at least 1 px
(`stock/sdk/notecore/editor/extension/EditorBundlesKt.java:315-338`). `EraseHandler` returns `false`
there, so erasing is deliberately allowed in the gaps
(`stock/note/note/handler/scribble/EraseHandler.java:116-119`). Ten typed overlay slots are
registered through `NoteDocViewInfo.addExcludeRect(type, rect)`
(`stock/sdk/notecore/editor/data/NoteDocViewInfo.java:41,124,128`;
`stock/sdk/notecore/editor/data/ExcludeRectType.java:9-21`), always pushed together with the limit
list by `UpdateRawDrawRectAction` (`stock/note/note/action/pen/UpdateRawDrawRectAction.java:51-79`).
Nothing in stock ever pushes one without the other.

Our sheet is a single continuous page, so the gap case does not arise — but it is worth knowing that
"stock uses exclude rects" is two mechanisms, not one, and only the overlay half is comparable.

### 2e. The SDK caches the view position

`setLimitRect` resolves the host view's screen position when it is called; mokke rediscovered this
the hard way and documents it at
`ref/mokke-ink-notes/.../HandwritingCanvasView.kt:626-639` ("the Onyx SDK caches the view's screen
position at attach time"), which is why it debounces a full detach/reattach by 500 ms on any size
change. Stock re-pushes rects on every resume; saber re-arms with a
`setRawDrawingEnabled(false); setRawDrawingEnabled(true)` pair
(`ref/saber/packages/onyxsdk_pen/android/src/main/kotlin/com/example/onyxsdk_pen/OnyxsdkPenArea.kt:243-251`).
We re-push in `RawDrawingGate.pushRects()` from S3 onward (`kt/RawDrawingGate.kt:34-40`).

---

## 3. Touch and palm

|             | `enableFingerTouch`            | `setPostInputEvent` | CTP disable region                                                        | palm strategy                                                             |
| ----------- | ------------------------------ | ------------------- | ------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| **S1–S2**   | never                          | `false`             | **armed over the whole limit rect on every pen-down**, reset on end/pause | firmware CTP cutout + limit rect                                          |
| **S3–S5**   | never                          | `false`             | **never armed**; `appResetCTPDisableRegion` once at init                  | limit rect only                                                           |
| **stock**   | never (no-op anyway)           | `true`              | never (`setAppCTPDisableRegion` ships with no live caller)                | limit/exclude geometry + `getToolType` routing; **no palm detector runs** |
| **notate**  | documented as ignored under SF | not set             | none                                                                      | limit + exclude rects                                                     |
| **notable** | none                           | none                | none                                                                      | `dispatchTouchEvent` eats stylus pointers                                 |
| **PngNote** | none                           | none                | none                                                                      | toolbar never overlaps the canvas                                         |
| **saber**   | none                           | none                | none                                                                      | limit rect                                                                |
| **mokke**   | none                           | none                | none                                                                      | limit rect + `TouchFilter` size gate                                      |

Notes.

- `SFTouchRender.enableFingerTouch(boolean)` is an **empty body** (`pen/touch/SFTouchRender.java:342`;
  SDK REPORT §3 lists it alongside `onlyEnableFingerTouch`, `enableFingerTouchPressure`,
  `setFingerTouchPressure`, `setTouchListenerEnabled` and
  `RawInputManager.setHostViewScrollListenerEnabled`). There is no SDK-level finger switch on this
  device class. notate documents the same conclusion at `OnyxCanvasView.kt:891-892`.
  Our `setHostViewScrollListenerEnabled(false)` (`kt/OnyxInk.kt:338`) is therefore also a no-op —
  harmless, but it buys nothing.
- `setPostInputEvent` does not inject `MotionEvent`s; it gates posting `PenActiveEvent` /
  `PenDeactivateEvent` onto the SDK's `TouchEventBus` (`pen/RawInputReader.java:374-376`; SDK REPORT
  §3). Stock sets it `true` and uses those events for pen-proximity and auto-save
  (stock REPORT §2 "Pen hover"). We set it `false` and subscribe to nothing, so we have no
  proximity signal at all.
- **Stock has no palm rejection in its shipping path.** `stock/note/note/touch/PalmDetector.java`
  (size-based, `getTouchMajor() > largeObjectSize`, `:26-38`) exists but is **dead code with no
  references anywhere in the APK** — this corrects the earlier stock report, which listed it as the
  live strategy. What actually happens is firmware region confinement plus `getToolType`
  discrimination in `ScribbleTouchDistributor.onTouchEvent`
  (`stock/note/note/touch/ScribbleTouchDistributor.java:368-404`, hard `!hasWindowFocus() → false`
  gate at `:375`), plus dropping stroke events during multi-touch
  (`stock/note/note/touch/BaseTouchDetector.java:19-21`). Of the reference apps only mokke does real
  palm work (`ref/mokke-ink-notes/app/src/main/java/com/writer/view/TouchFilter.kt:16-138`:
  pen-hover suppression, a 500 ms pen-up cooldown, a 120 dp contact-size threshold, multi-touch
  reject, a stationary timeout).
- **`setAppCTPDisableRegion` was our own invention.** S1 armed it over the entire limit rect in
  screen coordinates at pen-down and cleared it at pen-up (prototype `OnyxInk.kt:201-218`). It is not
  a bad API in itself — it is app-scoped, reversible, and its full signature
  `setAppCTPDisableRegion(Context, Rect[] disable, Rect[] exclude)` even takes an array of holes to
  punch in the disabled band (SDK REPORT §4) — but we passed only the disable array over the whole
  sheet, so every capacitive touch there died, which is what made the soft keyboard unusable, and a
  dropped `onEndRawDrawing` left it armed indefinitely. `5b61507` removed it from the gesture path;
  `kt/OnyxInk.kt:637-647` now documents why and keeps a single reset at init (`:240-241`) to clear
  one a crashed older build may have left behind. Neither stock
  (`stock/note/note/action/ui/TouchAreaIgnoreAction.java:9-19` is the only reference, and both its
  methods are uncalled) nor any of the five apps arms one.

---

## 4. Enabling and disabling raw drawing

|             | mechanism                                                                                                                                           | who cycles it, on what                                                                            |
| ----------- | --------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| **S1–S2**   | `setRawDrawingEnabled(true/false)` + a separate `setRawDrawingRenderEnabled` for the tool                                                           | window focus, lifecycle, `configure()`, eraser gate                                               |
| **S3–S5**   | **two switches through `RawDrawingGate`**: pause = render off → input off; resume = push rects → `resetPenDefaultRawDrawing` → input on → render on | + every named pause reason, after a 150/500 ms delay                                              |
| **stock**   | two switches, always separate; `setRawDrawingEnabled` only when both flags coincide                                                                 | a ~15-flag predicate in `PenEventHandler`, plus `InvalidateScreenAction` on nearly every UI event |
| **notate**  | `setRawDrawingEnabled(false/true)`                                                                                                                  | pen popup, sidebar, pan/zoom; TEXT tool kills render only                                         |
| **notable** | `setRawDrawingEnabled` from an `isDrawingAllowed` flag                                                                                              | modals, window focus                                                                              |
| **PngNote** | close + reopen raw drawing when the size changed; `withTempNoRawRendering` around every surface repaint                                             | `onWindowFocusChanged`, `onStop`                                                                  |
| **saber**   | `setRawDrawingEnabled(false); …(true)` to re-arm                                                                                                    | layout change; `Tool.textEditing` → disabled stroke style                                         |
| **mokke**   | `pauseRawDrawing()`/`resumeRawDrawing()`; `closeRawDrawing()` for child activities                                                                  | every dialog, finger scroll                                                                       |

Notes.

- `TouchHelper.setRawDrawingEnabled(enabled)` is `setRawDrawingRenderEnabled(enabled)` +
  `setRawInputReaderEnable(enabled)` + `resetPenDefaultRawDrawing()`
  (`pen/TouchHelper.java:170-177`). Both individual setters are **deduplicated** —
  `if (this.f76a && this.f77b != enabled)` (`:188-195`) and `:213-220` — so a redundant call is a
  no-op, and a helper whose internal flags have desynced from the firmware can only be forced back
  with `forceSetRawDrawingEnabled` (`:196-208`), which we never use and stock uses once at bind
  (`stock/sdk/notecore/editor/NoteManager.java:246-250`).
- The **order** on resume matters. Stock does input first, render last, after a sleep:

  ```java
  // stock/note/note/request/pen/ResumeRawDrawingRequest.java:126-137
  ThreadUtils.mySleep(this.j.getC());      // DELAY_ENABLE_RAW_DRAWING_MILLS
  o();                                     // rects, pen-up interval, single-region, pen args
  if (g()) getNoteManager().setRawInputReaderEnable(true);
  if (f()) getNoteManager().setRawDrawingRenderEnabled(true);
  ```

  `kt/RawDrawingGate.kt:31-42` mirrors this exactly, and documents that the other order paints
  ghost ink from events that arrive before the reader is armed.

- `resetPenDefaultRawDrawing()` = `setBrushRawDrawingEnabled(true)` + `setEraserRawDrawingEnabled(false, 5)`
  (`pen/TouchHelper.java:348-351`). These flags are per _device_, not per session, so a resume that
  skips it can inherit a brush another app switched off — the reason `RawDrawingGate` calls it
  explicitly (`kt/RawDrawingGate.kt:15-21`) now that we no longer go through `setRawDrawingEnabled`.
  The flip side is that the combined `setRawDrawingEnabled` silently clobbers any eraser style you
  configured, which is precisely why an app with its own eraser gate must use the split switches.
- **`openRawDrawing()` on its own delivers no callbacks.** `RawInputReader.start()` sets
  `reportData = false` (`pen/RawInputReader.java:319-326`) and every dispatcher is gated on it
  (`:640-648`, `:762-773`, `:776-787`, `:791-797`); `setRawInputReaderEnable(true)` →
  `resume()` is what sets it (`:328-333`). Our `configure()` gets this right only because `resume()`
  runs immediately after (`kt/OnyxInk.kt:359`).
- Two more pieces of state the SDK resets under you: `openDrawing()` and `closeDrawing()` both force
  the stroke style back to `STROKE_STYLE_PENCIL` (`pen/touch/SFTouchRender.java:170-179`, `:265-275`),
  and `quit()` resets stroke width to 7.2 and colour to black and nulls the callback
  (`pen/RawInputReader.java:341-353`). Our ordering — `openRawDrawing()` first, style and width after
  (`kt/OnyxInk.kt:340`, `:344-358`) — is correct, but only by construction, not by intent.
- `leaveScribbleMode` / `setScreenHandWritingPenState` are never called by us directly; they are
  what `setRawDrawingRenderEnabled` becomes inside `SFTouchRender` (SDK REPORT §3, stock REPORT
  line 203). notate is the one reference app that drives the pen state itself
  (`ref/notate/.../OnyxCanvasView.kt:89`, `:95`, `:542`, `:743`, `:861`).

---

## 5. The per-stroke path

|             | where the live stroke is drawn                                                                 | what is persisted, when                                                                                     | thread                                                                  |
| ----------- | ---------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| **S1**      | firmware, when render on                                                                       | JS canvas, on `onRawDrawingTouchPointListReceived` (pen-up batch)                                           | main → Tauri event → JS                                                 |
| **S2–S5**   | firmware, when render on; JS preview frames when render off (lasso/eraser)                     | same, plus throttled `preview` at ≥32 ms                                                                    | same                                                                    |
| **stock**   | firmware only                                                                                  | `Shape` built on pen-down, points added on move, flushed to the page bitmap on `onRawDrawingPointsReceived` | main throughout (SDK post, then EventBus `ThreadMode.MAIN`)             |
| **notate**  | firmware only until pen-up                                                                     | in-memory + tiles on pen-up; disk debounced 500 ms                                                          | main; commit hops to `Dispatchers.Default`/`IO`                         |
| **notable** | firmware; `LineRenderer` draws into a bitmap and calls `refreshScreenRegion` per segment       | bitmap + DB on `Dispatchers.IO`                                                                             | main; per-mode hops after the callback                                  |
| **PngNote** | firmware only — the move callback is an **empty body**                                         | offscreen bitmap on pen-up; PNG written after 5 s idle                                                      | main                                                                    |
| **saber**   | firmware only; the plugin **discards** the points it collects (`drawPreview` just clears them) | nothing — Flutter's own gesture pipeline owns the stroke                                                    | main; `forceRefresh` on a `java.util.Timer` thread                      |
| **mokke**   | firmware only until pen-up                                                                     | document storage; `appendLastStrokeToBitmap()` incremental                                                  | main, including a synchronous `lockCanvas` compose inside `onStrokeEnd` |

Notes.

- Stock's callback does literally nothing but post events
  (`stock/sdk/notecore/editor/NoteManager.java:449-507`), and every visual step lives in
  `EpdShapeHandler`: `onBeginRawDraw` builds a `RenderContext` and a `Shape`,
  `onRawDrawingPointsMoveReceived` appends, `onEndRawDrawing` does **nothing visual**, and the real
  work is in `onRawDrawingPointsReceived` — deep-copy the shape, allocate a `ShapeGrayscaleBean`,
  `flushShapeToPage`, plain `View.invalidate()`, and an `AddShapesBackgroundAction` whose completion
  sets `renderToBitmap = true` and calls `P0()` (stock REPORT §1, "Per-stroke render and refresh
  sequence"; the handler is `stock/note/note/handler/common/EpdShapeHandler.java`).
- We deliver the whole batch to JS as one event and rebuild the stroke there
  (`kt/OnyxInk.kt:757-772`, `ours/src/features/handwriting/onyx-ink.ts:26-43`). Point budget 150 000
  (`kt/OnyxInk.kt:762`). Pressure is normalised against `EpdController.getMaxTouchPressure()`
  (`kt/OnyxInk.kt:323`).
- Our `preview` channel (`kt/OnyxInk.kt:783-792`) exists because when render is off — the eraser and
  the non-free lasso — the firmware draws nothing, so JS must draw the live feedback itself. It is
  throttled to one event per 32 ms and coalesced to one animation frame in JS
  (`ours/src/features/handwriting/onyx-ink.ts:231-243`). Stock has no equivalent: its eraser and
  lasso still render through the firmware (§9, §10).
- **Nobody rasterises during the stroke.** Even stock's `O0(...)` →
  `shape.onLowLatencyTouchData(...)` is a `return true` no-op on the base shape, and geometry
  pre-computation at most on the pencil and charcoal families — no drawing. In every implementation
  surveyed, what the user sees mid-stroke is 100 % firmware transient ink. Our `preview` channel is
  the one exception, and only for the tools where we deliberately turn firmware render off.
- The reference apps span the whole range of how seriously the points are taken: mokke and notate
  build a real stroke model on pen-up; PngNote leaves the move callback empty and rasterises a
  `quadTo` path on the point list; saber collects points, keeps every twentieth, and then throws them
  away — its `drawPreview()` is just `currentStroke.clear()`, and a `Paint` is constructed and never
  used. saber's real stroke data comes from Flutter's own gesture pipeline, with the `AndroidView`
  composited at `Opacity(0)` beneath it.

---

## 6. Handover and reconcile — the core of the problem

|             | mechanism                                                                                                                                                                                                            | region                                                                                    | rendezvous                                                                                                                    |
| ----------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| **S1**      | `setRawDrawingRenderEnabled(false)` → `EpdController.invalidate(view, DU)` → render back on                                                                                                                          | whole view                                                                                | `postVisualStateCallback` + 120 ms                                                                                            |
| **S2–S4**   | `setViewDefaultUpdateMode(HAND_WRITING_REPAINT_MODE)` + `invalidate()` + `registerFrameCommitCallback`, **and** `EpdController.handwritingRepaint(view, region)`                                                     | accumulated JS damage ±2 px, clipped to limit                                             | visual-state callback + 120 ms; `25f92c3` added putting un-repainted damage back                                              |
| **S5**      | mode-marked frame **only**; no `handwritingRepaint`                                                                                                                                                                  | pen-up union (stroke ∪ previous stroke), or the whole region after anything else drew     | **two-sided**: SDK pen-up refresh _and_ the canvas frame acknowledgement, whichever is last, with a `PEN_UP_WAIT_MS` fallback |
| **stock**   | `setViewDefaultUpdateMode(HAND_WRITING_REPAINT_MODE)` → `displayNoteDirtyRect` → `View.invalidate()` → `resetViewUpdateMode` in `doFinally`                                                                          | union of pending beans, or the whole `scribbleRect` when `isEnabledPenDirtyRect == false` | pen-up timer **and** bitmap-flush completion, whichever is last — **but gated off on this device (see 6c)**                   |
| **notate**  | lock the SurfaceView, re-render tiles, unlock; `invalidate(DU)` for undo/redo, region `invalidate(GC)` for erase                                                                                                     | whole view (erase: padded stroke bounds)                                                  | pen-up + commit coroutine                                                                                                     |
| **notable** | `resetScreenFreeze()`: render off → delay → render on, with a coalescing job; plus `refreshScreenRegion(…, ANIMATION_MONO)` per drawn segment; `HAND_WRITING_REPAINT_MODE` as the view mode, falling back to `REGAL` | per-segment dirty rect                                                                    | delays 500 ms colour / 300 ms mono / 150 ms stroke-erase                                                                      |
| **PngNote** | full-surface repaint inside `withTempNoRawRendering`, debounced 2000 ms after the last point                                                                                                                         | whole surface                                                                             | a 10 s `isDrawing` guard, no rendezvous                                                                                       |
| **saber**   | fixed 1000 ms after pen-up: `setRawDrawingEnabled(false)` → `invalidate(view, GC)` → `(true)`                                                                                                                        | whole view                                                                                | none; the author's comment concedes it "still sometimes fails"                                                                |
| **mokke**   | `syncOverlay(force)` = `setRawDrawingEnabled(false)` then `(true)`, deferred one vsync via a Choreographer callback                                                                                                  | unioned regions, `null` = whole surface                                                   | **explicit**: deferred while `daemonStrokeInProgress`, replayed from `onStrokeEnd`                                            |

### 6a. What stock actually does

```java
// stock/note/note/handler/common/EpdShapeHandler.java:248-262
private final void P0() {
    List<ShapeGrayscaleBean> listB0 = B0();
    if (!listB0.isEmpty() && OnyxSystemConfig.isUseHWUpdateForPenUpRefresh()) {
        RectF rectF = new RectF();
        for (ShapeGrayscaleBean b : listB0) rectF = unionOrCopy(rectF, b.getA());
        if (!noteDocViewInfo(editorBundle).getF()) {              // isEnabledPenDirtyRect
            rectF = toRectF(scribbleRect(editorBundle));          // whole writing area
            noteDocViewInfo(editorBundle).setEnabledPenDirtyRect(true);
        }
        new GrayscaleRefreshAction(this.editorBundle, rectF).execute();
    }
}
```

```java
// stock/note/note/action/render/GrayscaleRefreshAction.java:58-71
final View b = getK().getB().getB();
EpdController.setViewDefaultUpdateMode(b, UpdateMode.HAND_WRITING_REPAINT_MODE);
DisplayActionsKt.displayNoteDirtyRect$default(this, list, this.q, false, 4, null)
    .doFinally(() -> GrayscaleRefreshAction.y(b))   // resetViewUpdateMode
    .map(...)
```

`displayNoteDirtyRect` ends in a plain `view.invalidate()`. **There is no `handwritingRepaint`, no
`refreshScreenRegion`, no pen-state transition and no sleep in the stock ink path.**
`EpdController.handwritingRepaint` has exactly one caller in the whole APK, a data-binding adapter
for ordinary UI views, and never runs on the canvas (stock REPORT line 194, 208). `34dc906` deleted
our call for that reason (`kt/OnyxInk.kt:484-490` carries the reasoning).

`P0()` is called from two places and only acts when both have happened — `onPenUpRefresh`
(`EpdShapeHandler.java:365-373`) and the bitmap-flush completion (`:215`) — because `B0()`
(`:126`) only takes beans whose `renderToBitmap` is true _and_ whose rect is non-empty, and leaves
the rest for the next round. `kt/InkReconcileGate.kt` reproduces this shape, with two additions
stock never needs: an erasing gesture receives no pen-up refresh at all, and a firmware that never
delivers the callback must not deadlock the reconcile (`kt/InkReconcileGate.kt:12-18`).

### 6b. The pen-up refresh, and what disabling it cost us

```java
// pen/RawInputReader.java:131-137
private int f61n = 500;                       // penUpRefreshTimeMs
private volatile boolean f62o = true;         // penUpRefreshEnabled — DEFAULT ON
```

```java
// pen/RawInputReader.java:604-616  (pen-up path)
if (erasing) { return; }
m37l();                                       // arms the timer only for non-erasing strokes
```

```java
// pen/RawInputReader.java:744-759  — the rect handed to onPenUpRefresh
RectF rectF = new RectF(this.f65r);           // this stroke's accumulated bounds
RectUtils.expand(rectF, this.f57j);           // expanded by stroke width
if (this.f66s != null) rectF.union(this.f66s); // unioned with the PREVIOUS stroke
this.f66s = new RectF(this.f65r);
```

The union with the previous stroke is what guarantees that consecutive repaints **overlap**, so a
region latched by one stroke is covered again by the next. That property is free if you use the
callback, and impossible to reconstruct from a JS damage rectangle.

`TouchHelper.setPenUpRefreshEnabled` is `@Deprecated` (`pen/TouchHelper.java:252-257`) and stock
never calls it — it only re-applies `setPenUpRefreshTimeMs` on every resume
(`ResumeRawDrawingRequest.java:109`). **S1 through S4 called `setPenUpRefreshEnabled(false)`**
(prototype `OnyxInk.kt:102`, still at `f74ace4:OnyxInk.kt:273`), with the comment "Refresh only
after the web canvas acknowledges its frame." That single line removed the firmware's own
rendezvous signal for four of the five states, leaving the reconcile driven purely by
`postVisualStateCallback` — which, as the stock report notes, is a WebView _readiness_ signal, not
proof the ink is in a composited buffer. `34dc906` dropped the call and drives the reconcile from
`onPenUpRefresh` (`kt/OnyxInk.kt:333-336`, `:466-482`, `:803-808`).

Also worth knowing: `TouchHelper.setFilterRepeatMovePoint(boolean)` is wired to
`TouchRender.setPenUpRefreshEnabled(filter)` (`pen/TouchHelper.java:260-265`) — a genuine SDK bug.
Anyone calling `setFilterRepeatMovePoint(false)` silently disables the pen-up refresh. We do not
call it; nothing here depends on it, but it is a trap.

### 6c. On a colour tablet, stock does not reconcile per stroke at all

```java
// stock/sdk/data/config/system/OnyxSystemConfig.java:145-150
public static boolean isUseHWUpdateForPenUpRefresh() {
    if (SystemPropertiesUtil.isTablet()) { return false; }
    return a().enablePenUpRefresh;
}
```

```java
// stock/sdk/data/config/system/data/SystemConfigBean.java:21-22
public int penUpRefreshTimeMs = 500;
public boolean enablePenUpRefresh = !DeviceInfoUtil.isColorDevice();
```

`isTablet()` reads the system property `vendor.onyx.tablet`
(`stock/sdk/utils/SystemPropertiesUtil.java:57-59`). On a Note Air 4C — a colour panel, and almost
certainly a "tablet" by that property — **`P0()` is a no-op**: stock leaves firmware ink standing
after every stroke and never marks a handwriting-repaint frame while writing. What actually heals
the panel there is the render-flag cycle described in §7, which stock runs constantly. This is a
setting (`stock/sdk/note/ui/setting/viewmodel/SettingViewModel.java:111`), so a user can turn it on,
but it is off by default on this hardware.

This matters for HEAD: `34dc906` aligned our _mechanism_ with stock's, but stock does not run that
mechanism on this device. Our per-stroke reconcile is a deliberate departure — we need it, because
unlike stock's `EditorView` our page lives in a WebView whose content the firmware ink hides — but
it is not what the stock app does here, and no stock-derived timing evidence covers it.

### 6d. Region choice at HEAD

`kt/InkReconcileRegion.kt` implements both stock rules: the stroke ∪ previous-stroke union
(`:48-64`), and a whole-region fallback armed by `invalidateAll()` (`:39-42`) whenever anything else
drew over the panel — which stock does by clearing `isEnabledPenDirtyRect`
(`stock/note/note/eventhandler/DeviceReceiverEventHandler.java:89`). We arm it in `pause()`
(`kt/OnyxInk.kt:585`), on an overlay-only reconfigure (`:301`) and on `close()`/`reset()`.
`25f92c3` added `putBack` (`kt/InkReconcileRegion.kt:66-70`): a region consumed by a frame that
never reached the panel goes back on the pile, because an e-ink panel keeps whatever was last pushed
to it and would otherwise show a stale image there for good.

---

## 7. Update modes and refresh policy

|             | view default mode                                                                                  | transient mode                                                                                        | app-scope profile                                                                                                | GC / full refresh                                                                                                                                                                        |
| ----------- | -------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **S1**      | none                                                                                               | none                                                                                                  | none                                                                                                             | none                                                                                                                                                                                     |
| **S2**      | layered stack: `BASE`=REGAL/GU profile, `SESSION`=GU, `TRANSIENT`=`HAND_WRITING_REPAINT_MODE`      | `applyTransientUpdate(ANIMATION_QUALITY)` for any interaction/eraser gesture, cleared after 5 s quiet | none                                                                                                             | `requestFullRefresh` (GC) after a 300 ms settle, on navigation/overlay close                                                                                                             |
| **S3**      | same                                                                                               | same                                                                                                  | same                                                                                                             | + one GC when the keyboard hides                                                                                                                                                         |
| **S4**      | same, plus a short-lived `TEXT` DU layer (`e1a9da7`) reverted in `65885dc`                         | same                                                                                                  | `setAppScopeRefreshMode(FAST)` probed (`419ac86`); EinkWise "Speed" written into the stored EAC JSON (`2d3ee10`) | + one GC when the sheet closes                                                                                                                                                           |
| **S5**      | same                                                                                               | `ANIMATION_QUALITY` **only for a selection drag**                                                     | same                                                                                                             | same                                                                                                                                                                                     |
| **stock**   | `HAND_WRITING_REPAINT_MODE` only around the reconcile frame                                        | `ANIMATION_QUALITY` only around selection transforms, 5 s auto-exit                                   | untouched                                                                                                        | `applyGCWithInterval` counted on page turns and menu actions, never on a stroke                                                                                                          |
| **notate**  | `setViewDefaultUpdateMode(decorView, DU)` + `setDisplayScheme(SCHEME_SCRIBBLE)`                    | `applyTransientUpdate(ANIMATION)` + `setEpdTurbo(true)` via reflection                                | none                                                                                                             | explicit `requestEpdRefresh(GC)`                                                                                                                                                         |
| **notable** | tries `HAND_WRITING_REPAINT_MODE` at surface init, falls back to `REGAL` (`einkHelper.kt:119-124`) | `applyTransientUpdate(ANIMATION_X)` in an arbiter                                                     | `setAppScopeRefreshMode(UpdateOption.NORMAL)` (`einkHelper.kt:38-41`)                                            | `repaintEveryThing(REGAL_PLUS)`; per partial update `setDisplayScheme(SCHEME_SCRIBBLE)` + `enableA2ForSpecificView` + `setEpdTurbo(100)` + a render-flag cycle (`einkHelper.kt:283-290`) |
| **PngNote** | none                                                                                               | none                                                                                                  | none                                                                                                             | none                                                                                                                                                                                     |
| **saber**   | none                                                                                               | none                                                                                                  | none                                                                                                             | `invalidate(view, GC)` on arm/disarm                                                                                                                                                     |
| **mokke**   | none                                                                                               | none                                                                                                  | none                                                                                                             | `invalidate(view, DU)` after a repaint                                                                                                                                                   |

Notes.

- `DisplayModeStack` (`kt/DisplayModeStack.kt`) exists because three owners wanted the view's default
  mode and the last writer won — a closed editor used to leave the whole app in its writing mode.
  The stack restores the _raw_ pre-capture value through `View.setDefaultUpdateMode(int)` rather than
  an enum (`kt/ViewDisplayMode.kt:39-50`), because the firmware's "inherit the system default"
  sentinel has no `UpdateMode` value.
- Stock's transient mode is bounded to selection transforms: `StartTransformAction:227` posts
  `ApplyFastModeEvent(true)`, `QuitTransformAction:545` posts `false`, and `EpdEventHandler:109-124`
  turns that into `applyTransientUpdate(ANIMATION_QUALITY)` with a 5 s auto-exit (recorded in
  `ours/docs/planning/handwriting-lasso-notes.md:107-117`). S2–S4 requested it for _any_ interaction,
  eraser or erasing gesture (`b9e3342:OnyxInk.kt` `displayPolicy.begin((args.interaction || args.eraser
|| erasing) && !fastPreview)`); `34dc906` narrowed it to a selection drag (`kt/OnyxInk.kt:724-726`).
- `EpdController.applyTransientUpdate` discards the firmware's success boolean, so we call
  `Device.currentDevice().clearTransientUpdate(false)` directly to retain it
  (`kt/OnyxInk.kt:150-155`), and clear any transient mode a crashed session left behind at
  construction (`:236-237`).
- The app-scope profile work (`419ac86`, `486fd20`, `2d3ee10`) is a genuine divergence from
  everything else here. It exists because the firmware refreshes the caret, touches and scrolling by
  the per-app EinkWise profile, which Regal turns into a full flash. The runtime call is guarded by a
  reflection probe first, because `SDMDevice` silently substitutes the **device-wide**
  `setSystemRefreshMode` when `EInkHelper.setAppScopeRefreshMode` is missing
  (`ours/plugins/.../MobileSystemPlugin.kt:302-322`; hazard documented in SDK REPORT §1). When the
  runtime call turned out to do nothing on this firmware, `2d3ee10` moved to writing the stored EAC
  JSON instead (`MobileSystemPlugin.kt:264-292`).
- Stock's fast mode is more than an update mode: `EpdEventHandler.a()` also sets the dither threshold
  to `isColorDevice ? 160 : 255` and calls `setEpdTurbo(deviceConfig.epdTurbo)` on entry, restoring
  the threshold to 128 on exit, and holds a wake-lock for the 5 s tail
  (`stock/note/note/eventhandler/EpdEventHandler.java:80-82,109-142`). We change neither dither nor
  turbo.
- **Stock does not touch `setDisplayScheme` or `setAppScopeRefreshMode` in the note editor at all** —
  they appear only in test activities and the material-centre browser. notate uses both, notable has
  both in a `prepareForPartialUpdate()` that turns out to have no caller
  (`ref/notable/.../einkHelper.kt:282-289`). Onyx's own SDM path abandoned display schemes:
  `EpdSDM.useFastScheme()` is an empty override. **Unexamined for us.**
- **Two diagnostics that can lie.** `EpdController.getViewDefaultUpdateMode` returns `UpdateMode.GU`
  when the reflective read _fails_
  (`.../onyxsdk-device-1.3.5.2/sources/com/onyx/android/sdk/device/SDMDevice.java:1947-1951`), so
  `status()`'s `viewUpdateMode` cannot distinguish "GU" from "could not read"; `readRaw()`
  (`kt/ViewDisplayMode.kt:65-67`) is the trustworthy one. And `UpdateMode` values are mapped to
  firmware ints by reflecting framework constants, where a missing constant yields **0** with no
  error; only the three REGAL modes have a fallback. `HAND_WRITING_REPAINT_MODE` has none, so on a
  firmware lacking that constant our whole reconcile would silently mark frames as mode 0. Comparing
  `lastRepaintModeRaw` against a known-good mode's raw value would settle it; we log it
  (`kt/OnyxInk.kt:136`) but never check it.
- Most of the `EpdController` facade discards the success boolean that `SDMDevice` computes —
  `setViewDefaultUpdateMode`, `resetViewUpdateMode`, `setDisplayScheme`, `setAppScopeRefreshMode`,
  `applyAppScopeUpdate` all return a hardcoded `true`. `applyTransientUpdate` is the exception and
  propagates, which is why `kt/OnyxInk.kt:146` can record `fastModeAccepted` at all.

---

## 8. Pause conditions

|             | IME                                                  | dialogs / popups                                                    | focus                                                                     | tool change                                       | lifecycle                                | resume delay                                                         |
| ----------- | ---------------------------------------------------- | ------------------------------------------------------------------- | ------------------------------------------------------------------------- | ------------------------------------------------- | ---------------------------------------- | -------------------------------------------------------------------- |
| **S1–S2**   | **none**                                             | none                                                                | `onWindowFocusChanged`                                                    | via `configure()`                                 | `onPause`/`onResume`                     | 0                                                                    |
| **S3–S5**   | window insets → `"ime"` reason                       | JS `suppress_onyx_ink("overlay:<id>")`, released 300 ms after close | `onWindowFocusChanged`                                                    | `configure()`, deferred while a gesture is open   | `onPause`/`onResume`                     | **150 ms mono / 500 ms colour**                                      |
| **stock**   | Onyx broadcast + `KeyboardHookPopup`                 | counter + named `Set<String>` registry                              | `ActivityFocusChangedEvent`; `ScribbleTouchDistributor` also drops events | `canPenRawRender()`/`isUsePenInput()` per handler | bind/attach                              | 150/500 ms; popups 300/500 ms; selection 500 ms; shape change 100 ms |
| **notate**  | **none** (`adjustResize`, no inset code)             | pen popup & sidebar disable; text/link dialogs do nothing           | —                                                                         | TEXT tool kills render only                       | `surfaceCreated`/`Destroyed`             | —                                                                    |
| **notable** | **none**                                             | modal flag → `isDrawingAllowed`                                     | `onWindowFocusChanged` is the backstop                                    | —                                                 | —                                        | —                                                                    |
| **PngNote** | none (no text input on that screen)                  | none                                                                | `onWindowFocusChanged(true)` re-arms                                      | —                                                 | `onStop` closes                          | —                                                                    |
| **saber**   | indirect: `Tool.textEditing` → disabled stroke style | **nothing** — ink paints over dialogs                               | —                                                                         | tool-driven                                       | —                                        | —                                                                    |
| **mokke**   | designed out (`stateHidden`)                         | always `pauseRawDrawing()` for the dialog's lifetime                | —                                                                         | —                                                 | `closeRawDrawing()` for child activities | —                                                                    |

Notes.

- The IME never takes our window focus — an input-method window is `FLAG_NOT_FOCUSABLE` — so
  `hasWindowFocus()` cannot see it, while the firmware happily paints ink over it because the limit
  rect is a _screen_ region with no notion of window z-order. This is the reference REPORT's headline
  finding, and it is why S1/S2 had a dead keyboard: the pen kept drawing over it while the CTP cutout
  killed the taps meant for it. We now watch `WindowInsetsCompat.Type.ime()` from the plugin, which
  owns the WebView's single insets listener (`ours/plugins/.../MobileSystemPlugin.kt:70-77`,
  `:94-102`).
- Named reasons. `kt/InkPauseRegistry.kt` is deliberately the same shape as stock's
  `PausePenEvent(className)` / `ResumePenEvent(className)` set (stock REPORT §2, "Dialogs, popups"):
  resumed only when the set is empty, so a doubly reported keyboard or two overlapping overlays
  cannot flap the pen. The page pushes reasons through
  `ours/src/app/ink-suppression.ts` — a `focusin`/`focusout` watch for editable elements
  (`:63-88`, with a deferred release so moving between two fields does not flap) and
  `useOverlayInkSuppression` for the shared Dialog/Popover/Select/menu wrappers
  (`:98-114`, `OVERLAY_RESUME_MS = 300`).
- **The colour-panel 500 ms.** `RawPenArgs.DELAY_ENABLE_RAW_DRAWING_MILLS = isColorDevice() ? 500 : 150`
  (stock REPORT §2). `kt/OnyxInk.kt:209-225` reproduces it, probing
  `Device.currentDevice().colorType > 0` once and defaulting to "colour" — hence the longer, safer
  delay — when the probe fails. Before `34dc906` we used a flat 150 ms constant
  (`5b61507:OnyxInk.kt:149`).
- `pauseStateChanged` (`kt/OnyxInk.kt:255-266`) is modelled on stock's `InvalidateScreenAction`:
  pause render and input, `webView.invalidate()` so the app's own content covers whatever the
  firmware left, then resume after the delay. The `invalidate()` in `pause()`
  (`kt/OnyxInk.kt:587-589`) was added in `34dc906` for exactly that reason.
- Stock's delay constants, all from `stock/sdk/notecore/editor/data/RawPenArgs.java:68-70` and
  `stock/note/note/event/PenEvent.java:8-21`: `DELAY_ENABLE_RAW_DRAWING_MILLS = isColorDevice() ? 500
: 150`, `POPUP_RESUME_PEN_TIME_MS = isColorDevice() ? 500 : 300`, `SELECTION_RESUME_PEN_TIME_MS =
500` and `SHAPE_CHANGE_RESUME_PEN_TIME_MS = 100` — the last two currently have no callers. The
  delay is applied as a blocking `ThreadUtils.mySleep(delay)` on a dedicated pen scheduler before the
  rects are re-pushed (`stock/note/note/request/pen/ResumeRawDrawingRequest.java:127`); ours is a
  `postDelayed` on the main looper, which is the right shape for a single-threaded adapter but means
  our resume competes with rendering work rather than sleeping beside it.
- Our overlay release of 300 ms (`ours/src/app/ink-suppression.ts:18`) happens to match stock's
  `POPUP_RESUME_PEN_TIME_MS` for a monochrome panel; on this colour device stock would use 500 ms.
  Unexamined.
- Note also that stock's `ResumeRawPenAction` does not call the SDK directly — it posts a `PenEvent`
  back onto the bus, which lands in `PenEventHandler` and re-evaluates the full predicate. So every
  resume in stock is a fresh policy decision, never a stored intention. `InkPauseRegistry` plus the
  guards at the top of `resume()` (`kt/OnyxInk.kt:559-560`) is the same idea in a much smaller space.
- A `configure()` while a gesture is open would pause raw drawing and truncate the stroke, so
  `useOnyxInk` defers it until pen-up (`ours/src/features/handwriting/onyx-ink.ts:157-167`,
  released at `:253-257`). `4c465bb` added the native `gestureOpen` flag because the React
  `interacting` prop lags a render.

---

## 9. Eraser

|                                        | hardware eraser                                                                       | side button                                                                                                               | render during erase                                                                                                              |
| -------------------------------------- | ------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| **S1**                                 | `onBeginRawErasing` → same gesture path; points sent with `erasing`                   | `enableSideBtnErase(true)`, `setEraserRawDrawingEnabled(false, 0)`                                                        | firmware render **stays on**                                                                                                     |
| **S2–S5**                              | same                                                                                  | same                                                                                                                      | `InkEraserRenderGate` pauses firmware render at erase-begin and only restores it once the erased canvas frame has been presented |
| **stock**                              | `EraseHandler.isUsePenInput()` is unconditionally `true`; render gated by eraser type | **never calls `enableSideBtnErase`** — side button arrives only as the `shortcut*` flags, which the shape handler ignores | firmware renders the eraser trace; under the selection provider the eraser channel becomes the same dashed renderer              |
| **notate/notable/PngNote/saber/mokke** | app-side erase from raw points                                                        | —                                                                                                                         | not gated                                                                                                                        |

Notes.

- `setEraserRawDrawingEnabled(false, 0)` (`kt/OnyxInk.kt:341`) turns the firmware's eraser ink off so
  the eraser leaves no trace of its own; `enableSideBtnErase(true)` (`:342`) still routes the side
  button to the erasing callbacks. `resetPenDefaultRawDrawing()` resets these to
  `(true)` / `(false, 5)` (SDK REPORT §3), which is why `RawDrawingGate.resume` re-runs it and the
  eraser configuration is re-applied only at helper creation — **a resume after a pause therefore
  restores eraser style 5, not 0.** Unexamined; it has not been observed to matter because
  `setEraserRawDrawingEnabled(false, …)` disables the channel either way.
- The gate (`kt/InkEraserRenderGate.kt`) exists because hardware erasing must pause the firmware pen
  layer even while Pen is the selected tool, while raw _input_ must keep flowing so the software
  eraser receives points (`kt/OnyxInk.kt:717-719`). It resets on any lifecycle pause so a shutdown
  can never re-enable pen rendering (`kt/InkEraserRenderGate.kt:23-25`).
- An erasing gesture receives **no** `onPenUpRefresh` (`pen/RawInputReader.java:613-616`), so the
  HEAD reconcile gate must not wait for one — `kt/InkReconcileGate.kt:44-48` sets
  `expecting = !erasing && penUpRefreshSeen`.
- Stock's eraser policy is the clearest example of the render/input split being used as a _tool_
  mechanism rather than a pause mechanism: `EraseHandler.isUsePenInput()` returns `true`
  unconditionally while `canPenRawRender()` is `EraseUtils.isEraseTrackByRawRender(eraserType)`
  (`stock/note/note/handler/scribble/EraseHandler.java:35-53`), and entering the tool runs a
  pause/resume with an explicit **150 ms** delay (`:60-66`). Its firmware parameters are re-applied on
  every resume, including the undocumented stroke-eraser style **8**
  (`stock/note/note/request/pen/ResumeRawDrawingRequest.java:64-69, 84-87`) — the same style notable
  uses (`ref/notable/.../einkHelper.kt:178-259`) and which `TouchHelper` does not expose as a
  constant. We use neither style 8 nor `Device.setStrokeParameters` for the eraser.
- **Stock never calls `enableSideBtnErase`.** `TouchHelper.enableSideBtnErase`
  (`pen/TouchHelper.java:108-113`) has zero callers in the APK; the side button reaches the app only
  as the `shortcutDrawing`/`shortcutErasing` booleans, which `EpdShapeHandler.onBeginRawDraw` ignores.
  We call it (`kt/OnyxInk.kt:342`), and it writes two places — the native reader _and_
  `Device.setEnablePenSideButton`, a **system-wide firmware setting that nothing in the SDK restores
  on close**. Unexamined: we may be leaving that flag set for other apps.

---

## 10. Lasso and selection

|                                        | live trace                                                                               | selection outline                                                                   | floating menu                                                                                                                                                                                   |
| -------------------------------------- | ---------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **S1**                                 | none (no lasso)                                                                          | —                                                                                   | —                                                                                                                                                                                               |
| **S2** (`5c63479`, `8a46ef9`)          | firmware, `STROKE_STYLE_PENCIL` — indistinguishable from ink                             | app-drawn                                                                           | in the tool-options popover                                                                                                                                                                     |
| **S3**→`3e0fc6a`                       | firmware `STROKE_STYLE_DASH` + `Device.setStrokeParameters(DASH, [5f])`                  | app-drawn                                                                           | popover                                                                                                                                                                                         |
| **S4** (`93c5724`)                     | firmware dash                                                                            | app-drawn, stock proportions                                                        | floating toolbar, **published as a firmware exclude rect**                                                                                                                                      |
| **S4** (`95e1b58`+)                    | firmware dash                                                                            | app-drawn dashed frame, padding 9 / frame 2 / trace 3 / dashes 7-7                  | floating toolbar as a **page overlay**: a pen-down on it is swallowed for that gesture, the region stays whole                                                                                  |
| **S5**                                 | same                                                                                     | same                                                                                | same; menu box included in the repaint damage, follows a drag one animation frame at a time                                                                                                     |
| **stock**                              | firmware `STROKE_STYLE_DASH`, `setStrokeParameters(5, [5.0f])`, inheriting the pen width | app-drawn dashed box, `DashPathEffect({10,10})`, stroke 3, padding 12, plus handles | `SelectionPopupMenu`, a plain `PopupWindow`, repositioned every ~10 ms during the drag, **registers no exclude rect** and overrides `windowChangeEvent` to a no-op so it does not pause the pen |
| **notate/notable/PngNote/saber/mokke** | no firmware lasso trace                                                                  | —                                                                                   | —                                                                                                                                                                                               |

All the stock citations are in `ours/docs/planning/handwriting-lasso-notes.md` (written from
`stock/sdk/notecore/editor/render/SelectionRenderer.java`,
`stock/note/note/menu/popup/SelectionPopupMenu.java:187-189`,
`stock/note/note/utils/SelectionPopupMenusKt.java:33-45`,
`stock/note/note/request/pen/ResumeRawDrawingRequest.java:52-59,91-103`).

The exclude-rect episode is the important one. `93c5724` pushed the menu's box through
`setLimitRect(limit, excludes)` "the way the stock Notes app registers its floating menu" — but the
stock app registers exclude rects for its _draggable pen toolbar_ (`ExcludeRectType` slot 3), not for
the selection popup, which has no slot at all. The menu sits just above the selection, so a lasso
drawn near an existing selection lost exactly the band of its dashed trace that crossed the menu.
`95e1b58` replaced it with `InkOverlayRects` — a JS-side hit test in the plugin that swallows one
gesture (`kt/OnyxInk.kt:705-715`, `kt/InkOverlayRects.kt:16-27`) — so the pen tap reaches the DOM
button while the handwriting region, and every trace through it, stays whole.

One nuance the lasso notes did not capture: stock's selection popup does not pause the pen, but stock
_does_ pause it for the transform itself. `SelectionHandler` posts
`PausePenEvent(SelectionHandler.class.getName())` when a transform begins and `ResumePenEvent` when it
completes (`stock/note/note/handler/scribble/SelectionHandler.java:533-537`, `:690-693`), and it is
also the one handler in the whole app that returns `true` from `supportFingerDraw()` (`:2881-2884`) —
which is why a finger can drag a selection there, and the reason `c5baff9` added the same affordance
here. Our equivalent is `interaction`/`hasSelection` gating render rather than pausing
(`kt/OnyxInk.kt:596-597`), which keeps raw input flowing for the drag preview.

None of the five reference apps has a firmware lasso trace. notate comes closest — it reuses the
_eraser_ channel for a hardware dashed trace (`setEraserRawDrawingEnabled(true, STROKE_STYLE_DASH)`
plus `setStrokeParameters(DASH, [5f, 5f])`), which is exactly what stock does under the selection
provider; notate's software `CursorView.updateLassoPath` is dead code. saber is the cautionary case:
it maps the select tool to a _pen_ stroke style rather than disabling it, so the firmware paints a
plain black line under the Flutter-drawn dashed outline until the 1 s GC invalidate wipes it.

---

## 11. An axis the evidence added: what heals the panel between strokes

Stock's real de-ghosting workhorse is not the per-stroke reconcile (which, per §6c, is off on this
device) but `InvalidateScreenAction`
(`stock/note/note/action/render/InvalidateScreenAction.java:29-38,85` →
`InvalidateViewWithPenControlAction.java:31-52`): pause render **and** input →
`renderManager.invalidate()` → resume both after `DELAY_ENABLE_RAW_DRAWING_MILLS`. That is precisely
the `setRawDrawingRenderEnabled(false)` → `leaveScribbleMode` + `PEN_PAUSE` → `PEN_DRAWING` cycle
that was observed to bring a dead rectangle back. It is bound to roughly thirty call sites — every
menu, popup, toast, tool change, pen change, page refresh, finger-touch toggle — so a stock user
tears down and rebuilds the firmware scribble state every few seconds of normal use.

notable arrived at the same primitive independently. Its `prepareForPartialUpdate`
(`ref/notable/app/src/main/java/com/ethran/notable/editor/utils/einkHelper.kt:283-290`) ends with

```kotlin
touchHelper.isRawDrawingRenderEnabled = false
touchHelper.isRawDrawingRenderEnabled = true
```

— a bare render-flag cycle, run before every partial update — and its
`resetScreenFreeze(touchHelper)` name for the same idea says what it is for. saber does the coarser
version with `setRawDrawingEnabled(false); setRawDrawingEnabled(true)` on every layout change
(`ref/saber/packages/onyxsdk_pen/android/src/main/kotlin/com/example/onyxsdk_pen/OnyxsdkPenArea.kt:249-250`),
PngNote wraps every surface repaint in `withTempNoRawRendering`, and mokke's own architecture notes
state the rule outright: "while TouchHelper has raw drawing enabled, nothing else is allowed to
refresh the panel… pause first, refresh, re-enable when ready"
(`ref/mokke-ink-notes/third_party/inksdk/docs/eink-pen-architectures.md:246-256`).

**Every implementation surveyed except ours cycles the render flag around its own drawing.** We only
cycle it on a named pause reason, on the eraser gate, and on `configure()`. A long, uninterrupted
writing session in our editor never resets the firmware's scribble state at all. With multi-region
mode also in force through S4, that is the most complete available explanation for a rectangle that
stops taking ink and only a render-flag toggle revives.

---

## 12. Chronological divergences from stock, and what each cost

1. **`setPenUpRefreshEnabled(false)` from the prototype onward** (`c8657b9`
   `OnyxInk.kt:102`, still present at `3e0036e`). The SDK enables it by default
   (`pen/RawInputReader.java:134`) and stock never touches the deprecated setter, only the interval
   (`ResumeRawDrawingRequest.java:109`).
   _Consequence:_ four of the five states had no firmware-side rendezvous signal. The reconcile fired
   on a Chromium readiness callback plus a fixed 120 ms, which is not proof the ink is in a composited
   buffer, and the region we repainted was a JS damage rectangle inflated by 2 px rather than the
   SDK's stroke ∪ previous-stroke union — so consecutive repaints were not guaranteed to overlap and
   a latched region was never re-covered. Fixed in `34dc906`.

2. **`setAppCTPDisableRegion` armed over the whole limit rect on every pen-down** (`c8657b9`
   `OnyxInk.kt:201-218`, through `b9e3342`). No stock or reference-app precedent.
   _Consequence:_ every capacitive touch inside the sheet band was killed for the duration of a
   gesture — and indefinitely whenever the firmware skipped an `onEndRawDrawing` — which is why the
   soft keyboard was untappable over the sheet. Removed in `5b61507`.

3. **No IME signal at all** (`c8657b9` through `b9e3342`). The only pause source was
   `onWindowFocusChanged`, which an input-method window never triggers.
   _Consequence:_ the pen kept painting firmware ink over the keyboard, above every window. Fixed in
   `5b61507` with window insets and the named pause registry.

4. **`setRawDrawingEnabled` used as the pause switch** (`c8657b9`–`b9e3342`). Stock keeps render and
   input as two switches and only collapses them when both flags coincide (stock REPORT §2).
   _Consequence:_ a paused pen was indistinguishable from a closed session, no rects were re-pushed
   on resume, and the resume order was wrong (render and input together, no delay), which the SDK's
   own resume path avoids by arming input first and render last after 150/500 ms. Fixed in `5b61507`
   (`kt/RawDrawingGate.kt`).

5. **`EpdController.handwritingRepaint(view, region)` as the handover** (`b9e3342` through
   `3e0036e`, `OnyxInk.kt:386`). Stock never calls it on the canvas; its one caller in the APK is a
   data-binding adapter for ordinary views.
   _Consequence:_ measured to be ineffective — a `handwritingRepaint` call after the fact does not
   remove fast ink on this firmware. The mode-marked _submitted frame_ is the mechanism. Removed in
   `34dc906`.

6. **Never calling `setSingleRegionMode()`** (`c8657b9` through `3e0036e`). `TouchHelper` sets no
   region mode at construction, so both halves stayed in mode 0; stock re-applies single-region mode
   on **every** resume (`ResumeRawDrawingRequest.java:110`), and notate sets it once
   (`ref/notate/.../OnyxCanvasView.kt:893`). No app anywhere calls `setMultiRegionMode`.
   _Consequence:_ on the native side, none — with one limit rect the two modes are equivalent (§2c).
   On the firmware side the effect is unknown from the sources, and the hypothesis that mode 0 makes
   the firmware accumulate one transient scribble region per stroke until the table fills — after
   which only a teardown of the scribble state recovers those rectangles — remains the best available
   explanation for the observed symptom, but is not proven. Changed in `34dc906`
   (`kt/OnyxInk.kt:339`, `:113-122`), untested on hardware.

7. **`applyTransientUpdate(ANIMATION_QUALITY)` while writing or erasing** (`b9e3342` through
   `3e0036e`). Stock scopes it to selection transforms only.
   _Consequence:_ the whole app surface was put into an animation mode during ordinary handwriting,
   which is both wrong for ink quality and an unnecessary global state to leave behind if a session
   dies. Narrowed to a selection drag in `34dc906` (`kt/OnyxInk.kt:724-726`).

8. **The floating selection menu published as a firmware exclude rect** (`93c5724`, live until
   `95e1b58`). Stock's `SelectionPopupMenu` registers no exclude rect; only the draggable pen toolbar
   does.
   _Consequence (immediate):_ a hole in the handwriting region is also a hole in every trace crossing
   it, so a lasso drawn near an existing selection lost the segment of its dashed outline that ran
   through the menu band.
   _Consequence (lasting, and the most specific dead-rectangle candidate here):_
   `RawInputReader.setExcludeRect` returns early on an empty list (`pen/RawInputReader.java:418-421`),
   so no later build's `emptyList()` can withdraw the exclusion from the **native evdev filter**. But
   because those builds run mask 3, the `AppTouchRender` half — which has no empty guard
   (`pen/touch/AppTouchInputReader.java:151-157`) — _did_ clear the **firmware** half. The result on
   any device that ran a build from that window is a band above the last selection where the firmware
   still paints ink and the native reader still filters every point out: the pen appears to write,
   nothing is stored, and the next reconcile wipes it. It survives closing the editor — the native
   `PenManager` is process-global and `closeRawDrawing` does not clear it — but not restarting the
   app. The Java side of this is directly readable; the native filtering semantics come from
   disassembling `libonyx_pen_touch_reader.so`, so treat the exact behaviour as strongly indicated
   rather than proven. Our own clear path (`kt/OnyxInk.kt:183-189`) would not work as written — §2a.

   **This is the one item here that is worth testing first**, because it predicts something specific
   and cheap to check: on an affected device the symptom should be a band, at the height of a past
   selection menu, that survives navigation but disappears after a force-stop.

9. **A reconfigure allowed to run mid-gesture** (up to `4c465bb`). `configure()` pauses raw drawing;
   hiding or moving the menu changed the args and reconfigured while the pen was down.
   _Consequence:_ the pause truncated the firmware lasso trace, so segments were missing from the
   outline exactly where the menu had been. Fixed by deferring the reconfigure to pen-up
   (`4c465bb`) and by making an overlay-only change update in place without a restart (`f74ace4`,
   `kt/OnyxInk.kt:290-303`).

10. **Committing a frame while the pen was down** (up to `b5c9382`). Compositing the web canvas over
    a trace the firmware is still drawing erases the part drawn before that moment.
    _Consequence:_ the first portion of every lasso disappeared, always in the same region because a
    shape is started the same way each time. Fixed by holding the commit until the gesture ends.

11. **Damage consumed before the repaint was known to have happened** (up to `25f92c3`).
    _Consequence:_ a frame dropped by the fence, by the lifecycle or by an empty intersection left
    that area showing an older image indefinitely — rectangular gaps in dense ink and a missing
    sector of a lasso trace. Fixed by returning unrepainted damage to the accumulator
    (`kt/InkReconcileRegion.kt:66-70`).

12. **Every gesture's reconciliation held back** (up to `3e0036e`). Firmware ink is transient and
    must be taken over promptly; deferring it left whole rectangles showing what the panel held
    before. Only a lasso trace, which the canvas cannot reproduce, still waits for pen-up.

---

## 13. Remaining divergences from stock at HEAD (`34dc906`)

**Deliberate.**

- _We reconcile per stroke; stock (on this colour tablet) does not._ §6c: `isUseHWUpdateForPenUpRefresh()`
  returns false for a tablet or a colour device, so stock's `P0()` never runs here. We need the
  handover because our page content lives under the firmware ink layer in a WebView, not in a
  view stock can simply leave alone. Reason: without it the sheet shows firmware ink that no
  scroll, zoom or undo can update.
- _We drive the reconcile off `onPenUpRefresh` with a timeout and an erasing fallback_
  (`kt/InkReconcileGate.kt:12-18`). Stock needs neither: it never erases through this path and its
  firmware always delivers. Reason: an erasing gesture provably gets no pen-up refresh
  (`pen/RawInputReader.java:613-616`), and a firmware that never delivers would deadlock us.
- _We invalidate the whole WebView rather than a writing-region view._ `kt/OnyxInk.kt:533-536`
  explains it: stock's editor view _is_ its writing region, ours is the whole screen. The mode is
  reset the moment the frame lands, so the over-coverage only helps a latched region.
- _No exclude rects at all_ (`kt/OnyxInk.kt:176-181`). Stock uses ten typed slots; we deliberately
  never punch the region, and handle floating controls by swallowing one gesture instead, because an
  exclusion cuts every trace crossing it and cannot be withdrawn.
- _`setPostInputEvent(false)`_ (`kt/OnyxInk.kt:337`). Stock sets `true` to get pen-proximity events.
  We subscribe to nothing, so posting them would only cost work.
- _No `setAppCTPDisableRegion`_ (`kt/OnyxInk.kt:637-647`). Matches stock and all five apps.
- _Pause reasons come from the page, not from Android._ Stock can watch its own dialogs; a DOM
  dialog raises no Android event, so `ours/src/app/ink-suppression.ts` reports them explicitly.
- _EinkWise "Speed" profile written into the app's stored EAC config_ (`MobileSystemPlugin.kt:264-292`).
  No stock or reference precedent; it exists because the firmware's own caret/touch/scroll refreshes
  are governed by that profile and Regal turns each into a full flash.
- _GC cadence driven by navigations and overlay closes_ (`ours/src/app/eink-refresh.ts:16-67`,
  `NAVIGATIONS_PER_REFRESH = 6`). Stock counts page turns and menu actions
  (`stock/note/common/utils/EpdDeviceManager.java:16-40`); the shape is the same, the triggers
  are ours.

**Unexamined.**

- _`FEATURE_SF_TOUCH_RENDER | FEATURE_APP_TOUCH_RENDER`._ Now matches stock's mask of 3, but was
  changed in `e1a9da7` on a hypothesis and the comment at `kt/OnyxInk.kt:324-326` says so. No
  hardware verification that `APP` adds anything with `touchListenerEnabled = false`.
- _No render-flag cycle during an uninterrupted session._ §11: stock runs `InvalidateScreenAction`
  on ~30 UI events, so it rebuilds the firmware scribble state constantly. We never do while writing.
  This is the largest structural gap left, and it is untested whether a periodic cycle would prevent
  the dead rectangle independently of region mode.
- _A latched exclude rect from the `93c5724`…`95e1b58` window is never cleared on startup_, and the
  clear we do have would not work: `EMPTY_EXCLUDE = listOf(Rect(0,0,0,0))` still excludes the panel's
  top-left corner once the probe point is expanded by half the stroke width (§2a). An off-panel
  rectangle pushed once at session creation would close this off for good.
- _`resetPenDefaultRawDrawing()` on every resume restores eraser style 5._ Our creation-time
  `setEraserRawDrawingEnabled(false, 0)` is never re-applied (§9). Probably harmless because the
  channel is disabled either way, but unverified. Stock re-applies its eraser parameters on every
  resume for exactly this reason.
- _`enableSideBtnErase(true)` also sets a system-wide firmware flag_ (`Device.setEnablePenSideButton`)
  that nothing restores on close. Stock never calls it at all.
- _`setHostViewScrollListenerEnabled(false)`_ (`kt/OnyxInk.kt:338`) is an empty body on this device
  (`pen/RawInputManager.java:196-197`). Dead code, kept.
- _We never call `enterScribbleMode`._ The SF path calls `leaveScribbleMode` (→ `enablePost(1)`) on
  every render-disable but never the inverse on re-enable
  (`pen/touch/SFTouchRender.java:194-203`, `:265-268`), so after our first render-off `enablePost`
  stays at 1 unless the app sets it back. notate does call `EpdController.enterScribbleMode`
  (`ref/notate/.../OnyxCanvasView.kt:895`); stock relies on the `AppTouchRender` half, which toggles
  `enablePost` itself (`pen/touch/AppTouchRender.java:186-206`) — and since `e1a9da7` we have that
  half too, so this may already be handled. Unverified either way.
- _`setEpdTurbo` and `setDisplayScheme` are untried_, as is the dither threshold that stock changes
  alongside its fast mode.
- _`applyAppScopeUpdate(pkg, enable, clear, UpdateMode, repeatLimit)`_ — a firmware-managed
  "N fast frames then a GC" — is implemented on `SDMDevice` and would replace our hand-rolled GC
  cadence. Never tried. Note its `SDMDevice` fallback path invokes a `View` instance method with a
  null receiver, so on a firmware missing the primary handle it fails silently and returns `false`.
- _`SimpleEACManage.setEACRefreshConfigEnable(ctx, false)`_ would stop EinkWise overriding our
  per-view modes entirely. Never tried.
- _The 120 ms `FRAME_DELAY_MS` and the `PEN_UP_WAIT_MS = 750` fallback_ are guesses, not measured
  values.
- _`onWindowFocusChanged` still calls `resume()` directly_ (`kt/OnyxInk.kt:555-557`) without the
  `RESUME_DELAY_MS` that `pauseStateChanged` uses, so a dialog that takes window focus resumes on a
  different path from one that reports itself as a pause reason.
- _Our `onPenUpRefresh` re-post is redundant_ (§1) and, more importantly, we now depend on a callback
  whose SDK wrapper is missing the null guard its nine siblings have — an NPE there shuts the reader
  down process-wide (`pen/touch/SFTouchRender.java:129-137`).
