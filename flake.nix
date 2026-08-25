{
  description = "Reproducible OpenPencil native renderer and raster exporter";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    crane,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachSystem ["x86_64-linux"] (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [(import rust-overlay)];
      };
      rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;
      src = pkgs.lib.cleanSourceWith {
        src = ./.;
        filter = path: type:
          pkgs.lib.cleanSourceFilter path type
          && !builtins.elem (baseNameOf path) [".nix-target" "result"];
      };
      skiaBinaries = pkgs.fetchurl {
        url = "https://github.com/rust-skia/skia-binaries/releases/download/0.97.2/skia-binaries-da8fc6731fc439bc3b6a-x86_64-unknown-linux-gnu-jpegd-jpege-pdf-textlayout.tar.gz";
        hash = "sha256-wGZlixPiV9QY9kdEfQbrioPLBgsDcijag4WJ3YY78FM=";
      };
      nativeBuildInputs = with pkgs; [
        clang
        cmake
        git
        mold
        pkg-config
        python3
      ];
      buildInputs = with pkgs; [
        fontconfig
        freetype
        stdenv.cc.cc.lib
      ];
      env = {
        CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER = "${pkgs.clang}/bin/clang";
        LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
        RUSTFLAGS = "-C link-arg=-fuse-ld=mold";
        SKIA_BINARIES_URL = "file://${skiaBinaries}";
      };
      commonArgs = {
        inherit src nativeBuildInputs buildInputs;
        inherit (env) CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER LIBCLANG_PATH RUSTFLAGS SKIA_BINARIES_URL;
        version = "0.8.1";
        strictDeps = true;
        doCheck = false;
      };
      mkPackage = {
        cargoExtraArgs,
        mainProgram,
        pname,
      }: let
        args = commonArgs // {inherit cargoExtraArgs pname;};
        cargoArtifacts = craneLib.buildDepsOnly args;
      in
        craneLib.buildPackage (args
          // {
            inherit cargoArtifacts;
            meta.mainProgram = mainProgram;
          });
      referenceRenderer = mkPackage {
        cargoExtraArgs = "-p op-reference-renderer";
        mainProgram = "op-reference-renderer";
        pname = "op-reference-renderer";
      };
      opCliRaster = mkPackage {
        cargoExtraArgs = "-p op-cli --features opui-raster";
        mainProgram = "op";
        pname = "op-cli-raster";
      };
    in {
      packages = {
        default = referenceRenderer;
        reference-renderer = referenceRenderer;
        op-cli-raster = opCliRaster;
      };

      checks = {
        inherit referenceRenderer opCliRaster;
        fmt = craneLib.cargoFmt {
          inherit src;
          pname = "openpencil";
          version = "0.8.1";
        };
      };

      devShells.default = craneLib.devShell {
        packages = nativeBuildInputs ++ buildInputs ++ [rustToolchain];
        inherit (env) CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER LIBCLANG_PATH RUSTFLAGS SKIA_BINARIES_URL;
        CARGO_TARGET_DIR = ".nix-target";
        shellHook = ''
          unset CARGO_HOME LIBRARY_PATH LD_LIBRARY_PATH CARGO_PROFILE_DEV_CODEGEN_BACKEND
        '';
      };
    });
}
