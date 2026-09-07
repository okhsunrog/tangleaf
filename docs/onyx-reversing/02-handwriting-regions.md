# Where handwriting region state lives, and how to clear it

> Written 2026-09-07 against the decompiled framework, the `com.onyx` system app, the Pen SDK and
> the on-device native libraries, before `/system/bin/surfaceflinger` was extracted from the firmware
> package. Statements marked as inference about the SurfaceFlinger side can now be checked against the
> binary — see [README](README.md). Absolute paths in this document refer to the scratch tree described
> in [01-sources-and-firmware.md](01-sources-and-firmware.md).

Device: Onyx BOOX Note Air 4C, `lito`, Android 13, FW 4.2-rel 2026-04-28. SDK device class `SDMDevice`.

Path shorthands:

| shorthand | real path                                                                                                                  |
| --------- | -------------------------------------------------------------------------------------------------------------------------- |
| `fw/`     | `/home/okhsunrog/tmp_zfs/onyx_framework/out/sources/android/onyx/` (decompiled `framework.jar`)                            |
| `kcb/`    | `/home/okhsunrog/tmp_zfs/onyx_framework/out-kcb/sources/` (decompiled `com.onyx` system app)                               |
| `sdk/`    | `/home/okhsunrog/tmp_zfs/onyx_sdk_decompiled/src/`                                                                         |
| `nat/`    | `/home/okhsunrog/tmp_zfs/onyx_framework/native/libonyx_pen_touch_reader.so` (arm64, file offset = vaddr)                   |
| `kt/`     | `/home/okhsunrog/code/rust/notes-rs/plugins/tauri-plugin-mobile-system/android/src/main/java/dev/okhsunrog/mobile_system/` |

Everything marked **inference** is a reading of behaviour the available binaries do not prove, because
`/system/bin/surfaceflinger` — the process that actually holds the SF-side region tables — is not
readable without root. Everything else is file:line or symbol/offset.

---

## 0. Verdicts in one line each

1. **Q1.** Exclude, limit and region-mode carry **no pid, no uid, no surface token, no window token** —
   only an int flag "was a View supplied" and the coordinate array. They live in the **SurfaceFlinger
   process, globally**; nothing reaps them when the caller dies. That is exactly why the dead zone
   survived a reinstall and died only at reboot (SF restart).
2. **Q2.** Native half: an empty array **does** reach `PenManager::setExcludeRegion` and **does** fully
   clear the table — but the Java wrapper never lets an empty list through. Framework half: an empty
   array parcels as length 0, a null array as length -1; the firmware itself uses neither — it uses a
   **null View plus a single `{0,0,0,0}` rect**.
3. **Q3.** Yes — one call, and only one, provably clears the SF-side exclude region from an unprivileged
   app: `ViewUpdateHelper.setScreenHandWritingRegionExclude(null, new int[]{0,0,0,0})`, reachable with
   no reflection as `EpdController.setScreenHandWritingRegionExclude(null, new Rect[]{new Rect(0,0,0,0)})`.
   It is the firmware's own reset idiom (`fw/optimization/screennote/EACScreenNoteUtils.java:115-117`).
   Nothing clears the **native** table except a rect that can never match, or process exit.
4. **Q4.** **Confirmed.** `inExcludeRegion` inflates the probe point by `strokeWidth/2` on all four sides
   and compares inclusively, so `Rect(0,0,0,0)` still excludes a `strokeWidth`-sized box at the mapped
   origin. `nat` @ `0xaa1c`.
5. **Q5.** The render-flag cycle is the only sequence in the stack that re-runs
   `SET_SCREEN_HANDWRITING_PEN_STATE`, the only region-family transaction that **carries
   `Process.myPid()`** — plus `ENABLE_POST(1)`, which lifts SF's scribble post-suppression. It makes SF
   tear down and rebuild the per-pid handwriting session rather than patch it.

---

## 1. What is parcelled, and what keys the state (Q1)

### 1.1 The transport

All three calls are raw Binder transactions to the `SurfaceFlinger` service, with the
`android.ui.ISurfaceComposer` interface token and **no reply parcel**:

- `fw/ViewUpdateHelper.java:1346-1350` — `surfaceComposerData()` = `Parcel.obtain()` +
  `writeInterfaceToken("android.ui.ISurfaceComposer")`.
- `fw/ViewUpdateHelper.java:1421-1438` — `transactData(code, data, reply)` =
  `ServiceManager.getService("SurfaceFlinger").transact(code, data, reply, 0)`, every exception
  swallowed to a log line. **A failed or unhandled transaction is silent.**

Transaction codes (`fw/ViewUpdateHelper.java:118-120,155` and the block at `:104-240`):

| code                | constant                                |
| ------------------- | --------------------------------------- |
| 16711693            | `SET_SCREEN_HANDWRITING_PEN_STATE`      |
| 16711694            | `SET_SCREEN_HANDWRITING_REGION_LIMIT`   |
| 16711714            | `SET_SCREEN_HANDWRITING_REGION_EXCLUDE` |
| 1048620             | `SET_SCREEN_HANDWRITING_REGION_MODE`    |
| 16711692            | `ENABLE_POST`                           |
| 16711717            | `APP_DIE`                               |
| 16711700 / 16711715 | `REPAINT_EVERY_THING` / `..._WITH_MODE` |
| 1048647             | `HANDWRITING_REPAINT`                   |
| 1049600             | `SAVE_PEN_ATTACHED_FB`                  |
| 1048643 / 1048641   | `GET_PEN_STATE` / `IS_PEN_STATE_VALID`  |

