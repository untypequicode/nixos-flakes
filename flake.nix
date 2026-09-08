{
  description = "Utilitaires NixOS de untypequicode";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;

      nixpkgsFor = forAllSystems (system: import nixpkgs { inherit system; });
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgsFor.${system};
        in
        {
          hello = pkgs.callPackage ./pkgs/hello { };
          nixos-check-flake-update = pkgs.callPackage ./pkgs/nixos-check-flake-update { };
          nixos-flake-update = pkgs.callPackage ./pkgs/nixos-flake-update { };
          nixos-auto-upgrade = pkgs.callPackage ./pkgs/nixos-auto-upgrade { };

          default = self.packages.${system}.hello;
        }
      );

      apps = forAllSystems (system: {
        nixos-check-flake-update = {
          type = "app";
          program = "${self.packages.${system}.nixos-check-flake-update}/bin/nixos-check-flake-update";
        };
        nixos-flake-update = {
          type = "app";
          program = "${self.packages.${system}.nixos-flake-update}/bin/nixos-flake-update";
        };
        nixos-auto-upgrade = {
          type = "app";
          program = "${self.packages.${system}.nixos-auto-upgrade}/bin/nixos-auto-upgrade";
        };
        hello = {
          type = "app";
          program = "${self.packages.${system}.hello}/bin/hello";
        };
      });

      nixosModules.auto-upgrade = import ./modules/auto-upgrade.nix;
      nixosModules.default = self.nixosModules.auto-upgrade;
    };
}
