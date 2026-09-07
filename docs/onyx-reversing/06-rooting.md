# Rooting a Note Air 4C on firmware 4.2, and why the obvious route fails

Done on 2026-09-07 on the device this project targets: Note Air 4C, platform `lito`,
Android 13, `2026-04-28_17-50_4.2-rel_04282_555977efe`, build 8548, slot `b` active.
No bootloader unlocking was needed and no data was wiped.

Root is not the goal in itself. It buys three things this project wants: attaching a debugger
or Frida to the live SurfaceFlinger to observe the handwriting region tables directly instead
of inferring them (see [02-handwriting-regions.md](02-handwriting-regions.md) and
[README](README.md)), reading `/proc/<pid>/maps` and other process state, and editing
`/onyxconfig/mmkv/onyx_config` by hand. The last of those turns out to be unnecessary — the EAC
service accepts writes from any app with no permission check at all
([04-system-screennote-pipeline.md](04-system-screennote-pipeline.md)).

## 1. The device state you start from

```bash
adb shell getprop ro.boot.verifiedbootstate   # orange
adb shell getprop ro.boot.flash.locked        # 0
adb shell getprop ro.boot.slot_suffix         # _b
adb shell uname -r                            # 4.19.157-perf-...
```

The bootloader ships **already unlocked** from the factory — no unlock step, no data wipe. That
is why none of the BOOX rooting guides mention unlocking.

The kernel is 4.19, built by Onyx, and no kernel sources for `lito` are published, so KernelSU
and KernelSU Next are out (they need GKI 5.10+ or a source build). APatch would work on 4.19 in
principle but has no track record on BOOX. Magisk is the trodden path here and is what this
document describes.

## 2. Why fastboot is a dead end

The bootloader answers `getvar` and reports itself unlocked:

```
unlocked: yes      secure: no      product: lito
current-slot: b    slot-count: 2   max-download-size: 805306368
```

but every command that would change anything is gone:

```
fastboot boot  boot.img   -> FAILED (remote: 'unknown command')
fastboot flash boot_b img -> FAILED (remote: 'unknown command')
fastboot reboot           -> prints "Rebooting" and does nothing
```

The last one is the nasty one: the device sits in the bootloader showing the BOOX logo (no
animation — that screen _is_ the bootloader, it is not a hang) and cannot be told to leave.
`fastboot continue` and `set_active` are equally inert. **Hold power for ~20 s to power it off,
then power on normally.** Nothing was written, so nothing is at risk.

Consequence: no `fastboot boot` means there is no way to test a patched boot image without
committing it to flash. Everything goes through EDL instead.

## 3. EDL: what it is and why it is safer here

EDL (Qualcomm Emergency Download, USB id `05c6:9008`) bypasses the bootloader entirely: the
SoC's mask ROM speaks the Sahara protocol, accepts a signed Firehose "programmer", and then
gives raw read/write access to every partition. Onyx cannot gut it — it is in silicon. It also
works when the device does not boot at all, which makes it the recovery path as well as the
installation path, and unlike fastboot it can **read** partitions, so real backups are possible.

```bash
# client and loader
git clone --depth 1 https://github.com/bkerler/edl.git edl-client
cd edl-client && uv venv --python 3.12 .venv && uv pip install --python .venv/bin/python -r requirements.txt
curl -L -o fhprg_lito.bin \
  https://github.com/bkerler/Loaders/raw/main/lenovo_motorola/0000000000000000_bdaf51b59ba21d8a_fhprg.bin
```

That loader is the one the Palma 2 / Note Air 4C guides use; both devices share the SoC. Stop
ModemManager first or it will grab the port. Then:

```bash
sudo systemctl stop ModemManager
adb reboot edl            # screen goes completely black — this is normal
lsusb | grep 9008
sudo .venv/bin/python edl.py printgpt --loader=../fhprg_lito.bin
```

A successful handshake prints `HWID: 0x0013f0e100000000`, `CPU detected: "bitra_SDM"`, and a
17-function Firehose set including `program`, `read`, `erase` and `getsha256digest`.

Leaving EDL: `edl.py reset`. It usually ends with a USB I/O error as the device drops off the
bus mid-command — that is the reset working, not a failure.