### 1.2 The payloads

**Exclude** — `fw/ViewUpdateHelper.java:1202-1217`:

```java
public static void setScreenHandWritingRegionExclude(View view, int[] iArr) {
    Parcel p = surfaceComposerData();
    if (view != null) {                       // rects rewritten IN PLACE to screen coords
        int[] loc = new int[2];
        view.getLocationOnScreen(loc);
        for (int i = 0; i < iArr.length / 2; i++) {
            iArr[i*2]   += loc[0];
            iArr[i*2+1] += loc[1];
        }
    }
    p.writeInt(view != null ? 1 : 0);         // the ONLY discriminator besides the array
    p.writeIntArray(iArr);
    transactData(SET_SCREEN_HANDWRITING_REGION_EXCLUDE, p, null);
}
```

**Limit** — `fw/ViewUpdateHelper.java:1223-1238`: byte-for-byte the same shape, different code. The
4-int convenience overload is `:1219-1221`.

**Region mode** — `fw/ViewUpdateHelper.java:1240-1244`: `writeInt(mode)` and nothing else. No View, no
pid, no per-window scoping. `0` = multi, `1` = single (`fw/inputreader/RawInputReader.java:360-374`).

**Pen state** — `fw/ViewUpdateHelper.java:1195-1200`, the one that differs:

```java
public static void setScreenHandWritingPenState(int i) {
    Parcel p = surfaceComposerData();
    p.writeInt(i);
    p.writeInt(Process.myPid());              // pid, only here
    transactData(SET_SCREEN_HANDWRITING_PEN_STATE, p, null);
}
```

`ENABLE_POST` is the other pid-carrying one — `fw/ViewUpdateHelper.java:602-608`:
`writeInt(-1); writeInt(enable); writeInt(Process.myPid())`.

### 1.3 What that means for keying and lifetime

- **Exclude, limit and mode are not keyed by anything the caller supplies.** The only per-caller
  information on the wire is the `view != null` flag, and even that only says whether the ints were
  already translated to screen coordinates. There is no handle SF could use to attribute a table to a
  client. **Inference:** SF keeps one global limit table, one global exclude table and one global mode
  int for the display.
- **Nothing reaps them on process death.** `APP_DIE(int)` exists (`fw/ViewUpdateHelper.java:296-300`)
  and is the obvious hook, but a grep over the whole decompiled `framework.jar`, the whole `com.onyx`
  system app and the whole SDK finds **zero callers** — it is a manual API nobody invokes. Any
  automatic cleanup would have to be inside SF, and it cannot be keyed by pid because the pid was
  never sent.
- **Therefore the lifetime is the SurfaceFlinger process.** That reconciles every observation:
  survives APK reinstall / process death (SF never learned our pid for these three calls and does not
  restart when we do); healed by reboot (SF restarts, tables reinitialised); appeared briefly in the
  **stock Notes app** (the tables are global, so a rect latched by one app is visible to the next).
  Stock recovers by itself because it, unlike us, runs the EAC screen-note handler, which resets the
  exclude table on window-focus gain — see 3.2.
- There is **no getter**. `ViewUpdateHelper` has `getPenState()` (`:720-727`) and `isValidPenState()`
  (`:853-860`) but no `getScreenHandWritingRegionExclude`. We cannot read SF's tables back, only
  overwrite them.

### 1.4 The other, entirely separate copy: the native evdev filter

The second sink is process-local and is **not** the surviving dead zone.

`nat` exports exactly eleven JNI entry points, all `Java_com_onyx_android_sdk_pen_RawInputReader_native*`.
Every one loads the **same `.bss` singleton** — `adrp x0, 0x16000; add x0, x0, 0x1b0`, a `TouchReader`
at vaddr `0x161b0` (r2 `axt 0x161b0` lists all eleven). `TouchReader::setExcludeRegion(float*,int)`
@ `0xc750` is a two-instruction thunk: `ldr x0,[x0,0x20]` (the `PenManager`) then tail-call
`PenManager::setExcludeRegion` @ `0xac44`.

`PenManager` field layout, read off the accessors:

| offset    | field                                         | evidence                                                                         |
| --------- | --------------------------------------------- | -------------------------------------------------------------------------------- |
| `+0x30`   | pen state                                     | `setPenState` @ `0xa898`                                                         |
| `+0x58`   | limit float array (1024 floats = 256 rects)   | `setLimitRegion` @ `0xaaa4` memsets `this+0x58`                                  |
| `+0x1058` | limit count                                   | `0xaab4`, `0xab34`                                                               |
| `+0x105c` | exclude float array (1024 floats = 256 rects) | `setExcludeRegion` @ `0xac44` memsets `this+0x105c`                              |
| `+0x205c` | exclude count                                 | `0xac54`, `0xacd8`, `0xacf0`                                                     |
| `+0x2060` | stroke width                                  | `setStrokeWidth` @ `0xa94c`                                                      |
| `+0x2064` | region mode                                   | `setRegionMode` @ `0xa954` — a single `str w1,[x0,0x2064]`                       |
| `+0x2068` | last-region-hit index                         | `setLimitRegion` writes `-1` at `0xaae4`; `inLimitRegion` latches it at `0xaa08` |

