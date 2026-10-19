# NixOS

The flake builds the Linux desktop app from its committed Cargo and Bun locks.
`nix build` produces `result/bin/tangleaf` and its desktop launcher. No AppImage
or FHS chroot is involved. The application keeps the usual Tangleaf app ID and
user data locations; secrets and note databases are runtime data, not inputs.

```nix
# flake inputs
inputs.tangleaf.url = "github:okhsunrog/tangleaf";
inputs.tangleaf.inputs.nixpkgs.follows = "nixpkgs";

# module, with tangleaf passed through specialArgs
environment.systemPackages = [ tangleaf.lib.mkPackage pkgs ];
```

Crane builds the external Rust dependencies once and reuses their artifacts
for application changes. The Rust compiler version comes from
`rust-toolchain.toml` through the pinned rust-overlay input; Android toolchains
are not downloaded for a Linux package. Frontend source changes only affect the
frontend and final app, not the Rust dependency derivation. Bun dependencies
have a fixed output hash in `nix/frontend.nix`.

After changing the Bun lock, build with `lib.fakeHash` temporarily in place of
the old output hash and copy the reported actual hash back. Update flake inputs
with `nix flake update`; commit the lock. The first build and final release
linking can still take time. Desktop/WebKit operation needs a graphical session;
a completed sandboxed build alone is not a GUI acceptance test.
