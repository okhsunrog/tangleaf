# SurfaceFlinger reverse-engineering — the receiving end of the Onyx handwriting stack

Target: `/home/okhsunrog/tmp_zfs/onyx_framework/native/surfaceflinger`
Device: Onyx BOOX Note Air 4C (`lito`, Android 13, build `2026-04-28_17-50_4.2-rel_04282_555977efe`).
ARM64, PIE, stripped **but** it carries a `.gnu_debugdata` MiniDebugInfo section (`llvm-objcopy --dump-section=.gnu_debugdata=… | xz -d`) with **22,519 local FUNC/OBJECT symbols** — so almost every Onyx method has a real name and address. All addresses below are file/virtual addresses (identity-mapped in the executable segment) and are quoted with the string or symbol that anchors them so they can be jumped to directly.

**Method note.** Everything stated as "reads … / writes … / calls …" was decoded from the instruction stream. Everything stated as "means / is" for a field name is inference from how the field is used, and is flagged. `android::EpdcWrapper::{mergeByMode,addEpdc,append,getBatch,clear,isEmpty}` are **UND imports** (resolved at runtime from the Onyx-patched `libui.so`, not present in this dump) — their bodies could not be read; I describe them only from their call sites. One thing I could **not** settle statically is named at the end of each section.

Throughout, two process-global singletons are referenced through the GOT:

- `*(0xac9fb8)` — the **EpdcManager / SurfaceFlingerProvider-helper singleton** (`HWEpdcManager` on this HW-Tcon colour device; see `setSurfaceFlingerProvider` @ `0x55eba4`, which reads `property_get_bool("vendor.onyx.htcon")` @ `0x55ebc4` and picks `HWEpdcManager` when true). Call it **EPDC**.
- `*(0xac9ea8)` — the **SurfaceFlingerProvider** (the bridge back into stock SurfaceFlinger; vtable calls only).
- `*(0xac9fc8)+0x120` — the **PenManager** (`handleSetHandWritingRegionExclude` @ `0x560e1c` loads `[0xac9fc8]`, then `+0x120`, then calls `PenManager::setExcludeRegion`).

---

## 1. Transaction dispatch table

`android::SurfaceFlingerHelper::processOnyxRequests(unsigned int code, const Parcel&, Parcel*)` @ **`0x55f260`** (5128 bytes; anchored by `"processOnyxRequests %x"` @ `0x4b11c`, logged on entry). It is the single Onyx transaction demuxer, reached from the stock `onTransact` for the private code range.

Dispatch is two hardware jump-tables (`br x10` off a `ldrh` index), which I decoded byte-for-byte:

- **Table A** at `0x341094`, base `0x55f2e0`, 0x1d8 entries, code range `0x10002a…0x1001ff` (the `0x10xxxx` family).
- **Table B** at `0x341444`, base `0x55f314`, 0x71 entries, code range `0xff0000…0xff0070` (the `0xff00xx` family).

Codes not in a table fall through to `0x5605fc` (no-op / return). Below are the handlers that matter to the handwriting/region/refresh problem, each mapped to what it mutates. (The full 100+ entry table is in the binary; this is the load-bearing subset, all confirmed by following the `bl` at the jump target.)