This is per-process by construction — it lives in our own copy of the `.so`, dies with the process, so
a **reinstall clears it**. `nativeRawClose` @ `0xd800` calls one function on the `TouchReader`
(`0x134c0`, the stop path) and never touches the arrays, so a latched exclusion here survives
`closeRawDrawing()` and re-opening the editor, but not an app restart. That matches
`docs/planning/handwriting-pen-stack-comparison.md` 2a — and it is why 2a's diagnosis, correct in
itself, **cannot explain the observed dead zone**, which survived a reinstall.

---

## 2. What an EMPTY or NULL array does on each side (Q2)

### 2.1 Native half — empty clears the table completely

`nativeSetExcludeRegion` @ `0xd944` (220 bytes), decoded:

```
len = env->GetArrayLength(arr)            ; JNIEnv+0x558 = index 171
buf = calloc(len, 4)                      ; NO zero check
ptr = env->GetFloatArrayElements(arr,&c)  ; JNIEnv+0x5e8 = index 189
memcpy(buf, ptr, len*4)
env->ReleaseFloatArrayElements(arr,ptr,0) ; JNIEnv+0x628 = index 197
TouchReader::setExcludeRegion(g_reader, buf, len)   ; via PLT 0x134a0
```

`nativeSetLimitRegion` @ `0xd868` is identical modulo the target; `nativeSetRegionMode` @ `0xda3c` is
three instructions and a tail-call. **There is no length guard in the JNI layer.** `calloc(0,4)`
returns a valid pointer on bionic, `memcpy` of 0 bytes is a no-op, and `len = 0` reaches C++.

`PenManager::setExcludeRegion(float*, int n)` @ `0xac44`:

```
w8 = this->[0x205c]                          ; old count
if (w8 >= 1) memset(this+0x105c, 0, w8*4)    ; 0xac6c-0xac7c  -> old table wiped FIRST, unconditionally
if (n <= 0) { this->[0x205c] = n; return; }  ; 0xac80-0xad00
...copy n floats to this+0x105c, store count, then normalise each quad so l<=r, t<=b
```

**A zero-length float array therefore fully clears the native exclude table** (count 0, bytes zeroed).
Same structure in `setLimitRegion` @ `0xaaa4`, which additionally stores `-1` into `+0x2068`
(last-region-hit) at `0xaae4`; `setExcludeRegion` does not.

Two follow-ups:

- `clearExcludeArray()` @ `0xa90c`, `clearLimitArray()` @ `0xa8c8`, `resetLastRegionHit()` @ `0x9fa8`
  and `PenManager::clear()` @ `0x9ecc` are **exported but have no callers inside the library** (r2
  `axt` on each returns nothing) — unreachable from any JNI entry point. `clearExcludeArray` is
  byte-identical in effect to `setExcludeRegion(anything, 0)`, so nothing is lost.
- `nativeSetExcludeRegion` **leaks** the `calloc` buffer: nothing frees `buf`, and
  `PenManager::setExcludeRegion` copies out of it. `len*4` bytes leak per call. Minor, but it argues
  against pushing exclude rects on a per-frame cadence.

A **null** jarray would fault in `GetArrayLength` before any of this; unreachable, because the Java
wrappers always pass a constructed array.

### 2.2 Java wrappers — empty never gets through

- framework: `fw/inputreader/RawInputReader.java:333-337`
  ```java
  public void setExcludeRect(List<Rect> list) {
      if (list == null || list.size() <= 0) { return; }        // silent
      nativeSetExcludeRegion(mapToRawTouchRect(list));
      ViewUpdateHelper.setScreenHandWritingRegionExclude(this.hostView, rectToIntArray(...));
  ```
- SDK: `sdk/onyxsdk-pen-1.5.4.3/sources/com/onyx/android/sdk/pen/RawInputReader.java:418-425` — same
  guard, then `EpdController.setScreenHandWritingRegionExclude(getHostView(), ...)`.
- `setLimitRect(List)` has the same guard (`fw/.../RawInputReader.java:352-358`); `setLimitRect(Rect)`
  guards only on null (`:343-350`), so a **zero-area limit rect goes through** and kills the pen
  everywhere.

`TouchHelper.setLimitRect(Rect, List)` fans out to `RawInputManager.setLimitRect(rect, excludes)`
(`sdk/onyxsdk-pen-1.5.4.3/.../RawInputManager.java:149-153`), literally
`setLimitRect(rect); setExcludeRect(excludes);` — so **`setLimitRect(limit, emptyList())` leaves any
previously pushed exclusion latched on both sinks.** This is the confirmed reason
`setExcludeRect(emptyList())` did nothing on device.

### 2.3 Framework/SF half — what the wire actually carries

`Parcel.writeIntArray` writes `-1` for null, otherwise `length` followed by the elements. So:

| Java argument                         | on the wire                                            |
| ------------------------------------- | ------------------------------------------------------ |
| `int[0]`                              | flag, then `0`                                         |
| `null`                                | flag, then `-1`                                        |
| `{0,0,0,0}`, view null                | `0`, then `4`, then `0,0,0,0`                          |
| `{0,0,0,0}`, view non-null at (vx,vy) | `1`, then `4`, then `vx,vy,vx,vy` — **not** the origin |

The firmware never sends `int[0]` or `null`. Its one and only reset is a **null View with a single
degenerate rect** (3.1). **Inference:** SF's handler most likely reads the count and iterates, so
`int[0]` would clear too — but since the vendor deliberately chose `{0,0,0,0}` with a null view, that
is the form to use; the zero-length form is a device test, not a claim (see 8).

