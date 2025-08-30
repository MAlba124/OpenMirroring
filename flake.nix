{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs = {
        nixpkgs.follows = "nixpkgs";
      };
    };
  };
  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem
      (system:
        let
          overlays = [ (import rust-overlay) ];
          pkgs = import nixpkgs {
            inherit system overlays;
          };

          rustTargets = [
            "aarch64-unknown-linux-gnu"
            "x86_64-unknown-linux-gnu"
          ];

          rustToolchain = pkgs.rust-bin.nightly.latest.complete.override {
            targets = rustTargets;
          };
        in
        let
          nativeBuildInputs = with pkgs; [
            pkg-config
            clang
            dig
            graphviz
            rustToolchain
            graphviz
            slint-lsp
            heaptrack
            slint-viewer
          ];
          buildInputs = with pkgs; [
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
        in
        with pkgs;
        {
          devShells.default = mkShell {
            LD_LIBRARY_PATH = lib.makeLibraryPath buildInputs;
            BINDGEN_EXTRA_CLANG_ARGS = [
                ''-I"${pkgs.llvmPackages.libclang.lib}/lib/clang/${pkgs.llvmPackages.libclang.version}/include"''
            ];
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

            inherit buildInputs nativeBuildInputs;
          };
        }
      );
}