| Code (dec)                 | Name (Java)                          | Handler @                                                                                                                           | Mutates                                                                                     |
| -------------------------- | ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `16711680` `0xff0000`      | REFRESH_SCREEN? (mode overload path) | reads int, → helper                                                                                                                 | —                                                                                           |
| `16711681` `0xff0001`      | REPAINT region                       | `handleRefreshScreen(x,y,w,h,mode)` @ `0x561038`                                                                                    | schedules a bounded refresh                                                                 |
| `16711692` `0xff000c`      | **ENABLE_POST**                      | `handleEnablePost(a,b,c)` @ `0x560bf8`                                                                                              | **conditionally calls `EpdcManager::stopHandwriting`** (see below)                          |
| `16711693` `0xff000d`      | **SET\_…PEN_STATE**                  | `handleSetHandWritingPenState(state,pid)` @ `0x560c4c`                                                                              | stores pid at `EPDC+0xf90`; `addToWatchApp(pid)`; calls provider vtable `+0xe8` (pen-state) |
| `16711694` `0xff000e`      | **…REGION_LIMIT**                    | `handleSetHandWritingRegionLimit(v,int*,n)` @ `0x560d0c`                                                                            | `PenManager::setLimitRegion`                                                                |
| `16711697` `0xff0011`      | START_STROKE                         | `handleStartStroke(...)` @ `0x560f2c`                                                                                               | pen path                                                                                    |
| `16711698/9` `0xff0012/13` | ADD_STROKE_POINT / FINISH_STROKE     | `readFloat×6` → pen path                                                                                                            | pen path                                                                                    |
| `16711700` `0xff0014`      | **REPAINT_EVERYTHING**               | `0x56007c`: `str wzr,[EPDC+0x2c]` then provider vtable `+0x10`                                                                      | forces full recomposite                                                                     |
| `16711703` `0xff0017`      | WAIT_FOR_UPDATE                      | `handleWaitForUpdateFinished` @ `0x561564`                                                                                          | —                                                                                           |
| `16711712` `0xff0020`      | (mode read)                          | `0x5600b8`: `ldr s0,[EPDC+0x110]`                                                                                                   | —                                                                                           |
| `16711713` `0xff0021`      | REPAINT_EVERYTHING (mode)            | `0x5601a4`: `str wzr,[EPDC+0x2c]`, `str mode,[0xac9dd8]`, `str 1,[0xac9dd8+8]`, provider vtable `+0x10`                             | full recomposite w/ mode                                                                    |
| `16711714` `0xff0022`      | **…REGION_EXCLUDE**                  | `handleSetHandWritingRegionExclude(v,int*,n)` @ `0x560e1c`                                                                          | `PenManager::setExcludeRegion`                                                              |
| `16711715` `0xff0023`      | REPAINT_EVERYTHING mode-overload     | `0x5601a4` region (mode+region)                                                                                                     | `EPDC+0xdd8` update entry                                                                   |
| `16711716` `0xff0024`      | SET_UPD_LIST_SIZE                    | `handleSetUpdListSize` @ `0x561764`                                                                                                 | `FBDev::setUpdListSize`                                                                     |
| `16711717` `0xff0025`      | **APP_DIE**                          | `handleAppDie(pid)` @ `0x561798`                                                                                                    | see §2 — real cleanup, but no Java caller                                                   |
| `1048620` `0x10002c`       | **…REGION_MODE**                     | `handleSetHandWritingRegionMode(m)` @ `0x561ad4` → lambda `0x564f74`: `str m,[PenManager+0x38]`                                     | pen region mode                                                                             |
| `1048647` `0x100047`       | **HANDWRITING_REPAINT**              | `handleHandWritingRepaint(x,y,w,h,bool)` @ `0x5616bc` → lambda `0x5655e0` → `EpdcManager::setHandWritingRepaintUpdate` @ `0x55828c` | sets a **pending** ink-repaint entry; **stays in handwriting schema**                       |
| `1048661` `0x100055`       | SET_EPD_TURBO                        | `MemoryPainter::saveAttachedFB` path / `setEpdTurbo`                                                                                | turbo                                                                                       |
| `1048658` `0x100052`       | SET_GC_REFRESH_INTERVAL              | reads int → helper                                                                                                                  | GC interval                                                                                 |
| `1048722` `0x100092`       | SET_AUTO_SYNC_BUF_ENABLE             | readBool → helper                                                                                                                   | `enableExtBufAutoSync`                                                                      |
| `1049600` `0x100200`       | SAVE_PEN_ATTACHED_FB                 | `MemoryPainter::saveAttachedFB` @ `0x525148`                                                                                        | pen FB snapshot                                                                             |

Notably **REPAINT_EVERYTHING (`0xff0014`/`0xff0021`) and ENABLE_POST both route into provider vtable `+0x10` / `stopHandwriting`, whereas HANDWRITING_REPAINT (`0x100047`) does not** — this is the whole of the answer to question 1's "why repaint doesn't heal."

Access control: `CheckTransactCodeCredentials(code)` @ `0x55f1d0` exists and is called from `onTransact`, but the handwriting/region/repaint codes are in the permitted set — an app already holding the vendor's hidden-API access (which yours does) reaches every handler above with no per-call check.

---

## 2. The region stores and their lifetimes

### 2.1 The exclude / limit stores are in **PenManager**, global, and not keyed by anything

`handleSetHandWritingRegionExclude` @ `0x560e1c`:

```
0x560e58  pthread_mutex_lock(0xac9eb0)          ; the single pen mutex
0x560e68  loop: dumpMessage("Set exclude region: (%d %d) - (%d %d)")   ; str @ 0x18be40
0x560e88  if (view!=null && n>=2) { transform each point by provider vtable +0x30 }
0x560ee8  PenManager::setExcludeRegion(int* pts, int n)   ; via [0xac9fc8]+0x120
```

`handleSetHandWritingRegionLimit` @ `0x560d0c` is byte-identical with `"Set limit region: (%d %d) - (%d %d)"` (`0x217239`) and `PenManager::setLimitRegion`.

**`PenManager::setExcludeRegion(int* src, int n)` @ `0x52ea58`** — decoded exactly:

```
if (this->count(+0x2040) >= 1) memset(this+0x1040 .. , 0, count words)   ; zero old array
this->count(+0x2040) = 0
if (n <= 0) { this->count = n; return }        ; <-- empty array fully clears
copy n ints from src to this+0x1040
this->count(+0x2040) = n
(SIMD max/min normalisation of the rects)
```

`setLimitRegion` @ `0x52e948` is the same shape writing `count` at **`+0x103c`** and the array at **`+0x104c`**.

PenManager field map (from these two functions + `inValidRegion`):

| Offset            | Meaning (inferred)                   | Evidence                                                              |
| ----------------- | ------------------------------------ | --------------------------------------------------------------------- |
| `+0x34`           | stroke width (float)                 | `inValidRegion` `ldr s2,[x0,#0x34]`, `×0.5` inflate                   |
| `+0x38`           | region **mode** (SET\_…REGION_MODE)  | lambda `0x564f74` `str m,[+0x38]`; read in `inValidRegion` `0x52e648` |
| `+0x103c`         | **limit count**                      | `setLimitRegion`; `inValidRegion` `0x52e5c8`                          |
| `+0x104c`         | limit rect array                     | `setLimitRegion` copy loop                                            |
| `+0x1040`         | **exclude rect array** (4 ints/rect) | `setExcludeRegion` copy loop                                          |
| `+0x2040`         | **exclude count**                    | `setExcludeRegion` `str w2,[+0x2040]`                                 |
| `+0x2044`         | last-region-hit index                | `PenManager::onEvent` `0x52e3a8`; `inValidRegion` `0x52e674`          |
| `+0x205c/+0x2060` | probe x/y                            | `inValidRegion` `ldr s1,[+0x205c]; ldr s0,[+0x2060]`                  |

**Lifetime / ownership — the crucial finding.** Neither handler reads a pid, uid, surface token or window token. There is exactly **one** PenManager (`*(0xac9fc8)+0x120`), one exclude array, one limit array, one mode. This confirms the Java-side conclusion in `REPORT-regions.md §1`: **the exclude and limit tables are process-global in SurfaceFlinger, shared by every client, and survive the death of whichever app set them.** Nothing in `handleAppDie` (§2.3) touches the exclude/limit arrays. A reboot restarts the surfaceflinger process and re-zeroes them; nothing else in the transaction surface clears them except a fresh `setExcludeRegion`/`setLimitRegion` call.

**The "null view / `{0,0,0,0}`" reset, confirmed at the receiving end.** When the Java side sends the vendor's `resetScreenHandWritingRegionExclude` idiom — `setScreenHandWritingRegionExclude(null, {0,0,0,0})` — the parcel carries `view==0` and the rect payload. In `handleSetHandWritingRegionExclude` the `view!=null` branch (`cbz w21` @ `0x560e88`) is skipped, so **no per-point transform is applied**, and `setExcludeRegion(pts, n)` runs. The firmware's reset actually sends `n` such that the store is wiped: the decisive line is `setExcludeRegion`'s `if (n<=0) {count=n; return}` at `0x52ea88`/`0x52eb3c` — an **empty** array zeroes the count and leaves an empty exclude set. (A literal one-rect `{0,0,0,0}` payload with `n=4` instead **stores one degenerate rect**, which `inValidRegion` still inflates by `strokeWidth/2` on all sides and excludes a small box at the origin — matching `REPORT-regions.md §4`. So the true reset is the _empty_-array form, which is what `resetScreenHandWritingRegionExclude` and the framework's own focus-gain handler send.)

### 2.2 `inValidRegion` — the pen gate (the "pen cannot ink here" half)