The `view != null` flag bites us directly: pushing `Rect(0,0,0,0)` through `TouchHelper.setExcludeRect`
sends it with the **host view's screen offset added**, i.e. `{vx,vy,vx,vy}` with flag 1, not the
origin with flag 0. `kt/OnyxInk.kt:180-189`'s `EMPTY_EXCLUDE` therefore does not send the bytes the
firmware's own reset sends.

---

## 3. Is there any unprivileged call that provably clears a latched exclude region? (Q3)

**Yes — for the SF half, exactly one, and it is the vendor's own.**

### 3.1 The winning call

`fw/optimization/screennote/EACScreenNoteUtils.java:115-117`:

```java
public static void resetScreenHandWritingRegionExclude() {
    ViewUpdateHelper.setScreenHandWritingRegionExclude(null, new int[]{0, 0, 0, 0});
}
```

Null view => the offset loop at `ViewUpdateHelper.java:1204-1212` is skipped, the flag written is `0`,
and the payload is literally `4, 0,0,0,0`.

Reachable from an unprivileged app **with no reflection**, through the public SDK:

```
EpdController.setScreenHandWritingRegionExclude(View, Rect[])
    sdk/onyxsdk-device-1.3.5.2/.../api/device/epd/EpdController.java:198-200
  -> SDMDevice.setScreenHandWritingRegionExclude(View, Rect[])
    sdk/.../device/SDMDevice.java:3439-3454        (flattens Rect[] to int[], min/max normalised)
  -> SDMDevice.setScreenHandWritingRegionExclude(View, int[])
    kcb/com/onyx/android/sdk/device/SDMDevice.java:2380-2386
       ReflectUtil.invokeMethodSafely(A0, view, view, iArr)
  -> handle A0 = ViewUpdateHelper.setScreenHandWritingRegionExclude(View, int[])
    sdk/.../device/SDMDevice.java:1113   (target class android.onyx.ViewUpdateHelper; same block as :1110-1112)
```

`view = null` propagates all the way: `SDMDevice:3439-3454` only touches the `Rect[]`, and
`ReflectUtil.invokeMethodSafely` ignores the receiver for a static method. So

```kotlin
EpdController.setScreenHandWritingRegionExclude(null, arrayOf(Rect(0, 0, 0, 0)))
```

is byte-for-byte the firmware's `resetScreenHandWritingRegionExclude()`. Hidden-API access is already
exempted: `ReflectUtil`'s static initialiser installs `VMRuntime.setHiddenApiExemptions({"L"})`
(`sdk/.../utils/ReflectUtil.java:27-52,439`).

`EACScreenNoteUtils.resetScreenHandWritingRegionExclude()` is also a public static on a public
framework class and callable by reflection, but there is no reason to prefer it.

### 3.2 Proof that this is the intended reset: where the firmware calls it

`fw/optimization/screennote/handler/BaseHandler.java`, the EAC screen-note event handler the framework
installs inside every app:

- **on window-focus gain**, `onWindowFocusChangedImpl` `:305-321`:
  ```java
  pauseEACScreenNote();                                        // -> setScreenHandWritingPenState(3)
  ViewUpdateHelper.repaintEverything();
  if (hasFocus) {
      EACScreenNoteUtils.resetScreenHandWritingRegionExclude();  // :313
      setScreenHandWritingRegionLimit(view, EACScreenNoteUtils.getLocalVisibleRectFromView(view));
      applyStrokeParam(view, style);                            // ends in resumeEACScreenNote -> penState(2)
  }
  ```
- **on session start**, `startEACScreenNote` `:330-341`:
  ```java
  EACScreenNoteUtils.resetScreenHandWritingRegionExclude();      // :334
  setScreenHandWritingRegionLimit(view, rect);
  setScreenHandWritingPenState(1);
  applyStrokeParam(view, style);
  ```

The vendor's order is **pause -> repaintEverything -> reset exclude -> set limit -> resume**. Note what
is absent: the handler never sets a non-empty exclude region at all. The only thing it ever does to the
exclude table is wipe it. Exclude rects come from apps; the framework only clears them.

**This is why stock Notes healed and we do not.** The whole path is gated on
`isDrawingViewWithEnableConfig(view, noteConfig)` (`BaseHandler.java:96-98`) =
`noteConfig.isEnable() && noteConfig.matchDrawView(view)` — the EinkWise per-app note config must name
the drawing view. Stock Notes matches; a Tauri WebView does not. Stock therefore gets a free
exclude-table wipe on every window-focus gain, and we get nothing.

### 3.3 Everything else considered, and why it does not clear

