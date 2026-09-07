# Onyx BOOX teardown: where the sources came from and how to get them again

Reference for the handwriting/e-ink work. Everything below was obtained on 2026-09-07 from a
Note Air 4C (`ro.product.model = NoteAir4C`, platform `lito`, Android 13, firmware
`2026-04-28_17-50_4.2-rel_04282_555977efe`, build number 8548, Kaleido 3 colour panel),
over plain `adb` with **no root and no bootloader unlock**.

Companion documents:

- `handwriting-pen-stack-comparison.md` — our implementation vs the stock app vs five open-source apps.
- `~/tmp_zfs/onyx_framework/REPORT-regions.md` — where handwriting region state lives and what clears it.
- `~/tmp_zfs/onyx_framework/REPORT-renderflag.md` — what the raw-drawing render-flag cycle does.
- `~/tmp_zfs/onyx_framework/REPORT-screennote.md` — the system's own handwriting pipeline and whether to opt in.
- `~/tmp_zfs/onyx_framework/REPORT-refresh.md` — panel refresh levers reachable from an app.

## 1. Layout of the working tree

```
~/tmp_zfs/onyx_framework/
  framework.jar            42 MB, 5 dex   -> out/sources/android/onyx/**      (1080 classes)
  services.jar             18 MB          -> out-services/sources/**          (no Onyx code)
  kcb.apk                 120 MB          -> out-kcb/sources/**               (30501 classes)
  ota.apk                 382 kB          -> out-ota/sources/**               (local installer only)
  native/*.so                             (libonyx_pen_touch_reader, libonyx_epd_listener,
                                           libneo_pen, libonyx_cfa, libonyx_dsl,
                                           libonyx_neo_dither, libonyx_babylon,
                                           libSurfaceFlingerProp)
  jadx/                    jadx 1.5.6
  REPORT-*.md              the four analyses
~/tmp_zfs/onyx_sdk_decompiled/            the Pen SDK we link against
~/tmp_zfs/reversed_onyx_notes_app/        the stock Notes app
~/tmp_zfs/reference_notes_apps/           Notate, Notable, PngNote, Saber, Mokke
~/tmp_zfs/boox_fw/                        firmware package + decryptor
```

## 2. Pulling system files without root

`adb pull` fails on most of `/system` (SELinux), but `adb shell cat` succeeds for anything
world-readable, which covers every jar, apk and `.so` we needed:

```bash
adb shell cat /system/framework/framework.jar        > framework.jar
adb shell cat /system/framework/services.jar         > services.jar
adb shell cat /system/priv-app/kcb-release/kcb-release.apk > kcb.apk
adb shell cat /system/lib64/libonyx_pen_touch_reader.so > native/libonyx_pen_touch_reader.so
```

Find the apk behind a package with `adb shell pm path <pkg>` (or `pm list packages -f`).

**Not readable without root:** `/system/bin/surfaceflinger`. This is the one file that matters
and the one we cannot have — Onyx patches the handwriting layer (ink regions, exclusions,
update modes, live ink) directly into SurfaceFlinger. `libSurfaceFlingerProp.so` is stock
configstore and contains nothing of interest. Nothing in `/vendor` carries the EPD logic
either. That is the reason for section 4.

Useful negative results, so nobody repeats them:

- `services.jar` contains **no** `android.onyx` code. The EAC/screennote service is registered
  from system_server but its classes live elsewhere; the pen state machine itself is in
  `framework.jar` (`android/onyx/optimization/screennote/handler/BaseHandler.java`).
- `kcb-release.apk` (package `com.onyx`) is the content-browser app plus a large bundled SDK.
  It carries the EAC config beans and the factory preset table but **not** the stock Notes
  render actions.
- `OnyxOtaService.apk` is only the local package installer (`RsaUtil`, `UpdateManager`); it
  performs no network check, so it is not where the update URL comes from.

## 3. Decompiling