`PenManager::inValidRegion()` @ `0x52e5c4` is the pen-side geometry test, called from `BaseReader::onEvent` @ `0x52dfcc` before `moveTo`/`quadTo`. It:

1. Inflates the probe point by `strokeWidth/2` (`fmul s5,s2,#0.5`).
2. Walks the **limit** array (`+0x103c`/`+0x104c`): the point must be inside a limit rect (returns `false`/0 if outside any, via the `0x52e678` loop).
3. Walks the **exclude** array (`+0x2040`/`+0x1040`, the `0x52e5dc` loop): if the inflated point is inside an exclude rect it returns 0.

So a latched exclude rect makes every pen sample inside it return "not valid" → the stroke is dropped before `moveTo`. This is the **"pen cannot ink there"** half, and it is purely PenManager state — independent of the compositing schema.

### 2.3 App-death handling exists in SF but is never triggered

`SET_…PEN_STATE` (`0xff000d`) records the caller pid at `EPDC+0xf90` and calls `addToWatchApp(pid)` @ `0x562c70`, which maintains a plain **`std::vector<int>` of pids** at `0xac9ed8` (begin/end/cap). `afterStartHandwriting` @ `0x562c08` also re-adds the current pen pid.

`handleAppDie(pid)` @ `0x561798` posts a lambda (`$_24` @ `0x565358`) onto the composite thread that:

- logs `"App die %d %d"` (`0x1f56cc`),
- if the dying pid equals the pen-owner pid (`EPDC+0xf90`, compared at `0x56545c`), calls **`handleSetHandWritingPenState(0 /*STOP*/, pid)`** @ `0x565478` and then provider vtable `+0x10` (**repaintEverything**, `0x565494`),
- removes the pid from the watch vector (`memmove` @ `0x565528`).

**This is a correct, complete teardown — but it is only reachable through transaction `0xff0025`, and the Java framework has no caller of `APP_DIE`.** There is **no `linkToDeath`/binder death-recipient** anywhere on the Onyx path: the "watch app" list is just an integer pid vector with no binder handle, so SF cannot notice a client dying on its own. Consequently, if the app that started a pen session dies without someone sending `APP_DIE`, the session is **never torn down**: the schema stays Handwriting and the pen pid stays latched. This is the mechanism by which the dead state "survives the app process dying and a reinstall."

---

## 3. The handwriting state machine

Two independent state variables, plus the pen-input gate, must all be understood:

- **Active EPDC schema id** at `EPDC+0x9c` (== `EpdcSchemaManager` (embedded at `EPDC+0x98`) field `+0x4`). Values seen at `activate(id)` call sites: `0`=Normal, `1`=Debouncer, `5`=PowerOff (`shutdownRequest`), `6`=Boot, `7`=**Handwriting** (`startHandwriting` @ `0x5585dc` calls `activate(7)`), `8`=Dream (`dozingRequest`).
- **`androidDrawing`** = `EPDC+0x28` (byte). Set to `0` by `afterStartHandwriting` @ `0x562c40` (`strb wzr,[+0x28]`) and to `1` by `afterStopHandwriting` @ `0x562f10` (with `"Post enabled!"` log `0x21722b`). Inference: when 0, SF is _not_ posting composited Android content to the panel — only the ink path draws.
- **`penTriggered`** = `0xaca244` (byte), set 1 by `handlePenTriggerImpl`/`afterStartHandwriting`, cleared by `afterStopHandwriting`. **`mHWRepaintPending`** = `EPDC+0xe60`, set by `setHandWritingRepaintUpdate` @ `0x5582b8`, cleared by `HandwritingSchema::onDeactivate` @ `0x55b820`. `EPDC+0xea0` is **the SF debug-log flag** (`applyDebug`), _not_ a state bit — the many `ldrb [x8,#0xea0]; cbnz` are log gates; do not read them as logic.

The four `pen control state` strings decoded from `applySFDebug` @ `0x561b78` (`"pen control state: %s, penTriggered: %d, current epdc schema: %d, androidDrawing: %d"` @ `0x80238`) are the four variables above: `%s` is `TouchReader::ctrlStateStr` @ `0x52c504` (0=`stop`,1=`start`,2=`running`,3=`toPause`,4=`paused`,5=`toResume`; and loop states `0x100`+), `penTriggered`=`0xaca244`, schema=`getCurrentSchema` (`EPDC+0x9c`), `androidDrawing`=`EPDC+0x28`.

