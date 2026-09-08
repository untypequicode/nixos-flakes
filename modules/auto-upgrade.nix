{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.nixos-auto-upgrade-custom;
  autoUpgradePkg = pkgs.callPackage ../pkgs/nixos-auto-upgrade { };
in
{
  options.services.nixos-auto-upgrade-custom = {
    enable = lib.mkEnableOption "Mise à jour automatique des flakes NixOS";

    interval = lib.mkOption {
      type = lib.types.str;
      default = "daily";
      description = "Fréquence d'exécution de la mise à jour automatique.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ autoUpgradePkg ];

    systemd.services.nixos-auto-upgrade-custom = {
      description = "Mise à jour automatique du système NixOS via Flakes";
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];

      serviceConfig = {
        Type = "oneshot";
        ExecStart = "${autoUpgradePkg}/bin/nixos-auto-upgrade";
      };
    };

    systemd.timers.nixos-auto-upgrade-custom = {
      description = "Timer pour la mise à jour automatique NixOS";
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnCalendar = cfg.interval;
        Persistent = true;
      };
    };
  };
}
