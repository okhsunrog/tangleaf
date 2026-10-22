{ pkgs }:
with pkgs;
[
  openssl
  gtk3
  glib
  gdk-pixbuf
  cairo
  dbus
  webkitgtk_4_1
  libsoup_3
  stdenv.cc.cc.lib
  glibc
]
