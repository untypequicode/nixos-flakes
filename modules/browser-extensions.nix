self:

# `self` doit être LE self de *ce* flake (nixos-flakes). C'est pourquoi ce
# fichier est appliqué partiellement dans flake.nix :
#   nixosModules.image-downloader-visible = import ./modules/browser-extensions.nix self;
# plutôt que de compter sur specialArgs côté utilisateur.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.image-downloader-visible;
  system = pkgs.stdenv.hostPlatform.system;

  hasSignedXpi = builtins.pathExists (
    self + "/pkgs/firefox-image-downloader-visible-signed/image-downloader-visible-signed.xpi"
  );
in
{
  options.programs.image-downloader-visible = {
    enable = lib.mkEnableOption "Force-installer l'extension Image Downloader Visible dans Firefox";

    useSigned = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        true (par défaut) : utilise le .xpi signé par Mozilla — nécessite
        d'avoir committé pkgs/firefox-image-downloader-visible-signed/image-downloader-visible-signed.xpi
        (voir README.md, section "Signer l'extension").
        false : utilise le .xpi non signé construit par Nix — ne fonctionne
        que sur Firefox ESR/Nightly/Developer Edition avec la vérification de
        signature désactivée (voir README.md, section "Mode dev / ESR").
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = !cfg.useSigned || hasSignedXpi;
        message = ''
          programs.image-downloader-visible.useSigned = true, mais
          pkgs/firefox-image-downloader-visible-signed/image-downloader-visible-signed.xpi
          est introuvable dans nixos-flakes. Générez-le avec
          `web-ext sign --channel=unlisted` (voir README.md), ou passez
          useSigned = false pour le mode dev/ESR.
        '';
      }
    ];

    programs.firefox.policies.ExtensionSettings =
      self.lib.mkImageDownloaderExtensionSettings {
        inherit system;
        useSigned = cfg.useSigned;
      };
  };
}
