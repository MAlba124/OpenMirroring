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

          rustAndroidTargets = [
            "aarch64-linux-android"
            "x86_64-linux-android"
          ];

          rustAndroidToolchain = pkgs.rust-bin.stable.latest.complete.override {
            targets = rustAndroidTargets;
          };
        in
        let
          nativeBuildInputs = with pkgs; [
            pkg-config
            clang
            dig
            rustAndroidToolchain
            graphviz
            android-studio
            cargo-ndk
            cargo-apk
            zulu
            gnumake
            patchelf
            graphviz
            slint-lsp
            android-tools
          ];
          buildInputs = with pkgs; [];
        in
        with pkgs;
        {
          devShells.default = mkShell {
            LD_LIBRARY_PATH = lib.makeLibraryPath buildInputs;
            BINDGEN_EXTRA_CLANG_ARGS = [
                ''-I"${pkgs.llvmPackages.libclang.lib}/lib/clang/${pkgs.llvmPackages.libclang.version}/include"''
                "-I ${pkgs.glibc.dev}/include"
            ];
            # TOD: Fix these hacks
            shellHook = ''
                export ANDROID_HOME="$HOME/Android/Sdk";
                export ANDROID_NDK_ROOT="$HOME/Android/Sdk/ndk/29.0.13113456";
                # export GSTREAMER_ROOT_ANDROID="$(pwd)/gst-android-1.0-1.24.12";
                export GSTREAMER_ROOT_ANDROID="$(pwd)/gst-android-1.0-1.26.1";
                export PKG_CONFIG_ALLOW_CROSS=1

                # Add more when targeting other archs
                export PKG_CONFIG_PATH="$PKG_CONFIG_PATH:$GSTREAMER_ROOT_ANDROID/x86_64/lib/pkgconfig"
            '';

            inherit buildInputs nativeBuildInputs;
          };
        }
      );
}