`HWEpdcManager::setHandWritingPenState(int state)` @ `0x5666bc` (vtable slot `+0xe8`; `handleSetHandWritingPenState` at `0xff000d` calls it via provider vtable `+0xe8`). Jump-table at `0x34152a`:

```
state 0 (STOP)    @0x5666f4: EBAS log; getFBDev(); FBDev::enableExtBufAutoSync(0);
                             TouchReader::stop(); enableCytpLoFilter(0)
state 1 (START)   @0x566744: FBDev::enableExtBufAutoSync(EBAS); TouchReader::start();
                             enableCytpLoFilter(); return 1
state 2 (RESUME)  @0x5667a0: TouchReader::ensureStart(); TouchReader::resume();
                             (does NOT restore the EPDC schema)
state 3 (PAUSE)   @0x5667d4: TouchReader::pause();
                             tail-call EpdcManager::stopHandwriting()   <-- schema restore
```

`startHandwriting` (schema→7) is entered **not** by a pen-state transaction but by `handlePenTriggerImpl` @ `0x562f24` (guarded by `penTriggered`) and by `moveToImpl`/`PenManager::moveTo` @ `0x52e81c` when `canScribble()` is true. `canScribble()` @ `0x55915c` = (`schema==7` AND panel type `EPDC+0x24 ∈ {1,2,3}` — the colour/CFA class).

Text diagram (schema + androidDrawing + pen), one panel:

```
                       boot: activate(0)=Normal, androidDrawing=1
                                     │
        pen down in a valid limit region, canScribble()  │  (PenManager::moveTo → startHandwriting)
                                     ▼
   ┌─────────────────────────  HANDWRITING (schema=7)  ─────────────────────────┐
   │  afterStartHandwriting: androidDrawing=0, penTriggered=1                    │
   │  HandwritingSchema::output (0x55b828):                                      │
   │     collectLayerEpdcList → push ONLY ink epdc; Android layers NOT           │
   │     composited over the handwriting area.                                  │
   │     "### warning: commit epdc in handwriting mode." (0xf8764) if forced     │
   │  HANDWRITING_REPAINT (0x100047): setHandWritingRepaintUpdate → mHWRepaint   │
   │     pending=1, pushes ONE ink update via HandwritingSchema::output.         │
   │     ***STAYS in schema 7. androidDrawing STAYS 0.***                        │
   └────────────────────────────────────────────────────────────────────────────┘
        │pen state 3 (PAUSE)        │ENABLE_POST(int2)      │REPAINT_EVERYTHING (0xff0014/21)
        │→ stopHandwriting          │→ stopHandwriting      │→ provider vtable +0x10
        ▼                           ▼ (if androidDrawing!=1)▼
   stopHandwriting (0x558630): if schema==7 → EpdcSchemaManager::restore →
        activate(lastSchema, else Normal=0);  onDeactivate: mHWRepaint=0
        afterStopHandwriting: androidDrawing=1, penTriggered=0
                                     ▼
                       NORMAL (schema=0): NormalSchema::output composites the
                       whole screen ("### sf epdc region received" 0x1f56ab),
                       full-screen Android content is posted again.
```

`stopHandwriting` @ `0x558630` decoded: takes the 300 ms `timed_mutex`, then `if (EPDC+0x9c == 7) EpdcSchemaManager::restore(EPDC)` (`0x5586b0`); otherwise logs `"### not in handwriting schema."` (`0x3327f`) and does nothing. `EpdcSchemaManager::restore` @ `0x55cdf0` re-`activate`s the saved `lastSchema` (or Normal with `"### Restore schema to normal schema with invalid lastSchema"` @ `0x1ca06e`).

`android::EpdcManager::applyUpdateAction` @ `0x558ce8` contains the **`"### do not switch epd Schema in handwriting mode"`** guard (`0x104f3c` @ `0x558e60`): while in the handwriting schema, ordinary layer-driven schema switches are suppressed — so a stuck handwriting schema will not self-correct from normal app drawing.

