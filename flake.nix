{
  description = "Tangleaf desktop application";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/fe13242281a67641c0102cdb1087f0ebe586c7df";
    # WebKitGTK 2.54.2 fixes Skia animation-completion flashes.
    nixpkgs-webkit.url = "github:NixOS/nixpkgs/512527cdaf7eab2f9e4c5e6e3935c1648d156621";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs =
    {
      self,
      nixpkgs,
      nixpkgs-webkit,
      crane,
      rust-overlay,
    }:
    let
      system = "x86_64-linux";
      mkPackage =
        pkgs:
        let
          toolchains = import nixpkgs {
            inherit (pkgs.stdenv.hostPlatform) system;
            overlays = [ rust-overlay.overlays.default ];
          };
          channel = (builtins.fromTOML (builtins.readFile ./rust-toolchain.toml)).toolchain.channel;
          craneLib = (crane.mkLib pkgs).overrideToolchain toolchains.rust-bin.stable.${channel}.minimal;
        in
        pkgs.callPackage ./nix/package.nix {
          inherit craneLib;
          src = self;
        };
      webkitPkgs = import nixpkgs-webkit { inherit system; };
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ (_final: _prev: { webkitgtk_4_1 = webkitPkgs.webkitgtk_4_1; }) ];
      };
      desktopLibraries = import ./nix/desktop-libraries.nix { inherit pkgs; };
    in
    {
      packages.${system} = {
        default = self.packages.${system}.tangleaf;
        tangleaf = mkPackage pkgs;
        desktop-runtime = pkgs.callPackage ./nix/desktop-runtime.nix { };
      };
      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          rustup
          just
          pkg-config
          cmake
          nasm
          shellcheck
          desktop-file-utils
          jq
        ];
        buildInputs = with pkgs; [
          openssl
          gtk3
          webkitgtk_4_1
        ];
        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath desktopLibraries;
      };
      lib.mkPackage = mkPackage;
      checks.${system}.package = self.packages.${system}.tangleaf;
      formatter.${system} = pkgs.nixfmt-tree;
    };
}
