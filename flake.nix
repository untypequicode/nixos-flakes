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

      # true si le .xpi signé par Mozilla est commité (voir README.md, section
      # Extension Firefox/Zen). Sinon le paquet "-signed" n'est pas exposé.
      hasSignedXpi = builtins.pathExists (
        ./pkgs/firefox-image-downloader-visible-signed/image-downloader-visible-signed.xpi
      );
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgsFor.${system};
        in
        buildFor.${system}.packages
        // {
          firefox-image-downloader-visible = pkgs.callPackage ./pkgs/firefox-image-downloader-visible { };
        }
        // nixpkgs.lib.optionalAttrs hasSignedXpi {
          firefox-image-downloader-visible-signed =
            pkgs.callPackage ./pkgs/firefox-image-downloader-visible-signed
              { };
        }
      );

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

      # Fonction partagée : construit l'entrée `ExtensionSettings` (schéma des
      # policies entreprise Firefox/Gecko) pour l'extension Image Downloader
      # Visible. Réutilisable telle quelle pour :
      #   - programs.firefox.policies.ExtensionSettings (voir nixosModules.image-downloader-visible)
      #   - l'override extraPolicies de youwen5/zen-browser-flake (voir README.md)
      lib = {
        mkImageDownloaderExtensionSettings =
          {
            system,
            useSigned ? true,
          }:
          let
            xpi =
              if useSigned then
                self.packages.${system}.firefox-image-downloader-visible-signed
              else
                self.packages.${system}.firefox-image-downloader-visible;
          in
          {
            "image-downloader-visible@untypequicode.dev" = {
              installation_mode = "force_installed";
              install_url = "file://${xpi}/image-downloader-visible.xpi";
            };
          };
      };

      overlays.default =
        final: _prev:
        (import ./nix {
          pkgs = final;
          root = ./.;
        }).tools;

      nixosModules = {
        nixos-flakes = import ./modules/nixos-flakes.nix self;
        image-downloader-visible = import ./modules/browser-extensions.nix self;
        default = self.nixosModules.nixos-flakes;
      };
    };
}