**Which state stops SF compositing the app's own layers:** the **Handwriting schema (id 7) with `androidDrawing==0`.** In that state `HandwritingSchema::output` (`0x55b828`) only ever calls `collectLayerEpdcList` + `createEpdcByUpdateEntry` for the ink/update entries and `BaseEpdcSchema::commitEpdc`; it does not do the full-screen `NormalSchema` composite. What returns it is anything that calls `stopHandwriting` → `EpdcSchemaManager::restore` → `NormalSchema::output`, i.e. **PAUSE(3), ENABLE_POST, or REPAINT_EVERYTHING**.

Static gap I could not close: whether, on this build, a stuck region is the _schema_ latch (whole panel, but visually looks rectangular because only the limit-region area was being ink-drawn) or specifically the exclude rect. Both are consistent with the symptom. The single on-device observation that settles it: with the region dead, read logcat for `applySFDebug`'s `"pen control state…"` line — if `current epdc schema: 7` and/or `androidDrawing: 0` it is the schema latch; if schema is 0 and the app still can't ink, it is the exclude table.

---

## Answer to question 1 — the dead rectangle, stated plainly

There are **two independent latches**, and the permanently-dead rectangle is produced when both are set and the normal teardown never runs:

1. **Pen half ("cannot ink"):** a non-empty rect in the **global** `PenManager` exclude array (EPDC-side singleton `*(0xac9fc8)+0x120`, field `+0x1040`, count `+0x2040`), set by `SET_SCREEN_HANDWRITING_REGION_EXCLUDE` (`0xff0022`). It is **not keyed by pid/uid/window**, is **not cleared by any pen-state transition, by `ENABLE_POST`, by `REPAINT_EVERYTHING`, by re-setting the limit, or by `handleAppDie`**, and it survives app death and reinstall because the surfaceflinger process outlives the app. It is cleared only by another `setExcludeRegion` with an **empty** array (`n<=0` at `0x52ea88`), or by rebooting surfaceflinger.

2. **Content half ("not composited"):** SurfaceFlinger stuck in the **Handwriting EPDC schema (id 7) with `androidDrawing==0`**. In that schema `HandwritingSchema::output` composites only ink/update entries, and `applyUpdateAction` actively refuses to switch schema (`"### do not switch epd Schema in handwriting mode"`). It leaves this state only via `EpdcManager::stopHandwriting` → `EpdcSchemaManager::restore` → `NormalSchema::output`. The session is entered on pen-trigger and is expected to be torn down by the owning app; **SF has no binder death recipient** (the "watch app" list at `0xac9ed8` is a bare pid vector), and the one path that would auto-recover on death, `handleAppDie` (`0xff0025`, which does STOP + repaintEverything), **has no Java caller** — so a crashed or ungraceful client leaves the schema latched indefinitely.

**Cheapest heal for an unprivileged app:** you must clear _both_ latches, and the two candidates each clear exactly one:

- Candidate (a) `setScreenHandWritingRegionExclude(null, {0,0,0,0})` — reaches `setExcludeRegion` with the empty-array reset and **clears the pen-exclude latch**, but does **nothing** to the schema/compositing latch (that handler never touches EPDC schema).
- Candidate (b) the render-flag cycle `ENABLE_POST(-1,1,pid)` + pen state 3 (PAUSE) + pen state 2 (RESUME) — pen-state **3** tail-calls `stopHandwriting`, and `ENABLE_POST` also calls `stopHandwriting` when `androidDrawing != 1`, so this **clears the schema/compositing latch** and repaints, but it does **not** touch the exclude array.

So the minimal robust sequence is **(b) then (a)** (or (a) then (b)): send **pen state 3 (PAUSE)** — this alone runs `stopHandwriting`→`restore`→`NormalSchema` and recomposites the whole screen; then send **`setScreenHandWritingRegionExclude(null, empty)`** to wipe the pen-exclude table. `ENABLE_POST` and `REPAINT_EVERYTHING` are equivalent to the PAUSE for the content half, but `REPAINT_EVERYTHING` is the cleanest single call for it (`0xff0014` → provider `repaintEverything`, no side effects on pen state). `HANDWRITING_REPAINT` (`0x100047`) will **not** heal either half — it stays in schema 7 and only queues one ink entry (`setHandWritingRepaintUpdate`, `mHWRepaintPending=1`). This is exactly why, in your logs, a per-stroke repaint request never recovered the region while a pause/resume did.

---

## Question 2 — pen-session ownership and lifetime

