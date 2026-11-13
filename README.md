# nixos-flakes

Recueil d'outils Nix/NixOS écrits en Rust. Le premier outil gère les **flakes de la machine** : il repère tous les dépôts flake, te laisse choisir ceux que tu maintiens, les met à jour, propose les commits git au bon moment et applique la configuration système.

| Outil | Binaire | Rôle |
| --- | --- | --- |
| CLI | `nixos-flakes` | Vérifier, mettre à jour, appliquer, mise à niveau automatique, démon, diagnostic, sortie Waybar |
| GUI | `nixos-flakes-gui` | Interface moderne (iced), reliée au démon s'il tourne |
| Extension Firefox/Zen | `firefox-image-downloader-visible` | Liste et télécharge les images/vidéos visibles à l'écran (voir [la section dédiée](#extension-firefoxzen--image-downloader-visible)) |

D'autres outils viendront dans `crates/` : tout crate binaire est automatiquement exposé par le flake (voir [Ajouter un outil](#ajouter-un-outil)).

Vocabulaire : [GLOSSARY.md](GLOSSARY.md). Conception : [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Concepts

- **Dépôt détecté** : un dossier avec un `flake.nix` trouvé par le scan. Il n'est **jamais touché** tant que tu ne l'as pas choisi.
- **Dépôt suivi** : un dépôt que tu as choisi de maintenir.
- **Dépôt système** : le dépôt suivi qui décrit ta machine. Il n'est pas forcément `/etc/nixos` (voir [Emplacement de la configuration NixOS](#emplacement-de-la-configuration-nixos)). Seul lui peut être appliqué au système.
- **Ignoré** : masqué définitivement de la liste.

La configuration est dans `~/.config/nixos-flakes/config.toml` (racines de scan, profondeur, rôles, fréquence de vérification).

## Essayer sans installer

```bash
nix run github:untypequicode/nixos-flakes -- list --all        # l'app par défaut est la CLI
nix run github:untypequicode/nixos-flakes#nixos-flakes-gui
```

## CLI

```text
nixos-flakes list [--all] [--check] [--json]   dépôts suivis (--all : + détectés)
nixos-flakes scan [--json]                     dépôts détectés sous les racines de scan
nixos-flakes check [PATH] [--waybar] [--exit-code]   mises à jour disponibles (sans rien modifier)
nixos-flakes update [PATH] [--commit] [--input NOM] [--dry-run]   nix flake update (+ commit de flake.lock seul)
nixos-flakes apply [PATH] [--attr NOM] [-y]    nixos-rebuild switch via pkexec
nixos-flakes upgrade [PATH] [--attr NOM] [--notify] [--sudo] [-y]   mise à jour + application, retour arrière si échec
nixos-flakes sync [PATH] [--check]             git fetch, avance/retard, puis pull --ff-only (--check : sans pull)
nixos-flakes status [PATH] [--json]            résumé : dépôts, git, mises à jour connues, démon, dernière mise à niveau
nixos-flakes doctor                            diagnostic de l'installation (✓ / ⚠ / ✗ + piste de correction)
nixos-flakes daemon [--gui-command CMD]        démon de session : icône d'état, vérifications, D-Bus (instance unique)
nixos-flakes completions SHELL                 script de complétion (bash, zsh, fish, elvish, powershell)
nixos-flakes track|untrack|ignore PATH         gérer les rôles
nixos-flakes set-system PATH [--attr NOM]      définir le dépôt système
```

`PATH` vaut par défaut le dépôt système. Options globales : `--config FICHIER` (autre fichier de configuration) et `-q`/`--quiet` (n'affiche que les résultats et les erreurs). La couleur est automatique (terminal ; `NO_COLOR` et `TERM=dumb` respectés). Les erreurs s'affichent en `erreur : …` suivi d'une `piste : …`.

- `--json` (`list`, `scan`, `status`) : sortie pour les scripts ; le schéma de `status --json` est stable.
- `--dry-run` : montre ce qui changerait sans rien écrire (`update`, `pkg add|remove|move`).
- `--yes`/`-y` : pas de confirmation (`apply`, `upgrade`, `pkg`) ; la question `[o/N]` n'est de toute façon posée que sur un terminal.
- `--exit-code` (`check`) : sort avec le code 10 s'il y a des mises à jour.
- `--attr NOM` : applique `nixosConfigurations.NOM` (défaut : l'attribut enregistré, sinon le nom d'hôte).

Codes de sortie :

| Code | Sens |
| --- | --- |
| 0 | succès |
| 1 | erreur (`doctor` : au moins un ✗) |
| 2 | usage incorrect |
| 3 | annulé par l'utilisateur |
| 4 | dépôt occupé (une autre opération est en cours dessus) |
| 5 | délai dépassé |
| 10 | `check --exit-code` : des mises à jour sont disponibles |

Les complétions bash, zsh et fish sont installées par le paquet ; pour un autre interpréteur : `nixos-flakes completions SHELL`.

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
- Réglages : racines de scan, profondeur, fréquence de vérification et **Dépôt système** (chemin et attribut `nixosConfigurations`).
- **Page Paquets** (barre latérale) : recherche nixpkgs (async, badge « cache » hors ligne), puis choix du Fichier de config et de l'Emplacement avec aperçu du diff et confirmation avant écriture ; liste des Paquets déclarés du Dépôt système avec Supprimer / Déplacer (Emplacements générés : « modifiable en code seulement »). Rien n'est appliqué tant que tu n'as pas lancé l'Application. Écriture avec élévation (pkexec) si le dépôt appartient à root. Vue de démo : `--demo --demo-view paquets` (ou `paquets-ajout`) ; la fausse config reste en mémoire.
- Design inspiré de Linear : thème sombre/clair selon le système, police Inter, transitions fluides (rien ne tourne au repos).
- **Clavier** : voir [Raccourcis clavier](#raccourcis-clavier-gui). `?` ouvre l'aide qui les liste.
- **Confirmations** : appliquer au système, pousser, retirer une racine de scan et les changements de rôle destructifs (ne plus suivre, définir le dépôt système, ignorer) demandent confirmation ; `Entrée` valide, `Échap` annule.
- `--demo` : lance l'interface avec de fausses données (rien n'est lu ni écrit), pratique pour essayer.
- **Démon** : si `nixos-flakes daemon` tourne, la barre latérale affiche son état (mis à jour en direct) et le bouton **Vérifier maintenant** lui demande une vérification. Sans démon, la GUI fonctionne seule ; le démon n'a pas besoin d'elle. La GUI ne crée jamais d'icône et fermer sa fenêtre la quitte (le démon continue). Les notifications de bureau « mises à jour disponibles » sont envoyées par le démon.
- **Synchroniser** (config système) : `git fetch` à l'ouverture de la GUI et sur **Actualiser** seulement ; la barre latérale indique l'avance/retard, et **Synchroniser** lance `git pull --ff-only`. Refusé avec un message clair si l'arbre est sale ou si les historiques ont divergé. Équivalent CLI : `nixos-flakes sync [CHEMIN] [--check]`.
- **Terminal** (barre latérale) : terminal intégré (`iced_term`) qui lance `$SHELL` dans le dépôt de config système, aux couleurs du thème de la GUI. Toutes les touches vont au shell tant que l'onglet est affiché.

L'application du système passe par `pkexec` (fenêtre polkit) ; la mise à jour des verrous se fait toujours avec tes droits, donc `flake.lock` ne devient jamais propriété de root.

### Raccourcis clavier (GUI)

| Touche | Action |
| --- | --- |
| `j` / `↓`, `k` / `↑` | Dépôt suivant / précédent |
| `1` · `2` · `3` | Onglet Mises à jour · Git · Dépôt |
| `r` | Vérifier |
| `u` | Mettre à jour |
| `a` | Appliquer au système (avec confirmation) |
| `Ctrl+K` | Palette de commandes |
| `,` | Réglages |
| `/` | Rechercher (page Paquets) |
| `?` | Aide des raccourcis |
| `Échap` | Fermer, annuler ou revenir |
| `Entrée` | Valider une confirmation |

Les touches simples sont inactives dans un champ de texte ou un dialogue.

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

### Zone de notification (icône du démon)

L'icône utilise le standard StatusNotifierItem, sans code propre à un bureau. Il faut un hôte qui l'affiche :

- **KDE Plasma** : natif, rien à ajouter ;
- **GNOME** : installer l'extension AppIndicator / KStatusNotifierItem ;
- **Hyprland, niri** et autres compositeurs : une barre avec hôte StatusNotifier, par exemple le module `tray` de Waybar.

Le service utilisateur `nixos-flakes-daemon` est lié à `graphical-session.target` : avec `niri-session` et `services.nixos-flakes.enable = true`, il démarre seul. Avec un simple `niri` lancé depuis un TTY ou un gestionnaire de connexion, le service ne démarre pas : ajoute dans `~/.config/niri/config.kdl`

```kdl
spawn-at-startup "nixos-flakes" "daemon"
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
        nixos-flakes.nixosModules.default   # alias de nixosModules.nixos-flakes
      ];
    };
  };
}
```

`configuration.nix` :

```nix
{
  services.nixos-flakes = {
    enable = true;               # démon de session : icône d'état dans la zone de notification
    user = "moi";                # utilisateur du démon ; flake.lock reste à ton nom après une mise à niveau
    interval = "daily";          # optionnel : mise à niveau automatique ("weekly", "04:00"…) ; null = aucune
    flakePath = "/etc/nixos";    # chemin absolu du dépôt système (défaut : /etc/nixos)
    # flakeAttr = "mon-pc";      # nixosConfigurations.<nom> ; défaut : nom d'hôte
    gui.enable = true;           # installe la GUI, ouvrable depuis l'icône
  };
}
```

Si la mise à jour ou le commit depuis la GUI échoue avec `path '/etc/nixos' is not owned by current user` (le dépôt système appartient à root, et `nix` refuse qu'un autre utilisateur y écrive), ajoute dans `configuration.nix` :

```nix
{
  services.nixos-flakes.systemRepo.owner = "moi";
}
```

Cela accorde à `moi` la lecture et l'écriture sur `/etc/nixos` via une ACL (héritée par les nouveaux fichiers) et ajoute ce chemin exact à `safe.directory` de git. Le dépôt reste la propriété de root, et l'application du système passe toujours par `pkexec`. Nécessite un système de fichiers avec ACL (ext4, btrfs, xfs…). `systemRepo.path` vaut par défaut `flakePath`.

Sans cette option, la vérification d'un `/etc/nixos` appartenant à root fonctionne quand même, sans reconstruction : seuls `flake.nix` et `flake.lock` sont copiés pour être évalués. Écrire le `flake.lock` (mise à jour) exige en revanche `services.nixos-flakes.systemRepo.owner`.

L'ancien nom `services.nixos-auto-upgrade-custom.{enable,interval}` et les anciennes options `services.nixos-flakes.autoUpgrade.*` fonctionnent encore (avertissement de renommage ; `enable = true` sans `interval` vaut `interval = "daily"`). `gui.tray` n'existe plus : l'icône est portée par le démon (`enable`).

Le timer lance `nixos-flakes upgrade --notify` : mise à jour des verrous, application si quelque chose a changé, et en cas d'échec restauration du `flake.lock` puis retour à la génération précédente. Il ne commite rien : la GUI te proposera de commiter le verrou modifié.

## Emplacement de la configuration NixOS

Le dépôt système n'est pas forcément `/etc/nixos`. Ordre de résolution (le premier qui répond gagne) :

1. `system_repo` (et `system_attr`) dans `~/.config/nixos-flakes/config.toml` ;
2. la variable `NIXOS_FLAKES_SYSTEM_REPO` (attribut : `NIXOS_FLAKES_SYSTEM_ATTR`) ;
3. `/etc/nixos-flakes/config.toml`, écrit par le module NixOS à partir de `flakePath` et `flakeAttr` ;
4. `/etc/nixos`, seulement s'il contient un `flake.nix`.

Sans résultat, les commandes s'arrêtent avec une piste ; `nixos-flakes doctor` indique d'où vient le dépôt retenu. Trois façons de le définir :

```nix
# 1. Module NixOS (valable pour tous les utilisateurs et pour la mise à niveau automatique)
services.nixos-flakes = {
  enable = true;
  user = "moi";
  flakePath = "/home/moi/nixos-config";
  flakeAttr = "mon-pc";   # nixosConfigurations.mon-pc ; null = nom d'hôte
};
```

```bash
# 2. CLI (écrit dans ta configuration utilisateur, qui prime sur le module)
nixos-flakes set-system ~/nixos-config --attr mon-pc
```

3. **GUI** : Réglages > Dépôt système.

Pour un dépôt dans le dossier personnel, le module adapte les droits : si `systemRepo.owner` est dans son dossier personnel, **aucune ACL n'est posée** (il en est déjà propriétaire) ; le dépôt est ajouté à `safe.directory` de git (chemin exact) dès que `owner` ou `user` est défini ; et après chaque mise à niveau (lancée en root), `flake.lock` et `.git/*` sont rendus à leur propriétaire d'origine, sans suivre de lien symbolique ; en secours, l'unité du module refait un `chown -h` de `flake.lock` à `user` (un lien symbolique est ignoré). `flakePath`, `systemRepo.path` et `githubTokenFile` doivent être des chemins absolus. Si le flake est dans un sous-dossier d'un dépôt git, ajoute aussi la racine du dépôt à `programs.git.config.safe.directory`.

## Robustesse

- **Délais** : chaque commande externe a un délai (30 min pour `nix`, 60 s pour les lectures git ; 15 min par cycle pour le démon), réglable avec `NIXOS_FLAKES_TIMEOUT_SECS`. Ce réglage ne s'applique pas à `nixos-rebuild switch`, fixé à 6 h. Un dépassement sort avec le code 5. Si le délai est dépassé pendant l'application, il n'y a ni retour arrière ni restauration de `flake.lock` : un `switch` interrompu peut encore tourner.
- **Verrou par dépôt** : `update`, `apply` et `upgrade` ne tournent jamais en même temps sur un même dépôt ; la seconde opération sort avec le code 4 (après ~250 ms de nouvelles tentatives). Le verrou (`flock`) est pris sur le dossier du dépôt lui-même : aucun fichier n'est créé.
- **Écritures atomiques** : configuration, état du démon et `flake.lock` restauré sont écrits par fichier temporaire puis renommage ; une coupure ne laisse pas de fichier tronqué.
- **Démon** : arrêt propre sur SIGTERM/SIGINT, icône relancée avec attente croissante (5 s à 5 min), configuration relue quand elle change.
- **Diagnostic** : `nixos-flakes doctor` vérifie les outils, la configuration, le dépôt système (provenance, git, propriétaire, `nixosConfigurations`), le jeton et le démon.

## Développement

```bash
nix develop                         # cargo, clippy, rustfmt, libs de la GUI
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p nixos-flakes -- list --all
cargo run -p nixos-flakes-gui
nix flake check                     # build + tests + clippy + rustfmt + nixfmt + nixos-module-eval
```

Organisation :

```text
crates/nixos-flakes-core/   logique partagée (config, scan, lock, git, nix, upgrade…)
crates/nixos-flakes/        CLI
crates/nixos-flakes-gui/    GUI iced (liée au démon)
modules/nixos-flakes.nix    module NixOS
nix/                        packaging (auto-découverte des crates)
```

Sorties du flake : `packages` (un paquet par crate, plus l'extension), `apps` (`nix run .` lance `nixos-flakes`), `overlays.default` (les outils et les paquets de l'extension), `nixosModules.default` (= `nixos-flakes`) et `nixosModules.image-downloader-visible`, `lib.mkImageDownloaderExtensionSettings`, `formatter` (nixfmt). `checks` : build, tests, clippy, rustfmt, `nixfmt`, `nixos-module-eval` (évalue le module avec plusieurs variantes d'options et leurs assertions) et, si le `.xpi` signé est présent, `image-downloader-signed-version`.

### Ajouter un outil

```bash
cargo new crates/mon-outil
git add crates/mon-outil
nix build .#mon-outil     # paquet et app générés automatiquement
```

Réutilise `nixos-flakes-core` si besoin (`nixos-flakes-core = { path = "../nixos-flakes-core" }`). Pour un module NixOS dédié, ajoute `modules/mon-outil.nix`.

## Extension Firefox/Zen : Image Downloader Visible

Source (manifest V3) dans `pkgs/firefox-image-downloader-visible/src`. Le popup liste les images et vidéos *visibles à l'écran* de l'onglet actif (balises, fonds CSS, `<canvas>`, Shadow DOM) pour les prévisualiser, ouvrir, copier ou télécharger. Le script d'analyse n'est injecté qu'à l'ouverture du popup (`activeTab` + `scripting`), jamais en permanence. Permissions : `activeTab`, `scripting`, `downloads`.

**Tout télécharger** continue même si le popup est fermé (la file est portée par `background.js`) ; rouvert, le popup affiche la progression. Sur une page trop volumineuse pour être analysée en entier, un bandeau « Résultats partiels » invite à faire défiler jusqu'aux médias voulus puis à actualiser.

Popup : bouton **Actualiser** pour réanalyser la page, zone d'état (recherche, aucune image, erreur), analyse des iframes (repli sur le cadre principal si Firefox refuse un cadre), noms de fichiers propres et uniques au téléchargement (suffixe `-1`, `-2` en cas de collision), thème clair/sombre selon le système.

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

   **Attention** : ce contrôle ne compare que le numéro de version. Après toute modification des sources de l'extension, le `.xpi` signé est périmé : incrémente `version` dans `manifest.json` puis re-signe.

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
