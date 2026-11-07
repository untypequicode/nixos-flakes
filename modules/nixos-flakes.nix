# Module NixOS de nixos-flakes. `self` est le flake (voir flake.nix).
self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.nixos-flakes;
  system = pkgs.stdenv.hostPlatform.system;
  inherit (lib) mkOption mkEnableOption types;
in
{
  imports = [
    (lib.mkRenamedOptionModule
      [ "services" "nixos-auto-upgrade-custom" "enable" ]
      [ "services" "nixos-flakes" "autoUpgrade" "enable" ]
    )
    (lib.mkRenamedOptionModule
      [ "services" "nixos-auto-upgrade-custom" "interval" ]
      [ "services" "nixos-flakes" "autoUpgrade" "interval" ]
    )
  ];

  options.services.nixos-flakes = {
    package = mkOption {
      type = types.package;
      default = self.packages.${system}.nixos-flakes;
      defaultText = lib.literalExpression "nixos-flakes.packages.\${system}.nixos-flakes";
      description = "Paquet de la CLI `nixos-flakes`.";
    };

    guiPackage = mkOption {
      type = types.package;
      default = self.packages.${system}.nixos-flakes-gui;
      defaultText = lib.literalExpression "nixos-flakes.packages.\${system}.nixos-flakes-gui";
      description = "Paquet de l'interface graphique `nixos-flakes-gui`.";
    };

    autoUpgrade = {
      enable = mkEnableOption "la mise à niveau automatique du dépôt système";

      interval = mkOption {
        type = types.str;
        default = "daily";
        example = "04:00";
        description = "Fréquence (format `systemd.time`) de la mise à niveau automatique.";
      };

      flakePath = mkOption {
        type = types.str;
        default = "/etc/nixos";
        description = "Chemin du dépôt système (dépôt flake de la configuration NixOS).";
      };

      user = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "alice";
        description = ''
          Propriétaire du dépôt système. Si défini, le service reste lancé en root
          (nécessaire à l'application), mais `flake.lock` est rendu à cet utilisateur
          après chaque exécution et git accepte le dépôt malgré son propriétaire différent,
          pour que `flake.lock` ne reste pas la propriété de root.
          `null` : tout reste à root.
        '';
      };
    };

    systemRepo = {
      owner = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "alice";
        description = ''
          Utilisateur autorisé à écrire dans le dépôt système (`systemRepo.path`) sans
          être root, par exemple pour que la GUI lance `nix flake update` et valide
          `flake.lock`. Le dépôt reste la propriété de root : on accorde une ACL
          (lecture/écriture, héritée par les nouveaux fichiers) via systemd-tmpfiles,
          et git accepte ce dépôt (`safe.directory`, chemin exact, jamais `*`).
          Nécessite un système de fichiers avec ACL (ext4, btrfs, xfs…).
          `null` : aucune modification des droits.
        '';
      };

      path = mkOption {
        type = types.str;
        default = "/etc/nixos";
        description = "Chemin du dépôt système concerné par `systemRepo.owner`.";
      };
    };

    githubTokenFile = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "/run/secrets/github-token";
      description = ''
        Chemin d'un fichier contenant le jeton GitHub (jeton brut, `github.com=JETON` ou
        `access-tokens = github.com=JETON`), par exemple géré par sops-nix ou agenix.
        Exporté dans `NIXOS_FLAKES_GITHUB_TOKEN_FILE` (session, service de la zone de
        notification et mise à niveau automatique) : l'interface, la CLI, la zone de
        notification et la mise à niveau l'utilisent en lecture seule.
        Le type est `str` et non `path` pour qu'un chemin d'exécution ne soit pas copié dans
        le store : le contenu du fichier n'est jamais écrit dans la configuration Nix.
        Le fichier doit être lisible par chaque utilisateur concerné ; pour le service
        de mise à niveau (root), root doit pouvoir le lire.
        `null` : jeton géré depuis l'interface (`~/.config/nixos-flakes/github-token`).
        Remarque : `/etc/nix/access-tokens.conf` (ligne `extra-access-tokens = github.com=JETON`),
        souvent en root 600, n'est pas lisible par `nix` lancé en utilisateur : son jeton est alors
        ignoré (limite de 60 requêtes/h). nixos-flakes le détecte et l'utilise s'il est lisible ;
        pour le rendre lisible, par exemple
        `install -m 640 -g users /chemin/source /etc/nix/access-tokens.conf`, ou enregistrez
        un jeton dans l'interface, ou pointez cette option vers un fichier lisible.
      '';
    };

    gui = {
      enable = mkEnableOption "l'interface graphique nixos-flakes-gui";

      tray = mkEnableOption ''
        le lancement automatique de l'interface dans la zone de notification (`nixos-flakes-gui --tray`).
        Le service est lié à `graphical-session.target` : celui-ci n'est atteint que si la session
        le démarre (`niri-session`, ou tout lanceur de session qui active `graphical-session.target`
        et importe `WAYLAND_DISPLAY` dans l'environnement systemd utilisateur). Avec un simple
        `niri` lancé depuis un TTY ou un gestionnaire de connexion, le service ne démarre pas :
        lancez plutôt `nixos-flakes-gui --tray` via `spawn-at-startup` dans la configuration de niri
        '';
    };
  };

  config = lib.mkMerge [
    (lib.mkIf cfg.autoUpgrade.enable {
      environment.systemPackages = [ cfg.package ];

      systemd.services.nixos-flakes-upgrade = {
        description = "Mise à niveau automatique du système NixOS via flakes";
        after = [ "network-online.target" ];
        wants = [ "network-online.target" ];
        # Les binaires externes (git, nix, nixos-rebuild, notify-send) sont résolus via PATH.
        path = [
          pkgs.git
          pkgs.nix
          pkgs.nixos-rebuild
          pkgs.libnotify
        ];
        environment =
          lib.optionalAttrs (cfg.autoUpgrade.user != null) {
            GIT_CONFIG_COUNT = "1";
            GIT_CONFIG_KEY_0 = "safe.directory";
            GIT_CONFIG_VALUE_0 = "*";
          }
          // lib.optionalAttrs (cfg.githubTokenFile != null) {
            NIXOS_FLAKES_GITHUB_TOKEN_FILE = cfg.githubTokenFile;
          };
        serviceConfig = {
          Type = "oneshot";
          ExecStart = "${lib.getExe cfg.package} upgrade --notify ${lib.escapeShellArg cfg.autoUpgrade.flakePath}";
        }
        // lib.optionalAttrs (cfg.autoUpgrade.user != null) {
          # `+` : exécuté avec tous les privilèges, même après un échec de l'upgrade.
          ExecStopPost = "+${pkgs.coreutils}/bin/chown ${lib.escapeShellArg cfg.autoUpgrade.user} ${lib.escapeShellArg "${cfg.autoUpgrade.flakePath}/flake.lock"}";
        };
      };

      systemd.timers.nixos-flakes-upgrade = {
        description = "Minuteur de la mise à niveau automatique NixOS";
        wantedBy = [ "timers.target" ];
        timerConfig = {
          OnCalendar = cfg.autoUpgrade.interval;
          Persistent = true;
        };
      };
    })

    (lib.mkIf cfg.gui.enable {
      environment.systemPackages = [
        cfg.package
        cfg.guiPackage
      ];

      systemd.user.services.nixos-flakes-gui-tray = lib.mkIf cfg.gui.tray {
        description = "nixos-flakes dans la zone de notification";
        wantedBy = [ "graphical-session.target" ];
        partOf = [ "graphical-session.target" ];
        after = [ "graphical-session.target" ];
        environment = lib.optionalAttrs (cfg.githubTokenFile != null) {
          NIXOS_FLAKES_GITHUB_TOKEN_FILE = cfg.githubTokenFile;
        };
        serviceConfig = {
          ExecStart = "${lib.getExe' cfg.guiPackage "nixos-flakes-gui"} --tray";
          Restart = "on-failure";
          # Si WAYLAND_DISPLAY n'est pas encore importé, on retente un peu plus tard.
          RestartSec = 2;
        };
      };
    })

    (lib.mkIf (cfg.systemRepo.owner != null) {
      systemd.tmpfiles.rules = [
        # `A+` : ACL récursive (X = exécution sur les dossiers seulement) ; `d:` = ACL par défaut
        # héritée par les nouveaux fichiers.
        "A+ ${cfg.systemRepo.path} - - - - u:${cfg.systemRepo.owner}:rwX,d:u:${cfg.systemRepo.owner}:rwX"
      ];

      programs.git = {
        enable = true;
        config.safe.directory = [ cfg.systemRepo.path ];
      };
    })

    (lib.mkIf (cfg.githubTokenFile != null) {
      environment.sessionVariables.NIXOS_FLAKES_GITHUB_TOKEN_FILE = cfg.githubTokenFile;
    })

    {
      assertions = [
        {
          assertion = !cfg.gui.tray || cfg.gui.enable;
          message = "services.nixos-flakes.gui.tray nécessite services.nixos-flakes.gui.enable.";
        }
      ];
    }
  ];
}
