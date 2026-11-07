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

    gui = {
      enable = mkEnableOption "l'interface graphique nixos-flakes-gui";

      tray = mkEnableOption "le lancement automatique de l'interface dans la zone de notification (`nixos-flakes-gui --tray`)";
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
        environment = lib.mkIf (cfg.autoUpgrade.user != null) {
          GIT_CONFIG_COUNT = "1";
          GIT_CONFIG_KEY_0 = "safe.directory";
          GIT_CONFIG_VALUE_0 = "*";
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
        serviceConfig = {
          ExecStart = "${lib.getExe' cfg.guiPackage "nixos-flakes-gui"} --tray";
          Restart = "on-failure";
        };
      };
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
