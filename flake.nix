{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
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
        in
        let
          nativeBuildInputs = with pkgs; [
            pkg-config
            clang
            dig
            graphviz
            rust-bin.stable.latest.complete
            graphviz
            slint-lsp
            heaptrack
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
          ];
        in
        with pkgs;
        {
          devShells.default = mkShell {
            LD_LIBRARY_PATH = lib.makeLibraryPath buildInputs;
            BINDGEN_EXTRA_CLANG_ARGS = [
                ''-I"${pkgs.llvmPackages.libclang.lib}/lib/clang/${pkgs.llvmPackages.libclang.version}/include"''
                "-I ${pkgs.glibc.dev}/include"
            ];
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

            inherit buildInputs nativeBuildInputs;
          };
        }
      );
}
