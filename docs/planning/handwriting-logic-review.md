# Handwriting logic review — every decision in the pen path, re-examined against the system

Written after `e7946ff` ("clear the latched ink exclusion and rebuild the scribble session"), which is the
first change in this area made with the receiving end of the stack actually readable. The purpose of
this document is not to repeat the reverse-engineering notes but to **decide**: for each axis of the
pen path, what our old logic did, what HEAD does, what the stock app and the five reference apps do,
what the system actually does, and what we should therefore do — separating the parts the evidence
settles from the parts that are a judgement call.

## How to read this

- **settled** — the evidence decides it. It goes in the work list.
- **fork** — a real judgement call with a cost on each side. It goes in "Forks for the user".
- `[inference]` marks anything not directly read out of a binary or a source file.

Comparison rows are: **old** (our states S1–S4, `c8657b9`…`3e0036e`), **HEAD** (`e7946ff`), **stock**
(`com.onyx.android.note` 45326), then **notate / notable / PngNote / saber / mokke**.

### Source shorthands

| tag      | root                                                                                                                       |
| -------- | -------------------------------------------------------------------------------------------------------------------------- |
| `kt/`    | `/home/okhsunrog/code/rust/notes-rs/plugins/tauri-plugin-mobile-system/android/src/main/java/dev/okhsunrog/mobile_system/` |
| `ts/`    | `/home/okhsunrog/code/rust/notes-rs/src/`                                                                                  |
| `pen/`   | `/home/okhsunrog/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-pen-1.5.4.3/sources/com/onyx/android/sdk/pen/`                    |
| `dev/`   | `/home/okhsunrog/tmp_zfs/onyx_sdk_decompiled/src/onyxsdk-device-1.3.5.2/sources/com/onyx/android/sdk/device/`              |
| `fw/`    | `/home/okhsunrog/tmp_zfs/onyx_framework/out/sources/android/onyx/`                                                         |
| `stock/` | `/home/okhsunrog/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/`                                             |
| `ref/`   | `/home/okhsunrog/tmp_zfs/reference_notes_apps/`                                                                            |
| `SF`     | `/home/okhsunrog/tmp_zfs/onyx_framework/REPORT-surfaceflinger.md` and the binary it analyses                               |

Companion documents, built on rather than repeated: `docs/onyx-reversing/{02,03,04,05}-*.md`,
`docs/planning/handwriting-pen-stack-comparison.md` (the per-state comparison; **corrected below**),
`/home/okhsunrog/tmp_zfs/onyx_framework/REPORT-surfaceflinger.md`. Panel-refresh-profile specifics
(EinkWise, `refreshModeIndex`, waveform costs) belong to `REPORT-einkwise.md` / `05-panel-refresh-levers.md`
and are cross-referenced, not re-derived.

---

## 0. The one thing that reorganises everything else

There are **two** pen state machines with the same shape, in two different processes, and almost every
confusion in the older notes comes from treating them as one.

1. **In SurfaceFlinger.** `PenManager` at `*(0xac9fc8)+0x120` holds a limit array (`+0x104c`, count
   `+0x103c`), an exclude array (`+0x1040`, count `+0x2040`), a region mode (`+0x38`) and a stroke
   width (`+0x34`) — SF §2.1. SF reads the digitizer itself (`TouchReader::start/stop/pause/resume`,
   reached from `HWEpdcManager::setHandWritingPenState` @ `0x5666bc`) and gates each sample through
   `PenManager::inValidRegion` @ `0x52e5c4`. **This is the copy that survives our process.** Nothing in
   the transaction carries a pid, uid, surface or window token (`fw/ViewUpdateHelper.java:1202-1217`),
   nothing reaps it on death, and `handleAppDie` @ `0x561798` — which would do a correct teardown — is
   reachable only through transaction `0xff0025`, whose only Java wrapper is
   `fw/ViewUpdateHelper.java:296-300` and which **has no caller anywhere in `framework.jar`,
   `services.jar`, `kcb.apk` or the SDK** (verified by grep). SurfaceFlinger installs no binder death
   recipient; the "watch app" list at `0xac9ed8` is a bare `std::vector<int>` of pids.

2. **In our own process.** `libonyx_pen_touch_reader.so` has its own `PenManager` singleton at
   `nat 0x161b0` with the same field shape (`02-handwriting-regions.md` §1.4). It dies with the app.

`RawInputReader.setExcludeRect` writes **both** (`pen/RawInputReader.java:418-425`);
`AppTouchInputReader.setExcludeRectList` writes **only the SF one**
(`pen/touch/AppTouchInputReader.java:150-156`). That asymmetry is the whole story of §3 below.

Separately, SF holds a **compositing schema**: `EPDC+0x9c`, value `7` = Handwriting, with
`androidDrawing` (`EPDC+0x28`) forced to 0 by `afterStartHandwriting` @ `0x562c40`. In that state
`HandwritingSchema::output` @ `0x55b828` composites ink entries only and never runs the full-screen
`NormalSchema` composite, and `EpdcManager::applyUpdateAction` @ `0x558ce8` **refuses** ordinary schema
switches with `"### do not switch epd Schema in handwriting mode"` (`0x104f3c` @ `0x558e60`). The only
exits are `stopHandwriting` @ `0x558630`, reached from pen state `3` (PAUSE, `0x5667d4`, a tail call),
from `ENABLE_POST` (`handleEnablePost` @ `0x560bf8`), and from `REPAINT_EVERYTHING` (`0xff0014`).

**So the "dead rectangle" is two independent latches** — a stale rect in SF's global exclude array
(pen cannot ink) and a stuck Handwriting schema (our content is not composited) — and they are cleared
by disjoint transactions. `HANDWRITING_REPAINT` (`0x100047` → `setHandWritingRepaintUpdate` @
`0x55828c`) clears **neither**: it only sets `mHWRepaintPending` and stays in schema 7. That is why
every single-lever probe failed, and it retroactively explains why our old `handwritingRepaint`-based
handover could never self-heal.

### 0.1 Corrections this forces on the existing documents

| #   | Prior claim                                                                                                                                                                                                                              | Correction                                                                                                                                                                                                                                                                                                                                                                                                                                | Evidence                                                                                                                                                                   |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| C1  | "the firmware's own reset — `setScreenHandWritingRegionExclude(null, {0,0,0,0})` — is the one call that provably clears the SF exclude table" (`02-handwriting-regions.md` §3.1; the reasoning `kt/OnyxInk.kt:185-197` was written from) | **It does not clear it.** `PenManager::setExcludeRegion` @ `0x52ea58` clears only on `n <= 0` (branch at `0x52ea88`); a 4-int payload **stores one degenerate rect at the origin**, which `inValidRegion` then inflates by `strokeWidth/2`. Onyx evidently treats a corner exclusion as harmless; it is not a clear.                                                                                                                      | SF §2.1; `fw/EACScreenNoteUtils.java:115-116` sends `new int[]{0,0,0,0}`                                                                                                   |
| C2  | Not previously noticed                                                                                                                                                                                                                   | **We already clear it correctly, by accident.** Since the mask went to `SF｜APP` (`e1a9da7`), every `helper.setLimitRect(limit, NO_EXCLUDES)` reaches `AppTouchInputReader.setExcludeRectList`, which guards only `null` — so an empty list becomes `Rect[0]` → `int[0]` → `n = 0` → **table wiped**.                                                                                                                                     | `pen/touch/AppTouchInputReader.java:150-156`; `pen/touch/AppTouchRender.java:271-275`; `dev/SDMDevice.java:3439-3454`; `fw/ViewUpdateHelper.java:1202-1217`; SF `0x52ea88` |
| C3  | "a latched exclusion survives closing the ink session … **but not restarting the app**" (`handwriting-pen-stack-comparison.md` §2a); "§12 item 8 … should disappear after a force-stop"                                                  | True of the **in-process** table only. The **SF-side** table survives process death, force-stop and reinstall, and is cleared only by another `setExcludeRegion` or a reboot. This is what was actually observed.                                                                                                                                                                                                                         | SF §2.1 "Lifetime / ownership"; `e7946ff` commit message                                                                                                                   |
| C4  | "`EMPTY_EXCLUDE`/an off-panel rect such as `Rect(-10,-10,-9,-9)` is what is wanted" (§2a, §13)                                                                                                                                           | Right for the **native** table (which has no reachable empty path — `pen/RawInputReader.java:419-421`), wrong for the SF table, where the empty array is exact. At HEAD we never push a non-empty exclude at all, so the native table is always clean in any process running HEAD: **this concern is now a no-op.**                                                                                                                       | `kt/OnyxInk.kt:183`, `:123`, `:349`, `:358`, `:363`                                                                                                                        |
| C5  | Region mode: "what the firmware does with mode 1 is not visible in any of these sources" (§2c)                                                                                                                                           | Now visible. `handleSetHandWritingRegionMode` @ `0x561ad4` → lambda `0x564f74` does `str m,[PenManager+0x38]`, read by `inValidRegion` @ `0x52e648`. It is the same first-rect-index latch the native side has (`03-raw-drawing-render-flag.md` §2.2), applied to SF's own limit array. **With one limit rect it is a no-op on both sides.** The "transient region table fills up" hypothesis is not supported by anything in the binary. | SF §1, §2.1 field map                                                                                                                                                      |
| C6  | Implicit in §11: "a render-flag cycle" is a single primitive                                                                                                                                                                             | It is four transactions with three effects, and `setRawDrawingRenderEnabled` **short-circuits** when the flag already holds the value (`pen/TouchHelper.java:188-198`), so with the eraser or lasso active the heal silently emits nothing. `e7946ff`'s `cycleScribbleSession` is exactly the dedup-free form, and it is the same pair `SFTouchRender.m212g` already emits.                                                               | `pen/touch/SFTouchRender.java:194-197`; `pen/touch/AppTouchRender.java:200-206`; `kt/OnyxInk.kt:644-649`                                                                   |

---

## 1. Session lifecycle