| candidate                                            | verdict                                                                                                                                                                                                                                                                                                     | evidence                                                                                                                    |
| ---------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `TouchHelper.setExcludeRect(emptyList())`            | **no-op on both sinks**                                                                                                                                                                                                                                                                                     | guard at `sdk/.../RawInputReader.java:419-421`                                                                              |
| `TouchHelper.setLimitRect(limit, emptyList())`       | pushes the limit only; exclusion stays latched                                                                                                                                                                                                                                                              | `RawInputManager.java:149-153` + the same guard                                                                             |
| degenerate `Rect(0,0,0,0)` via `TouchHelper`         | clears the **native** table (overwritten with one rect) but on SF writes `{vx,vy,vx,vy}` with flag 1, not the firmware idiom; and natively it still excludes a `strokeWidth` box at the view origin                                                                                                         | `ViewUpdateHelper.java:1204-1212`; `nat` `0xaa1c`                                                                           |
| off-screen rect via `TouchHelper`                    | clears the native table effectively; on SF replaces the table with one off-screen rect — good in practice, not the vendor idiom                                                                                                                                                                             | same                                                                                                                        |
| re-setting the limit                                 | never touches the exclude table on either side                                                                                                                                                                                                                                                              | `setLimitRegion` @ `0xaaa4` memsets only `+0x58..+0x1058`; `SET_SCREEN_HANDWRITING_REGION_LIMIT` is a different transaction |
| `setSingleRegionMode()` / `setMultiRegionMode()`     | writes one int; `setRegionMode` @ `0xa954` is a single `str`                                                                                                                                                                                                                                                | `fw/inputreader/RawInputReader.java:360-374`                                                                                |
| pen-state transitions 0/1/2/3/4                      | do not clear the exclude table — proven because the firmware's own `startEACScreenNote` calls `resetScreenHandWritingRegionExclude()` _before_ `setScreenHandWritingPenState(1)`; if the state change cleared it the call would be redundant                                                                | `BaseHandler.java:334-336`                                                                                                  |
| `repaintEverything()`                                | a repaint, not a region call — but the firmware runs it immediately before its reset, so include it                                                                                                                                                                                                         | `ViewUpdateHelper.java:1057-1065`; `BaseHandler.java:311`                                                                   |
| `handwritingRepaint(view,l,t,r,b)`                   | repaint of a rect through `HANDWRITING_REPAINT`; no region state                                                                                                                                                                                                                                            | `ViewUpdateHelper.java:781-806`                                                                                             |
| `EpdController.clearScreenHandWritingRegionConfig()` | **does not exist on this firmware.** `ViewUpdateHelper` has no `RegionConfig` method at all (grep: 0 hits), so `SDMDevice`'s handles `f996s3`/`f997t3` (`sdk/.../SDMDevice.java:1257-1258`) resolve to null and both calls are silent no-ops. This retires suggestion #4 of `onyx_sdk_decompiled/REPORT.md` | `fw/ViewUpdateHelper.java`                                                                                                  |
| `appDie(pid)`                                        | exists, unprivileged, **zero callers anywhere** — semantics unknown, worth a probe                                                                                                                                                                                                                          | `ViewUpdateHelper.java:296-300`                                                                                             |
| `resetEpdPost()`                                     | `ENABLE_POST(-1,1,myPid)` then `setScreenHandWritingPenState(0)`. Not a region call, but it is what the Onyx launcher runs at startup to un-wedge the panel (`kcb/com/onyx/ContentBrowserApplication.java:394-399`: `resetEpdPost(); byPass(0); enableScreenUpdate(null,true)`)                             | `ViewUpdateHelper.java:1071-1078`                                                                                           |

### 3.4 The native half has no clean clear

Because the wrappers guard on emptiness and `clearExcludeArray` is unreachable, the only unprivileged
ways to empty the native table are:

1. **Overwrite it with a rect that can never match** — the practical answer. A rect entirely outside
   the limit rect is guaranteed inert, because `inValidRegion` @ `0xadf4` requires `inLimitRegion` to
   pass _before_ it consults the exclude table.
2. **Reflect the private native directly**:
   `RawInputReader.class.getDeclaredMethod("nativeSetExcludeRegion", float[].class)` invoked with
   `new float[0]` on the live reader (reachable via `TouchHelper.f79d` -> `SFTouchRender.f225d` ->
   `RawInputManager` -> its `RawInputReader`). This provably clears (`0xac80 -> 0xacf0`). Fragile
   (obfuscated field names) and unnecessary if (1) works.
3. Restart the process.

Note `mapToRawTouchRect` (`sdk/.../RawInputReader.java:583-598`) dereferences the mapped `RectF`
unconditionally at `:592` after only _logging_ about a null — so if `EpdController.mapToRawTouchPoint`
ever returns null, any `setExcludeRect` NPEs. Prefer a rect that maps sanely (just outside the limit)
over an extreme off-panel one.

---

## 4. How the exclusion test is implemented (Q4) — the strokeWidth/2 claim is CONFIRMED

`android::PenManager::inExcludeRegion(float x, float y)` @ `nat` `0xaa1c`, in full:

```
0xaa1c  ldr  w8, [x0, 0x205c]        ; count
0xaa24  b.lt 0xaa9c                  ; count < 1 -> return 0
0xaa28  fmov s2, 0.5
0xaa2c  ldr  s3, [x0, 0x2060]        ; strokeWidth
0xaa3c  fmul s4, s3, s2              ; half = strokeWidth * 0.5
0xaa40  fadd s2, s4, s0              ; xhi = x + half
0xaa44  fsub s0, s0, s4              ; xlo = x - half
0xaa48  fadd s3, s4, s1              ; yhi = y + half
0xaa4c  fsub s1, s1, s4              ; ylo = y - half
loop over rects at this+0x105c, stride 0x10:
  0xaa64  ldur s4,[x10,-0xc]  fcmp s4,s2  b.hi next   ; if (l  > xhi) continue
  0xaa70  ldur s4,[x10,-4]    fcmp s0,s4  b.hi next   ; if (xlo > r ) continue
  0xaa7c  ldur s4,[x10,-8]    fcmp s4,s3  b.hi next   ; if (t  > yhi) continue
  0xaa88  ldr  s4,[x10]       fcmp s1,s4  b.hi next   ; if (ylo > b ) continue
  0xaa94  return 1
return 0
```

