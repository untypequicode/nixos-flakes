# Outils non-Rust : extension Firefox/Zen "Image Downloader Visible".
# Retourne { packages, checks }. Le paquet "-signed" (et son check) n'existe que
# si le .xpi signé par Mozilla est commité (voir README.md, section Extension).
{ pkgs, root }:
let
  inherit (pkgs) lib;
  hasSignedXpi = builtins.pathExists (
    root + "/pkgs/firefox-image-downloader-visible-signed/image-downloader-visible-signed.xpi"
  );
  signed = pkgs.callPackage ../pkgs/firefox-image-downloader-visible-signed { };
in
{
  packages = {
    firefox-image-downloader-visible = pkgs.callPackage ../pkgs/firefox-image-downloader-visible { };
  }
  // lib.optionalAttrs hasSignedXpi {
    firefox-image-downloader-visible-signed = signed;
  };

  # Le .xpi signé commité doit avoir la même version que les sources.
  checks = lib.optionalAttrs hasSignedXpi {
    image-downloader-signed-version =
      pkgs.runCommand "image-downloader-signed-version"
        {
          nativeBuildInputs = [
            pkgs.unzip
            pkgs.jq
          ];
        }
        ''
          v=$(unzip -p ${signed}/image-downloader-visible.xpi manifest.json | jq -r .version)
          if [ "$v" != "${signed.version}" ]; then
            echo "xpi signé en $v, sources en ${signed.version} : re-signer (voir README)" >&2
            exit 1
          fi
          touch $out
        '';
  };
}
