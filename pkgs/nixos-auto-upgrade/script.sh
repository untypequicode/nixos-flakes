#!/usr/bin/env bash
set -euo pipefail

FLAKE_DIR="/etc/nixos"
SUDO="/run/wrappers/bin/sudo"
LOCK_FILE="$FLAKE_DIR/flake.lock"
BACKUP_LOCK="$FLAKE_DIR/flake.lock.bak"

send_notification() {
    local title="$1"
    local message="$2"
    local urgency="$3"

    local uid
    uid=$(id -u "${SUDO_USER:-$USER}" 2>/dev/null || echo 1000)

    $SUDO -u "#$uid" DISPLAY=:0 DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus" \
        notify-send -u "$urgency" "$title" "$message" 2>/dev/null || true
}

cd "$FLAKE_DIR"
$SUDO cp "$LOCK_FILE" "$BACKUP_LOCK"

cleanup() {
    $SUDO rm -f "$BACKUP_LOCK"
}
trap cleanup EXIT

echo "Vérification des mises à jour des flakes..."

if ! $SUDO nix flake update --flake "$FLAKE_DIR"; then
    send_notification "NixOS Update" "Erreur lors du téléchargement des mises à jour." "critical"
    exit 1
fi

if $SUDO cmp -s "$LOCK_FILE" "$BACKUP_LOCK"; then
    echo "Tout est déjà à jour. Aucune action requise."
    exit 0
fi

echo "Des mises à jour ont été trouvées. Application en cours..."

if $SUDO nixos-rebuild switch --flake "$FLAKE_DIR"; then
    send_notification "NixOS Update" "Mise à jour du système réussie avec succès !" "normal"
else
    echo "Échec ! Restauration de l'ancien lock..."
    $SUDO mv "$BACKUP_LOCK" "$LOCK_FILE"
    $SUDO nixos-rebuild switch --flake "$FLAKE_DIR" || true

    send_notification "NixOS Update" "Échec de la mise à jour ! Restauration de l'ancienne version." "critical"
    exit 1
fi