The predicate is

```
l <= x + w/2  &&  x - w/2 <= r  &&  t <= y + w/2  &&  y - w/2 <= b
```

— an axis-aligned box of side `strokeWidth` centred on the probe point, tested for intersection with
the rect, **inclusive on every edge**. `inLimitRegion` @ `0xa95c` and the limit half of `inValidRegion`
@ `0xadf4` use the identical inflation.

Consequences:

- `Rect(0,0,0,0)` is **not** inert. It excludes every point within `strokeWidth/2` of the mapped
  origin, in **raw digitizer** coordinates (the rect went through
  `EpdController.mapToRawTouchPoint(hostView, ...)`, `sdk/.../RawInputReader.java:587`). With the host
  view at the top-left of the sheet that is a small dead dot in the corner of the WebView — not the
  whole corner of the panel, but real, and it scales with pen width.
- Every exclude rect is effectively **grown by `strokeWidth/2` on all four sides** at test time. A menu
  rect published as an exclusion kills a band slightly larger than the menu.
- `strokeWidth` (`+0x2060`) is global to the reader, so the effective size of every exclusion changes
  whenever the tool width changes. `TouchHelper.setStrokeWidth` writes both this and SF's
  `SET_STROKE_WIDTH` (`fw/inputreader/RawInputReader.java:376-380`).
- `setExcludeRegion`/`setLimitRegion` also **normalise** each quad in place (the `fcsel` min/max block
  at `0xad70-0xada8` / `0xabc4-0xabfc`), so an inverted rect is silently swapped rather than ignored.
  There is no way to express "empty" as a rect.
- Both tables are a fixed 4 KB (256 rects) with **no bounds check** in `setLimitRegion`/`setExcludeRegion`
  and no locking against the reader thread concurrently running `inValidRegion` — changing regions
  mid-stroke is a genuine data race. Confirms `handwriting-pen-stack-comparison.md` 2b.

---

## 5. Why cycling the raw-drawing RENDER flag heals it (Q5)

### 5.1 What the cycle actually is

```
TouchHelper.setRawDrawingRenderEnabled(b)          sdk/.../pen/TouchHelper.java:188-198
  -> TouchRender.setDrawingRenderEnabled(b)
     SFTouchRender.setDrawingRenderEnabled          sdk/.../pen/touch/SFTouchRender.java:277-284
        false -> m212g()   :195-198
                   EpdController.leaveScribbleMode(hostView)
                   EpdPenManager.pauseDrawing()
        true  -> m213j()   :200-202
                   EpdPenManager.resumeDrawing()
```

- `EpdPenManager.pauseDrawing()` = `EpdController.setScreenHandWritingPenState(view, 3 /*PEN_PAUSE*/)`
  (`sdk/.../pen/EpdPenManager.java:42-44`) -> `SDMDevice.setScreenHandWritingPenState`
  (`sdk/.../device/SDMDevice.java:1827-1835`, handle `f847z0` =
  `ViewUpdateHelper.setScreenHandWritingPenState(int)`, `:1110`) -> transaction `16711693` with `{3, myPid}`.
- `EpdPenManager.resumeDrawing()` = the same with `2 /*PEN_DRAWING*/` (`EpdPenManager.java:38-40`).
- `EpdController.leaveScribbleMode(view)` = `SDMDevice.leaveScribbleMode` = `enablePost(view, 1)`
  (`sdk/.../device/SDMDevice.java:1763-1766`, `:1768-1776`, handle `f843v0` =
  `ViewUpdateHelper.enablePost(int)`, `:1106`) -> transaction `16711692` with `{-1, 1, myPid}`.

So one render off/on cycle puts exactly this on the wire:

```
ENABLE_POST      { -1, 1, myPid }        ; "resume posting my surfaces to the panel"
PEN_STATE        { 3 , myPid }           ; PEN_PAUSE
PEN_STATE        { 2 , myPid }           ; PEN_DRAWING
```

Nothing else in the stack sends that. `handwritingRepaint`, `invalidate(view, GU)`,
`setSingleRegionMode`, `setExcludeRect(emptyList())` and cycling the input reader
(`RawInputReader.pause()`/`resume()` = `nativeSetPenState(4)` / `nativePausePen()`, purely in-process,
`fw/inputreader/RawInputReader.java:307-327`) send none of it.

### 5.2 Why that is the thing that heals

1. **It is the only region-adjacent transaction carrying `Process.myPid()`**
   (`ViewUpdateHelper.java:1195-1200`, `:602-608`). Everything else about handwriting is anonymous.
   Whatever per-client session SF maintains for the firmware ink layer, this is the only handle we
   have on it, and PEN_PAUSE -> PEN_DRAWING is a full close/open of it.
2. **`ENABLE_POST(1)` is the counterpart of `enterScribbleMode` = `enablePost(0)`**
   (`SDMDevice.java:1759-1762`). Scribble mode is precisely SF _withholding_ the app's own surface
   posts over the ink area so firmware-drawn ink is not composited away. That is the mechanism whose
   failure mode is "the app's content is not shown in this rectangle" — the second half of the observed
   symptom, and the half that `handwritingRepaint` and `invalidate(GU)` cannot touch, because both go
   through exactly the update path that scribble-mode suppression drops.
