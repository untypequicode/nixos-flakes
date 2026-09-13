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

      # true si vous avez committé le .xpi signé par Mozilla (voir README.md,
      # section "Signer l'extension"). Tant que ce fichier n'existe pas, le
      # paquet "-signed" n'est simplement pas exposé.
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
        {
          hello = pkgs.callPackage ./pkgs/hello { };
          nixos-check-flake-update = pkgs.callPackage ./pkgs/nixos-check-flake-update { };
          nixos-flake-update = pkgs.callPackage ./pkgs/nixos-flake-update { };
          nixos-auto-upgrade = pkgs.callPackage ./pkgs/nixos-auto-upgrade { };

          firefox-image-downloader-visible = pkgs.callPackage ./pkgs/firefox-image-downloader-visible { };
        }
        // nixpkgs.lib.optionalAttrs hasSignedXpi {
          firefox-image-downloader-visible-signed =
            pkgs.callPackage ./pkgs/firefox-image-downloader-visible-signed { };
        }
        // {
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

      nixosModules.auto-upgrade = import ./modules/auto-upgrade.nix;
      nixosModules.image-downloader-visible = import ./modules/browser-extensions.nix self;
      nixosModules.default = self.nixosModules.auto-upgrade;
    };
}
