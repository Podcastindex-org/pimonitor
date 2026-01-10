{
  description = "Pimonitor development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        # Use the latest stable Rust toolchain
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
          ];
        };

        # Build-time dependencies (e.g., compilers, config tools)
        nativeBuildInputs = with pkgs; [
          pkg-config
        ];

        # Runtime/Link-time dependencies
        buildInputs =
          with pkgs;
          [
            # Add common dependencies here if any
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
            # Linux specific dependencies for rodio/cpal
            alsa-lib
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            # macOS specific frameworks for rodio/cpal and reqwest/security
            darwin.apple_sdk.frameworks.Security
            darwin.apple_sdk.frameworks.CoreFoundation
            darwin.apple_sdk.frameworks.CoreAudio
            darwin.apple_sdk.frameworks.AudioUnit
          ];

      in
      {
        devShells.default = pkgs.mkShell {
          inherit buildInputs nativeBuildInputs;

          packages = [
            rustToolchain
          ];

          # Set environment variables if needed
          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
        };

        # Package definition for building pimonitor
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "pimonitor";
          version = "0.1.0";

          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          inherit nativeBuildInputs buildInputs;

          meta = with pkgs.lib; {
            description = "Podcast Index monitor TUI application";
            homepage = "https://github.com/suorcd/pimonitor";
            license = licenses.mit;
            maintainers = [ ];
          };
        };

        # Apps for running pimonitor
        apps = {
          default = {
            type = "app";
            program = "${self.packages.${system}.default}/bin/pimonitor";
          };
          
          vim = {
            type = "app";
            program = pkgs.writeShellScript "pimonitor-vim" ''
              ${self.packages.${system}.default}/bin/pimonitor --vim
            '';
          };
        };
      }
    );
}
