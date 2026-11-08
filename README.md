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
nixos-flakes update [PATH] [--commit] [--input NOM]   nix flake update (+ commit de flake.lock seul)
nixos-flakes apply [PATH]                 nixos-rebuild switch via pkexec
nixos-flakes upgrade [PATH] [--notify]    mise à jour + application, retour arrière si échec
nixos-flakes track|untrack|ignore PATH    gérer les rôles
nixos-flakes set-system PATH              définir le dépôt système
```

`PATH` vaut par défaut le dépôt système. `--config FICHIER` change le fichier de configuration.

### Paquets déclarés (`pkg`)

```text
nixos-flakes pkg search REQUÊTE [--limit N]      cherche dans nixpkgs (search.nixos.org, puis nix search, puis cache hors ligne)
nixos-flakes pkg list [--repo CHEMIN]            Fichiers de config, Emplacements (lecture seule signalés) et paquets déclarés
nixos-flakes pkg add NOM --file F --location ID  déclare un paquet
nixos-flakes pkg remove NOM --file F --location ID
nixos-flakes pkg move NOM --from F:ID --to F:ID
```

Les commandes qui modifient affichent le diff, valident le Nix puis demandent confirmation (`--yes` pour passer, `--dry-run` pour ne rien écrire). `--repo` remplace le dépôt système. Si le dossier n'est pas modifiable, l'écriture passe par `pkexec` (ou `sudo` avec `--sudo`). Rien n'est appliqué : lance ensuite `nixos-flakes apply`. Les résultats de recherche sont gardés dans `~/.cache/nixos-flakes/search/`.

### Mise à jour sélective

Un dépôt flake peut déclarer plusieurs inputs ; tu peux n'en mettre à jour qu'une partie.

- **GUI** : dans l'onglet **Mises à jour**, chaque input a une case à cocher et un bouton propre. Le bouton **Mettre à jour la sélection** ne touche que les inputs cochés.
- **CLI** : `nixos-flakes update [PATH] --input NOM`, répétable pour plusieurs inputs (`--input nixpkgs --input home-manager`).

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
- Mise à jour d'un sous-ensemble d'inputs : voir [Mise à jour sélective](#mise-à-jour-sélective).
- Console de logs en direct.
- **Flux git** : avant la mise à jour, si le dépôt a des modifications non commitées, proposition de les commiter ; après, proposition de commiter `flake.lock` seul. Jamais de push automatique.
- Réglages : racines de scan, profondeur, tray, fréquence de vérification.
- **Page Paquets** (barre latérale) : recherche nixpkgs (async, badge « cache » hors ligne), puis choix du Fichier de config et de l'Emplacement avec aperçu du diff et confirmation avant écriture ; liste des Paquets déclarés du Dépôt système avec Supprimer / Déplacer (Emplacements générés : « modifiable en code seulement »). Rien n'est appliqué tant que tu n'as pas lancé l'Application. Écriture avec élévation (pkexec) si le dépôt appartient à root. Vue de démo : `--demo --demo-view paquets` (ou `paquets-ajout`) ; la fausse config reste en mémoire.
- Design inspiré de Linear : thème sombre/clair selon le système, police Inter, transitions fluides (rien ne tourne au repos).
- **Clavier** : `Ctrl+K` palette de commandes, `j`/`k` ou flèches pour naviguer, `r` vérifier, `u` mettre à jour, `Échap` fermer.
- `--demo` : lance l'interface avec de fausses données (rien n'est lu ni écrit), pratique pour essayer.
- **Tray optionnel** (`nixos-flakes-gui --tray` ou réglage) : états à jour / mises à jour / erreur, menu Ouvrir / Vérifier / Quitter. Nécessite un hôte StatusNotifier, voir [Zone de notification sous niri](#zone-de-notification-sous-niri).

L'application du système passe par `pkexec` (fenêtre polkit) ; la mise à jour des verrous se fait toujours avec tes droits, donc `flake.lock` ne devient jamais propriété de root.

### Jeton GitHub

Un jeton GitHub relève la limite de requêtes de GitHub pour les mises à jour des dépôts. Tu peux le saisir dans **Réglages** :

- stocké dans `~/.config/nixos-flakes/github-token`, en `0600` (lisible par toi seul) ;
- passé à `nix` via la variable `NIX_CONFIG` ;
- jamais transmis à `pkexec` : l'application du système ne le reçoit pas.

Si ta configuration NixOS gère déjà un fichier de jeton, indique son chemin sans jamais écrire le jeton dans la configuration Nix :

```nix
services.nixos-flakes.githubTokenFile = "/run/secrets/github-token";
```

Le fichier peut contenir le jeton seul, `github.com=JETON` ou `access-tokens = github.com=JETON`. Réglages affiche alors « Jeton configuré par la configuration Nix » et la saisie est désactivée. Sans cette option, le jeton se saisit dans l'interface.

Un script NixOS peut aussi écrire le jeton dans `/etc/nix/access-tokens.conf`, au format `extra-access-tokens = github.com=JETON` (mode `600`, propriétaire root) : l'outil le reconnaît. Mais un `nix` lancé par ton utilisateur ne peut pas lire un fichier réservé à root. Pour le rendre lisible, passe-le en `640` avec un groupe accessible, par exemple avec `install -m 640 -g users`, ou enregistre un jeton dans **Réglages**.

Ordre de priorité : `githubTokenFile` (configuration Nix) > jeton saisi dans l'interface > `/etc/nix/access-tokens.conf`.

### Zone de notification : prérequis par bureau

L'icône utilise le standard StatusNotifierItem, sans code propre à un bureau. Il faut un hôte qui l'affiche :

- **KDE Plasma** : natif, rien à ajouter ;
- **GNOME** : installer l'extension AppIndicator / KStatusNotifierItem ;
- **Hyprland, niri** et autres compositeurs : une barre avec hôte StatusNotifier, par exemple le module `tray` de Waybar.

### Zone de notification sous niri

Le tray a besoin d'un **hôte StatusNotifier** pour afficher son icône. Avec niri, il faut donc :

1. une barre qui fournit cet hôte, par exemple le module `tray` de Waybar ;
2. lancer l'interface dans la session :
   - avec `niri-session` (qui atteint `graphical-session.target`) et `services.nixos-flakes.gui.tray = true`, le service utilisateur `nixos-flakes-gui-tray` démarre seul ;
   - avec un simple `niri` lancé depuis un TTY ou un gestionnaire de connexion, le service ne démarre pas : ajoute plutôt dans la configuration de niri (`~/.config/niri/config.kdl`) :

     ```kdl
     spawn-at-startup "nixos-flakes-gui" "--tray"
     ```

Si l'icône n'apparaît pas au démarrage, vérifie que la barre tourne déjà et que son module `tray` est activé.

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
        nixos-flakes.nixosModules.nixos-flakes
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

Si la mise à jour ou le commit depuis la GUI échoue avec `path '/etc/nixos' is not owned by current user` (le dépôt système appartient à root, et `nix` refuse qu'un autre utilisateur y écrive), ajoute dans `configuration.nix` :

```nix
{
  services.nixos-flakes.systemRepo.owner = "moi";
}
```

Cela accorde à `moi` la lecture et l'écriture sur `/etc/nixos` via une ACL (héritée par les nouveaux fichiers) et ajoute ce chemin exact à `safe.directory` de git. Le dépôt reste la propriété de root, et l'application du système passe toujours par `pkexec`. Nécessite un système de fichiers avec ACL (ext4, btrfs, xfs…). Pour un autre chemin que `/etc/nixos`, utilise `services.nixos-flakes.systemRepo.path`.

Sans cette option, la vérification d'un `/etc/nixos` appartenant à root fonctionne quand même, sans reconstruction : seuls `flake.nix` et `flake.lock` sont copiés pour être évalués. Écrire le `flake.lock` (mise à jour) exige en revanche `services.nixos-flakes.systemRepo.owner`.

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

Source (manifest V3) dans `pkgs/firefox-image-downloader-visible/src`. Le popup liste les images et vidéos *visibles à l'écran* de l'onglet actif (balises, fonds CSS, `<canvas>`, Shadow DOM) pour les prévisualiser, ouvrir, copier ou télécharger. Le script d'analyse n'est injecté qu'à l'ouverture du popup (`activeTab` + `scripting`), jamais en permanence. Permissions : `activeTab`, `scripting`, `downloads`, `clipboardWrite`.

**Signature Mozilla.** Firefox release et Zen refusent toute extension non signée, même via les policies (`ExtensionSettings`). Deux modes :

| Mode | Paquet | Navigateur |
|------|--------|------------|
| Dev / ESR | `firefox-image-downloader-visible` (`.xpi` non signé, construit depuis les sources) | Firefox ESR, Nightly, Developer Edition |
| Production | `firefox-image-downloader-visible-signed` (`.xpi` signé commité) | Firefox release, Zen |

### Mode dev / ESR

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
      ExtensionSettings = inputs.nixos-flakes.lib.mkImageDownloaderExtensionSettings {
        system = pkgs.stdenv.hostPlatform.system;
        useSigned = false;
      };
    };
  };
}
```

