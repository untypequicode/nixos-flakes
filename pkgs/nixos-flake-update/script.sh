#!/usr/bin/env bash

set -euo pipefail

SUDO="/run/wrappers/bin/sudo"

cd /etc/nixos
$SUDO nix flake update
$SUDO nixos-rebuild switch --flake /etc/nixos
