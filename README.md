# NixOS Flakes

Voici une simple collection de scripts et d'utilitaires que j'ai développé afin de simplifier la gestion de mon système NixOS sur Hyprland.

## Paquets disponibles

- **`nixos-auto-upgrade`** : Met à jour automatiquement le flake et le système avec gestion des erreurs et restauration en cas d'échec.
- **`nixos-flake-update`** : Met à jour le flake système (`/etc/nixos`) et applique la configuration via `nixos-rebuild`.
- **`nixos-check-flake-update`** : Vérifie les mises à jour disponibles pour un flake et retourne un format JSON (idéal pour intégrer dans des barres d'état comme Waybar ou Polybar).
- **`hello`** : Paquet d'exemple pour tester l'environnement.
- **`firefox-image-downloader-visible`** : Extension Firefox/Zen qui liste et télécharge les images/vidéos visibles à l'écran (voir section dédiée ci-dessous).

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

## Extension Firefox/Zen : Image Downloader Visible

Le dossier `pkgs/firefox-image-downloader-visible/src` contient le code source
de l'extension (manifest V3). Elle liste toutes les images et vidéos
*visibles à l'écran* sur l'onglet actif (y compris les images de fond CSS, le
contenu de `<canvas>`, le Shadow DOM…) et permet de les prévisualiser, les
ouvrir, les copier ou les télécharger, individuellement ou en masse.

Contrairement à la version d'origine, elle n'injecte plus de script en
permanence sur `<all_urls>` : le code d'analyse (`content.js`) n'est exécuté
qu'à la demande, sur l'onglet actif, quand vous ouvrez le popup
(`browser.scripting.executeScript` + permission `activeTab`). C'est plus
respectueux de la vie privée et plus léger.

### ⚠️ Signature Mozilla : ce qu'il faut savoir

Firefox et Zen (basés tous les deux sur le canal *release* de Gecko)
**refusent d'installer en permanence une extension qui n'est pas signée par
Mozilla**, y compris via les policies entreprise (`ExtensionSettings`). Seuls
les canaux ESR, Nightly et Developer Edition permettent de désactiver cette
vérification. Deux chemins sont donc proposés :

#### Option A — Mode dev / ESR (100 % reproductible, aucun secret nécessaire)

Le paquet `firefox-image-downloader-visible` construit un `.xpi` **non
signé** directement depuis les sources, à chaque build. Utilisable seulement
avec `pkgs.firefox-esr` (ou Nightly/Developer Edition), en désactivant la
vérification de signature :

```nix
{ pkgs, inputs, ... }:
{
  programs.firefox = {
    enable = true;
    package = pkgs.firefox-esr;
    policies = {
      Preferences."xpinstall.signatures.required" = {
        Value = false;
        Status = "locked";
      };
    } // {
      ExtensionSettings =
        inputs.untypequicode-flakes.lib.mkImageDownloaderExtensionSettings {
          system = pkgs.system;
          useSigned = false;
        };
    };
  };
}
```

#### Option B — Mode production (Firefox release + Zen)

1. Créez un compte sur [addons.mozilla.org](https://addons.mozilla.org),
   récupérez une clé API (JWT issuer/secret) dans vos paramètres développeur.
2. Depuis `pkgs/firefox-image-downloader-visible/src`, générez le `.xpi`
   signé (une seule fois, à chaque nouvelle version) :

   ```bash
   npx web-ext sign \
     --channel=unlisted \
     --api-key=$AMO_JWT_ISSUER \
     --api-secret=$AMO_JWT_SECRET \
     --source-dir=pkgs/firefox-image-downloader-visible/src \
     --artifacts-dir=/tmp/web-ext-artifacts
   ```

3. Copiez le fichier généré dans le repo et committez-le :

   ```bash
   cp /tmp/web-ext-artifacts/*.xpi \
     pkgs/firefox-image-downloader-visible-signed/image-downloader-visible-signed.xpi
   ```

4. Le paquet `firefox-image-downloader-visible-signed` (et le module
   `nixosModules.image-downloader-visible`) deviennent automatiquement
   disponibles — Nix ne fait plus que copier ce `.xpi` déjà signé, sans appel
   réseau au moment du build.

**Pour Firefox**, via le module fourni :

```nix
{ inputs, ... }:
{
  imports = [ inputs.untypequicode-flakes.nixosModules.image-downloader-visible ];

  programs.firefox.enable = true;
  programs.image-downloader-visible.enable = true; # useSigned = true par défaut
}
```

**Pour Zen Browser**, en réutilisant la même fonction partagée via l'override
`extraPolicies` exposé par `youwen5/zen-browser-flake` :

```nix
{ inputs, pkgs, ... }:
{
  environment.systemPackages = [
    (inputs.zen-browser.packages.${pkgs.stdenv.hostPlatform.system}.default.override {
      extraPolicies = {
        ExtensionSettings =
          inputs.untypequicode-flakes.lib.mkImageDownloaderExtensionSettings {
            system = pkgs.stdenv.hostPlatform.system;
            useSigned = true;
          };
      };
    })
  ];
}
```

### Développement / test rapide sans signer

Pour itérer sur le code sans passer par la signature à chaque fois, chargez
l'extension temporairement (elle disparaît au redémarrage de Firefox, mais ne
nécessite ni policy ni signature) :

1. Ouvrez `about:debugging#/runtime/this-firefox`.
2. Cliquez sur « Charger un module complémentaire temporaire… ».
3. Sélectionnez `pkgs/firefox-image-downloader-visible/src/manifest.json`.

Ou en une commande, via `web-ext` :

```bash
nix run nixpkgs#nodePackages.web-ext -- run \
  --source-dir=pkgs/firefox-image-downloader-visible/src
```
