{
  lib,
  writeShellApplication,
  coreutils,
  git,
  jq,
  nix,
}:

writeShellApplication {
  name = "nixos-check-flake-update";

  runtimeInputs = [
    coreutils
    git
    jq
    nix
  ];

  text = builtins.readFile ./script.sh;

  meta = with lib; {
    description = "Vérifie les mises à jour disponibles pour un Flake NixOS";
    license = licenses.mit;
  };
}