|                    | open                                                                                                                                                          | close                                                                                   | defensive at startup                                                                                                                                                                                                                          | on process death |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------- |
| **old**            | `TouchHelper.create(SF)` + `openRawDrawing`, then `setRawDrawingEnabled(true)` (`c8657b9:OnyxInk.kt:101,158`)                                                 | `closeRawDrawing`                                                                       | `clearTransientUpdate`, `appResetCTPDisableRegion`                                                                                                                                                                                            | nothing          |
| **HEAD**           | `create(SF｜APP)` + `openRawDrawing` (`kt/OnyxInk.kt:336-349`), `resume()` (`:368`)                                                                           | `close()` → `pause()` → `closeRawDrawing()` → GC after 300 ms (`kt/OnyxInk.kt:693-715`) | `clearTransientUpdate(false)` (`:245`), `resetPalm()` (`:249`); **plus**, since `e7946ff`, the first `configure()` runs `pause()` → `cycleScribbleSession()` (`:313`, `:592`) and the first `resume()` runs `clearLatchedExcludes()` (`:122`) | nothing          |
| **stock**          | `TouchHelper.create(view, 3, cb, false)` at `stock/sdk/notecore/editor/NoteManager.java:220-255`, then `forceSetRawDrawingEnabled` once at bind (`:246-250`)  | bind/attach lifecycle                                                                   | none found                                                                                                                                                                                                                                    | nothing          |
| **firmware (EAC)** | `startEACScreenNote`: **reset exclude → set limit → `PEN_START(1)` → stroke params → resume** (`fw/optimization/screennote/handler/BaseHandler.java:331-342`) | `stopEACScreenNote`: `PEN_STOP(0)` + unregister (`:344-351`)                            | the exclude reset at `:334` **is** the defensive step                                                                                                                                                                                         | nothing          |
| notate             | `TouchHelper.create(view,true,cb)`; drives pen state itself (`ref/notate/.../OnyxCanvasView.kt:89,95,542,743,861`)                                            | —                                                                                       | `enterScribbleMode` (`:895`)                                                                                                                                                                                                                  | nothing          |
| notable            | `create(view,cb)`; `isDrawingAllowed` flag                                                                                                                    | —                                                                                       | none                                                                                                                                                                                                                                          | nothing          |
| PngNote            | close + reopen on size change                                                                                                                                 | `onStop` closes                                                                         | none                                                                                                                                                                                                                                          | nothing          |
| saber              | `setRawDrawingEnabled(false); (true)` to re-arm (`ref/saber/.../OnyxsdkPenArea.kt:243-251`)                                                                   | —                                                                                       | none                                                                                                                                                                                                                                          | nothing          |
| mokke              | `pauseRawDrawing`/`resumeRawDrawing`; `closeRawDrawing` for child activities                                                                                  | —                                                                                       | 500 ms debounced detach/reattach                                                                                                                                                                                                              | nothing          |

**What the system does.** The session is keyed by **one global pid slot** at `EPDC+0xf90`, written by
_every_ `SET_SCREEN_HANDWRITING_PEN_STATE` regardless of the state value
(`handleSetHandWritingPenState` @ `0x560c4c`, pid read at `0x560cbc`), and mirrored into
`addToWatchApp` @ `0x562c70`. There is no per-session object and no binder token. `handleAppDie` @
`0x561798` does the right thing — `handleSetHandWritingPenState(0, pid)` then `repaintEverything` — but
**nothing sends it** (`fw/ViewUpdateHelper.java:296-300` has no callers). So a client that is killed
leaves the schema latched and the exclude table dirty, indefinitely, for every later app.

Our teardown, when we get to run it, is genuinely complete: `closeRawDrawing()` →
`SFTouchRender.closeDrawing()` = `leaveScribbleMode` + `PEN_PAUSE(3)` + `quitRawInputReader` +
`PEN_STOP(0)` (`pen/touch/SFTouchRender.java:263-268`, `:191-193`, `pen/EpdPenManager.java:47-50`),
and `AppTouchRender.closeDrawing()` adds `enablePost(1)` + `PEN_STOP(0)`
(`pen/touch/AppTouchRender.java:193-197`). That is exactly the pair the SF report names as the minimal
robust sequence, minus the exclude wipe.

**Recommendation — settled.** Keep the teardown. Add the one thing every implementation surveyed
lacks and the system makes necessary: a **defensive reset at startup, before the first session opens**,
in the firmware's own order — `PEN_PAUSE(3)` → `repaintEverything()` → empty exclude → limit. Today the
equivalent only happens when the ink sheet is first configured (`kt/OnyxInk.kt:313`), which is late: a
previous run's latch is live for the whole of the app's launch, home screen and note list. `OnyxInk` is
constructed lazily, so this belongs in the plugin's `load` (`kt/MobileSystemPlugin.kt:69-79`), not in
`OnyxInk.init`.

**Recommendation — settled.** Do **not** call `ViewUpdateHelper.appDie` for other pids. It is
unreachable through the SDK, requires raw `transact`, and the same effect is available to us safely as
`PEN_PAUSE` + `repaintEverything` on our own pid.

**Fork.** Whether the startup reset should also run on every `onResume` of the app (not just the first
configure). See fork F3.

---

## 2. Pause and resume

|                    | reasons                                                                                                                                  | order (pause / resume)                                                                                                                                                                            | delay                                                                                                                                                                   | re-pushed on resume                                                                                              |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| **old**            | focus, lifecycle, `configure()`; **no IME** until `5b61507`                                                                              | `setRawDrawingEnabled(false)` / `(true)` — both switches at once                                                                                                                                  | 0, later flat 150 ms                                                                                                                                                    | nothing                                                                                                          |
| **HEAD**           | named registry: `ime`, `text-focus`, `overlay:<id>` (`ts/app/ink-suppression.ts:15,71,99`); focus; lifecycle; `configure()`; eraser gate | render off → input off / **push rects → `resetPenDefaultRawDrawing` → input on → render on** (`kt/RawDrawingGate.kt:31-42`)                                                                       | `RESUME_DELAY_MS` = 500 colour / 150 mono (`kt/OnyxInk.kt:223-224`)                                                                                                     | `setSingleRegionMode`, `setPenUpRefreshTimeMs`, `clearLatchedExcludes`, `setLimitRect` (`kt/OnyxInk.kt:120-123`) |
| **stock**          | a ~15-flag predicate in `PenEventHandler`, plus `InvalidateScreenAction` on **39 call sites** (`03-raw-drawing-render-flag.md` §3.1)     | two switches, always separate; resume is **input first, render second** after a blocking sleep (`stock/note/note/request/pen/ResumeRawDrawingRequest.java:126-137`)                               | `DELAY_ENABLE_RAW_DRAWING_MILLS = isColorDevice() ? 500 : 150` (`stock/sdk/notecore/editor/data/RawPenArgs.java:68-70`); popup 500/300; selection 500; shape change 100 | limit+exclude, pen-up interval, single-region mode, pen args (`ResumeRawDrawingRequest.java:107-113`)            |
| **firmware (EAC)** | finger down; stylus down outside the draw view; `TOOL_TYPE_ERASER`; focus change; layout change; IME show/hide; visibility               | `PEN_PAUSE(3)` / `PEN_DRAWING(2)`; **every pause is paired with `repaintEverything()`** (`BaseHandler.java:195,243,311`)                                                                          | **none** — no `postDelayed` anywhere on the pause/resume path                                                                                                           | on focus gain: exclude reset, limit, stroke params (`:312-318`); on layout change: limit only (`:159-165`)       |
| notate             | pen popup, sidebar, pan/zoom                                                                                                             | `setRawDrawingEnabled(false/true)`                                                                                                                                                                | —                                                                                                                                                                       | —                                                                                                                |
| notable            | modals, focus                                                                                                                            | `isDrawingAllowed` → `setRawDrawingEnabled`; separately a bare render cycle in `prepareForPartialUpdate` (`ref/notable/.../einkHelper.kt:282-289`), coalesced in `resetScreenFreeze` (`:333-346`) | 500 colour / 300 mono / 150 stroke-erase                                                                                                                                | —                                                                                                                |
| PngNote            | focus, `onStop`                                                                                                                          | close + reopen                                                                                                                                                                                    | —                                                                                                                                                                       | limit rect                                                                                                       |
| saber              | layout change                                                                                                                            | `setRawDrawingEnabled(false); (true)`                                                                                                                                                             | —                                                                                                                                                                       | limit rect                                                                                                       |
| mokke              | every dialog, finger scroll                                                                                                              | `pause`/`resumeRawDrawing`                                                                                                                                                                        | 500 ms debounce on reattach                                                                                                                                             | limit rect                                                                                                       |

**What the system does.** The two switches are not symmetric and not equivalent.

- `setRawDrawingRenderEnabled(false)` emits, in order: `ENABLE_POST{-1,1,pid}` (SF leg,
  `pen/touch/SFTouchRender.java:194-197`), `PEN_STATE{3,pid}`, `ENABLE_POST{-1,1,pid}` (APP leg,
  `pen/touch/AppTouchRender.java:200-206`). On the SF side pen state 3 **tail-calls
  `EpdcManager::stopHandwriting`** (`0x5667d4`), which restores the schema and puts
  `androidDrawing` back to 1. So a render-disable _is_ the content-half heal.
- `setRawDrawingRenderEnabled(true)` emits only `PEN_STATE{2,pid}`. Pen state 2 (`0x5667a0`) is
  `TouchReader::ensureStart` + `resume` and **explicitly does not restore the schema** — it does not need
  to, because `startHandwriting` is re-entered on the next pen trigger.
- `setRawInputReaderEnable` never reaches SurfaceFlinger at all — it is `nativeSetPenState(4)` /
  `nativePausePen()` in our own process (`pen/RawInputReader.java:326-338`).
- **Both setters short-circuit** on `f77b != enabled` / `f78c != enabled`
  (`pen/TouchHelper.java:188-198`, `:213-223`).

**Recommendation — settled.** Keep the split switches, the order, and the colour-panel delay. All three
are directly corroborated by `ResumeRawDrawingRequest.java:126-137` and by the SF-side asymmetry above:
arming input before render is what stops the firmware painting ghost ink from events that arrive before
the reader is ready.

**Recommendation — settled.** `e7946ff` routed `onWindowFocusChanged(true)` through `resumeGate`
(`kt/OnyxInk.kt:561-567`). Keep it — it removes the last resume path that ignored the delay — but note
it is a **divergence from the firmware**, which resumes on focus gain with no delay at all
(`BaseHandler.java:312-318`). The firmware can afford that because it also emits `repaintEverything()`
first; we should keep the delay because we do not.

