{ pkgs }:
let
  libraries = import ./desktop-libraries.nix { inherit pkgs; };
in
pkgs.stdenvNoCC.mkDerivation {
  name = "tangleaf-desktop-runtime";
  dontUnpack = true;
  dontConfigure = true;
  dontBuild = true;
  nativeBuildInputs = [
    pkgs.makeWrapper
    pkgs.wrapGAppsHook3
  ];
  buildInputs = [
    pkgs.gtk3
    pkgs.webkitgtk_4_1
  ];
  installPhase = ''
    runHook preInstall
    mkdir -p "$out/bin"
    cat > "$out/bin/tangleaf-local-run" <<'EOF'
    #!${pkgs.runtimeShell}
    exec "$@"
    EOF
    chmod +x "$out/bin/tangleaf-local-run"
    runHook postInstall
  '';
  # Local Cargo outputs do not receive the package build's RPATH fixup.
  # These references also retain the linked libraries in the runtime GC root.
  preFixup = ''
    gappsWrapperArgs+=(--prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath libraries}")
  '';
}
