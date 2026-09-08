{
  lib,
  writeShellApplication,
  coreutils,
  git,
  nix,
}:

writeShellApplication {
  name = "nixos-flake-update";

  runtimeInputs = [
    coreutils
    git
    nix
  ];

  text = builtins.readFile ./script.sh;

  meta = with lib; {
    description = "Met à jour un Flake NixOS";
    license = licenses.mit;
  };
}
