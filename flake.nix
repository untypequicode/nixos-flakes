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

      extensionsFor = forAllSystems (
        system:
        import ./nix/extensions.nix {
          pkgs = nixpkgsFor.${system};
          root = ./.;
        }
      );
    in
    {
      packages = forAllSystems (system: buildFor.${system}.packages // extensionsFor.${system}.packages);

      apps = forAllSystems (
        system:
        let
          tools = builtins.mapAttrs (_: pkg: {
            type = "app";
            program = nixpkgs.lib.getExe pkg;
            meta = pkg.meta or { };
          }) buildFor.${system}.tools;
        in
        tools // { default = tools.nixos-flakes; }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = nixpkgsFor.${system};
          nixFiles = nixpkgs.lib.fileset.toSource {
            root = ./.;
            fileset = nixpkgs.lib.fileset.fileFilter (f: f.hasExt "nix") ./.;
          };
        in
        buildFor.${system}.checks
        // extensionsFor.${system}.checks
        // {
          # Formatage de tous les .nix (même formateur que `nix fmt`).
          nixfmt =
            pkgs.runCommand "nixos-flakes-nixfmt"
              {
                nativeBuildInputs = [
                  pkgs.nixfmt
                  pkgs.findutils
                ];
              }
              ''
                find ${nixFiles} -name '*.nix' -print0 | xargs -0 nixfmt --check
                touch $out
              '';

          # Évaluation (sans construction) d'un système NixOS minimal important les modules.
          nixos-module-eval =
            let
              inherit (nixpkgs) lib;
              mkEval =
                nixosFlakes:
                nixpkgs.lib.nixosSystem {
                  inherit system;
                  modules = [
                    self.nixosModules.default
                    self.nixosModules.image-downloader-visible
                    {
                      system.stateVersion = "25.11";
                      boot.loader.grub.enable = false;
                      fileSystems."/" = {
                        device = "/dev/null";
                        fsType = "ext4";
                      };
                      users.users.alice.isNormalUser = true;
                      programs.firefox.enable = true;
                      programs.image-downloader-visible.enable = true;
                      services.nixos-flakes = {
                        enable = true;
                        gui.enable = true;
                        interval = "daily";
                        user = "alice";
                      }
                      // nixosFlakes;
                    }
                  ];
                };

              # Défaut : /etc/nixos, dépôt à root avec ACL pour alice.
              defaults = mkEval { systemRepo.owner = "alice"; };
              # Dépôt personnel (chemin avec espace) et configuration nommée : pas d'ACL.
              home = mkEval {
                flakePath = "/home/alice/ma config";
                flakeAttr = "laptop";
                systemRepo.owner = "alice";
              };
              # Dépôt hors /etc et hors du dossier personnel : ACL sur ce chemin.
              srv = mkEval {
                flakePath = "/srv/nixos";
                systemRepo.owner = "alice";
              };

              etc = e: e.config.environment.etc."nixos-flakes/config.toml".text;
              unit = e: e.config.systemd.services.nixos-flakes-upgrade.serviceConfig;
              rules = e: e.config.systemd.tmpfiles.rules;
              has = lib.any (lib.hasInfix "A+ /srv/nixos");
              checked =
                assert lib.assertMsg (lib.hasInfix ''system_repo = "/etc/nixos"'' (
                  etc defaults
                )) "défaut /etc/nixos";
                assert lib.assertMsg (!lib.hasInfix "system_attr" (etc defaults)) "pas d'attribut par défaut";
                assert lib.assertMsg (lib.hasInfix ''system_repo = "/home/alice/ma config"'' (
                  etc home
                )) "flakePath personnalisé";
                assert lib.assertMsg (lib.hasInfix ''system_attr = "laptop"'' (etc home)) "flakeAttr écrit";
                assert lib.assertMsg (lib.hasInfix "--attr laptop" (unit home)
                .ExecStart) "flakeAttr passé au service";
                assert lib.assertMsg (lib.hasInfix "'/home/alice/ma config'" (unit home)
                .ExecStart) "chemin avec espace échappé";
                assert lib.assertMsg (
                  !lib.any (lib.hasInfix "A+ /home/alice") (rules home)
                ) "pas d'ACL dans le dossier personnel";
                assert lib.assertMsg (lib.any (lib.hasInfix "A+ /etc/nixos") (rules defaults)) "ACL sur /etc/nixos";
                assert lib.assertMsg (has (rules srv)) "ACL sur /srv/nixos";
                [
                  defaults
                  home
                  srv
                ];
            in
            pkgs.runCommand "nixos-flakes-module-eval" { } ''
              ${lib.concatMapStringsSep "\n" (
                e: "echo ${builtins.unsafeDiscardStringContext e.config.system.build.toplevel.drvPath} >> $out"
              ) checked}
            '';
        }
      );

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
        }).tools
        // (import ./nix/extensions.nix {
          pkgs = final;
          root = ./.;
        }).packages;

      nixosModules = {
        default = self.nixosModules.nixos-flakes;
        nixos-flakes = import ./modules/nixos-flakes.nix self;
        image-downloader-visible = import ./modules/browser-extensions.nix self;
      };
    };
}
