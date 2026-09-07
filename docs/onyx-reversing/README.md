# Reverse engineering the Onyx BOOX pen and e-ink stack

Working notes for the handwriting path of this app on an Onyx BOOX Note Air 4C
(`ro.product.model = NoteAir4C`, platform `lito`, Android 13, firmware
`2026-04-28_17-50_4.2-rel_04282_555977efe`, build 8548, Kaleido 3 colour panel).

These documents exist because the vendor SDK is a thin wrapper over undocumented system
behaviour, and because guessing at that behaviour cost us several wrong fixes. Everything here
is cited to a file and line, a symbol, or a virtual address; anything that is inference says so.

## The five layers

Understanding any symptom means knowing which of these it lives in:

1. **Digitizer.** The pen's evdev device. Read by `libonyx_pen_touch_reader.so`
   (`android::TouchReader`, `android::PenManager`) inside _our_ process, loaded by the SDK.
   Holds its own copy of the limit and exclude regions, and a sticky "last region hit" latch.
2. **Onyx Pen SDK** (`TouchHelper`, `RawInputReader`, `SFTouchRender`, `EpdController`) — the
   Java API we call. Mostly plumbing; several of its methods are silent no-ops on this device
   because the reflective lookup they depend on does not resolve.
3. **The patched framework** (`framework.jar`): `android.onyx.ViewUpdateHelper` turns every
   drawing and configuration call into a Binder transaction to SurfaceFlinger, and
   `android.onyx.optimization.**` holds the system's own handwriting state machine and the EAC
   (EinkWise) config model. `android.onyx.optimization.OECService` is the system service, hosted
   in system_server.
4. **SurfaceFlinger**, patched by Onyx: the real owner of the ink layer, the handwriting
   regions, the EPDC schemas and the live low-latency stroke. Not readable from a running
   device; we extracted it from the official firmware package.
5. **The panel and its EPDC**: waveforms, update modes, dithering, the colour filter array.

## The documents

| file                                                                 | what it answers                                                                                                                                                      |
| -------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [01-sources-and-firmware.md](01-sources-and-firmware.md)             | how every artefact was obtained: pulling system files without root, decompiling, and the full recipe for downloading, decrypting and unpacking the official firmware |
| [02-handwriting-regions.md](02-handwriting-regions.md)               | where the limit/exclude/region-mode state lives, its lifetime and scope, and the one call that clears a latched exclusion                                            |
| [03-raw-drawing-render-flag.md](03-raw-drawing-render-flag.md)       | what the raw-drawing render-flag cycle emits, why it is the only thing that heals a stuck region, what it costs, and when to fire it                                 |
| [04-system-screennote-pipeline.md](04-system-screennote-pipeline.md) | the system's own handwriting pipeline, how an app becomes eligible for it, and why we deliberately stay out of it                                                    |
| [05-panel-refresh-levers.md](05-panel-refresh-levers.md)             | every refresh lever an unprivileged app can reach, what each does on a colour panel, and which of ours are pointless                                                 |

Related, outside this directory: [`../planning/handwriting-pen-stack-comparison.md`](../planning/handwriting-pen-stack-comparison.md)
compares our implementation against the stock Notes app and five open-source BOOX note apps,
state by state through our git history.

## What is settled

- Handwriting region state is **global to the SurfaceFlinger process**. The transactions carry no
  pid, no window and no surface, and nothing reaps them when the caller dies — so a stale
  exclusion survives an app reinstall and clears only at reboot. This is the dead-rectangle bug.
- The only call that clears it is the vendor's own, and it needs a **null View**:
  `EpdController.setScreenHandWritingRegionExclude(null, arrayOf(Rect(0,0,0,0)))`. An empty list
  is discarded inside the SDK, and a degenerate rect cannot work because the native test inflates
  every exclusion by half the stroke width.
- The firmware performs that reset itself on window-focus gain and session start — but only for
  views it recognises as note views, which is why the stock app heals and we do not.
- The render-flag cycle is `enablePost(1)` plus pen state PAUSE→DRAWING, the only handwriting
  transactions carrying our pid, and therefore the only app-reachable way to rebuild the
  SurfaceFlinger ink session.
- Joining the system pipeline is possible (the service enforces no permission at all) but costs us
  ownership of pen state, region limit and stroke parameters, and adds a full-panel repaint on
  every finger touch. We stay out.
- On this device there are three refresh profiles, not five, and "Speed" (A2) is already the
  factory default for third-party apps — so the profile write we added changes nothing, while A2
  itself disables the firmware's counted auto-GC, its post-input quality repaint and dithering.

## What is still open

- The SurfaceFlinger side of all of the above. Documents 02-05 were written before the binary was
  extracted, so their "not readable without root" caveats are now addressable; a document covering
  the binary itself is in progress. The device is now rooted ([06](06-rooting.md)), so the
  remaining questions can be settled by observing the live process rather than by static reading.
- Which single sequence an app should send to clear a dead rectangle, verified on hardware rather
  than derived. Each document ends with experiments phrased as one testable action each.
