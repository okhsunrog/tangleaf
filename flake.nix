{
  description = "Tangleaf desktop application";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/fe13242281a67641c0102cdb1087f0ebe586c7df";
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
      pkgs = import nixpkgs { inherit system; };
    in
    {
      packages.${system} = {
        default = self.packages.${system}.tangleaf;
        tangleaf = mkPackage pkgs;
      };
      lib.mkPackage = mkPackage;
      checks.${system}.package = self.packages.${system}.tangleaf;
      formatter.${system} = pkgs.nixfmt-tree;
    };
}
