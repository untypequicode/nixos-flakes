{
  lib,
  stdenvNoCC,
}:

# Ce paquet attend un fichier `image-downloader-visible-signed.xpi` déjà signé
# par Mozilla (via `web-ext sign --channel=unlisted`, voir README.md), présent
# à côté de ce default.nix. Il se contente de le copier dans le store Nix afin
# d'obtenir un chemin /nix/store/... stable, utilisable comme install_url
# ("file://...") dans les policies Firefox/Zen, sans dépendre du réseau au
# moment du build.
stdenvNoCC.mkDerivation {
  pname = "firefox-image-downloader-visible-signed";
  version = "1.2";

  src = ./image-downloader-visible-signed.xpi;

  dontUnpack = true;
  dontBuild = true;
  dontConfigure = true;

  installPhase = ''
    runHook preInstall
    mkdir -p "$out"
    cp "$src" "$out/image-downloader-visible.xpi"
    runHook postInstall
  '';

  meta = with lib; {
    description = "Extension Firefox/Zen signée par Mozilla : télécharge les images et vidéos visibles à l'écran";
    homepage = "https://github.com/untypequicode/nixos-flakes";
    license = licenses.gpl3Only;
    platforms = platforms.all;
  };
}
