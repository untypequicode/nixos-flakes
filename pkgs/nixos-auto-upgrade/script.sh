#!/usr/bin/env bash
set -euo pipefail

FLAKE_DIR="/etc/nixos"
SUDO="/run/wrappers/bin/sudo"
LOCK_FILE="$FLAKE_DIR/flake.lock"
BACKUP_LOCK="$FLAKE_DIR/flake.lock.bak"

# Fonction utilitaire pour envoyer des notifications graphiques à l'utilisateur actif
send_notification() {
    local title="$1"
    local message="$2"
    local urgency="$3" # normal, critical

    # Trouve l'UID de l'utilisateur connecté graphiquement
    local uid
    uid=$(id -u "${SUDO_USER:-$USER}" 2>/dev/null || echo 1000)

    # Exécute notify-send dans la session de l'utilisateur s'il est connecté
    $SUDO -u "#$uid" DISPLAY=:0 DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus" \
        notify-send -u "$urgency" "$title" "$message" 2>/dev/null || true
}

cd "$FLAKE_DIR"
cp "$LOCK_FILE" "$BACKUP_LOCK"

cleanup() {
    rm -f "$BACKUP_LOCK"
}
trap cleanup EXIT

echo "Vérification des mises à jour des flakes..."

if ! $SUDO nix flake update --flake "$FLAKE_DIR"; then
    send_notification "NixOS Update" "Erreur lors du téléchargement des mises à jour." "critical"
    exit 1
fi

if git diff --quiet "$LOCK_FILE"; then
    echo "Tout est déjà à jour."
    exit 0
fi

echo "Application des mises à jour..."

if $SUDO nixos-rebuild switch --flake "$FLAKE_DIR"; then
    send_notification "NixOS Update" "Mise à jour du système réussie avec succès !" "normal"
else
    echo "Échec ! Restauration de l'ancien lock..."
    mv "$BACKUP_LOCK" "$LOCK_FILE"
    $SUDO nixos-rebuild switch --flake "$FLAKE_DIR" || true

    send_notification "NixOS Update" "Échec de la mise à jour ! Restauration de l'ancienne version." "critical"
    exit 1
fi
