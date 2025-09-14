{ pkgs ? import <nixpkgs> {} }:

let
  # target = "aarch64-unknown-linux-gnu";
  # crossPkgs = pkgs.pkgsCross.aarch64-multiplatform;
  # targetLib = name: crossPkgs.${name};
  rustPlatform = pkgs.rustPlatform;
in
rustPlatform.buildRustPackage rec {
  # pname = "my-rust-app-aarch64";
  pname = "my-rust-app";
  version = "0.1.0";
  src = ./.;

  target = "receiver";

  nativeBuildInputs = with pkgs; [
    cargo
    rustc
    pkg-config
    clang
  ];
  buildInputs = with pkgs; [
    openssl
  ] ++ [
    # crossPkgs.gcc
    # crossPkgs.binutils
    # (crossPkgs.openssl)
    # (crossPkgs.libGL)
    # (crossPkgs.libxkbcommon)
    # (crossPkgs.wayland)
    # (crossPkgs.xorg.libX11)
    # (crossPkgs.xorg.libXcursor)
    # (crossPkgs.xorg.libXi)
    # (crossPkgs.xorg.libXrandr)
    # (crossPkgs.pipewire)
    # (crossPkgs.alsa-lib)
    # (crossPkgs.libclang)
    # (crossPkgs.dbus)
    # (crossPkgs.gst_all_1.gstreamer)
    # (crossPkgs.gst_all_1.gst-plugins-base)
    # (crossPkgs.gst_all_1.gst-plugins-good)
    # (crossPkgs.gst_all_1.gst-plugins-bad)
    # (crossPkgs.gst_all_1.gst-plugins-ugly)
    # (crossPkgs.gst_all_1.gst-plugins-rs)
    # (crossPkgs.gst_all_1.gst-rtsp-server)
    # (crossPkgs.gst_all_1.gst-libav)
    # (crossPkgs.glib)
    # (crossPkgs.openssl)
    # (crossPkgs.libnice)
    # (crossPkgs.fontconfig)

    # KMS:
    seatd
    libgbm
    systemdLibs
    libinput

    gcc
    binutils
    openssl
    libGL
    libxkbcommon
    wayland
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
    pipewire
    alsa-lib
    libclang
    dbus
    gst_all_1.gstreamer
    gst_all_1.gst-plugins-base
    gst_all_1.gst-plugins-good
    gst_all_1.gst-plugins-bad
    gst_all_1.gst-plugins-ugly
    gst_all_1.gst-plugins-rs
    gst_all_1.gst-rtsp-server
    gst_all_1.gst-libav
    glib
    openssl
    libnice
    fontconfig

    # KMS:
    seatd
    libgbm
    systemdLibs
    libinput
  ];

  cargoHash = "sha256-+Tjc06BfotUN/TuUdr3wsIAjOJ7IqmITy4plsofAOMk=";
  cargoLcok = ./Cargo.lock;

  release = true;

  meta = with pkgs.lib; {
    description = "Cross-compiled aarch64 GNU Rust binary";
    license = licenses.gpl3;
  };
}