**Recommendation — settled.** Pair the pause with a repaint the way the firmware does, but _bounded_:
`pause()` already calls `webView.invalidate()` (`kt/OnyxInk.kt:600`) and `region.invalidateAll()`
(`:596`), which together are the app-scoped equivalent of `repaintEverything()` and cost far less. This
is already right; the thing to add is that the invalidate must land **after** the render flag is down,
which it does (`:591` before `:600`).

**Fork.** `pause()` now also calls `cycleScribbleSession()` (`:592`) on **every** pause — including the
`pause(releaseDisplay = false)` that `configure()` runs (`:313`), which fires on every scroll event
(`ts/features/handwriting/onyx-ink.ts:297`). See fork F2.

### 2a. The eraser gate and the IME

The eraser gate (`kt/InkEraserRenderGate.kt`) pauses **render only** and keeps raw input flowing, so the
software eraser still receives points (`kt/OnyxInk.kt:763`, `:129-132`). The firmware's own rule is
blunter and points the same way: `applyStrokeParam` returns early when the style is the eraser
(`BaseHandler.java:66-70`, `isEarsingStroke` = `strokeStyle == 5` at `:102-104`), so an erasing tool
**never resumes the firmware pen at all**, and `TOOL_TYPE_ERASER` on a motion event is an unconditional
`PEN_PAUSE` (`:132-134`). Stock is the same shape via `EraseHandler.isUsePenInput() == true` with
`canPenRawRender()` gated on the eraser type (`stock/note/note/handler/scribble/EraseHandler.java:35-53`).

**Settled:** the gate is correct and matches all three references. One real gap: `resetPenDefaultRawDrawing()`
runs on every resume (`kt/RawDrawingGate.kt:39`) and restores `setEraserRawDrawingEnabled(false, 5)`
(`pen/TouchHelper.java:348-351`), while our creation-time `setEraserRawDrawingEnabled(false, 0)`
(`kt/OnyxInk.kt:350`) is never re-applied. Stock re-applies its eraser parameters on every resume for
exactly this reason (`ResumeRawDrawingRequest.java:64-69,84-87`). Harmless today because the channel is
disabled either way, but it is a latent divergence — move it into `pushRects()`.

The IME never takes our window focus, so `hasWindowFocus()` cannot see it while the firmware happily
paints over it (the limit rect is a _screen_ region with no window z-order). We watch
`WindowInsetsCompat.Type.ime()` from the plugin (`kt/MobileSystemPlugin.kt:74-77`, `:95-103`). The
firmware reaches the same conclusion by a different route: `SingleDrawViewHandler` silently refuses to
resume the pen **or** to set the limit region while the IME is visible
(`fw/.../handler/SingleDrawViewHandler.java:26-41`). **Settled: ours is equivalent and better scoped**
(we can distinguish a DOM field from the system keyboard; the firmware cannot).

---

## 3. Regions

|                    | limit                                                     | exclude                                                                                                                                                                            | region mode                                                            | recomputed on                                           |
| ------------------ | --------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- | ------------------------------------------------------- |
| **old**            | one rect from CSS geometry                                | `emptyList()` — and, `93c5724`…`95e1b58`, **the selection menu box as a real exclude rect**                                                                                        | never set → mode 0                                                     | `configure()`, later + resume                           |
| **HEAD**           | one rect, clipped to viewport (`kt/OnyxInk.kt:322-324`)   | `NO_EXCLUDES` (`:183`) — never a real one; plus `EXCLUDE_RESET` (`:198`, `:623`)                                                                                                   | `setSingleRegionMode()` at create and on every resume (`:348`, `:120`) | `configure()` (scroll/resize/visibility) + every resume |
| **stock**          | `List<Rect>` = `scribbleRect`                             | **10 typed overlay slots + inter-page gaps**, always pushed with the limit (`stock/sdk/notecore/editor/extension/EditorBundlesKt.java:315-338`, `NoteDocViewInfo.java:41,124,128`) | `setSingleRegionMode()` every resume                                   | every resume, every rect change                         |
| **firmware (EAC)** | the draw view's visible rect (`BaseHandler.java:438-442`) | **never sets one — the only thing it ever does to the exclude table is wipe it** (`:313`, `:334`)                                                                                  | never set                                                              | layout change, focus gain, visibility                   |
| notate             | `getLocalVisibleRect`                                     | toolbar + open sidebar                                                                                                                                                             | single                                                                 | surface events                                          |
| notable            | view minus a 40 dp strip                                  | the toolbar strip                                                                                                                                                                  | none                                                                   | surface, size, toolbar toggle                           |
| PngNote            | `getLocalVisibleRect` offset `(0,-40)`                    | none                                                                                                                                                                               | none                                                                   | layout, surface, resume, focus                          |
| saber              | `getLocalVisibleRect`                                     | none                                                                                                                                                                               | none                                                                   | layout listener                                         |
| mokke              | the SurfaceView's visible rect                            | none                                                                                                                                                                               | none                                                                   | debounced detach/reattach                               |

**What the system does.** `PenManager::inValidRegion` @ `0x52e5c4` inflates the probe point by
`strokeWidth/2` (`fmul s5,s2,#0.5`), requires it inside a limit rect (loop @ `0x52e678`), then rejects
it if it is inside any exclude rect (loop @ `0x52e5dc`). `setExcludeRegion` @ `0x52ea58` zeroes the old
array first, unconditionally, then `if (n <= 0) { count = n; return; }` — **the empty array is the only
true clear**. A one-rect `{0,0,0,0}` payload stores a rect that the inflation then grows to a box of
side `strokeWidth` at the origin.

The table is display-global, unowned, and unreadable: there is no
`getScreenHandWritingRegionExclude` anywhere (`02-handwriting-regions.md` §1.3). Every write costs a
`memset` + copy under the single pen mutex `0xac9eb0`, and on the native side leaks its `calloc`
buffer (`nativeSetExcludeRegion` @ `nat 0xd944`).

**Recommendation — settled.** Never publish an app overlay as an exclude rect. Three independent
reasons now, not one: it cuts every trace crossing it (which is what broke the dashed lasso in
`93c5724`); it is write-only and unowned, so a crash strands it for every other app until reboot; and
the firmware's own handler never sets one either. HEAD is already right (`kt/OnyxInk.kt:178-183`) — the
gesture-swallow mechanism (`kt/InkOverlayRects.kt`, `kt/OnyxInk.kt:753-759`) is the correct answer and
should stay.

**Recommendation — settled.** Replace `EXCLUDE_RESET = arrayOf(Rect(0,0,0,0))` with
`emptyArray<Rect>()`. Correction C1: the current payload does not clear the table, it replaces it with
a corner exclusion. The empty form provably clears (`0x52ea88`), and is exactly what the `APP` half of
`setLimitRect(limit, NO_EXCLUDES)` already sends one line later (`kt/OnyxInk.kt:123`) — so today the
sequence arms a corner rect and then clears it. Fixing the constant makes the intent match the effect;
alternatively, **delete `clearLatchedExcludes()` entirely** and rely on the `APP` half, which is the
smaller change but leaves the clear as an implicit side effect of a call named `setLimitRect`. Prefer
fixing the constant and keeping the explicit call, because the explicit call also documents _why_.

**Recommendation — settled.** `setSingleRegionMode()` is a no-op with one limit rect, on both the
native side (`03-raw-drawing-render-flag.md` §2.2) and the SF side (correction C5). It is cheap and it
matches stock, so keep it — but the comment at `kt/OnyxInk.kt:115-119`, which claims the firmware
"keeps one transient region per stroke; once that table fills, whole rectangles stop taking ink", is
**not supported by the binary** and should be corrected. The real mechanism is the schema latch.

**Recommendation — settled.** Re-pushing the limit rect is the only app-reachable way to clear the
native `lastRegionHit` latch (`03-…` §2.2), so keeping it in `pushRects()` is right. But it must never
happen mid-stroke: both region arrays are fixed 4 KB with no bounds check and no locking against the
reader thread (`02-…` §1.4, §4). `configure()` is already deferred to pen-up in JS
(`ts/features/handwriting/onyx-ink.ts:164-167`) — keep that invariant explicit.

---

## 4. Per-stroke handover — the central design fork

|                    | what happens on pen-up                                                                                                                                                     | region                                                                                                                         | rendezvous / debounce                                                                                                                                   |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **old (S1)**       | `setRawDrawingRenderEnabled(false)` → `invalidate(view, DU)` → render on                                                                                                   | whole view                                                                                                                     | visual-state callback + 120 ms                                                                                                                          |
| **old (S2–S4)**    | mode-marked frame **and** `EpdController.handwritingRepaint(view, region)` (`3e0036e:OnyxInk.kt:386`)                                                                      | JS damage ±2 px                                                                                                                | visual-state callback + 120 ms; `setPenUpRefreshEnabled(false)` from `c8657b9:OnyxInk.kt:102` onwards                                                   |
| **HEAD**           | `setViewDefaultUpdateMode(TRANSIENT, HAND_WRITING_REPAINT_MODE)` → `webView.invalidate()` → clear on `registerFrameCommitCallback` (`kt/OnyxInk.kt:537-542`, `:529`)       | pen-up union = stroke ∪ previous stroke, or whole region after anything else drew (`kt/InkReconcileRegion.kt:48-64`, `:39-42`) | **two-sided**: SDK pen-up refresh **and** the canvas frame, whichever is last (`kt/InkReconcileGate.kt:70-75`), with a 750 ms net (`kt/OnyxInk.kt:798`) |
| **stock**          | **nothing at all on this device.** `EpdShapeHandler.P0()` is gated on `isUseHWUpdateForPenUpRefresh()`, which returns `false` for a tablet _and_ for a colour device       | (whole `scribbleRect` when `isEnabledPenDirtyRect == false`)                                                                   | (SDK pen-up timer + bitmap-flush completion, when it runs at all)                                                                                       |
| **firmware (EAC)** | `handwritingRepaint(view, wholeVisibleRect, false)` **after `repaintLatency` = 500 ms**, **cancelled by the next pen-down** (`BaseHandler.java:268-283`, `:234`, `:88-96`) | the whole draw view                                                                                                            | debounce with cancel-on-pen-down                                                                                                                        |
| notate             | **nothing** for a plain stroke (`ref/notate/.../PenInputHandler.kt:742-747`); eraser only gets a bracketed region GC (`:145-179`)                                          | —                                                                                                                              | 2000 ms, armed on **hover-exit** only                                                                                                                   |
| notable            | **nothing** — `onBeginRawDrawing`/`onEndRawDrawing` are empty stubs (`ref/notable/.../OnyxInputHandler.kt:82,85`); `onPenUpRefresh` calls `super` only (`:120-121`)        | —                                                                                                                              | erase path only: 150/300/500 ms                                                                                                                         |
| PngNote            | **nothing** but arming a debounce (`ref/PngNote/.../CanvasBoox.kt:103-105`)                                                                                                | whole surface                                                                                                                  | 2000 ms trailing edge (`:381-392`)                                                                                                                      |
| saber              | **nothing** until a 1 s `java.util.Timer` fires `enable(false)` → `invalidate(GC)` → `enable(true)` (`ref/saber/.../OnyxsdkPenArea.kt:235-239`)                            | whole view                                                                                                                     | 1000 ms, cancelled on the next stroke                                                                                                                   |
| mokke              | **nothing** but an incremental bitmap append (`ref/mokke-ink-notes/.../HandwritingCanvasView.kt:597-600`)                                                                  | —                                                                                                                              | Choreographer coalescing                                                                                                                                |