## 4. Backups first

```bash
sudo .venv/bin/python edl.py r boot_a ../backup/boot_a_stock.img --loader=../fhprg_lito.bin
sudo .venv/bin/python edl.py r boot_b ../backup/boot_b_stock.img --loader=../fhprg_lito.bin
```

Worth checking: `boot_b_stock.img` should be byte-identical to `boot.img` extracted from the
official firmware package for the same build (see
[01-sources-and-firmware.md](01-sources-and-firmware.md)). On this device it was —
`926326d137bfa8d764e469cf2b5783ef` both ways. That single check proves the firmware package
matches the device, that the slot is untouched, and that Magisk will be patching the right
image.

Only the active slot is written. The other slot keeps its stock image as a fallback.

## 5. Patch and flash

```bash
curl -L -O https://github.com/topjohnwu/Magisk/releases/download/v30.7/Magisk-v30.7.apk
adb install -r Magisk-v30.7.apk
adb push boot.img /sdcard/Download/boot_stock_4.2.img
```

On the device: Magisk → **Install** → **Select and Patch a File** → pick that image. It writes
`magisk_patched-<ver>_<rand>.img` next to it. Pull it, write it to the active slot, read it
back, compare:

```bash
adb pull /sdcard/Download/magisk_patched-30700_*.img boot_magisk.img
sudo .venv/bin/python edl.py w boot_b ../boot_magisk.img --loader=../fhprg_lito.bin
sudo .venv/bin/python edl.py r boot_b ../backup/boot_b_verify.img --loader=../fhprg_lito.bin
md5sum ../backup/boot_b_verify.img ../boot_magisk.img    # must match
sudo .venv/bin/python edl.py reset --loader=../fhprg_lito.bin
```

The device boots normally and `/product/bin/magisk` reports `30.7:MAGISK:R`. Root is installed —
but not yet usable, because of the next section.

## 6. The BOOX AMS defect, and the deadlock it creates

On firmware 4.1 and later the Magisk app hangs on its splash screen and never opens. The cause
is a defect Onyx introduced in `ActivityManagerService.addPackageDependency`. In the decompiled
`services.jar` from this device the tail of the method reads:

```java
if (processRecord != null) {
    ...                       // AOSP body
}
new UpdateWebViewUsedPkgsAction(processRecord.info.packageName, str).execute();   // outside!
```

The Onyx call — WebView usage tracking, added for e-ink refresh optimisation — sits **outside**
the null check. Any caller that is not in the system's pid map dereferences null. Magisk's root
service is exactly that: uid 0, no `ProcessRecord`. Confirmed on this device:

```
E ActivityManager: Activity Manager Crash. UID:0 PID:5165 TRANS:88
E ActivityManager: java.lang.NullPointerException: Attempt to read from field
    'android.content.pm.ApplicationInfo com.android.server.am.ProcessRecord.info'
    ... at ActivityManagerService.addPackageDependency(ActivityManagerService.java:4249)
E IPC: at android.app.LoadedApk.getClassLoader(LoadedApk.java:1126)
E IPC: at com.topjohnwu.superuser.internal.RootServerMain.main
```

