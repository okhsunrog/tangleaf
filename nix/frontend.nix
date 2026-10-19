{
  lib,
  stdenv,
  bun,
  cacert,
  nodejs_24,
  autoPatchelfHook,
  src,
}:
let
  modules = stdenv.mkDerivation {
    pname = "tangleaf-node-modules";
    version = (builtins.fromJSON (builtins.readFile "${src}/package.json")).version;
    inherit src;
    nativeBuildInputs = [ bun ];
    dontConfigure = true;
    dontFixup = true;
    buildPhase = ''
      export HOME="$TMPDIR"
      bun install --frozen-lockfile --ignore-scripts --no-progress
    '';
    installPhase = "mkdir -p $out; cp -a node_modules $out/";
    outputHashMode = "recursive";
    outputHash = "sha256-7EofVF4w1E7F2199LXQf3mWRBVEh6lovICqJVAbWCa8=";
  };
in
stdenv.mkDerivation {
  pname = "tangleaf-frontend";
  version = modules.version;
  inherit src;
  nativeBuildInputs = [
    bun
    nodejs_24
    autoPatchelfHook
  ];
  buildInputs = [ stdenv.cc.cc.lib ];
  autoPatchelfIgnoreMissingDeps = [ "libc.musl-x86_64.so.1" ];
  env.SSL_CERT_FILE = "${cacert}/etc/ssl/certs/ca-bundle.crt";
  dontConfigure = true;
  buildPhase = ''
    export HOME="$TMPDIR"
    cp -a ${modules}/. .
    chmod -R u+w .
    patchShebangs node_modules
    autoPatchelf node_modules
    export PATH="$PWD/node_modules/.bin:$PATH"
    vp build
  '';
  installPhase = "cp -a dist $out";
  passthru.nodeModules = modules;
}