**What the system does.**

```java
// stock/sdk/data/config/system/OnyxSystemConfig.java:145-150
public static boolean isUseHWUpdateForPenUpRefresh() {
    if (SystemPropertiesUtil.isTablet()) return false;      // vendor.onyx.tablet
    return a().enablePenUpRefresh;                          // = !DeviceInfoUtil.isColorDevice()
}
```

`SystemConfigBean.java:22` defines `enablePenUpRefresh = !DeviceInfoUtil.isColorDevice()`, and
`isColorDevice()` is `Device.currentDevice().getColorType() > 0` (`stock/sdk/utils/DeviceInfoUtil.java:136-138`)
— the same probe our `colorPanel()` uses (`kt/OnyxInk.kt:232`). **On a Note Air 4C this is `false` by
the colour branch alone, so stock's per-stroke reconcile never runs.** `GrayscaleRefreshAction`
(`stock/note/note/action/render/GrayscaleRefreshAction.java:58-79`) — the mode-marked frame we
copied — is dead code on this hardware. What keeps stock's colour panel clean is the render-flag
cycle on ~40 UI events (§5).

The SDK's own timer, however, always runs, on every device: `RawInputReader` arms it on pen-up
(`m37l()` @ `pen/RawInputReader.java:278-283`) and **cancels it on the next pen-down and on every move**
(`m39o()` @ `:286-289`, called from `m32a` @ `:708` and `m33a` @ `:725`). The interval defaults to
500 ms (`:131`) and it is enabled by default (`:134`); `setPenUpRefreshEnabled` is `@Deprecated`
(`pen/TouchHelper.java:252-257`) and neither stock nor any reference app calls it. The rect it delivers
is this stroke's bounds expanded by the stroke width and **unioned with the previous stroke's**
(`m38a()` @ `:747-758`) — which is what guarantees consecutive repaints overlap.

**We cannot adopt stock's answer wholesale.** Stock's `EditorView` _is_ its writing region: leaving
firmware ink standing over it is harmless, because the view underneath already holds the same pixels in
a bitmap it blits on the next ordinary draw. Our page lives in a WebView **under** the firmware ink
layer, and while the pen session is live SurfaceFlinger is in schema 7 and does not composite our
layers at all (§0). Ink we never hand over is ink no scroll, zoom or undo can ever update. So a
per-stroke handover is a **necessary divergence**, not a stylistic one — and it has no precedent in any
of the six references, which is why it deserves the scrutiny below.

**Recommendation — settled.** Keep the mode-marked frame, drop nothing. It is `GrayscaleRefreshAction`'s
exact shape (`GrayscaleRefreshAction.java:60` set, `:78` reset in `doFinally`), and the SF side confirms
why the _frame_ and not a later call is the mechanism: `HANDWRITING_REPAINT` cannot leave schema 7
(SF §1, `0x100047` → `setHandWritingRepaintUpdate` @ `0x55828c`), so `34dc906` was right to delete our
`handwritingRepaint` call and it must not come back.

**Recommendation — settled.** Keep the two-sided rendezvous and the stroke-∪-previous-stroke region.
Both are directly justified: the union is free from `onPenUpRefresh` and impossible to reconstruct from
a JS damage rect (`pen/RawInputReader.java:747-758`), and the gate reproduces `P0()`'s
"only when both have happened" shape (`stock/note/note/handler/common/EpdShapeHandler.java:248-262`,
`B0()` at `:126-138`).

**Correction to the brief: we already have the firmware's debounce, for ink.** The cancel-on-pen-down
the firmware implements in Java (`BaseHandler.java:234` → `cancelPendingRedraw()`) the SDK implements
in the reader (`RawInputReader.java:708`, `:725`), with the same 500 ms
(`EACNoteConfig.java:21` `repaintLatency = 500`; `RawInputReader.java:131` `f61n = 500`). Since
`34dc906` we drive the reconcile off that callback, so an ordinary stroke reconciles ~500 ms after the
last point and a stroke started inside that window postpones it. **Three gaps remain:**

1. **Erasing gestures have no debounce.** An erase gets no `onPenUpRefresh` at all
   (`pen/RawInputReader.java:613-616`), so `InkReconcileGate.began(erasing = true)` sets
   `expecting = false` (`kt/InkReconcileGate.kt:47`) and the canvas frame alone triggers
   `Next.SOON` → 120 ms. Rapid erasing therefore reconciles at up to ~8 Hz. **settled:** give the
   erasing path its own debounce of `PEN_UP_REFRESH_MS`, cancelled on the next `begin()`.
2. **The first stroke of a session has no debounce**, because `expecting` requires `penUpRefreshSeen`
   (`kt/InkReconcileGate.kt:47`). That guard exists to avoid deadlocking on a firmware that never
   delivers; the 750 ms net at `kt/OnyxInk.kt:798` already covers that case, so the guard could be
   dropped. **settled, low value.**
3. **`end()` arms an unconditional 750 ms refresh** (`kt/OnyxInk.kt:798`) on top of the gate. It is
   harmless (a new `begin()` removes it at `:772`, and `canPresent()` refuses while drawing) but it
   means two independent timers race for every stroke. **settled, cosmetic:** fold it into the gate.

**Fork.** Whether to keep reconciling per stroke at all, or move to stock's model. See fork F1.

---

## 5. Render-flag cycling

|                    | when it fires                                                                                                                                                                                   | what it costs                                                  |
| ------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------- |
| **old**            | named pause reasons, eraser gate, `configure()`                                                                                                                                                 | —                                                              |
| **HEAD**           | same, plus `cycleScribbleSession()` on **every** `pause()` (`kt/OnyxInk.kt:592`)                                                                                                                | 4 binder transactions + the 500 ms resume delay                |
| **stock**          | **40 `InvalidateScreenAction` sites**: every menu, popup, toast, tool change, page turn, scroll/zoom end, rotation, focus change, CTP change, GC cadence (`03-raw-drawing-render-flag.md` §3.1) | `mySleep(500)` on the pen scheduler + a full re-push           |
| **firmware (EAC)** | `PEN_PAUSE` + `repaintEverything()` on finger-down, stylus-outside, eraser tool, focus change (`BaseHandler.java:195,243,311,132`)                                                              | a full-panel recomposite each time                             |
| notate             | as a **tool property** — on for pen/lasso, off for eraser/rect-select/text and for strokes > 10 mm (`ref/notate/.../PenToolConfigurator.kt:36-78`)                                              | re-configured at every stroke start (`PenInputHandler.kt:347`) |
| notable            | `resetScreenFreeze`: `false → delay(150/300/500) → true`, **coalesced into one cancellable job** (`ref/notable/.../einkHelper.kt:340-345`)                                                      | one delay per burst, not per frame                             |
| PngNote            | never cycles the flag; closes and reopens the whole session instead (`ref/PngNote/.../CanvasBoox.kt:362-375`)                                                                                   | a full close/open per repaint                                  |
| saber              | never cycles it; toggles `setRawDrawingEnabled` around a 1 s GC                                                                                                                                 | whole-screen GC                                                |
| mokke              | **never uses the API**; `syncOverlay(force)` toggles enable instead (`ref/mokke-ink-notes/.../OnyxInkController.kt:118-122`)                                                                    | —                                                              |

**What the system does.** The cycle is four transactions (§2) and its cost is **not** the transactions —
those are sub-millisecond, one-way, with a null reply (`fw/ViewUpdateHelper.java:1421-1440`). The cost
is threefold: the **500 ms colour resume delay** during which the pen writes nothing; the fact that the
`false` leg is `PEN_PAUSE`, which **kills the firmware ink of a stroke in flight** — so it must never
run mid-gesture; and, if paired with `repaintEverything()` the way the firmware does it, a full-panel
recomposite. `stopHandwriting` on its own only restores the schema; the next ordinary frame then
composites normally, so a bare `PEN_PAUSE` is markedly cheaper than `REPAINT_EVERYTHING`
`[inference — the SF report could not read `EpdcWrapper::mergeByMode`, so the exact per-commit rect
count is unresolved; measurable with `dumpEpdcList`, "### sf epdc region received" @ `0x1f5679`]`.

`e7946ff` added `cycleScribbleSession()` for exactly the right reason — `setRawDrawingRenderEnabled`
short-circuits (`pen/TouchHelper.java:189`), and with the eraser or lasso active the flag is already
`false` (`kt/OnyxInk.kt:607-608`), so the heal emitted nothing. Note it is not a novel primitive: it is
byte-for-byte what `SFTouchRender.m212g` already emits (`pen/touch/SFTouchRender.java:194-197`).
`TouchHelper.forceSetRawDrawingEnabled` (`pen/TouchHelper.java:200-211`) is the SDK's own escape hatch
and would work too, but it also flips input, which we do not want here.

**Recommendation — settled.** Keep `cycleScribbleSession()`, but **guard it**: it must not run while a
gesture is open (`gesture.drawing`), and it should not run on a `configure()` triggered by scrolling.
Today `pause()` is called unconditionally from `configure()` (`kt/OnyxInk.kt:313`), and `configure()` is
wired to `window.addEventListener("scroll", configure, true)`
(`ts/features/handwriting/onyx-ink.ts:297`) with no debounce — every scroll event that changes
`rect.top` changes the config key and therefore emits a pause + schema teardown + rebuild. That is the
one place where `e7946ff` made a hot path materially more expensive.

