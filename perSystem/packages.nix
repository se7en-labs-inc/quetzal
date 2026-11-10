{inputs, ...}: {
  perSystem = {
    inputs',
    system,
    config,
    lib,
    pkgs,
    ...
  }: let
    toolchain = with inputs'.fenix.packages;
      combine [
        stable.rustc
        stable.cargo
        stable.clippy
        stable.rustfmt
      ];

    craneLib = (inputs.crane.mkLib pkgs).overrideToolchain toolchain;

    src = lib.fileset.toSource {
      root = ./..;
      fileset = lib.fileset.unions [
        ../Cargo.lock
        ../Cargo.toml
        ../crates
      ];
    };

    commonArgs = {
      inherit src;
      pname = "quetzal";
      version = "0.1.0";
      strictDeps = true;

      nativeBuildInputs = with pkgs; [
        pkg-config
        openssl
      ];

      buildInputs = with pkgs; [
        openssl
      ];

      meta = {
        mainProgram = "quetzal";
        maintainers = [];
        license = with lib.licenses; [asl20];
      };
    };

    # Build workspace dependencies separately for better caching
    cargoArtifacts = craneLib.buildDepsOnly commonArgs;
  in {
    checks = {
      clippy = craneLib.cargoClippy (commonArgs
        // {
          inherit cargoArtifacts;
          cargoClippyExtraArgs = "--all-targets -- --deny warnings";
        });

      fmt = craneLib.cargoFmt {inherit src;};
    };

    packages = {
      default = config.packages.quetzal;

      quetzal = craneLib.buildPackage (commonArgs
        // {
          inherit cargoArtifacts;
          doCheck = true;
        });
    };
  };
}