The session is keyed by **one global pid slot** (`EPDC+0xf90`), written on every `SET_SCREEN_HANDWRITING_PEN_STATE` (`handleSetHandWritingPenState` @ `0x560cbc`) and mirrored into the "watch app" pid vector at `0xac9ed8` via `addToWatchApp`. There is **no per-session object, no binder token, and no `linkToDeath`** — SurfaceFlinger cannot observe a client dying. The only death path is the `APP_DIE` transaction (`0xff0025` → `handleAppDie` @ `0x561798`), which, when the dying pid matches the pen owner, does a genuine teardown — `handleSetHandWritingPenState(0=STOP, pid)` then `repaintEverything` — but nothing in the Java framework sends `APP_DIE`, so this never fires in practice. **Pen state 3 (PAUSE)** and **0 (STOP)** differ precisely: PAUSE (`0x5667d4`) does `TouchReader::pause()` **and tail-calls `EpdcManager::stopHandwriting()`** (schema restore + full recomposite), while STOP (`0x5666f4`) tears down the input side (`enableExtBufAutoSync(0)`, `TouchReader::stop()`, `enableCytpLoFilter(0)`) but by itself does **not** call `stopHandwriting`; the schema is restored on STOP only through the higher-level `afterStopHandwriting`/`ENABLE_POST` path. That asymmetry is why a **pause/resume pair heals a stuck region** — PAUSE runs `stopHandwriting`→`EpdcSchemaManager::restore`→`NormalSchema::output`, which recomposites the whole panel and drops the handwriting schema — **whereas a `HANDWRITING_REPAINT` does not**: it calls `setHandWritingRepaintUpdate` (`0x55828c`), which only sets `mHWRepaintPending` and a single update entry and re-enters `HandwritingSchema::output`, never leaving schema 7 or setting `androidDrawing=1`, so the app's own layers are still not composited over the region.

## Question 3 — what the update modes really do

Requested mode → waveform is resolved twice, in `FBDev::refreshScreen` @ `0x55a588` (the `SET_EBC_SEND_UPDATE` ioctl `0x700c` path, string `"refresh screen (%d,%d-%d,%d) waveform_mode %d flags 0x%x"` @ `0x77a7d`) and `FBDev::hwScreenRefresh` @ `0x55b074` (HW-Tcon path). Both switch on `mode & 0xF` through a jump-table; I decoded them: on the full path `1→wf1(DU/GU), 2→2, 3→3, 4→4, 5→255(INIT/GC), 6→5, 11→11, 12→12, 7..10→"onyx_epdc_update_to_display(): waveform_mode wrong"` (`0x262522`, downgraded to `0` = no waveform), `13→special`. On the **HW-Tcon** path `hwScreenRefresh` accepts only `mode&0xF ∈ {1..6}`; anything else logs **`"onyx_epdc_hwscreenRefresh(): waveform_mode wrong!"`** (`0x1d161a`) and is rejected. High mode bits are decoded as flags into the ioctl `flags` word: bit12→`0x8000` (a "no-partial"/special), bit16→`0x10000`, bit17→`0x20000`; **`HAND_WRITING_REPAINT_MODE = 524290 = UI_GU_MODE | 0x80000`** feeds bit19 → `0x80000` in `refreshScreen` (`0x55a6b0`), i.e. GU waveform plus the "handwriting-repaint" region flag that keeps it on the fast ink path rather than a full GC; bit18→`0x40000`; `0x500000` mask→`0x200000`. So `HAND_WRITING_REPAINT_MODE` is a plain GU waveform with the `0x80000` region flag set, distinguishing it from a bare GU so the panel treats it as an incremental ink refresh, not a screen-wide grey update. On this Kaleido/CFA colour panel (`isColorDevice`/`getCfaMode` @ `0x5593f0`, `isHWTconColorDevice` @ `0x559614` true → `HWEpdcManager`), the `hwScreenRefresh` restriction to modes 1–6 means A2/DU4-style high modes are silently rejected/downgraded. `EpdcWrapper::mergeByMode` and `getBatch` (called from `EpdcManager::collectLayerEpdcList` @ `0x557e3c`/`0x557e44`) coalesce the per-layer `hwc_epdc_llist` entries **grouped by waveform mode** into batched `SET_EBC_SEND_UPDATE` lists before commit — so region updates that share a mode are merged into one ioctl, and updates with different modes are serialised into separate batches. **Caveat:** `mergeByMode`/`getBatch`/`addEpdc` are UND imports resolved from the Onyx-patched `libui.so` and their bodies are **not in this binary**, so the exact merge rule (union vs. bounding-box, and whether adjacent same-mode rects are combined or kept as a list) cannot be read here; the single on-device observation that settles it is to log the `dumpEpdcList` output (`"### sf epdc region received: %d %d %d %d mode: %d"` @ `0x1f5679`, emitted from `EpdcManager::dumpEpdcList` @ `0x55816c`) across a multi-stroke gesture and count how many rects survive per commit.

