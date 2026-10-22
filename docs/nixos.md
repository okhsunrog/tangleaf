# NixOS

## Local development and desktop installation

Nix supplies the compiler environment and GTK/WebKit libraries. Rustup uses the
compiler pinned in `rust-toolchain.toml`; Vite+ manages Node.js and Bun. Application
builds use the ordinary Cargo caches, independently of NixOS rebuilds.

With direnv and nix-direnv enabled, allow the checkout once:

```sh
direnv allow
vp install
just install-desktop
```

The environment activates automatically when entering the directory. Without
direnv, run `nix develop`, then the same commands. For frontend hot reload use
`vp run desktop:dev` inside this environment.

`just install-desktop` builds the frontend and an optimized Rust executable with
profile `desktop` (no LTO, 16 codegen units, incremental compilation), then installs
`~/.local/bin/tangleaf`, the icon and the user desktop entry. The launcher points
to this checkout's `target/desktop/tangleaf` (or Cargo's configured target directory).
The app ID and existing user data locations stay the same.

For subsequent changes:

```sh
just build-desktop
```

Restart the app to use the rebuilt executable. No reinstall or NixOS rebuild is
needed. `just update-desktop` also refreshes the launcher and runtime wrapper;
use it after moving the checkout or updating the flake's runtime libraries.

The launcher uses a small Nix wrapper for GTK/GIO desktop environment setup.
`~/.local/state/tangleaf/runtime` is its GC root, keeping the required runtime
libraries available after leaving the development shell or running Nix GC.
The installer respects `XDG_DATA_HOME` and `XDG_STATE_HOME` when set. Keep the
checkout and runtime link while using the local app. To uninstall, remove
`~/.local/bin/tangleaf`, the user `dev.okhsunrog.tangleaf.desktop` entry, the
`tangleaf.png` icon and the runtime link.

## Reproducible release package

`nix build` remains available for a reproducible Linux package from the committed
Cargo and Bun locks. It produces `result/bin/tangleaf` and a desktop launcher.
This uses the size-optimized release profile, so it is slower than local iteration.
No AppImage or FHS chroot is involved. Secrets and note databases are runtime data,
not build inputs.

Crane caches external Rust dependencies separately from application changes.
The release compiler comes from `rust-toolchain.toml` through the pinned
rust-overlay input. Frontend source changes affect only the frontend and final
app. Bun dependencies have a fixed output hash in `nix/frontend.nix`.
After changing the Bun lock, build with `lib.fakeHash` temporarily in place of
the old output hash and copy the reported actual hash back. Update flake inputs
with `nix flake update`; commit the lock.

Desktop/WebKit operation needs a graphical session; a successful build alone
is not a GUI acceptance test.