**Recommendation — settled.** Adopt notable's cancellation discipline explicitly. Its
`cancelPendingScreenFreezeReset` exists because "a pending resume firing after raw drawing was turned
off would hand the screen back to the firmware with input disabled — a frozen screen nothing unfreezes"
(`ref/notable/.../einkHelper.kt:326-328`). We are already safe — `pause()` removes `resumeGate`
(`kt/OnyxInk.kt:588`) — but the invariant is worth a test, not just an ordering accident.

**Fork.** Whether to add a _periodic_ in-session heal. See fork F1 (they are the same decision seen
from two sides).

---

## 6. Eraser

|                    | hardware eraser                                                                                                                                                                                 | side button                                                             | render during erase                                                                                                                                      | quality mode                                                                                             |
| ------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| **old (S1)**       | erasing callbacks → same gesture path                                                                                                                                                           | `enableSideBtnErase(true)`, `setEraserRawDrawingEnabled(false, 0)`      | **stays on**                                                                                                                                             | `ANIMATION_QUALITY` requested for erasing too (`b9e3342`)                                                |
| **HEAD**           | same                                                                                                                                                                                            | same (`kt/OnyxInk.kt:350-351`)                                          | `InkEraserRenderGate` pauses render at erase-begin, restores after the erased frame is presented (`kt/InkEraserRenderGate.kt`)                           | none — narrowed to a selection drag in `34dc906` (`kt/OnyxInk.kt:770`)                                   |
| **stock**          | native `shortcutErasing` flags; `EraseHandler.isUsePenInput()` unconditionally `true` (`stock/note/note/handler/scribble/EraseHandler.java:50-53`)                                              | **never calls `enableSideBtnErase`** — zero callers in the APK          | gated by eraser type: `canPenRawRender()` = `isEraseTrackByRawRender(type)` (`:35-43`); enter tool = pause/resume with a hardcoded **150 ms** (`:59-65`) | **none while erasing** — `BaseHandler.renderInFastMode()` returns `false` and `EraseHandler` inherits it |
| **firmware (EAC)** | `TOOL_TYPE_ERASER` → unconditional `PEN_PAUSE` (`BaseHandler.java:132-134`)                                                                                                                     | —                                                                       | an erasing stroke style **never resumes the pen** (`:66-70`, `:102-104`, `:230-232`)                                                                     | `allowApplyTransientUpdate` returns **false** for any stylus event in the draw view (`:357-365`)         |
| notate             | hover-detected `TOOL_TYPE_ERASER` **and** `BUTTON_STYLUS_PRIMARY` (`ref/notate/.../OnyxCanvasView.kt:501-503`); suppresses the _system_ side button by reflection (`OnyxSystemHelper.kt:22-23`) | see left                                                                | render off for non-lasso erasers, app-side cursor (`PenToolConfigurator.kt:44-45`)                                                                       | A2 around every stroke                                                                                   |
| notable            | eraser channel with the undocumented **style 8**, reverse-engineered from kreader (`ref/notable/.../einkHelper.kt:180-186`, `:243-252`)                                                         | firmware-routed                                                         | input **muted** with `setRawInputReaderEnable(false)` around the commit (`CanvasRefreshManager.kt:85,101`)                                               | refcounted `EpdRefreshArbiter`                                                                           |
| PngNote            | erasing callbacks; forces a sync every time because "eraser tends to fail for update screen" (`ref/PngNote/.../CanvasBoox.kt:115-118`)                                                          | none                                                                    | not gated                                                                                                                                                | none                                                                                                     |
| saber              | none in Kotlin; Dart maps eraser → `StrokeStyle.Disabled`                                                                                                                                       | treated identically to the tip (`canvas_gesture_detector.dart:471-472`) | gated via the tool map; author says it still leaks ink (`OnyxsdkPenArea.kt:118-124`)                                                                     | none                                                                                                     |
| mokke              | **no hardware eraser at all** — all four callbacks are empty stubs (`ref/mokke-ink-notes/.../OnyxInkController.kt:247-250`)                                                                     | none                                                                    | —                                                                                                                                                        | none                                                                                                     |

**Recommendation — settled.** The gate is right and is the most defensible piece of the eraser path:
pausing render while keeping input alive is precisely what the firmware and stock both do, expressed
more precisely (we can restore on the _presented frame_ rather than on a fixed delay). Keep it.

**Recommendation — settled.** `34dc906` narrowing `applyTransientUpdate(ANIMATION_QUALITY)` to a
selection drag is confirmed correct from three directions: stock scopes it to selection transforms
(`stock/note/note/eventhandler/EpdEventHandler.java:109-116`, entered only from
`ViewportEventHandler:82,116` and handlers whose `renderInFastMode()` is true), `BaseHandler` in the
firmware **actively refuses** transient updates for stylus events in the draw view
(`BaseHandler.java:357-365`), and no reference app puts the panel into a transient mode while writing.

**Recommendation — settled.** Reconsider `enableSideBtnErase(true)` (`kt/OnyxInk.kt:351`). It writes
two places — the native reader _and_ `Device.setEnablePenSideButton`, a **system-wide firmware setting
that nothing in the SDK restores on close**. Stock never calls it (zero callers in the APK) and notate
goes the other way, explicitly turning the system side button _off_ by reflection at startup
(`ref/notate/.../OnyxSystemHelper.kt:22-23`). We are currently mutating a device-global setting on
behalf of the user and leaking it. Either restore it in `close()`, or drop the call and read the side
button from the `shortcutErasing` flag the callback already carries
(`pen/RawInputCallback.onBeginRawErasing(shortcut, point)`, surfaced but ignored at
`kt/OnyxInk.kt:843`).

**Recommendation — settled.** Move `setEraserRawDrawingEnabled(false, 0)` into `pushRects()` so it
survives `resetPenDefaultRawDrawing()` (§2a).

---

## 7. Lasso and selection

|                         | live trace                                                                                                                                                                                 | menu                                                                                                                        | exclude rects for the menu                                                                             |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| **old (S2)**            | firmware, `STROKE_STYLE_PENCIL` — indistinguishable from ink                                                                                                                               | tool popover                                                                                                                | —                                                                                                      |
| **old (S4, `93c5724`)** | firmware dash                                                                                                                                                                              | floating toolbar                                                                                                            | **yes — published as a real exclude rect**                                                             |
| **HEAD**                | firmware `STROKE_STYLE_DASH` + `Device.setStrokeParameters(DASH, [5f])` (`kt/OnyxInk.kt:357-361`)                                                                                          | DOM overlay, position written directly to `style.left/top` during a drag (`ts/features/handwriting/ink-canvas.tsx:150-160`) | **no** — one gesture is swallowed instead (`kt/OnyxInk.kt:753-759`)                                    |
| **stock**               | firmware `STROKE_STYLE_DASH` via the _eraser_ channel under the selection provider (`stock/note/note/request/pen/ResumeRawDrawingRequest.java:60-71`)                                      | `SelectionPopupMenu`, a plain `PopupWindow`                                                                                 | **no** — it uses the named pause registry instead (`SelectionHandler.java:536`, `:692`, 500 ms resume) |
| notate                  | firmware, eraser channel, `STROKE_STYLE_DASH` (`ref/notate/.../PenToolConfigurator.kt:52-57`)                                                                                              | `SelectionActionPopup`                                                                                                      | **no** — forces a GC blit instead (`SelectionActionPopup.kt:64`)                                       |
| notable                 | firmware, thin grey ballpen (`ref/notable/.../OnyxInputHandler.kt:149-150`)                                                                                                                | `SelectorBitmap`                                                                                                            | **no** — disables raw drawing entirely (`EditorViewModel.kt:107`)                                      |
| PngNote / mokke         | no lasso                                                                                                                                                                                   | —                                                                                                                           | —                                                                                                      |
| saber                   | firmware, **accidentally** — `ToolId.select` maps to `OnyxStrokeStyle.pen`, so a plain black line follows the lasso until the 1 s GC (`ref/saber/lib/components/canvas/canvas.dart:62-63`) | none                                                                                                                        | —                                                                                                      |

**What the system does.** An exclude rect is a hole in the pen's input gate
(`PenManager::inValidRegion` @ `0x52e5dc`), not a hole in a window. It therefore cuts **every** trace
crossing it, which is exactly what `93c5724` produced: a lasso drawn near an existing selection lost the
band of its dashed outline that ran through the menu. Worse, per §0 it is unowned global state that
outlives the process.

**Recommendation — settled, with unusually strong consensus.** Never use an exclude rect for a floating
menu. **Six of six references agree** — stock, notate, notable and (by omission) the rest all protect a
floating menu by pausing, disabling, or forcing a refresh, never by punching the region. Our
gesture-swallow (`kt/InkOverlayRects.kt`) is the cheapest member of that family and the only one that
keeps the trace whole. Keep it, and keep the `f74ace4` in-place overlay update that avoids restarting
raw drawing for a menu move (`kt/OnyxInk.kt:304-312`).

**Recommendation — settled.** The one thing worth importing from stock is that it _does_ pause the pen
for the **transform itself** (`SelectionHandler.java:533-537`, `:690-693`) while leaving the popup
unpaused. We gate render rather than pausing (`kt/OnyxInk.kt:607-608`), which keeps raw input flowing
for the drag preview — a deliberate and better-scoped divergence. No change.

---

## 8. Touch, finger and palm

