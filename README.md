# nixos-flakes

Recueil d'outils Nix/NixOS écrits en Rust. Le premier outil gère les **flakes de la machine** : il repère tous les dépôts flake, te laisse choisir ceux que tu maintiens, les met à jour, propose les commits git au bon moment et applique la configuration système.

| Outil | Binaire | Rôle |
| --- | --- | --- |
| CLI | `nixos-flakes` | Vérifier, mettre à jour, appliquer, mise à niveau automatique, sortie Waybar |
| GUI | `nixos-flakes-gui` | Interface moderne (iced) avec icône de zone de notification optionnelle |
| Extension Firefox/Zen | `firefox-image-downloader-visible` | Liste et télécharge les images/vidéos visibles à l'écran (voir [la section dédiée](#extension-firefoxzen--image-downloader-visible)) |

D'autres outils viendront dans `crates/` : tout crate binaire est automatiquement exposé par le flake (voir [Ajouter un outil](#ajouter-un-outil)).

Vocabulaire : [GLOSSARY.md](GLOSSARY.md). Conception : [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Concepts

- **Dépôt détecté** : un dossier avec un `flake.nix` trouvé par le scan. Il n'est **jamais touché** tant que tu ne l'as pas choisi.
- **Dépôt suivi** : un dépôt que tu as choisi de maintenir.
- **Dépôt système** : le dépôt suivi qui décrit ta machine (par défaut `/etc/nixos`). Seul lui peut être appliqué au système.
- **Ignoré** : masqué définitivement de la liste.

La configuration est dans `~/.config/nixos-flakes/config.toml` (racines de scan, profondeur, rôles, tray, fréquence de vérification).

## Essayer sans installer

```bash
nix run github:untypequicode/nixos-flakes#nixos-flakes -- list --all
nix run github:untypequicode/nixos-flakes#nixos-flakes-gui
```

## CLI

```text
nixos-flakes list [--all] [--check]       dépôts suivis (--all : + détectés)
nixos-flakes scan                         dépôts détectés sous les racines de scan
nixos-flakes check [PATH] [--waybar]      mises à jour disponibles (sans rien modifier)
nixos-flakes update [PATH] [--commit]     nix flake update (+ commit de flake.lock seul)
nixos-flakes apply [PATH]                 nixos-rebuild switch via pkexec
nixos-flakes upgrade [PATH] [--notify]    mise à jour + application, retour arrière si échec
nixos-flakes track|untrack|ignore PATH    gérer les rôles
nixos-flakes set-system PATH              définir le dépôt système
```

`PATH` vaut par défaut le dépôt système. `--config FICHIER` change le fichier de configuration.

### Waybar

```jsonc
"custom/flakes": {
  "exec": "nixos-flakes check --waybar",
  "return-type": "json",
  "interval": 3600
}
```

Classes CSS : `uptodate`, `outdated` (texte `❆ N`), `error`.

## GUI

- Liste des dépôts groupés Système / Suivis / Détectés / Ignorés avec un badge d'état.
- Détail : diff par input, boutons Vérifier / Mettre à jour / Appliquer / Suivre / Ignorer.
- Console de logs en direct.
- **Flux git** : avant la mise à jour, si le dépôt a des modifications non commitées, proposition de les commiter ; après, proposition de commiter `flake.lock` seul. Jamais de push automatique.
- Réglages : racines de scan, profondeur, tray, fréquence de vérification.
- Design inspiré de Linear : thème sombre/clair selon le système, police Inter, transitions fluides (rien ne tourne au repos).
- **Clavier** : `Ctrl+K` palette de commandes, `j`/`k` ou flèches pour naviguer, `r` vérifier, `u` mettre à jour, `Échap` fermer.
- `--demo` : lance l'interface avec de fausses données (rien n'est lu ni écrit), pratique pour essayer.
- **Tray optionnel** (`nixos-flakes-gui --tray` ou réglage) : états à jour / mises à jour / erreur, menu Ouvrir / Vérifier / Quitter. Nécessite un hôte StatusNotifier (module `tray` de Waybar, par exemple).

L'application du système passe par `pkexec` (fenêtre polkit) ; la mise à jour des verrous se fait toujours avec tes droits, donc `flake.lock` ne devient jamais propriété de root.

## Installation sur NixOS

`flake.nix` de ta machine :

```nix
{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    nixos-flakes.url = "github:untypequicode/nixos-flakes";
    nixos-flakes.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { nixpkgs, nixos-flakes, ... }: {
    nixosConfigurations.mon-pc = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        nixos-flakes.nixosModules.default
      ];
    };
  };
}
```

`configuration.nix` :

```nix
{
  services.nixos-flakes = {
    autoUpgrade = {
      enable = true;
      interval = "daily";        # ou "weekly", "04:00"…
      flakePath = "/etc/nixos";
      user = "moi";              # optionnel : flake.lock reste à ton nom
    };
    gui = {
      enable = true;             # installe la GUI et la CLI
      tray = true;               # lance la GUI dans la zone de notification à la session
    };
  };
}
```

L'ancien nom `services.nixos-auto-upgrade-custom.{enable,interval}` fonctionne encore (avertissement de renommage).

Le timer lance `nixos-flakes upgrade --notify` : mise à jour des verrous, application si quelque chose a changé, et en cas d'échec restauration du `flake.lock` puis retour à la génération précédente. Il ne commite rien : la GUI te proposera de commiter le verrou modifié.

## Développement

```bash
nix develop                         # cargo, clippy, rustfmt, libs de la GUI
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p nixos-flakes -- list --all
cargo run -p nixos-flakes-gui
nix flake check                     # build + tests + clippy + rustfmt
```

Organisation :

```text
crates/nixos-flakes-core/   logique partagée (config, scan, lock, git, nix, upgrade…)
crates/nixos-flakes/        CLI
crates/nixos-flakes-gui/    GUI iced + tray
modules/nixos-flakes.nix    module NixOS
nix/                        packaging (auto-découverte des crates)
```

### Ajouter un outil

```bash
cargo new crates/mon-outil
git add crates/mon-outil
nix build .#mon-outil     # paquet et app générés automatiquement
```

Réutilise `nixos-flakes-core` si besoin (`nixos-flakes-core = { path = "../nixos-flakes-core" }`). Pour un module NixOS dédié, ajoute `modules/mon-outil.nix`.

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

## Licence

GPL-3.0-or-later — voir [LICENSE](LICENSE).
