{
  description = "Command-line tools for Xbox 360 file formats";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
    }:
    let
      inherit (nixpkgs) lib;

      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];

      # Each command-line tool: the Cargo package and its executable.
      tools = {
        acceleration = {
          cargoPackage = "acceleration_cli";
          description = "Inspect and extract Xbox 360 STFS packages";
        };
        xex2ool = {
          cargoPackage = "xex2ool";
          description = "Inspect and modify Xbox 360 XEX2 executables";
        };
        xflash = {
          cargoPackage = "xflash";
          description = "Inspect and extract Xbox 360 devkit flash images";
        };
        shadowboot = {
          cargoPackage = "shadowboot";
          description = "Compile Xbox 360 devkit shadowboot images";
        };
      };

      version = (lib.importTOML ./Cargo.toml).workspace.package.version;
      rustChannel = (lib.importTOML ./rust-toolchain.toml).toolchain.channel;

      # Cargo resolves the whole workspace, so every member manifest is
      # needed; the Buck2 files and the third-party rules are not.
      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./crates
          ./third-party/rust/Cargo.toml
          ./third-party/rust/lib.rs
        ];
      };

      packagesFor =
        pkgs:
        let
          rust = pkgs.rust-bin.stable.${rustChannel}.minimal;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rust;
            rustc = rust;
          };

          build =
            {
              pname,
              names,
              description,
              mainProgram ? null,
            }:
            rustPlatform.buildRustPackage {
              inherit pname version src;
              cargoLock.lockFile = ./Cargo.lock;
              cargoBuildFlags = lib.concatMap (name: [
                "-p"
                tools.${name}.cargoPackage
              ]) names;
              # CI runs the tests with Buck2.
              doCheck = false;
              meta = {
                inherit description;
                homepage = "https://github.com/landaire/acceleration";
                license = [
                  lib.licenses.mit
                  lib.licenses.asl20
                ];
                platforms = systems;
              }
              // lib.optionalAttrs (mainProgram != null) { inherit mainProgram; };
            };

          perTool = lib.mapAttrs (
            name: tool:
            build {
              pname = name;
              names = [ name ];
              inherit (tool) description;
              mainProgram = name;
            }
          ) tools;
        in
        perTool
        // {
          default = build {
            pname = "acceleration-tools";
            names = lib.attrNames tools;
            description = "Command-line tools for Xbox 360 file formats";
          };
        };

      forAllSystems =
        f:
        lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ rust-overlay.overlays.default ];
            }
          )
        );
    in
    {
      packages = forAllSystems packagesFor;
    };
}
