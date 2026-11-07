{
  description = "Recueil d'outils Nix/NixOS de untypequicode";

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

      # { packages, checks, tools, guiLibs } par système
      buildFor = forAllSystems (system: import ./nix { pkgs = nixpkgsFor.${system}; });
    in
    {
      packages = forAllSystems (system: buildFor.${system}.packages);

      apps = forAllSystems (
        system:
        builtins.mapAttrs (_: pkg: {
          type = "app";
          program = nixpkgs.lib.getExe pkg;
          meta = pkg.meta or { };
        }) buildFor.${system}.tools
      );

      checks = forAllSystems (system: buildFor.${system}.checks);

      formatter = forAllSystems (system: nixpkgsFor.${system}.nixfmt);

      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgsFor.${system};
          guiLibs = buildFor.${system}.guiLibs;
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              rustc
              clippy
              rustfmt
              rust-analyzer
              pkg-config
              git
              nix
              libnotify
              nixfmt
            ];
            buildInputs = guiLibs;
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath guiLibs;
          };
        }
      );

      overlays.default =
        final: _prev:
        (import ./nix {
          pkgs = final;
          root = ./.;
        }).tools;

      nixosModules = {
        nixos-flakes = import ./modules/nixos-flakes.nix self;
        default = self.nixosModules.nixos-flakes;
      };
    };
}