```bash
curl -sL -o jadx.zip https://github.com/skylot/jadx/releases/download/v1.5.6/jadx-1.5.6.zip
unzip -q jadx.zip -d jadx && chmod +x jadx/bin/jadx
./jadx/bin/jadx -j 8 -d out --no-res --no-debug-info framework.jar
```

jadx handles the multidex jar directly. `framework.jar` takes a few minutes, `kcb.apk` about
ten. For the native libraries, `radare2` and `objdump` are enough — the interesting symbols are
unstripped C++ mangled names, e.g. `android::PenManager::clearExcludeArray()` in
`libonyx_pen_touch_reader.so`.

## 4. Downloading the firmware package

Since 2023 Onyx no longer publishes packages on the website; the device fetches them itself.
The check API is open, needs no authentication, and can be queried directly.

### The endpoint

```
GET http://en-data.boox.com/api/1/firmware/update?where=<url-encoded JSON>
```

(`http` only — TLS to that host does not complete from a desktop. The CN cluster is
`data.boox.com`, same API.) The JSON is a serialized `Firmware` bean; the fields the server
requires are model, fingerprint, submodel, buildNumber, buildType, fwType, lang, widthPixels,
heightPixels, deviceMAC. Omit one and the server answers with a raw Node stack trace naming
what it choked on, which is a convenient oracle.

Responses: `204 No Content` means "nothing newer" **and** "no such device in my table" — the two
are indistinguishable, which is what makes blind guessing useless.

### The one thing that is not obvious

`buildNumber` is parsed out of the fingerprint (`FirmwareUtils.getBuildIdFromFingerprint`:
split on `/`, take the second-to-last element, take the part before `:`), and the server matches
on **Onyx's own fingerprint format**, not the Android one. The device's
`ro.build.fingerprint` is `ONYX/TabBoox/TabBoox:13/TKQ1.230615.001/GV2.027.SQ83A:user/release-keys`,
which carries no build number at all. The format the server expects is:

```
Onyx/<model>/<model>:11/<buildDisplayId>/<buildNumber>:user/release-keys
```

Any older, correctly shaped fingerprint for the model works — the server answers with whatever is
newer. Known-good older ones for NoteAir4C are in the MobileRead thread linked below.

### Worked example

```bash
FP='Onyx/NoteAir4C/NoteAir4C:11/2025-07-26_19-00_4.1-rel_0726_509651412/4593:user/release-keys'
BODY="{\"model\":\"NoteAir4C\",\"fingerprint\":\"$FP\",\"submodel\":\"\",\"buildNumber\":4593,
       \"buildType\":\"user\",\"fwType\":\"release\",\"lang\":\"en_US\",
       \"widthPixels\":1860,\"heightPixels\":2480,\"deviceMAC\":\"00:11:22:33:44:55\"}"
curl -s -G http://en-data.boox.com/api/1/firmware/update --data-urlencode "where=$BODY"
```

The reply carries `md5`, `size`, `buildDisplayId`, the changelog, and `downloadUrlList`. Packages
are served by md5, unauthenticated:

```
http://firmware-us.boox.com/<md5>/update.upx
```

For our exact build: md5 `0cd82055053aa6787b5375757889e36e`, 1 983 772 531 bytes,
`2026-04-28_17-50_4.2-rel_04282_555977efe`, build 8548.

Older NoteAir4C packages, from
<https://www.mobileread.com/forums/showthread.php?t=369169>:

| build                                              | md5                                | size          |
| -------------------------------------------------- | ---------------------------------- | ------------- |
| `2025-01-09_20-02_4.0_43a9adea0` (2009)            | `91defa9bf4fb9d94d7d85398c7b4b354` | 1 932 759 570 |
| `2025-06-13_22-34_4.0.1-rel_0613_17903883e` (3746) | `1e8e256ad1613982cbd4440055132160` | 138 608 668   |
| `2025-07-26_19-00_4.1-rel_0726_509651412` (4593)   | `3f4de14a058b8263e1bdaf47f351ca0c` | 353 069 994   |

