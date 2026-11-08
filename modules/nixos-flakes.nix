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

  guiExe = lib.getExe' cfg.guiPackage "nixos-flakes-gui";

  # Anciens `enable` (booléens) : la mise à niveau automatique s'active désormais
  # en donnant une valeur à `services.nixos-flakes.interval`.
  legacyEnablePaths = [
    [
      "services"
      "nixos-auto-upgrade-custom"
      "enable"
    ]
    [
      "services"
      "nixos-flakes"
      "autoUpgrade"
      "enable"
    ]
  ];
  legacyEnableSet = lib.filter (p: lib.getAttrFromPath p config != null) legacyEnablePaths;
  legacyEnabled = lib.any (p: lib.getAttrFromPath p config == true) legacyEnablePaths;

  renamed =
    from: to:
    lib.mkRenamedOptionModule from [
      "services"
      "nixos-flakes"
      to
    ];
in
{
  imports = [
    (renamed [ "services" "nixos-auto-upgrade-custom" "interval" ] "interval")
    (renamed [ "services" "nixos-flakes" "autoUpgrade" "interval" ] "interval")
    (renamed [ "services" "nixos-flakes" "autoUpgrade" "flakePath" ] "flakePath")
    (renamed [ "services" "nixos-flakes" "autoUpgrade" "user" ] "user")

    # Anciens `enable` : acceptés avec un avertissement. `true` sans `interval` garde l'ancien
    # comportement (mise à niveau quotidienne).
    {
      options = lib.foldl' lib.recursiveUpdate { } (
        map (
          p:
          lib.setAttrByPath p (mkOption {
            type = types.nullOr types.bool;
            default = null;
            visible = false;
            description = "Obsolète : voir `services.nixos-flakes.interval`.";
          })
        ) legacyEnablePaths
      );
      config = {
        warnings = map (
          p:
          "L'option `${lib.showOption p}` est obsolète : la mise à niveau automatique s'active en "
          + "définissant `services.nixos-flakes.interval` (par exemple \"daily\"). Par compatibilité, "
          + "`true` sans `interval` équivaut à `interval = \"daily\"`."
        ) legacyEnableSet;
        services.nixos-flakes.interval = lib.mkIf legacyEnabled (lib.mkDefault "daily");
      };
    }
  ];

  options.services.nixos-flakes = {
    enable = mkEnableOption ''
      le démon nixos-flakes : processus de fond de la session qui héberge l'icône d'état dans la
      zone de notification et surveille les mises à jour. Le service utilisateur est lié à
      `graphical-session.target` : celui-ci n'est atteint que si la session le démarre
      (`niri-session`, ou tout lanceur de session qui active `graphical-session.target` et importe
      `WAYLAND_DISPLAY` dans l'environnement systemd utilisateur)'';

    package = mkOption {
      type = types.package;
      default = self.packages.${system}.nixos-flakes;
      defaultText = lib.literalExpression "nixos-flakes.packages.\${system}.nixos-flakes";
      description = "Paquet de la CLI `nixos-flakes` (qui contient le démon).";
    };

    guiPackage = mkOption {
      type = types.package;
      default = self.packages.${system}.nixos-flakes-gui;
      defaultText = lib.literalExpression "nixos-flakes.packages.\${system}.nixos-flakes-gui";
      description = "Paquet de l'interface graphique `nixos-flakes-gui`.";
    };

    interval = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "04:00";
      description = ''
        Fréquence (format `systemd.time`) de la mise à niveau automatique du dépôt système
        (service root + minuteur). `null` : pas de mise à niveau automatique.
      '';
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
        Utilisateur de la machine. Le démon (service utilisateur) n'est lancé que pour lui.
        La mise à niveau automatique reste lancée en root (nécessaire à l'application), mais
        `flake.lock` est rendu à cet utilisateur après chaque exécution et git accepte le dépôt
        malgré son propriétaire différent, pour que `flake.lock` ne reste pas la propriété de root.
        `null` : le démon peut être lancé pour tout utilisateur, et tout reste à root.
      '';
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
        default = cfg.flakePath;
        defaultText = lib.literalExpression "config.services.nixos-flakes.flakePath";
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
        Exporté dans `NIXOS_FLAKES_GITHUB_TOKEN_FILE` (session, démon de session
        et mise à niveau automatique) : l'interface, la CLI, le démon
        et la mise à niveau l'utilisent en lecture seule.
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
      enable = mkEnableOption ''
        l'interface graphique nixos-flakes-gui (nécessite `services.nixos-flakes.enable` : elle
        se connecte au démon). L'entrée « Ouvrir l'interface » de l'icône d'état la lance'';
    };
  };

  config = lib.mkMerge [
    (lib.mkIf cfg.enable {
      environment.systemPackages = [ cfg.package ];

      systemd.user.services.nixos-flakes-daemon = {
        description = "Démon nixos-flakes (icône d'état et vérification des mises à jour)";
        wantedBy = [ "graphical-session.target" ];
        partOf = [ "graphical-session.target" ];
        after = [ "graphical-session.target" ];
        unitConfig = lib.optionalAttrs (cfg.user != null) {
          # Réservé à l'utilisateur configuré ; les autres sessions ne le lancent pas.
          ConditionUser = cfg.user;
        };
        # Binaires externes (git, nix, notify-send) résolus via PATH.
        path = [
          pkgs.git
          pkgs.nix
          pkgs.libnotify
        ];
        environment = lib.optionalAttrs (cfg.githubTokenFile != null) {
          NIXOS_FLAKES_GITHUB_TOKEN_FILE = cfg.githubTokenFile;
        };
        serviceConfig = {
          ExecStart = lib.concatStringsSep " " (
            [
              (lib.getExe cfg.package)
              "daemon"
            ]
            ++ lib.optionals cfg.gui.enable [
              "--gui-command"
              (lib.escapeShellArg guiExe)
            ]
          );
          Restart = "on-failure";
          # Si WAYLAND_DISPLAY ou le bus ne sont pas encore prêts, on retente un peu plus tard.
          RestartSec = 2;
        };
      };
    })

    (lib.mkIf cfg.gui.enable {
      environment.systemPackages = [
        cfg.package
        cfg.guiPackage
      ];
    })

    (lib.mkIf (cfg.interval != null) {
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
          lib.optionalAttrs (cfg.user != null) {
            GIT_CONFIG_COUNT = "1";
            GIT_CONFIG_KEY_0 = "safe.directory";
            GIT_CONFIG_VALUE_0 = cfg.flakePath;
          }
          // lib.optionalAttrs (cfg.githubTokenFile != null) {
            NIXOS_FLAKES_GITHUB_TOKEN_FILE = cfg.githubTokenFile;
          };
        serviceConfig = {
          Type = "oneshot";
          ExecStart = "${lib.getExe cfg.package} upgrade --notify ${lib.escapeShellArg cfg.flakePath}";
          # /var/lib/nixos-flakes/status.json : état de la dernière exécution, lu par le démon
          # de session (d'où le dossier lisible par tous).
          StateDirectory = "nixos-flakes";
          StateDirectoryMode = "0755";
        }
        // lib.optionalAttrs (cfg.user != null) {
          # `+` : exécuté avec tous les privilèges, même après un échec de l'upgrade.
          ExecStopPost = "-+${pkgs.coreutils}/bin/chown ${lib.escapeShellArg cfg.user} ${lib.escapeShellArg "${cfg.flakePath}/flake.lock"}";
        };
      };

      systemd.timers.nixos-flakes-upgrade = {
        description = "Minuteur de la mise à niveau automatique NixOS";
        wantedBy = [ "timers.target" ];
        timerConfig = {
          OnCalendar = cfg.interval;
          Persistent = true;
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
          assertion = !cfg.gui.enable || cfg.enable;
          message = "services.nixos-flakes.gui.enable nécessite services.nixos-flakes.enable (l'interface se connecte au démon).";
        }
      ];
    }
  ];
}
