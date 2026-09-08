{
  lib,
  writeShellApplication,
  coreutils,
  git,
  nix,
  nixos-rebuild,
  libnotify,
}:

writeShellApplication {
  name = "nixos-auto-upgrade";

  runtimeInputs = [
    coreutils
    git
    nix
    nixos-rebuild
    libnotify
  ];

  text = builtins.readFile ./script.sh;

  meta = with lib; {
    description = "Met à jour automatiquement le flake et le système avec gestion des erreurs";
    license = licenses.mit;
  };
}
