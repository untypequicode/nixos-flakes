{
  lib,
  stdenvNoCC,
  zip,
}:

stdenvNoCC.mkDerivation {
  pname = "firefox-image-downloader-visible";
  version = "1.2";

  src = ./src;

  nativeBuildInputs = [ zip ];

  dontBuild = true;
  dontConfigure = true;

  # Produit un .xpi NON SIGNÉ (c'est un simple zip du dossier source).
  # Firefox release / Zen refuseront de l'installer en force_installed tant
  # qu'il n'est pas signé par Mozilla (voir README.md : "Installer l'extension").
  installPhase = ''
    runHook preInstall
    mkdir -p "$out"
    zip -r -X "$out/image-downloader-visible.xpi" . -x '*.git*'
    runHook postInstall
  '';

  meta = with lib; {
    description = "Extension Firefox/Zen : télécharge les images et vidéos visibles à l'écran (xpi non signé, à usage de test/ESR)";
    homepage = "https://github.com/untypequicode/nixos-flakes";
    license = licenses.gpl3Only;
    platforms = platforms.all;
  };
}
