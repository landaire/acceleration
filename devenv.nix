{ pkgs, ... }:

let
  # Cargo, for reindeer and build_defs/update_buck_files.py, and
  # rust-analyzer. Buck2 downloads the same release itself. The rust-overlay
  # toolchain propagates a Nix C compiler for linking; only its files are
  # taken, so the host's compiler stays the one on PATH.
  rustToolchain = pkgs.buildEnv {
    name = "rust-toolchain";
    paths = [ (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml) ];
    pathsToLink = [ "/bin" "/etc" "/lib" "/libexec" "/share" ];
  };
in
{
  # Buck2 links with the host's C compiler driver (cc, c++ and ar on Linux,
  # Xcode's clang on macOS), so the shell provides none: the binaries then
  # depend on the host's C library rather than on Nix store paths.
  stdenv = pkgs.stdenvNoCC;

  packages = [
    rustToolchain
    # One of its tests fails on darwin.
    (pkgs.reindeer.overrideAttrs { doCheck = false; })
    pkgs.python3
  ];

  # bin/buck2 runs the Buck2 release that toolchains/buck2 pins, through the
  # DotSlash that bootstrap.sh installs, as outside the shell. DotSlash and
  # the Linux Buck2 builds are static, so they run on NixOS.
  enterShell = ''
    export PATH="$DEVENV_ROOT/bin:$PATH"
  '';

  # bazel-remote serves the CI action cache that ci/start-action-cache.sh
  # starts.
  profiles.ci.module.packages = [ pkgs.bazel-remote ];
}
