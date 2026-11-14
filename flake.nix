{
  description = "Reminders: Android reminder app with a Rust core";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { nixpkgs, ... }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-linux"
      ];

      forEachSystem = nixpkgs.lib.genAttrs systems;

      pkgsFor =
        system:
        import nixpkgs {
          inherit system;
          config = {
            allowUnfree = true;
            android_sdk.accept_license = true;
          };
        };
    in
    {
      # Filled in by later stages of NIX_PLAN.md: toolchain, per-ABI Rust
      # derivations, UniFFI bindings, APK derivations, and the dev shell.
      packages = forEachSystem (_system: { });
    };
}