### Mode production

1. Récupérez une clé API (JWT issuer/secret) sur [addons.mozilla.org](https://addons.mozilla.org) (paramètres développeur).
2. À chaque changement de `version` dans `manifest.json`, signez (canal non listé) :

   ```bash
   nix run nixpkgs#web-ext -- sign --channel=unlisted \
     --api-key="$AMO_JWT_ISSUER" --api-secret="$AMO_JWT_SECRET" \
     --source-dir=pkgs/firefox-image-downloader-visible/src \
     --artifacts-dir=/tmp/web-ext-artifacts
   cp /tmp/web-ext-artifacts/*.xpi \
     pkgs/firefox-image-downloader-visible-signed/image-downloader-visible-signed.xpi
   ```

3. Committez le `.xpi`. Le paquet `-signed` n'existe que s'il est présent, et `nix flake check` vérifie que sa version correspond aux sources.

Firefox, via le module :

```nix
{ inputs, ... }:
{
  imports = [ inputs.nixos-flakes.nixosModules.image-downloader-visible ];

  programs.firefox.enable = true;
  programs.image-downloader-visible.enable = true; # useSigned = true par défaut
}
```

Sans `.xpi` signé, `useSigned = true` échoue avec une assertion explicite ; passez `useSigned = false` pour le mode dev/ESR.

Zen Browser (via l'override `extraPolicies` de `youwen5/zen-browser-flake`) :

```nix
{ inputs, pkgs, ... }:
{
  environment.systemPackages = [
    (inputs.zen-browser.packages.${pkgs.stdenv.hostPlatform.system}.default.override {
      extraPolicies.ExtensionSettings = inputs.nixos-flakes.lib.mkImageDownloaderExtensionSettings {
        system = pkgs.stdenv.hostPlatform.system;
        useSigned = true;
      };
    })
  ];
}
```

### Test rapide sans signer

```bash
nix run nixpkgs#web-ext -- run --source-dir=pkgs/firefox-image-downloader-visible/src
```

Ou `about:debugging#/runtime/this-firefox` → « Charger un module complémentaire temporaire… » → `manifest.json` (disparaît au redémarrage).

## Licence

GPL-3.0-or-later — voir [LICENSE](LICENSE).
