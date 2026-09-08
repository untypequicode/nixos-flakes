# NixOS Flakes

Voici une simple collection de scripts et d'utilitaires que j'ai développé afin de simplifier la gestion de mon système NixOS sur Hyprland.

## Paquets disponibles

- **`nixos-auto-upgrade`** : Met à jour automatiquement le flake et le système avec gestion des erreurs et restauration en cas d'échec.
- **`nixos-flake-update`** : Met à jour le flake système (`/etc/nixos`) et applique la configuration via `nixos-rebuild`.
- **`nixos-check-flake-update`** : Vérifie les mises à jour disponibles pour un flake et retourne un format JSON (idéal pour intégrer dans des barres d'état comme Waybar ou Polybar).
- **`hello`** : Paquet d'exemple pour tester l'environnement.

## Utilisation à la volée (sans installation)

Vous pouvez exécuter n'importe quel script directement depuis GitHub grâce à la commande `nix run` :

```bash
# Vérifier les mises à jour (pointe vers /etc/nixos par défaut)
nix run github:untypequicode/nixos-flakes#nixos-check-flake-update

# Vérifier les mises à jour d'un dossier spécifique
nix run github:untypequicode/nixos-flakes#nixos-check-flake-update -- /chemin/vers/mon/flake

# Mettre à jour le système manuellement
nix run github:untypequicode/nixos-flakes#nixos-flake-update

```

## Installation permanente via Flakes

Pour avoir ces utilitaires disponibles en permanence sur votre système, vous pouvez les ajouter à votre configuration NixOS.

### 1. Modifier votre `flake.nix`

Ajoutez le dépôt dans vos `inputs` et passez-le à votre système via `specialArgs` :

```nix
{
  description = "Ma configuration NixOS";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    untypequicode-flakes.url = "github:untypequicode/nixos-flakes";
  };

  outputs = { self, nixpkgs, untypequicode-flakes, ... }@inputs: {
    nixosConfigurations.mon-pc = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      specialArgs = { inherit inputs; };

      modules = [
        ./configuration.nix
      ];
    };
  };
}

```

### 2. Activer les modules et installer les paquets dans `configuration.nix`

```nix
{ pkgs, inputs, ... }:

{
  imports = [
    inputs.untypequicode-flakes.nixosModules.auto-upgrade
  ];

  services.nixos-auto-upgrade-custom = {
    enable = true;
    interval = "daily"; # Fréquence : "daily", "weekly", ou une heure précise (ex: "04:00")
  };

  environment.systemPackages = with pkgs; [
    inputs.untypequicode-flakes.packages.${pkgs.system}.nixos-check-flake-update
    inputs.untypequicode-flakes.packages.${pkgs.system}.nixos-flake-update
  ];
}

```

Une fois ces modifications faites, appliquez-les avec :

```bash
sudo nixos-rebuild switch --flake /etc/nixos

```
