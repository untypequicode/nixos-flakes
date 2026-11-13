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

  # Chemin absolu en `str` (et non `path`) : un chemin d'exécution ne doit pas être copié dans le store.
  absPath = types.strMatching "/.*";

  hasUser = name: config.users.users ? ${name};

  guiExe = lib.getExe' cfg.guiPackage "nixos-flakes-gui";

  # Le dépôt système est dans le dossier personnel de son propriétaire déclaré : il lui appartient
  # déjà, inutile d'y poser une ACL.
  ownerHome =
    if cfg.systemRepo.owner != null && hasUser cfg.systemRepo.owner then
      config.users.users.${cfg.systemRepo.owner}.home
    else
      null;
  repoOwnedByOwner =
    ownerHome != null
    && ownerHome != ""
    && lib.hasPrefix "${lib.removeSuffix "/" ownerHome}/" cfg.systemRepo.path;

  # Défauts système lus par la CLI, le démon et l'interface (mêmes clés que le config.toml utilisateur).
  systemDefaults = ''
    system_repo = ${builtins.toJSON cfg.flakePath}
  ''
  + lib.optionalString (cfg.flakeAttr != null) ''
    system_attr = ${builtins.toJSON cfg.flakeAttr}
  '';

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
      type = absPath;
      default = "/etc/nixos";
      example = "/home/alice/nixos-config";
      description = ''
        Chemin absolu du dépôt système (dépôt flake de la configuration NixOS). Il peut se
        trouver n'importe où, par exemple dans le dossier personnel de `user` ou sous `/srv`.
        Il est écrit dans `/etc/nixos-flakes/config.toml`, lu par la CLI, le démon et l'interface
        comme défaut système (une valeur choisie par l'utilisateur dans sa configuration prime).
        Si le flake est dans un sous-dossier d'un dépôt git qui appartient à un autre
        utilisateur que root, ajoutez aussi la racine du dépôt à `programs.git.config.safe.directory`.
      '';
    };

    flakeAttr = mkOption {
      type = types.nullOr (types.strMatching "[A-Za-z0-9_][A-Za-z0-9._-]*");
      default = null;
      example = "laptop";
      description = ''
        Nom de la configuration `nixosConfigurations.<nom>` à appliquer. `null` : le nom
        d'hôte de la machine (comportement de `nixos-rebuild`). Transmis à la mise à niveau
        automatique (`--attr`) et écrit dans `/etc/nixos-flakes/config.toml`.
      '';
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
          `flake.lock`. Quand le dépôt appartient à root (par exemple `/etc/nixos`), on accorde une
          ACL (lecture/écriture, héritée par les nouveaux fichiers) via systemd-tmpfiles,
          et git accepte ce dépôt (`safe.directory`, chemin exact, jamais `*`).
          Nécessite un système de fichiers avec ACL (ext4, btrfs, xfs…).
          Quand `systemRepo.path` est dans le dossier personnel de cet utilisateur, il en est
          déjà propriétaire : aucune ACL n'est posée.
          `null` : aucune modification des droits.
        '';
      };

      path = mkOption {
        type = absPath;
        default = cfg.flakePath;
        defaultText = lib.literalExpression "config.services.nixos-flakes.flakePath";
        example = "/home/alice/nixos-config";
        description = "Chemin absolu du dépôt système concerné par `systemRepo.owner`.";
      };
    };

    githubTokenFile = mkOption {
      type = types.nullOr absPath;
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
    (lib.mkIf (cfg.enable || cfg.gui.enable || cfg.interval != null) {
      # Dépôt système par défaut de la machine : l'emplacement n'est pas figé à /etc/nixos.
      environment.etc."nixos-flakes/config.toml".text = systemDefaults;
    })

    (lib.mkIf cfg.enable {
      environment.systemPackages = [ cfg.package ];

      systemd.user.services.nixos-flakes-daemon = {
        description = "Démon nixos-flakes (icône d'état et vérification des mises à jour)";
        wantedBy = [ "graphical-session.target" ];
        partOf = [ "graphical-session.target" ];
        after = [ "graphical-session.target" ];
        unitConfig = {
          # Au plus 5 démarrages en 60 s : évite une boucle de redémarrage serrée.
          StartLimitIntervalSec = 60;
          StartLimitBurst = 5;
        }
        // lib.optionalAttrs (cfg.user != null) {
          # Réservé à l'utilisateur configuré ; les autres sessions ne le lancent pas.
          ConditionUser = cfg.user;
        };
        # Binaires externes (git, nix, notify-send) résolus via PATH.
        path = [
          pkgs.git
          config.nix.package
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
          TimeoutStopSec = 10;
          # Ni NoNewPrivileges ni filtre seccomp : le démon lance pkexec et la GUI.
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
        # Ne jamais redémarrer le service en plein `switch` (il se tuerait lui-même).
        restartIfChanged = false;
        # Les binaires externes (git, nix, nixos-rebuild, notify-send) sont résolus via PATH.
        path = [
          pkgs.git
          config.nix.package
          config.system.build.nixos-rebuild
          pkgs.libnotify
        ];
        environment = {
          # root lit un dépôt qui peut appartenir à l'utilisateur : git l'accepte (chemin exact).
          GIT_CONFIG_COUNT = "1";
          GIT_CONFIG_KEY_0 = "safe.directory";
          GIT_CONFIG_VALUE_0 = cfg.flakePath;
        }
        // lib.optionalAttrs (cfg.githubTokenFile != null) {
          NIXOS_FLAKES_GITHUB_TOKEN_FILE = cfg.githubTokenFile;
        };
        serviceConfig = {
          Type = "oneshot";
          # Reconstruction longue possible, mais pas indéfinie.
          TimeoutStartSec = "6h";
          # Travail de fond. Aucun bac à sable : le service doit pouvoir tout faire.
          Nice = 10;
          IOSchedulingClass = "idle";
          ExecStart =
            "${lib.getExe cfg.package} upgrade --notify"
            + lib.optionalString (cfg.flakeAttr != null) " --attr ${lib.escapeShellArg cfg.flakeAttr}"
            + " ${lib.escapeShellArg cfg.flakePath}";
          # /var/lib/nixos-flakes/status.json : état de la dernière exécution, lu par le démon
          # de session (d'où le dossier lisible par tous).
          StateDirectory = "nixos-flakes";
          StateDirectoryMode = "0755";
        }
        // lib.optionalAttrs (cfg.user != null) {
          # `+` : exécuté avec tous les privilèges, même après un échec ou un kill de l'upgrade
          # (le garde de propriétaire de nixos-flakes ne tourne alors pas). `flake.lock` est dans un
          # dépôt que l'utilisateur contrôle : `find` (sans suivre de lien) ne retient que le
          # fichier régulier, et `chown -h` ne suit jamais un lien. Un lien symbolique est ignoré.
          ExecStopPost = "-+${pkgs.findutils}/bin/find ${lib.escapeShellArg "${cfg.flakePath}/flake.lock"} -maxdepth 0 -type f -exec ${pkgs.coreutils}/bin/chown -h ${lib.escapeShellArg cfg.user} {} +";
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

    (lib.mkIf (cfg.systemRepo.owner != null && !repoOwnedByOwner) {
      # Chemin absent (pas encore créé) : systemd-tmpfiles ignore silencieusement la ligne.
      systemd.tmpfiles.rules = [
        # `A+` : ACL récursive (X = exécution sur les dossiers seulement) ; `d:` = ACL par défaut
        # héritée par les nouveaux fichiers.
        "A+ ${cfg.systemRepo.path} - - - - u:${cfg.systemRepo.owner}:rwX,d:u:${cfg.systemRepo.owner}:rwX"
      ];
    })

    # root (service de mise à niveau) et les outils lisent un dépôt qui n'est pas à lui : git et
    # nix (libgit2, qui lit /etc/gitconfig) doivent l'accepter malgré le propriétaire différent.
    (lib.mkIf (cfg.systemRepo.owner != null || cfg.user != null) {
      programs.git = {
        enable = true;
        config.safe.directory = lib.unique (
          [ cfg.flakePath ] ++ lib.optional (cfg.systemRepo.owner != null) cfg.systemRepo.path
        );
      };
    })

    (lib.mkIf (cfg.githubTokenFile != null) {
      environment.sessionVariables.NIXOS_FLAKES_GITHUB_TOKEN_FILE = cfg.githubTokenFile;
    })

    {
      assertions = [
        {
          assertion = cfg.user == null || hasUser cfg.user;
          message = "services.nixos-flakes.user = \"${toString cfg.user}\" n'existe pas dans users.users.";
        }
        {
          assertion = cfg.systemRepo.owner == null || hasUser cfg.systemRepo.owner;
          message = "services.nixos-flakes.systemRepo.owner = \"${toString cfg.systemRepo.owner}\" n'existe pas dans users.users.";
        }
        {
          assertion =
            cfg.systemRepo.owner == null
            || repoOwnedByOwner
            || builtins.match "[^[:space:],:]+" cfg.systemRepo.path != null;
          message = "services.nixos-flakes.systemRepo.path ne doit contenir ni espace, ni `:`, ni `,` quand une ACL est posée (règle systemd-tmpfiles/ACL ; hors du dossier personnel de systemRepo.owner).";
        }
        {
          assertion = !cfg.gui.enable || cfg.enable;
          message = "services.nixos-flakes.gui.enable nécessite services.nixos-flakes.enable (l'interface se connecte au démon).";
        }
      ];
    }
  ];
}
