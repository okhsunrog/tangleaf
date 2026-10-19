{
  lib,
  pkgs,
  craneLib,
  pkg-config,
  openssl,
  gtk3,
  webkitgtk_4_1,
  wrapGAppsHook3,
  src,
}:
let
  select =
    paths:
    lib.cleanSourceWith {
      inherit src;
      name = "source";
      filter =
        path: type:
        let
          relative = lib.removePrefix "${src}/" path;
        in
        lib.any (
          selected:
          relative == selected
          || lib.hasPrefix "${selected}/" relative
          || (type == "directory" && lib.hasPrefix "${relative}/" selected)
        ) paths;
    };
  rustSrc = select [
    "Cargo.toml"
    "Cargo.lock"
    "crates"
    "plugins"
    "server"
    "src-tauri"
    "tools/ink-storage-bench"
  ];
  frontend = pkgs.callPackage ./frontend.nix {
    src = select [
      "package.json"
      "bun.lock"
      "index.html"
      "vite.config.ts"
      "tsconfig.json"
      "tsconfig.node.json"
      "src"
      "public"
      "assets"
    ];
  };
  args = {
    pname = "tangleaf";
    version = (builtins.fromTOML (builtins.readFile "${src}/Cargo.toml")).workspace.package.version;
    src = rustSrc;
    strictDeps = true;
    cargoExtraArgs = "--locked -p tangleaf --bin tangleaf --features tauri/custom-protocol";
    nativeBuildInputs = [
      pkg-config
      pkgs.cmake
      pkgs.nasm
    ];
    buildInputs = [
      openssl
      gtk3
      webkitgtk_4_1
    ];
    doCheck = false;
  };
  cargoArtifacts = craneLib.buildDepsOnly args;
in
craneLib.buildPackage (
  args
  // {
    inherit cargoArtifacts;
    nativeBuildInputs = args.nativeBuildInputs ++ [ wrapGAppsHook3 ];
    preBuild = "cp -a ${frontend} dist";
    postInstall = ''
      install -Dm644 src-tauri/icons/128x128.png $out/share/icons/hicolor/128x128/apps/tangleaf.png
      install -Dm644 ${./tangleaf.desktop} $out/share/applications/dev.okhsunrog.tangleaf.desktop
      substituteInPlace $out/share/applications/dev.okhsunrog.tangleaf.desktop --replace-fail "Exec=tangleaf" "Exec=$out/bin/tangleaf"
    '';
    passthru = { inherit frontend cargoArtifacts; };
    meta = {
      description = "Graph-native personal knowledge application";
      homepage = "https://github.com/okhsunrog/tangleaf";
      mainProgram = "tangleaf";
      platforms = [ "x86_64-linux" ];
    };
  }
)