|                 | CTP disable region                                                                                                                   | palm strategy                                                                                                                 | finger handling                                                                                                                                                     |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **old (S1–S2)** | **armed over the whole limit rect on every pen-down**, reset on end/pause                                                            | firmware CTP cutout + limit rect                                                                                              | killed every capacitive touch in the sheet band                                                                                                                     |
| **HEAD**        | never armed; one `appResetCTPDisableRegion` at init (`kt/OnyxInk.kt:249`, `:689-691`)                                                | limit rect only                                                                                                               | normal WebView pointer events                                                                                                                                       |
| **stock**       | `TouchAreaIgnoreAction.disableFullScreenCTP()` and `resetCTPDisableRegion()` ship with **no callers**; only `dumpCTPInfo()` is wired | limit/exclude geometry + `getToolType` routing; **`PalmDetector` is dead code with zero references**                          | `ScribbleTouchDistributor.onTouchEvent` never consumes (`stock/note/note/touch/ScribbleTouchDistributor.java:368-399`), hard `!hasWindowFocus()` gate at `:377-382` |
| notate          | none                                                                                                                                 | limit + exclude rects + a 150 ms post-stroke settling window                                                                  | `TOOL_TYPE_STYLUS` dropped from the View pipeline                                                                                                                   |
| notable         | none                                                                                                                                 | `dispatchTouchEvent` consumes any event containing a stylus pointer (`ref/notable/.../DrawCanvas.kt:41`)                      | Compose gets finger-only events                                                                                                                                     |
| PngNote         | none                                                                                                                                 | none at all                                                                                                                   | toolbar never overlaps the canvas                                                                                                                                   |
| saber           | none                                                                                                                                 | Dart-side only — **and never forwarded to the firmware**, so `TouchHelper` inks a palm regardless                             | `EagerGestureRecognizer` claims touch                                                                                                                               |
| mokke           | none                                                                                                                                 | a 6-layer `TouchFilter` (hover suppression, 500 ms pen cooldown, 120 dp contact size, multi-touch reject, stationary timeout) | vertical finger scroll pauses ink for the gesture                                                                                                                   |

**What the system does.** There is no SDK-level finger switch on this device class:
`SFTouchRender.enableFingerTouch` is an empty body (`pen/touch/SFTouchRender.java:342`), as are
`onlyEnableFingerTouch`, `setFingerTouchPressure` and
`RawInputManager.setHostViewScrollListenerEnabled` — so our `setHostViewScrollListenerEnabled(false)`
(`kt/OnyxInk.kt:347`) is dead code. `setAppCTPDisableRegion` is real, app-scoped and reversible, but
**zero of six references use it**, and the reason our S1 use failed is now clear: it kills _every_
capacitive touch in the band, including the ones meant for the soft keyboard, and a dropped
`onEndRawDrawing` leaves it armed indefinitely.

**Recommendation — settled.** Nothing should replace it. Removing it in `5b61507` was correct, and the
single reset at init (`kt/OnyxInk.kt:249`) is the right residue — it clears one a crashed older build
may have left. Keep the reset; keep it as the only reference to that API.

**Recommendation — settled, small.** Delete `setHostViewScrollListenerEnabled(false)`
(`kt/OnyxInk.kt:347`) — it is provably a no-op. Reconsider `setPostInputEvent(false)`
(`kt/OnyxInk.kt:346`): stock sets it `true` to get pen-proximity events
(`stock/sdk/notecore/editor/NoteManager.java:235`), which is how it knows the pen is hovering. We
subscribe to nothing, so `false` is correct today — but proximity is the cheapest possible signal for
"a stroke is about to start", and mokke's palm filter is built on exactly that
(`ref/mokke-ink-notes/.../TouchFilter.kt:63`). Worth knowing we have chosen to have no hover signal.

---

## 9. Update modes and display-mode ownership

|                    | view default                                                                          | transient                                                                                                                                                                     | app-scope profile                                                                     | GC                                                                                                                                      |
| ------------------ | ------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| **old (S1)**       | none                                                                                  | none                                                                                                                                                                          | none                                                                                  | none                                                                                                                                    |
| **old (S2–S4)**    | layered stack, `BASE`=REGAL/GU, `SESSION`=GU, `TRANSIENT`=`HAND_WRITING_REPAINT_MODE` | `ANIMATION_QUALITY` for **any** interaction/eraser gesture                                                                                                                    | `setAppScopeRefreshMode(FAST)` probe; EinkWise "Speed" written to the stored EAC JSON | GC after 300 ms settle                                                                                                                  |
| **HEAD**           | same stack (`kt/DisplayModeStack.kt`, `kt/ViewDisplayMode.kt`)                        | `ANIMATION_QUALITY` **only for a selection drag** (`kt/OnyxInk.kt:770`)                                                                                                       | same                                                                                  | same + one GC on session close (`kt/OnyxInk.kt:712-715`) and one when the IME hides (`kt/MobileSystemPlugin.kt:102`)                    |
| **stock**          | `HAND_WRITING_REPAINT_MODE` only around the reconcile frame (dead on this device)     | `ANIMATION_QUALITY` only around selection/viewport transforms, 5 s auto-exit + dither threshold + `setEpdTurbo` (`stock/note/note/eventhandler/EpdEventHandler.java:109-142`) | **untouched**                                                                         | `applyGCWithInterval` counted on page turns and menu actions, never on a stroke (`stock/note/common/utils/EpdDeviceManager.java:16-40`) |
| **firmware (EAC)** | —                                                                                     | **refuses** a transient update for any stylus event in the draw view (`BaseHandler.java:357-365`)                                                                             | drives everything from the stored profile                                             | `repaintEverything()` paired with every pause                                                                                           |
| notate             | `DU` + `SCHEME_SCRIBBLE` at `onCreate`, never restored                                | reflection `applyTransientUpdate`/`setEpdTurbo`; A2 around **every stroke**                                                                                                   | none                                                                                  | explicit GC                                                                                                                             |
| notable            | `HAND_WRITING_REPAINT_MODE`, falling back to `REGAL`                                  | refcounted `EpdRefreshArbiter` with a delayed release                                                                                                                         | `setAppScopeRefreshMode(NORMAL)` once                                                 | `repaintEveryThing(REGAL_PLUS)`                                                                                                         |
| PngNote            | **none**                                                                              | none                                                                                                                                                                          | none                                                                                  | none                                                                                                                                    |
| saber              | none                                                                                  | none                                                                                                                                                                          | none                                                                                  | `invalidate(view, GC)` ×2                                                                                                               |
| mokke              | none                                                                                  | none                                                                                                                                                                          | none                                                                                  | none (a SurfaceView re-compose instead)                                                                                                 |

**Recommendation — settled.** The layered stack is the right shape and has no equivalent anywhere else
in the corpus; every other implementation sets a global mode and never restores it (notate at
`CanvasActivity.kt:261-262` is the clearest example). Keep `DisplayModeStack` — it is the one place our
design is strictly better than the references.

**Recommendation — settled.** Three calls in the profile path are now known to be no-ops on this
firmware and should be deleted or corrected (details and citations in `05-panel-refresh-levers.md` §5;
owned by the EinkWise report, listed here only so they reach the work list):
`applyAppRefreshProfile()` / `EpdController.setAppScopeRefreshMode(FAST)`
(`kt/MobileSystemPlugin.kt:294-322`) cannot take effect because `refreshModeIndex` overrides
`updateMode`; the `REGAL` BASE layer (`kt/MobileSystemPlugin.kt:245-251`) cannot take effect because
`supportRegal()` is false and the EAC layer rewrites mode 3 → 0; and `ensureSpeedRefreshProfile()`
(`:264-292`) writes the value this device already ships with. The comment at
`kt/MobileSystemPlugin.kt:461` calling Speed "partial GU updates with turbo" is wrong — it is A2
(`UI_A2_QUALITY_MODE` 2308).

**Recommendation — settled.** `status()` reports `viewUpdateMode` from
`EpdController.getViewDefaultUpdateMode`, which **returns `GU` when the reflective read fails**
(`dev/SDMDevice.java:1947-1951`) — so "GU" and "could not read" are indistinguishable. `readRaw()`
(`kt/ViewDisplayMode.kt:65-67`) is the trustworthy one. Also: `UpdateMode` → firmware int goes through
reflected framework constants and yields **0** for a missing one, with a fallback only for the REGAL
modes; `HAND_WRITING_REPAINT_MODE` has none. We log `lastRepaintModeRaw` (`kt/OnyxInk.kt:138`) but never
check it. **Assert it once at session start against the known value 524290** (`fw/ViewUpdateHelper.java:77`);
if it reads 0 our entire reconcile is marking frames as mode 0 and we would never know.

---

## 10. Recovery

|                    | what it does when it suspects a stuck state                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **old**            | nothing; `handwritingRepaint`, which provably cannot heal either latch                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| **HEAD**           | `cycleScribbleSession()` on every pause (heals the schema half); `clearLatchedExcludes()` on every resume (**does not actually clear** — C1); `debugRepaint()` behind a dev command (`kt/OnyxInk.kt:395-415`)                                                                                                                                                                                                                                                                                                    |
| **stock**          | no watchdog. The render-flag cycle on 40 UI events heals as a side effect; stuck-flag self-healing on touch-down (`stock/note/note/eventhandler/PenEventHandler.java:515-522`); `EpdController.resetEpdPost()` on editor destroy (`ScribbleFragment.onDestroy:1492`, `EpdEventHandler.destroy:197`) — which is `ENABLE_POST(-1,1,pid)` **plus `setScreenHandWritingPenState(0)`** (`fw/ViewUpdateHelper.java:1069-1077`); crash handler tears the session down before rethrowing (`NoteApplication.J():269-280`) |
| **firmware (EAC)** | `PEN_PAUSE` + `repaintEverything()` on essentially every input event                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| notate             | manual full-screen GC; defensive re-push of the whole tool config at every stroke start                                                                                                                                                                                                                                                                                                                                                                                                                          |
| notable            | a 3 s deadlock **logger** that does not repair; prevention via `cancelPendingScreenFreezeReset`                                                                                                                                                                                                                                                                                                                                                                                                                  |
| PngNote            | a 10 s stuck-`isDrawing` watchdog; three independent "always try enable" counters                                                                                                                                                                                                                                                                                                                                                                                                                                |
| saber              | `forceRefresh()` after every pen-up; author's note: "I gave up here"                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| mokke              | idempotent detach + re-attach on `onResume`, resize, and every layout switch                                                                                                                                                                                                                                                                                                                                                                                                                                     |

**What the system does.** Both latches and both clears are now known exactly (§0). The minimal robust
recovery is:

1. `EpdController.setScreenHandWritingPenState(view, PEN_PAUSE)` **or** `EpdController.repaintEveryThing()`
   — clears the schema latch. `REPAINT_EVERYTHING` (`0xff0014` → provider `repaintEverything`) is the
   cleaner primitive because it has no pen-state side effects; `PEN_PAUSE` also claims the global pen-owner
   pid slot (`EPDC+0xf90`).
2. `EpdController.setScreenHandWritingRegionExclude(<view or null>, emptyArray())` — clears the exclude
   table (`0x52ea88`).

`HANDWRITING_REPAINT` heals **neither** and must never be used for this.