---

## What this changes for our app

Concrete corrections to the plan in `docs/planning/handwriting-pen-stack-comparison.md` and the region/renderflag plans:

- **A single reset is never enough.** The exclude latch and the schema latch are independent stores cleared by disjoint transactions. Any "unstick" routine must do **both**: (1) `REPAINT_EVERYTHING` **or** pen-state PAUSE(3) to drop schema 7 / restore `androidDrawing=1`, and (2) `setScreenHandWritingRegionExclude(null, empty)` to wipe the pen-exclude table. Doing only (a) leaves the pen dead; doing only (b) leaves the content dead.
- **Prefer PAUSE(3) over HANDWRITING_REPAINT for recovery.** `HANDWRITING_REPAINT` provably cannot leave schema 7 (`setHandWritingRepaintUpdate` only sets a pending entry). Our per-stroke repaint reconcile will never self-heal a stuck region; the recovery button must send PAUSE/ENABLE_POST/REPAINT_EVERYTHING.
- **The empty-array form is the real reset**, not a `{0,0,0,0}` one-rect payload — `setExcludeRegion` only clears when `n<=0`; a one-rect degenerate payload stores a `strokeWidth`-inflated exclude box at the origin. Send a zero-length int array (null-view variant).
- **Do not push exclude rects per frame.** Each `SET_SCREEN_HANDWRITING_REGION_EXCLUDE` re-zeroes and re-copies the whole global array under the single pen mutex; the Java `nativeSetExcludeRegion` side also leaks its buffer (per `REPORT-regions.md`). Set exclude once per layout change, not per stroke.
- **Assume no death cleanup.** Because SF has no binder death recipient and nothing sends `APP_DIE`, our app must **explicitly** STOP the pen session and clear the exclude table on its own teardown (onPause/onDestroy and on crash-restart), or a killed session leaves the panel region dead for the next app. Consider always issuing the two-part reset on app **start** as well, defensively.
- **Colour-panel waveform limits are real.** On the HW-Tcon colour path, requesting a mode whose low nibble is outside 1–6 is rejected (`hwscreenRefresh … wrong!`); pick GU/`HAND_WRITING_REPAINT_MODE` for ink and reserve GC (mode 5→wf255) for deliberate full clears.

## On-device experiments (each one testable action)

1. With a region dead, grab logcat and read `applySFDebug`'s `"pen control state: … current epdc schema: N, androidDrawing: M"` — confirms whether the latch is schema (N=7 / M=0) or the exclude table (N=0 but pen still dead).
2. Send **only** `setScreenHandWritingRegionExclude(null, emptyIntArray)`; test if the app can ink there again and whether its content composites — isolates the pen half from the content half.
3. Send **only** `SET_SCREEN_HANDWRITING_PEN_STATE(3)` (PAUSE) with your own pid; confirm the whole panel recomposites (content half heals) but pen-exclude persists.
4. Send **only** `REPAINT_EVERYTHING` (`0xff0014`); compare against experiment 3 to confirm it heals the content half with no pen-state side effects (cleaner recovery primitive).
5. Send `HANDWRITING_REPAINT` over the dead rect; confirm it does **not** heal (stays schema 7) — validates the "repaint never recovers" claim.
6. Kill the pen-owning app process (no APP*DIE) and observe the region stays dead across relaunch — confirms the missing death recipient; then have a \_second* app send `APP_DIE(pid_of_dead_app)` (`0xff0025`) and confirm it triggers STOP + repaint (validates the dormant cleanup path).
7. Log `dumpEpdcList` (`"### sf epdc region received …"`) across a fast multi-stroke gesture to measure how `mergeByMode` coalesces per-stroke rects into batched ioctls — settles the serialise-vs-merge question that is static-only-unknowable here.
