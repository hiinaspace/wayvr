{
  description = "WayVR development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
    in {
      devShells.${system}.default = pkgs.mkShell {
        # Match the buildInputs from the nixpkgs-xr wayvr derivation exactly.
        # Run `nix develop` in this directory to enter the shell.
        buildInputs = with pkgs; [
          # Rust toolchain — after entering the shell, run: rustup default stable
          rustup

          # Native library headers (match wayvr's nixpkgs-xr derivation)
          alsa-lib.dev
          dbus.dev
          libxkbcommon.dev
          openssl.dev
          openxr-loader.dev
          pipewire.dev
          dav1d.dev
          systemdLibs.dev # libudev
          openvr

          # X11 (for the x11 feature)
          libx11.dev
          libxext.dev
          libxrandr.dev
          libxcb.dev

          # Build tools
          pkg-config
          shaderc.dev      # headers + pkg-config for shaderc
          shaderc.static   # libshaderc_combined.a (Rust shaderc crate links this)
          clang            # needed by rust-bindgen (input-linux, libmonado-rs)
          cmake            # whisper-rs-sys (default "whisper" feature)
          shaderc          # glslc for GGML_VULKAN
          vulkan-headers
          vulkan-loader
        ];

        shellHook = ''
          export LIBCLANG_PATH="${pkgs.libclang.lib}/lib"
          # The Rust `shaderc` crate links libshaderc_combined.a from this dir
          export SHADERC_LIB_DIR="${pkgs.shaderc.static}/lib"
          # vulkano-shaders proc macro needs these at runtime when rustc dlopen's it
          export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [
            pkgs.spirv-tools.lib
            pkgs.glslang
            pkgs.vulkan-loader
            pkgs.wayland
            pkgs.libxkbcommon
          ]}:$LD_LIBRARY_PATH"
          # nixpkgs libshaderc_combined.a is a thin wrapper; it leaves spirv-tools and
          # glslang symbols unresolved. Force the linker to pull them in as shared libs
          # so that the vulkano-shaders proc macro .so can be dlopen'd by rustc.
          export RUSTFLAGS="-L native=${pkgs.spirv-tools.lib}/lib -l dylib=SPIRV-Tools-shared -L native=${pkgs.glslang}/lib -l dylib=glslang ''${RUSTFLAGS:-}"
          export CARGO_HOME="''${CARGO_HOME:-$HOME/.cargo}"
          echo "WayVR dev shell ready. If first time: rustup default stable"
        '';
      };
    };
}