**Recommendation — settled.** Promote the two levers out of `debugRepaint` into a real, user-reachable
"repair the panel" action, and make it the two-step sequence rather than a menu of single levers. It is
the only thing that can rescue a device left dirty by a crash of ours or of another app, and it is
cheap.

**Recommendation — settled.** Add the diagnostic the SF report names: `EpdController.getPenState()`
exists (`dev/SDMDevice.java`, exposed at the `EpdController` facade) and `applySFDebug` logs
`"pen control state: %s, penTriggered: %d, current epdc schema: %d, androidDrawing: %d"`
(`applySFDebug` @ `0x561b78`, format string @ `0x80238`). Surfacing `getPenState()` in `status()` and
enabling `applySFDebug` behind the debug flag turns "the panel looks wrong" into a one-line answer about
_which_ latch is set.

**Recommendation — settled.** Copy stock's exit-time reset. Stock calls `EpdController.resetEpdPost()`
on editor destroy — `ENABLE_POST(-1,1,pid)` + `setScreenHandWritingPenState(0)` — which is a stronger
teardown than `closeRawDrawing()` alone. Our `destroy()` (`kt/OnyxInk.kt:717-723`) should end with
`resetEpdPost()` plus the empty-exclude push, so that a normal exit leaves the device clean for the next
app even if `closeRawDrawing` partially failed.

**Fork.** Whether the startup reset runs once or on every app resume — F3.

---

## Forks for the user

Four. Everything else in this document the evidence decides.

### F1 — Should an in-session heal run periodically, not only on a pause?

**The question.** Between `e7946ff` and today, the firmware scribble session is torn down and rebuilt
only when something pauses the pen. Should we also cycle it on a timer or a stroke count during a long,
uninterrupted writing session?

**Option A — leave it as it is.** The session is rebuilt on every pause reason, every geometry change,
every focus change, and now (since `e7946ff`) explicitly rather than as a side effect of the render flag.
_Buys:_ no interruption while writing; the pen never goes dead for 500 ms mid-page. _Costs:_ a user who
writes for twenty minutes without opening a menu, scrolling or losing focus never resets the SF-side
session at all. If schema 7 can wedge from something other than a crash — which we do not know — nothing
would recover it until the sheet closes.

**Option B — cycle on idle.** After N seconds with no pen activity and no gesture open, run
`cycleScribbleSession()` + a re-push. _Buys:_ bounded worst case; the heal lands in a window where the
500 ms resume delay is invisible because the user is not writing. _Costs:_ complexity, a new timer to get
wrong, and a real hazard the references document — notable's `cancelPendingScreenFreezeReset` exists
precisely because a delayed resume firing after raw drawing was disabled leaves "a frozen screen nothing
unfreezes" (`ref/notable/.../einkHelper.kt:326-328`).

**What the evidence says.** It cuts both ways, which is why this is a fork and not a recommendation.

- _For B:_ stock runs this cycle on 40 UI call sites (`03-raw-drawing-render-flag.md` §3.1) and the
  firmware's own handler runs `PEN_PAUSE` + `repaintEverything()` on **every finger-down**
  (`fw/.../BaseHandler.java:192-196`). A stock user rebuilds the session every few seconds. notable
  arrived at the same primitive independently (`einkHelper.kt:340-345`). We are the only implementation
  surveyed that can go minutes without one.
- _Against B:_ SurfaceFlinger has **no mechanism that degrades over time**. The exclude table is a plain
  array that only changes when written; the schema is a single enum that only latches when a teardown is
  skipped. The "transient region table fills up" hypothesis that motivated periodic healing is
  **not supported** by the binary (correction C5). The one documented way to strand schema 7 is a client
  dying without teardown (SF §2.3) — and that is a _startup_ problem, not a mid-session one.
- Also relevant: since `e1a9da7` we run mask `SF｜APP`, so a render-disable emits `ENABLE_POST` twice and
  `PEN_PAUSE` once; we already get two independent schema heals from every pause.

**My recommendation: A, with instrumentation instead of a timer.** The binary gives no mechanism by which
a healthy session decays, and a periodic 500 ms pen outage is a real, felt cost against a hypothetical
failure. Spend the effort on F3 (startup) and on the diagnostic in the work list (item 6) instead: log
schema and `androidDrawing` via `applySFDebug`, write for twenty minutes, and _measure_ whether it wedges.
If it does, B becomes settled and cheap to add; if it does not, we have saved ourselves a timer that can
freeze the screen.

### F2 — Should `pause()` still tear down the SF session when the pause is a scroll?

**The question.** `e7946ff` put `cycleScribbleSession()` in `pause()` (`kt/OnyxInk.kt:592`).
`configure()` calls `pause(releaseDisplay = false)` on every geometry change
(`kt/OnyxInk.kt:313`), and `configure()` is bound to `scroll` with no debounce
(`ts/features/handwriting/onyx-ink.ts:297`). So scrolling the sheet now emits a schema teardown and
rebuild per scroll event.

**Option A — keep it unconditional.** _Buys:_ one rule, no way to miss a heal, and the pause path stays
trivially correct. _Costs:_ on a hot path. Each teardown restores the schema, which means the next frame
takes the full `NormalSchema` composite path rather than the ink path, and the pen is dead for the
`RESUME_DELAY_MS` that follows each one — 500 ms on this panel. During a flick-scroll that is a
continuous outage.

**Option B — cycle only on a _real_ pause.** Gate `cycleScribbleSession()` on "something drew over the
panel or took the pen away" — the named-reason path, focus loss, lifecycle, tool change — and skip it for
a pure geometry update, which only needs `pushRects()`.

**What the evidence says.** The firmware distinguishes these two cases explicitly. `onDrawViewSizeChangedImpl`
— its geometry path — is `PEN_PAUSE` → `setLimit` → resume with **no `repaintEverything()`**
(`fw/.../BaseHandler.java:159-165`), whereas the focus and finger paths both add
`repaintEverything()` (`:311`, `:195`). It does still send `PEN_PAUSE` on a layout change, so a cycle per
geometry change is not unprecedented — but the firmware's layout events are Android layout passes, not
scroll frames, and `SingleDrawViewHandler` additionally suppresses the whole path when the rect did not
actually change (`fw/.../SingleDrawViewHandler.java:17-23`). notable and mokke both debounce
(coalesced job / 500 ms) for exactly this reason.

**My recommendation: B, plus a debounce in the JS.** Two changes that stack: skip the schema cycle when
only the geometry changed, and coalesce `configure()` on scroll. `SingleDrawViewHandler`'s
"ignore if the rect did not actually change" is worth copying verbatim — we already compute a config key
(`ts/features/handwriting/onyx-ink.ts:205-207`) but it includes the scroll-derived `top`, so it changes on
every frame.

### F3 — How aggressive should the defensive startup reset be?

**The question.** SurfaceFlinger keeps no owner for the pen session and nothing sends `APP_DIE`
(§0), so any app — ours included — can leave the device with a dead band that survives reinstall.
We should clear it at startup. How much should that clear do, and how often?

**Option A — quiet reset, once, at plugin load.** `PEN_PAUSE(3)` + empty exclude, before the first
session opens. _Buys:_ fixes the observed symptom at the earliest possible moment, invisible to the user.
_Costs:_ `PEN_PAUSE` alone restores the schema but does not force a recomposite, so if the panel is
already showing stale content the user sees it until something repaints.

**Option B — full reset, on every app resume.** Add `repaintEverything()` and repeat it on every
`onResume`. _Buys:_ also repairs damage another app did while we were backgrounded, and guarantees a clean
panel. _Costs:_ `repaintEverything()` is **global, not app-scoped** (`fw/ViewUpdateHelper.java:1057-1059`)
— it is a visible full-panel event, and doing it on every resume means a flash every time the user comes
back to the app.

**What the evidence says.** The firmware does the aggressive thing: `startEACScreenNote` resets the
exclude table before every session (`fw/.../BaseHandler.java:334`) and `onWindowFocusChangedImpl` does
`pause` → `repaintEverything()` → reset → limit on **every focus gain** (`:305-321`). Stock does neither
at startup but does the strong teardown at exit (`resetEpdPost()` at `ScribbleFragment.onDestroy:1492`).
No reference app does anything defensive at startup at all.

**My recommendation: A at plugin load, and stock's exit reset at teardown.** The startup reset should be
invisible; a launch flash is a daily cost paid against a rare fault. Pair it with `resetEpdPost()` in
`destroy()` (work-list item 7) so that _our_ exits are clean and the startup reset is only ever cleaning
up after someone else. If the user reports a dead band that survives a relaunch, escalating that one
launch to include `repaintEverything()` is a one-line change — and the manual repair action (item 4)
covers it in the meantime.

### F4 — Keep `enableSideBtnErase(true)`?

**The question.** `kt/OnyxInk.kt:351` enables side-button erasing. It writes the native reader **and**
`Device.setEnablePenSideButton`, a device-global firmware setting that nothing in the SDK restores.

**Option A — keep it, and restore it on close.** _Buys:_ the side button erases, which is what a BOOX user
expects. _Costs:_ we must remember to restore, and we cannot know the pre-existing value reliably.

**Option B — drop the call and read the side button from the callback.** `onBeginRawErasing(shortcut, point)`
already carries the flag; we ignore it (`kt/OnyxInk.kt:843`). _Buys:_ no global state touched at all.
_Costs:_ if the firmware's default is off, the side button does nothing for our users until they enable it
system-wide.

**What the evidence says.** Stock **never calls it** — zero callers in the APK — and takes the side button
purely as the `shortcutErasing` flag, which `EpdShapeHandler.onBeginRawDraw` then ignores. notate goes
further and explicitly turns the _system_ side button **off** by reflection at startup
(`ref/notate/.../OnyxSystemHelper.kt:22-23`). No reference app calls `enableSideBtnErase`. So the entire
corpus treats this as the user's system-wide setting, not an app's to change.

**My recommendation: B.** Nobody else writes this flag, and writing a device-global setting we cannot
restore is the kind of thing that makes a third-party app unwelcome on someone's device. Read the
`shortcut` flag we already receive instead; if the erase-by-side-button experience then regresses, that is
a user-visible setting they can change once, in one place, for every app.

---

## Work list

Ordered by value. Each item names the file and how it would be checked on the device.