Note the sizes: only the first is a full image; the others are incremental.

### Capturing the device's own request instead

If the query above ever stops working, read the real one off the device. No shared network is
needed — tunnel the proxy over USB:

```bash
adb reverse tcp:8080 tcp:8080
adb shell settings put global http_proxy 127.0.0.1:8080
mitmdump -w flows.mitm --set flow_detail=1
# press Settings -> Firmware Update -> check
adb shell settings delete global http_proxy
adb reverse --remove tcp:8080
```

For https, push `~/.mitmproxy/mitmproxy-ca-cert.cer` and install it as a user CA first.

### Decrypting and unpacking

```bash
git clone https://github.com/Hagb/decryptBooxUpdateUpx
cd decryptBooxUpdateUpx
python DeBooxUpx.py NoteAir4C ../update-4.2.upx ../update-4.2.zip   # keys ship in BooxKeys.csv
```

`BooxKeys.csv` already contains NoteAir4C (`CF26B9FD1C5F74A8170C24D389F9A92D` /
`3BC02F669B2C772CC3F9A583F1FC3263`). The result is an ordinary A/B OTA zip; from there:

```bash
# payload.bin -> partition images
python extract_android_ota_payload.py update-4.2.zip out/    # github.com/cyxx/extract_android_ota_payload
lpunpack super.img super_out/                                # dynamic partitions -> system.img
```

`system.img` then yields `/system/bin/surfaceflinger`, the piece adb cannot reach.

## 5. Findings this made possible

Short pointers only; the detail is in the four reports.

- Handwriting region state (limit, exclude, mode) is **global to the SurfaceFlinger process**.
  The transactions carry no pid, no window and no surface, and the "app died" transaction has no
  callers anywhere — so a stale exclusion survives an app reinstall and dies only at reboot.
- The only call that clears it is the vendor's own, and it needs a **null View**:
  `ViewUpdateHelper.setScreenHandWritingRegionExclude(null, new int[]{0,0,0,0})`, reachable as
  `EpdController.setScreenHandWritingRegionExclude(null, arrayOf(Rect(0,0,0,0)))`. An empty list
  is discarded by the SDK, and a degenerate rect does not work because the native test inflates
  every exclusion by `strokeWidth/2`.
- The firmware performs that reset itself on window-focus gain and session start, but only for
  views it recognises as note views — which is why the stock app heals and we do not.
- The raw-drawing render-flag cycle is `enablePost(1)` + pen state `PAUSE`→`DRAWING`, the only
  handwriting transactions that carry our pid. It is the sole app-reachable way to rebuild the
  SurfaceFlinger ink session, hence the only probe that healed the dead rectangle.
- Membership of the system handwriting pipeline is decided by a string match of the view's
  resource name or class name against `drawViewKey` in per-app EAC config. Six packages are
  provisioned at the factory (OneNote and Evernote among them, the latter by the class name of a
  WebView manager). The service enforces no permission at all — but opting in costs us ownership
  of pen state, region limit and stroke parameters, and adds a full-panel repaint on every finger
  touch, so we stay with our own implementation.
- The five EinkWise profiles are a myth on this device: `noteair4c_systemui.json` defines three
  (`refresh_mode_1` REGAL_PLUS, `refresh_mode_2` "New Speed" = A2 with turbo 5, `refresh_mode_4` HD),
  and `eac_noteair4c.json` already sets `refresh_mode_2` as the default for every third-party app —
  so the profile write we added changes nothing. Worse, A2 is outside
  `Constant.DEBOUNCER_UPDATE_MODE_MAP`, which switches off the firmware's post-input quality
  repaint, its counted auto-GC, dithering and the scroll helper — a complete mechanism for the
  ghosting we chased during ordinary UI use.