3. **The vendor uses the same primitive for the same purpose.** `BaseHandler` pauses to pen-state 3 and
   resumes to 2 around every meaningful transition — finger down (`:192-196`), stylus down outside the
   draw view (`:227-234`), size change (`:159-165`), window-focus change (`:305-321`). Stock Notes'
   `InvalidateScreenAction` does render-off + input-off -> invalidate -> both back on (see
   `handwriting-pen-stack-comparison.md` 11, ~30 call sites). notable's `resetScreenFreeze(touchHelper)`
   is a bare render-flag cycle before every partial update.

**Inference (the part SF would have to confirm):** the rectangle that goes dead is not the raw exclude
table itself but a structure SF _derives_ from it plus the limit rect plus per-stroke damage — the
composed handwriting region / pen-attached framebuffer bookkeeping (`SAVE_PEN_ATTACHED_FB = 1049600`
names such a thing, `ViewUpdateHelper.java:1080-1082`). Evidence: if the raw exclude table were what
made the pen dead, rebuilding the session from that same table would reproduce the dead rect and the
cycle would not heal anything. Since the cycle does heal, the dead rect lives in something the
PEN_PAUSE -> PEN_DRAWING transition **rebuilds from scratch** rather than patches. Consistent with this,
`ENABLE_POST` and `PEN_STATE` are the two pid-carrying calls: they address a per-client session object,
and it is that object, not the global tables, that is stale. The two readings are distinguishable on
device (see 8, tests 2 and 6).

Either way the practical conclusion is the vendor's: **pause the pen state, repaint, rewrite the region
tables from a known-good baseline, resume.** Never patch a region while the session is live.

---

## 6. Secondary findings worth carrying forward

- `getPenState()` (`ViewUpdateHelper.java:720-727`) and `isValidPenState()` (`:853-860`) are the only
  readbacks in the whole handwriting family, and both return real values — use them to verify a
  pen-state write landed. There is no readback for any region table.
- `enablePost(int)`'s first parcel int is `-1` in every caller, including `resetEpdPost` (`:1071-1078`).
  **Inference:** `-1` = "all layers of this pid".
- `EACScreenNoteUtils.repaintView(view)` (`:110-114`) = `enablePost(1)` then
  `refreshScreen(view, currentRefreshMode)` — a lighter recovery than `repaintEverything`.
- `getLocalVisibleRectFromView` (`EACScreenNoteUtils.java:38-70`) is what the firmware feeds to
  `setScreenHandWritingRegionLimit`: `getLocalVisibleRect`, replaced by `(0,0,w,h)` when the view is
  only partly on screen, and by the full display rect when the local visible rect is empty. Our limit
  rect should follow the same rules across a fullscreen<->windowed transition — which is the exact
  transition at which the dead zone appeared in stock Notes.
- `SingleDrawViewHandler.setScreenHandWritingRegionLimit`
  (`fw/.../handler/SingleDrawViewHandler.java:34-41`) **skips the limit push entirely while the IME is
  visible**, and `resumeEACScreenNote` skips the resume when the view lacks focus or the IME is up.
  Matches our own IME gating.
- `EpdController.setScreenHandWritingRegionConfig` / `clearScreenHandWritingRegionConfig` are
  **unavailable on this firmware** (3.3). `onyx_sdk_decompiled/REPORT.md`'s recommendation #4 should be
  struck.
- `nativeSetExcludeRegion` / `nativeSetLimitRegion` leak `len*4` bytes per call (2.1).
- `libSurfaceFlingerProp.so` and `libneo_pen.so` contain **no** handwriting-region symbols or strings —
  the SF-side handler is inside `/system/bin/surfaceflinger` itself and is not obtainable here.

---

## 7. What we should call, in order

Two distinct things are needed: a **one-shot repair** for a device that already has a rectangle
latched, and a **standing discipline** so it stops accumulating.

### 7.1 The repair sequence (mirrors `BaseHandler.onWindowFocusChangedImpl` exactly)

Run on `configure()`, on window-focus gain, and behind the existing `debugRepaint` hook
(`kt/OnyxInk.kt:385-407`, plugin command `debug_ink_repaint`, `kt/MobileSystemPlugin.kt:330-333`) as a
new kind, e.g. `"resetRegions"`. Must run with the pen paused, never inside a stroke.

```kotlin
val inert = Rect(limit.right + 1, limit.bottom + 1, limit.right + 2, limit.bottom + 2)

// 1. Pause the firmware session. render(false) already does leaveScribbleMode + PEN_PAUSE.
gate.pause()                                              // render(false) then input(false)

// 2. Repaint, as the firmware does before its own reset.
EpdController.repaintEveryThing()                         // -> ViewUpdateHelper.repaintEverything()

// 3. Clear the NATIVE exclude table (this process only). A list of size 1 passes the guard at
//    RawInputReader.java:419; the rect lies outside the limit rect, so inValidRegion rejects any
//    point on the limit test before the exclude test is ever reached.
helper.setExcludeRect(listOf(inert))

// 4. Clear the SF exclude table. MUST come after step 3, because step 3 also writes SF.
//    null view + {0,0,0,0} is byte-for-byte EACScreenNoteUtils.resetScreenHandWritingRegionExclude().
EpdController.setScreenHandWritingRegionExclude(null, arrayOf(Rect(0, 0, 0, 0)))

// 5. Re-establish geometry and mode from a known-good baseline.
helper.setSingleRegionMode()
helper.setLimitRect(limit, NO_EXCLUDES)                   // empty list -> exclude untouched, by design
helper.setPenUpRefreshTimeMs(PEN_UP_REFRESH_MS.toInt())

// 6. Resume: input first, then render (PEN_DRAWING). gate.resume() already has this order.
gate.resume(render = wantsFirmwareInk)
```