There is a community module for this, [`boox-ams-fix`](https://github.com/dynamicfire/boox-ams-fix),
but its prebuilt `services.jar` is from a P6 Pro; ART refuses a jar that does not match the
build and the device will not boot. The patch has to be made from **this** device's own file.

And that creates a deadlock: installing any Magisk module needs root, root needs a grant, the
grant dialog is drawn by the app, and the app is broken.

Two dead ends worth recording so nobody retries them. The su request dialog does appear **once**,
on the very first request after install (`SuRequestActivity` shows in the activity stack) — but
if it times out, or if the screen is locked so it cannot be shown, magiskd rejects every later
request instantly (`W Magisk: su: request rejected (2000)`) and never asks again. And rebooting
does not reset that, because by then the app itself no longer starts at all.

## 7. Breaking the deadlock with an `overlay.d` init rule

`magiskinit` loads `*.rc` files from `overlay.d` in the boot ramdisk and merges them into init's
configuration. That gives one root-privileged command at boot without any module and without the
app. One line is enough — write the su policy for the adb shell uid straight into Magisk's
database:

`overlay.d/init.amsfix.rc`:

```
on property:sys.boot_completed=1
    setprop sys.amsfix 1
    exec u:r:magisk:s0 root root -- /product/bin/magisk --sqlite "REPLACE INTO policies (uid,policy,until,logging,notification) VALUES(2000,2,0,0,0)"
    setprop sys.amsfix 2
```

Policy `2` is allow; uid `2000` is the adb shell. The two `setprop` lines are only markers, so
you can tell afterwards whether init parsed and ran the block. Use a `sys.`-prefixed name:
arbitrary property names are rejected by init's SELinux contexts.

Edit the ramdisk with `magiskboot`, which ships inside the Magisk apk and runs on the device:

```bash
unzip -o Magisk.apk 'lib/arm64-v8a/*'
adb push lib/arm64-v8a/libmagiskboot.so /data/local/tmp/magiskboot
adb push boot_magisk.img /data/local/tmp/boot_in.img
adb push init.amsfix.rc  /data/local/tmp/init.amsfix.rc
adb shell 'cd /data/local/tmp && chmod 755 magiskboot && ./magiskboot unpack boot_in.img \
  && ./magiskboot cpio ramdisk.cpio "add 0644 overlay.d/init.amsfix.rc init.amsfix.rc" \
  && ./magiskboot repack boot_in.img boot_out.img'
adb pull /data/local/tmp/boot_out.img boot_amsfix.img
```

**Verify the file is really in the repacked image before flashing** — unpack the output and list
`overlay.d`. An edit was silently lost once during this work and cost a flash-and-reboot cycle to
notice:

```bash
adb shell 'cd /data/local/tmp && mkdir -p v && cd v && cp ../magiskboot . \
  && ./magiskboot unpack ../boot_out.img && ./magiskboot cpio ramdisk.cpio "ls /overlay.d"'
```

Flash it the same way as before, reboot, then:

```bash
adb shell getprop sys.amsfix     # 2  -> the rule ran
adb shell su -c id               # uid=0(root) ... context=u:r:magisk:s0
```

## 8. Fixing the app properly

With root available, build a module from this device's own `services.jar` and let Magisk mount
it over the system one. Pull the jar (`adb shell cat /system/framework/services.jar > …`), find
which dex holds the class, and move the Onyx call inside the null check. The minimal edit is to
send the null branch to a `return-void` instead of to the Onyx block:

```bash
baksmali d classes.dex -o smali_out
# in ActivityManagerService.smali:
#   if-eqz v1, :cond_4c        ->  if-eqz v1, :cond_amsfix
#   and add, after the existing "return-void" that follows the Onyx call:
#       :cond_amsfix
#       return-void
smali a smali_out -o classes_patched.dex --api 33
```

Then rebuild the jar **keeping the dex stored uncompressed**, as it is in the original — `zip -X -0`,
not the default deflate. Package it as a Magisk module:

```
module.prop
system/framework/services.jar
```

and install it without the UI:

```bash
adb push boox-ams-npe-fix.zip /data/local/tmp/
adb shell su -c "magisk --install-module /data/local/tmp/boox-ams-npe-fix.zip"
adb reboot
```

After the reboot `md5sum /system/framework/services.jar` matches the patched file, the Magisk app
opens and stays in the foreground, and `addPackageDependency` no longer appears in logcat.
Behaviour for ordinary apps is unchanged: they have a `ProcessRecord`, so they still take the
Onyx branch.

## 9. Living with it

- The `overlay.d` rule is a scaffold, not a fix. Once the module works, root can be granted the
  normal way through the app; a clean Magisk-patched image can be flashed to drop the rule. It is
  worth keeping only if unattended `adb shell su` matters, as it does for automation here.
- **Incremental OTAs will not install on a rooted device**; only full packages will. Full packages
  wipe and reinstall the system partitions and simply remove root, which is then reinstalled the
  same way.
- The module is build-specific. Every firmware update ships a new `services.jar`, so the patch has
  to be rebuilt from the new file — otherwise ART rejects the mismatched jar.
- Recovery, if a boot image ever fails: `adb reboot edl` (or power on into EDL) and
  `edl.py w boot_b backup/boot_b_stock.img`. The other slot is still stock as a second line.

## Updating the firmware once rooted

Not yet done on this device — the steps below are derived from how the pieces work, and every one
that has not been executed says so. Read it before accepting an update, not after.

**Incremental packages will refuse to install.** They patch the existing partitions and verify them
first, and boot no longer matches. Only a full package will go on. Sizes tell them apart at a
glance (see the table in [01](01-sources-and-firmware.md)): 4.0 and 4.2 are full at ~2 GB, 4.0.1 and
4.1 are incremental at a few hundred MB. If the device offers an incremental update, the way
forward is to fetch the newest **full** package from the API instead, using the query in
[01](01-sources-and-firmware.md) with an older fingerprint.

**A full package removes root** — it rewrites boot along with everything else. That is not a
problem, it is the expected outcome: re-root afterwards exactly as in section 5, but patch the
**new** `boot.img`, taken from the new package, not the old file.

Order that keeps a working device at every step:

1. Read both boot partitions off the device again and keep them (section 4). They are the only way
   back if the update itself fails.
2. Fetch the new full package, decrypt it, and extract `boot.img` and `services.jar` from it —
   `payload.bin` gives the first, `system.img` plus `debugfs` the second.
3. Apply the update. Either through Settings, or by pushing the decrypted `update.zip` and starting
   the installer directly, which is what the vendor's own wiki documents:
   `adb push update.zip /sdcard/ && adb shell am start -n com.onyx.android.onyxotaservice/.OtaInfoActivity`.
   **Untested here.**
4. The device reboots unrooted. Verify it boots and that the build is what you expected before
   touching anything else.
5. Patch the new `boot.img` with the Magisk app and write it to the now-active slot over EDL, as in
   section 5. Note the active slot may have swapped — check `ro.boot.slot_suffix` and write to the
   slot the device is actually using.
6. Rebuild the `services.jar` module from the **new** file (section 8). The old module will not do:
   ART rejects a jar that does not match the build, and the device will not boot with it. Verify the
   Onyx defect is still there first — if a future firmware fixes `addPackageDependency`, the module
   becomes unnecessary and should simply be dropped.
7. Re-check anything the update may have reset: the EAC config store is rebuilt when an OTA bumps
   `jsonVersion`, so the app's refresh profile may need to be written again.

**Do not** try to restore the stock boot image to make an incremental update apply. The route is
untested here, and the guide this work followed reports soft-bricking a device that way.

## What root turned out to be worth

Two techniques, both used in anger, worth remembering because neither needs a rebuild of the app:

- **A thread dump of a live, wedged process**, without changing it:
  `adb shell su -c "debuggerd -b <pid>"`. This is what identified a stall as a long transaction on
  the database worker rather than a deadlock — every runtime thread parked, the SQLite thread alive
  inside a specific function, spilling a write-ahead log. Sampling it three times showed it
  advancing, which ruled out a lock.
- **Reading the EAC config store the firmware actually obeys**:
  `adb shell su -c "strings /onyxconfig/mmkv/onyx_config"`, then grepping for the package. The
  service's read path answers from a different key than its write path, so this is the only way to
  confirm what a configuration write really did — and it is how `supportEAC: false` was found on the
  stock Notes app and `true` on ours.

## Artefacts from this run

Kept outside the repo, under `~/tmp_zfs/boox_fw/`:

| path                                                 | what                                                      |
| ---------------------------------------------------- | --------------------------------------------------------- |
| `backup/boot_a_stock.img`, `backup/boot_b_stock.img` | factory boot images read off the device                   |
| `boot_magisk.img`                                    | Magisk-patched stock image                                |
| `boot_amsfix4.img`                                   | the above plus the `overlay.d` rule — what is flashed now |
| `amsfix/services_patched.jar`                        | patched `services.jar` for build 8548                     |
| `amsfix/boox-ams-npe-fix.zip`                        | the Magisk module built from it                           |
| `edl-client/`, `fhprg_lito.bin`                      | EDL client and Firehose loader                            |
