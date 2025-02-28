{ pkgs, lib, ... }:

{

  packages = [
    pkgs.protobuf3_21
    pkgs.just
    pkgs.graphviz
    pkgs.jq
  ] ++ lib.optionals pkgs.stdenv.isDarwin (with pkgs.darwin.apple_sdk; [
    frameworks.CoreServices
    frameworks.CoreFoundation
    frameworks.Security
    frameworks.SystemConfiguration
  ]);


  # https://devenv.sh/languages/
  languages.python = {
    enable = true;
    version = "3.10";
    uv.enable = true;
  };

  languages.rust = {
    channel = "stable";
    enable = true;
    components = [ "rustc" "cargo" "clippy" "rustfmt" "rust-analyzer" ];
  };

  # This allows building the type-check (pyo3) module on MacOSX "Apple Silicon"
  enterShell =
    if pkgs.stdenv.isDarwin && pkgs.stdenv.isAarch64 then ''
      export RUSTFLAGS="$RUSTFLAGS -C link-arg=-undefined -C link-arg=dynamic_lookup"
    '' else '''';
}