1. **Fix the exclude reset so it actually resets.** `kt/OnyxInk.kt:198` — `EXCLUDE_RESET =
arrayOf(Rect(0,0,0,0))` → `emptyArray<Rect>()`, and update the comment at `:185-197` (it currently
   states the opposite of what the binary does). _Why:_ `PenManager::setExcludeRegion` clears only on
   `n <= 0` (SF `0x52ea88`); the current payload stores a corner rect the pen gate then inflates by
   `strokeWidth/2`. _Verify:_ on a device with a known dead band, call
   `debug_ink_repaint("excludeReset")` alone and confirm the pen inks there again — today that lever
   proves nothing, because `setLimitRect` one line later is what does the work.
2. **Guard the schema cycle** (fork F2). `kt/OnyxInk.kt:592` — skip `cycleScribbleSession()` when
   `gesture.drawing`, and when the pause came from a geometry-only `configure()` (`:313`). Pair with a
   scroll debounce in `ts/features/handwriting/onyx-ink.ts:297` and a
   "rect did not actually change" check like `fw/.../SingleDrawViewHandler.java:17-23`. _Verify:_
   `adb logcat` for `enablePost` / pen-state lines while flick-scrolling a long sheet; count should drop
   from per-frame to per-gesture.
3. **Defensive reset at plugin load** (fork F3). `kt/MobileSystemPlugin.kt:69-79` — `PEN_PAUSE(3)` +
   empty exclude before any session exists, in the firmware's order
   (`fw/.../BaseHandler.java:331-342`). _Verify:_ force-stop the app mid-stroke, relaunch, confirm the
   band inks immediately rather than after opening the sheet.
4. **A user-reachable panel repair action.** Promote the two levers out of `debugRepaint`
   (`kt/OnyxInk.kt:395-415`) into one command that runs `repaintEveryThing()` **then** the empty-exclude
   push, and surface it in settings. _Why:_ it is the only recovery from damage another app left, and
   `HANDWRITING_REPAINT` provably cannot do it. _Verify:_ reproduce the dead band, tap it, confirm both
   halves heal in one action.
5. **Debounce the erasing reconcile.** `kt/InkReconcileGate.kt:44-48`, `kt/OnyxInk.kt:766` — an erase
   gets no `onPenUpRefresh` (`pen/RawInputReader.java:613-616`), so it falls through to `Next.SOON` and
   reconciles every 120 ms. Give it its own `PEN_UP_REFRESH_MS` timer, cancelled on the next `begin()`,
   mirroring `fw/.../BaseHandler.java:268-283` + `:234`. _Verify:_ count `repaintCount` in `status()`
   across ten seconds of continuous erasing; it should fall to roughly one per pause in erasing.
6. **Make the stuck state observable.** Add `EpdController.getPenState()` to `status()` and a debug
   toggle for `applySFDebug`, whose log line is
   `"pen control state: %s, penTriggered: %d, current epdc schema: %d, androidDrawing: %d"`
   (`applySFDebug` @ `0x561b78`). _Why:_ it turns "the panel looks wrong" into "schema 7,
   `androidDrawing` 0" and is the measurement fork F1 depends on. _Verify:_ read the line with a healthy
   session, then after forcing a dead band.
7. **Copy stock's exit reset.** `kt/OnyxInk.kt:717-723` — end `destroy()` with
   `EpdController.resetEpdPost()` (= `ENABLE_POST(-1,1,pid)` + `setScreenHandWritingPenState(0)`,
   `fw/ViewUpdateHelper.java:1069-1077`) and one empty-exclude push. Stock does this at
   `ScribbleFragment.onDestroy:1492` and `EpdEventHandler.destroy:197`. _Verify:_ close the app cleanly,
   then check from a second app (or after a relaunch) that the band is live.
8. **Assert the repaint mode is real.** `kt/OnyxInk.kt:138` — `lastRepaintModeRaw` is logged but never
   checked; `UpdateMode` → firmware int yields **0** for a missing constant with no error, and
   `HAND_WRITING_REPAINT_MODE` has no fallback. Compare once per session against `524290`
   (`fw/ViewUpdateHelper.java:77`) and report it in `status()`. _Verify:_ read `status()` on device; a 0
   would mean every reconcile frame this whole time has been marked as mode 0.
9. **Re-apply the eraser style on resume.** Move `setEraserRawDrawingEnabled(false, 0)`
   (`kt/OnyxInk.kt:350`) into `RawDrawingGate.pushRects()` (`kt/OnyxInk.kt:114-124`), because
   `resetPenDefaultRawDrawing()` restores style 5 on every resume (`pen/TouchHelper.java:348-351`).
   Stock re-applies its eraser parameters on every resume for this reason
   (`stock/note/note/request/pen/ResumeRawDrawingRequest.java:64-69`). _Verify:_ pause and resume, then
   erase; the eraser must still leave no firmware trace.
10. **Stop writing a device-global setting** (fork F4). `kt/OnyxInk.kt:351` — drop
    `enableSideBtnErase(true)` and use the `shortcut` flag already delivered to
    `onBeginRawErasing` (`kt/OnyxInk.kt:843`); or, if kept, restore it in `close()`. _Verify:_ check the
    system pen setting before and after a session.

Beyond the top ten:

11. Correct the region-mode comment (`kt/OnyxInk.kt:115-119`): the claim about a filling transient-region
    table is not supported by the binary (correction C5). Keep the call for parity with stock; fix the
    reasoning.
12. Fold the unconditional 750 ms net at `kt/OnyxInk.kt:798` into `InkReconcileGate` so one timer owns the
    reconcile, and drop the `penUpRefreshSeen` guard at `kt/InkReconcileGate.kt:47` now that the net covers
    the deadlock case.
13. Delete or correct the three profile calls that cannot take effect on this firmware:
    `applyAppRefreshProfile()` (`kt/MobileSystemPlugin.kt:294-322`), the `REGAL` BASE layer (`:245-251`),
    and `ensureSpeedRefreshProfile()` (`:264-292`), plus the wrong comment at `:461`. Owned by
    `05-panel-refresh-levers.md` §5 / the EinkWise report; listed here so it is not lost.
14. Update `docs/planning/handwriting-pen-stack-comparison.md` with corrections C1–C6, in particular §2a
    ("survives … but not restarting the app"), §12 item 8 (the force-stop prediction is falsified), §2c
    (region mode is now readable) and §13 (the off-panel-rect suggestion is superseded).

### Known no-ops — remove or annotate, do not "fix"

- `helper.setHostViewScrollListenerEnabled(false)` (`kt/OnyxInk.kt:347`) — `RawInputManager`'s
  implementation is an empty body on this device class. Dead code.
- `clearLatchedExcludes()` **as currently written** (`kt/OnyxInk.kt:622-625`) — does not clear
  (item 1), and is in any case redundant with the `APP` half of the `setLimitRect` on the next line
  (correction C2). Fixing the constant makes it real; leaving it as-is makes it noise.
- The `onPenUpRefresh` re-post to the main thread (`kt/OnyxInk.kt:848-852`) and its comment
  ("Delivered off the UI thread by an RxTimer") — `SFTouchRender` already wraps **every** callback in
  `getHostView().post(...)`, so the rect is on the main thread already and the extra hop only costs a
  message-loop turn. _(Related hazard worth a guard: `SFTouchRender`'s `onPenUpRefresh` wrapper is the one
  of ten that omits the null-host check its siblings have, and the host view is held weakly — an NPE
  there kills the reader for the whole process. Since `34dc906` this callback is on our critical path.)_
- `setSingleRegionMode()` (`kt/OnyxInk.kt:120`, `:348`) — behaviourally a no-op with one limit rect, on
  both the native and the SF side. Keep for parity with stock; do not attribute effects to it.
- `applyAppRefreshProfile()`, the `REGAL` BASE layer, `ensureSpeedRefreshProfile()` — see item 13.

### Verdict on the three unverified "fix" commits

All three predate the reversing work and none claims device verification. The binary now settles all
three, and all three should be **kept**.

- **`b5c9382` — "stop reconciling frames mid-gesture and let the menu track the drag."** _Keep, as
  already narrowed by `3e0036e`._ Its principle is now independently confirmed: compositing the web
  canvas over a trace the firmware is still drawing destroys the part drawn before that moment, and — a
  reason unavailable when it was written — the `false` leg of any render-flag or schema cycle is
  `PEN_PAUSE`, which kills the firmware ink of a stroke in flight (`pen/touch/SFTouchRender.java:194-197`
  → SF `0x5667d4`). "Never touch the session mid-gesture" is now a hard invariant, not a heuristic. Its
  second half — positioning the selection menu by writing `style.left/top` directly rather than through
  React state — matches stock, which repositions its `SelectionPopupMenu` every ~10 ms.
- **`25f92c3` — "never drop the damaged region of a frame that was not repainted."** _Keep; better
  justified now than when written._ The original reasoning ("a panel keeps whatever was last pushed to
  it") is correct, and §0 sharpens it: while the session is live SurfaceFlinger is in schema 7 and does
  **not** composite our layers, so a region dropped by the fence is not merely stale — nothing in the
  ordinary draw path will ever cover it again. `InkReconcileRegion.putBack` (`kt/InkReconcileRegion.kt:66-70`)
  is load-bearing.
- **`3e0036e` — "reconcile ink as it is written, hold only the lasso trace."** _Keep._ This is the
  commit that partially reverted `b5c9382`, and the binary explains why it had to: firmware ink is
  transient and the app is expected to take it over, and until it does, the region shows whatever the
  panel last held, because schema 7 refuses our frames. Holding only the lasso trace is right for the
  same reason inverted — the canvas has nothing to put in the trace's place, so reconciling mid-lasso
  erases it.

### On the debounce

The brief's premise — "our per-stroke reconcile has no debounce while the firmware's does" — is **half
true, and the half that is false is worth recording.** Since `34dc906` the reconcile is driven by
`onPenUpRefresh`, and the SDK's timer behind it _is_ a debounce with cancel-on-pen-down: armed on pen-up
(`pen/RawInputReader.java:278-283`), cancelled on the next pen-down and on every move (`:286-289`, called
from `:708` and `:725`), with the same 500 ms the firmware uses (`:131` vs
`fw/.../EACNoteConfig.java:21`). So ordinary handwriting is already debounced exactly as the firmware
debounces it. What is genuinely missing is the debounce on the **erasing** path, which gets no such
callback at all (work-list item 5), and — trivially — on the first stroke of a session (item 12).
