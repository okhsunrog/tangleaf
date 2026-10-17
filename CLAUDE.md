<!--VITE PLUS START-->

# Using Vite+, the Unified Toolchain for the Web

This project is using Vite+, a unified toolchain built on top of Vite, Rolldown, Vitest, tsdown, Oxlint, Oxfmt, and Vite Task. Vite+ wraps runtime management, package management, and frontend tooling in a single global CLI called `vp`. Vite+ is distinct from Vite, and it invokes Vite through `vp dev` and `vp build`. Run `vp help` to print a list of commands and `vp <command> --help` for information about a specific command.

Docs are local at `node_modules/vite-plus/docs` or online at https://viteplus.dev/guide/.

## Built-in Commands vs Scripts

`vp <name>` runs a built-in command. `vp run <name>` runs a `package.json` script or a `vite.config.ts` task. Scripts cannot overwrite built-ins, so `vp dev` and `vp run dev` may do different things. Check `package.json` and `vite.config.ts` first, and run `vp run <name>` when the project defines a script or task with that name.

## Tool Versions

Run `vp toolchain` to show versions and relationships in the active Vite+
release. Add a tool name to select part of the graph. For example, run
`vp toolchain vite`. Use `--global` to ignore the local `vite-plus` package. Use
`vp why <package>` to show the package-manager dependency graph.

## Review Checklist

- [ ] Run `vp install` after pulling remote changes and before getting started.
- [ ] Run `vp check` and `vp test` to format, lint, type check and test changes.
- [ ] Check if there are `vite.config.ts` tasks or `package.json` scripts necessary for validation, run via `vp run <script>`.
- [ ] If setup, runtime, or package-manager behavior looks wrong, run `vp env doctor` and include its output when asking for help.

<!--VITE PLUS END-->

## Build and device notes

- Rust command changes: regenerate `src/lib/bindings.ts` with `vp run bindings:generate`; never edit it by hand.
- Android debug APK for device work: `just build-android-debug` (dev config with the MCP bridge, optimized Rust, debug assertions). Release APKs: `just build-android-arm64`.
- Live inspection of a running debug build goes through the Tauri MCP bridge (`adb forward tcp:9223 tcp:9223` on Android, then a driver session on port 9223). Chrome DevTools also works on Android via `adb forward tcp:<port> localabstract:webview_devtools_remote_<pid>`.
- The BOOX handwriting path (`plugins/tauri-plugin-mobile-system`, `src/features/handwriting/ink-canvas.tsx`, `onyx-ink.ts`) is verified on hardware; do not change its frame fence, eraser gate or display-mode ownership without a device retest.