Ordering constraints, each load-bearing:

- 3 before 4 — step 3 writes both sinks and would otherwise clobber step 4's SF reset.
- 4 before 5 — matches the vendor's order and keeps the last SF exclude write as the reset.
- `NO_EXCLUDES` at step 5 is correct _because_ the empty list is a no-op on the exclude table; the
  exclude state we want is the one step 4 installed.
- Step 6's input-then-render order is already what `RawDrawingGate.resume` does
  (`kt/RawDrawingGate.kt:31-39`) and matches stock's `ResumeRawDrawingRequest`.

### 7.2 The standing discipline

1. **Never publish an app overlay as an exclude rect again.** The current `NO_EXCLUDES` policy
   (`kt/OnyxInk.kt:176-181`) is right and matches stock's selection popup. The exclusion mechanism is
   write-only from an app's point of view: one push can outlive the process and, on SF, every other app
   on the device until reboot.
2. **Delete `EMPTY_EXCLUDE = listOf(Rect(0,0,0,0))`** (`kt/OnyxInk.kt:189`). Section 4 proves it
   excludes a `strokeWidth` box at the mapped view origin, and 2.3 proves it does not send the
   firmware's reset bytes. Replace it with the 7.1 pair.
3. **Cycle the render flag around anything that repaints the panel** — the one behaviour
   `handwriting-pen-stack-comparison.md` 11 found in every surveyed implementation and not in ours.
   `TouchHelper.setRawDrawingRenderEnabled` dedups on its own `f77b` flag
   (`sdk/.../TouchHelper.java:188-198`), so a "cycle" that starts from the current value is a no-op;
   use `forceSetRawDrawingEnabled` (`:199-211`) or drive
   `EpdController.setScreenHandWritingPenState(view, 3)` / `(view, 2)` directly when a real cycle is
   required.
4. **Never change a region mid-stroke** — the native tables are memset-then-refill with no locking
   against the reader thread (section 4). Keep deferring reconfiguration to pen-up.
5. **Treat region state as write-only.** No getter exists; log `getPenState()` / `isValidPenState()`
   after every pause/resume and always rewrite regions in full from our own model.

---

## 8. Only settleable on device — one testable action each

1. **Prove the SF reset works.** With a dead rectangle present, call
   `EpdController.setScreenHandWritingRegionExclude(null, arrayOf(Rect(0,0,0,0)))` **alone** (no
   pen-state cycle, no repaint) and check whether the app's content returns to the rectangle.
2. **Prove the dead zone is the exclude table and not a derived session structure.** With a dead
   rectangle present, do the render off/on cycle _without_ any exclude reset. If it heals and the
   rectangle returns after the next `setLimitRect`, the exclude table is still dirty and 5.2's
   inference is wrong; if it heals and stays healed, the table is clean and the stale state was the
   session.
3. **Does a zero-length array clear on SF?** Call
   `EpdController.setScreenHandWritingRegionExclude(null, arrayOf<Rect>())` (parcels flag 0, length 0)
   with a dead rectangle present and see whether it heals like `{0,0,0,0}` does.
4. **Does the `view != null` flag matter?** Call
   `EpdController.setScreenHandWritingRegionExclude(webView, arrayOf(Rect(0,0,0,0)))` (flag 1, rect
   offset to the view origin) and compare with the null-view form on the same dead rectangle.
5. **What does `APP_DIE` do?** Reflect `ViewUpdateHelper.appDie(int)` with `Process.myPid()` while a
   dead rectangle is present, then re-open the ink session and see whether the rectangle is gone. Zero
   callers exist anywhere, so semantics are unknown — run it on a build you are willing to restart.
6. **Is the state really global across apps?** Latch a dead rectangle from our app, then open stock
   Notes and check whether the same screen rectangle refuses ink there _before_ stock's own focus-gain
   reset runs. A yes proves the SF tables are display-global, not per-client.
7. **Does `resetEpdPost()` alone heal it?** `EpdController.resetEpdPost()` = `ENABLE_POST(-1,1,pid)` +
   `PEN_STATE(0)`; run it standalone against a dead rectangle to separate the post-suppression half of
   the symptom from the ink half.
8. **Confirm the strokeWidth inflation is observable.** Push `setExcludeRect(listOf(Rect(0,0,0,0)))`
   with `setStrokeWidth(40f)` and check that a ~40 px square at the top-left of the WebView refuses
   ink; repeat at width 2 and check the dead square shrinks.
9. **Confirm an outside-the-limit exclude rect is inert.** Push
   `setExcludeRect(listOf(Rect(limit.right+1, limit.bottom+1, limit.right+2, limit.bottom+2)))` and
   verify inking is unaffected everywhere inside the limit rect — this is the rect 7.1 step 3 relies on.
10. **Confirm `repaintEverything` is not sufficient on its own.** Call `EpdController.repaintEveryThing()`
    alone against a dead rectangle; the firmware runs it before its reset, so it should _not_ heal by
    itself.
