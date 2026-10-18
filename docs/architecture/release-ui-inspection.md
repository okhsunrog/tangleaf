# Optimized desktop UI inspection

`runtime-inspection` explicitly enables the localhost MCP bridge in an optimized
build. It is off by default. Normal release builds keep the bridge disabled.
This feature does not change Rust optimization, debug assertions, durability or
the native handwriting path.

Build after coordinating a quiet build/measurement window:

```sh
vp run tauri build --no-bundle --features runtime-inspection \
  --config src-tauri/tauri.dev.conf.json
```

This uses the normal release profile (Thin LTO, one codegen unit) and bundled,
minified frontend assets. The explicit config overlay enables the global Tauri
API and the script CSP needed for inspection. Its name does not select a debug
build. Do not pass `--debug` or override release optimization/debug assertions.
Confirm the startup log contains `debug_assertions=false` and record the exact
command, commit, uncommitted source diff, binary hash, machine and window size.
Report measurements as an optimized release **with inspection instrumentation**.

Launch the built binary in an isolated virtual KWin session, with all four
`XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and `XDG_STATE_HOME` pointing
inside `/home/okhsunrog/tmp_zfs/codex_tmp/tangleaf-database-layer/stage5/release-gui/`.
Set `TMPDIR` there too. Prepare only a disposable copied/synthetic workspace;
do not copy synchronization credentials or modify the user's workspace. Use
ordinary INFO logging for comparison runs. DEBUG phase profiling is a separate
run, excluded from the comparison. Stop the virtual session when finished.

Connect after the application has started:

```sh
vpx @hypothesi/tauri-mcp-cli driver-session start --port 9223
vpx @hypothesi/tauri-mcp-cli driver-session status --json
```

Require `connected: true`; the start command alone does not establish that the
application is reachable. The bridge may select another free port if 9223 is
occupied; use the port reported by the application. End with `driver-session stop`.

## Split measurement

Inject the complete contents of [release-ui-probe.mjs](../../tools/release-ui-probe.mjs)
with `webview-execute-js`. Bundled `@tauri-apps/api` calls the read-only
`__TAURI_INTERNALS__.invoke`, so the helper observes its actual desktop IPC fetch
transport and callback registry. It leaves callback results unchanged and
restores its hooks on disposal. An untimed handshake rejects unsupported/fallen
back transports explicitly. The bridge's global API monitor alone does not
observe imported application calls.

Prepare a block containing 109 or 243 paragraphs outside the measured interval,
open it through the real application UI and allow pending saves/reads to settle.
The helper never invokes a mutation. Arm it using the fixture block UUID:

```js
await window.__tangleafReleaseProbe.arm("fixture-block-uuid", 243);
```

Use KWin to click the long-block information icon, then click the actual toast
action **Split**. The probe requires a trusted browser click on that action and
the corresponding `split_block` invocation for the armed source/count. It
correlates the mutation event with every returned split UUID, including newly
created UUIDs, rather than accepting any event for the same page.

Read `window.__tangleafReleaseProbe.result()` after completion. It returns only
UUIDs, counts, timings and status. Readiness requires the final block's editor
to be focused, visible, contain the expected paragraph, and remain ready over
two further animation frames. `editorReadyMs` starts at the browser's trusted
click handler; it excludes compositor/input delivery before the browser event.
It is an operational readiness proxy, not a measurement of physical display
presentation. `ipcAckMs` includes IPC and response decoding up to success callback
delivery, before the application's promise reactions; it is not just SQL commit.
Read requests observed before readiness have individual start/end timestamps;
these can overlap and must not be added together as independent costs.

Outside the timed interval:

```js
await window.__tangleafReleaseProbe.verifySplit();
// Click the real Undo button, then wait for the restored document.
await window.__tangleafReleaseProbe.verifyOriginal();
// Click Redo and wait for the split document.
await window.__tangleafReleaseProbe.verifySplit();
```

Require `exact: true`; verification compares all split paragraphs and unchanged
siblings, including UUID/order/style, without returning note text. Undo returns
to the original state for the next sample. Run a warm-up, then at least 30 warm
samples for each size; publish p50/p95 and failed/timeout counts, never silently
drop failed samples. Keep screenshots, verification IPC and background builds
outside timing. Record mounted rows at readiness to check virtualization.

Repeat acceptance at 1200×800 and 390×844: scroll/reveal, actual keyboard input,
autosave verified through public IPC, Undo/Redo and navigation away/back. Narrow
desktop resizing checks responsive UI; it does not establish Android/BOOX input
or display latency. A separate trace with DEBUG backend phases can explain slow
samples; the probe alone cannot isolate Rust execution, SQLite write-lock waits,
React work or CodeMirror layout. Dispose injected hooks after the run:

```js
await window.__tangleafReleaseProbe.dispose();
```
