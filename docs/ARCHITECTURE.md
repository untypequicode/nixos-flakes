# Architecture

`nixos-flakes` est un recueil d'outils Nix/NixOS écrits en Rust. Le vocabulaire est défini dans [`GLOSSARY.md`](../GLOSSARY.md) : l'utiliser tel quel dans le code, les messages et l'UI (français).

## Disposition

```
Cargo.toml                    workspace (members = crates/*)
crates/
  nixos-flakes-core/          lib : toute la logique (synchrone, sans async)
  nixos-flakes/               bin `nixos-flakes` : CLI (clap) + sous-commande `daemon` (tokio, zbus, ksni)
  nixos-flakes-gui/           bin `nixos-flakes-gui` : GUI iced, liée au démon
  <futur-outil>/              un crate par nouvel outil
pkgs/<outil>/                  outils non-Rust (ex. extension Firefox), packagés à la main via nix/extensions.nix
modules/                      modules NixOS : un fichier et un nom `nixosModules.<outil>` par outil (pas de module « default »)
nix/                          packaging (auto-découverte des crates)
docs/                         documentation
```

Règle : un crate binaire de `crates/` devient automatiquement un `package` et une `app` du flake. Ajouter un outil = `cargo new crates/mon-outil`.

## Contrat de `nixos-flakes-core`

Tout est synchrone. Les commandes longues prennent un callback `&mut dyn FnMut(&str)` qui reçoit chaque ligne de sortie (stdout+stderr fusionnés) ; la GUI les appelle depuis un thread. Erreurs : `thiserror` dans un type `Error` unique + `type Result<T>`. Chaque module est testé avec des dépôts temporaires (`tempfile`).

### `config`
```rust
pub struct Config {
    pub scan_roots: Vec<PathBuf>,        // défaut: [~, dépôt système par défaut s'il est hors de ~]
    pub scan_depth: u8,                  // défaut: 4
    pub system_repo: Option<PathBuf>,    // choix de l'utilisateur ; absent : voir la résolution ci-dessous
    pub system_attr: Option<String>,     // nixosConfigurations.<attr> ; absent : nom d'hôte
    pub tracked: BTreeSet<PathBuf>,      // dépôts suivis (hors système)
    pub ignored: BTreeSet<PathBuf>,      // ignorés définitivement
    pub check_interval_minutes: u32,     // défaut: 60
}
pub enum Role { System, Tracked, Detected, Ignored }
```

Résolution du dépôt système (`Config::system_repo_resolved()`, seule source de vérité ; aucun `/etc/nixos` en dur ailleurs), par priorité : (a) `system_repo` de la configuration utilisateur ; (b) variable `NIXOS_FLAKES_SYSTEM_REPO` (et `NIXOS_FLAKES_SYSTEM_ATTR`) ; (c) fichier de défauts système `/etc/nixos-flakes/config.toml` (mêmes clés `system_repo`, `system_attr`, écrit par le module NixOS ; chemin surchargeable par `NIXOS_FLAKES_SYSTEM_DEFAULTS`) ; (d) `/etc/nixos` seulement s'il contient `flake.nix`. Sinon aucun dépôt système : les commandes échouent avec la piste « nixos-flakes set-system <chemin> » ou `services.nixos-flakes.flakePath`. La provenance (`SystemRepoSource`) est affichée par `status`, `doctor` et la GUI. Les rôles sont comparés sur chemins canoniques (un lien `/etc/nixos` → `~/nixos` et sa cible désignent le même dépôt). Le dépôt système résolu est toujours listé, même hors des racines de scan.

```rust
impl Config {
    pub fn default_path() -> PathBuf;           // $XDG_CONFIG_HOME/nixos-flakes/config.toml
    pub fn load(path: &Path) -> Result<Config>; // fichier absent => Config::default()
    pub fn save(&self, path: &Path) -> Result<()>;
    pub fn role(&self, repo: &Path) -> Role;
    pub fn set_role(&mut self, repo: &Path, role: Role); // System est unique
}
```

### `scan`
```rust
pub fn scan(roots: &[PathBuf], max_depth: u8) -> Vec<PathBuf>;
```
Dossiers contenant un `flake.nix`, triés, dédoublonnés, chemins canonicalisés. Ignore `.git`, `target`, `node_modules`, `.direnv`, `result*`, `/nix`. Ne suit pas les liens symboliques de dossiers. Ne descend pas dans un dépôt flake trouvé sauf si un sous-dossier a son propre `flake.nix` (gérer le cas simplement : on continue la descente).

### `lock`
```rust
pub struct LockedInput { pub name: String, pub rev: Option<String>, pub last_modified: Option<i64> }
pub struct InputChange { pub name: String, pub old_rev: Option<String>, pub new_rev: Option<String>,
                         pub old_date: Option<i64>, pub new_date: Option<i64> }
pub fn parse_lock(json: &str) -> Result<Vec<LockedInput>>;   // ignore le nœud "root"
pub fn diff(old: &[LockedInput], new: &[LockedInput]) -> Vec<InputChange>; // changés, ajoutés, retirés
```

### `git`
```rust
pub struct GitState { pub is_repo: bool, pub branch: Option<String>,
                      pub dirty: Vec<String>,    // chemins modifiés/non suivis (hors flake.lock)
                      pub lock_dirty: bool }     // flake.lock modifié
pub fn state(repo: &Path) -> Result<GitState>;   // pas un dépôt git => is_repo=false, jamais d'erreur
pub fn commit_all(repo: &Path, message: &str, out: &mut dyn FnMut(&str)) -> Result<()>;
pub fn commit_lock(repo: &Path, message: &str, out: &mut dyn FnMut(&str)) -> Result<()>; // flake.lock seul
pub fn push(repo: &Path, out: &mut dyn FnMut(&str)) -> Result<()>;
pub fn fetch(repo: &Path, out: &mut dyn FnMut(&str)) -> Result<()>;      // git fetch
pub struct SyncStatus { branch, upstream: Option<String>, ahead: u32, behind: u32, dirty: Vec<String> }
pub fn sync_status(repo: &Path) -> Result<SyncStatus>;                  // sans réseau (voir fetch)
pub fn pull_ff(repo: &Path, out: &mut dyn FnMut(&str)) -> Result<()>;   // pull --ff-only ; Err(Error::Git) si arbre sale, divergence ou pas d'amont
pub fn lock_commit_message(changes: &[InputChange]) -> String;
```
S'appuie sur le binaire `git` (pas de libgit2).

### `nix`
```rust
pub struct UpdateReport { pub repo: PathBuf, pub changes: Vec<InputChange> }
/// Simule `nix flake update` SANS toucher au dépôt (--output-lock-file vers un fichier temporaire).
pub fn check(repo: &Path) -> Result<UpdateReport>;
/// `nix flake update` en utilisateur ; renvoie les changements réels.
pub fn update(repo: &Path, out: &mut dyn FnMut(&str)) -> Result<UpdateReport>;
/// `nixos-rebuild switch --flake <repo>` via pkexec (ou sudo si `Privilege::Sudo`, ou rien si déjà root).
pub enum Privilege { Pkexec, Sudo }
pub fn apply(repo: &Path, privilege: Privilege, out: &mut dyn FnMut(&str)) -> Result<()>;
pub fn rollback(privilege: Privilege, out: &mut dyn FnMut(&str)) -> Result<()>; // nixos-rebuild switch --rollback
```
Le chemin des binaires (`nix`, `git`, `nixos-rebuild`, `pkexec`, `sudo`, `notify-send`) se résout via `PATH` ; `/run/wrappers/bin` est ajouté au `PATH` des sous-processus.

### `upgrade` (mise à niveau automatique)
```rust
pub fn upgrade(repo: &Path, privilege: Privilege, out: &mut dyn FnMut(&str)) -> Result<UpgradeOutcome>;
pub enum UpgradeOutcome { AlreadyUpToDate, Applied(UpdateReport), RolledBack { report: UpdateReport, reason: String } }
```
Sauvegarde `flake.lock` en mémoire, `update`, `apply` si changement ; en cas d'échec d'`apply` : restaure le `flake.lock` d'origine puis `rollback`.

### `status` (agrégation pour GUI/CLI)
```rust
pub struct RepoStatus { pub path: PathBuf, pub role: Role, pub git: GitState,
                        pub pending: Option<Vec<InputChange>> /* None = pas encore vérifié */,
                        pub error: Option<String> }
pub fn status(path: &Path, role: Role, with_check: bool) -> RepoStatus;
```

### `waybar`
```rust
pub fn waybar_json(report: Result<&UpdateReport, &Error>) -> String; // {"text","tooltip","class"} compatible Waybar
```
Classes : `uptodate`, `outdated` (texte `❆ N`), `error` (texte `!`). Reprend le comportement de l'ancien script `nixos-check-flake-update`.

### `notify`
```rust
pub fn desktop(title: &str, body: &str, critical: bool); // notify-send, erreurs ignorées
```

### `daemon` (contrat démon / service root / GUI)
Constantes, types et mandataire D-Bus partagés (le démon lui-même est dans `crates/nixos-flakes/src/daemon/`). Voir « Démon » plus bas.
```rust
pub const BUS_NAME: &str;      // "io.github.untypequicode.NixosFlakes"
pub const OBJECT_PATH: &str;   // "/io/github/untypequicode/NixosFlakes"
pub const INTERFACE: &str;     // "io.github.untypequicode.NixosFlakes.Daemon"
pub fn status_file_path() -> PathBuf;   // `NIXOS_FLAKES_STATUS_FILE` ou /var/lib/nixos-flakes/status.json
pub struct UpgradeRecord { outcome: UpgradeOutcomeKind, finished_at: i64, message: String }
pub fn write_upgrade_record(path, &UpgradeRecord) -> Result<()>;   // atomique, mode 0644
pub fn read_upgrade_record(path) -> Option<UpgradeRecord>;
pub struct DaemonStatus { state: StateKind, updates: u32, message: Option<String>,
                          checked_at: Option<i64>, checking: bool, upgrade: Option<UpgradeRecord> }
impl DaemonStatus { fn combine(..) -> Self; fn to_json(&self) -> String; fn from_json(&str) -> Result<Self> }
#[zbus::proxy] trait Daemon { get_status(), check_now(), signal status_changed }   // DaemonProxy (async)
```

### `icon`
Rendu procédural de l'icône d'application (`app_rgba`) et du flocon coloré de la zone de notification (`flake_rgba`, `flake_argb`), partagé par la GUI et le démon.

## CLI `nixos-flakes`

```
nixos-flakes list [--all]                 dépôts (suivis par défaut ; --all = + détectés)
nixos-flakes scan                         met à jour le cache des dépôts détectés
nixos-flakes check [PATH] [--waybar]      défaut: dépôt système
nixos-flakes update [PATH] [--commit]     défaut: dépôt système
nixos-flakes apply [PATH] [--attr NOM]
nixos-flakes upgrade [PATH] [--attr NOM] [--notify]    mise à niveau automatique (utilisée par le timer systemd)
nixos-flakes sync [PATH] [--check]       fetch, avance/retard, puis pull --ff-only (--check : sans pull)
nixos-flakes track|untrack|ignore PATH    gestion des rôles
nixos-flakes set-system PATH [--attr NOM]   dépôt système (chemin relatif, ~, lien symbolique, espaces acceptés) et configuration nixosConfigurations.NOM
nixos-flakes daemon [--gui-command CMD]   démon de session (voir plus bas)
```
Codes de sortie : 0 ok, 1 erreur. `check --waybar` sort toujours 0 avec un JSON.

## Démon `nixos-flakes daemon`

Processus unique de fond de la session (service utilisateur systemd lié à `graphical-session.target`), indépendant de la GUI. Il :

- héberge l'icône d'état (`ksni`, StatusNotifierItem) : flocon vert (à jour ou inconnu), orange (mises à jour), rouge (erreur) ; menu : ligne d'état, « Vérifier maintenant », « Ouvrir l'interface » (seulement si `--gui-command` ou `NIXOS_FLAKES_GUI_COMMAND` est fourni, ce que fait le module Nix quand `gui.enable`) ; aucun « Quitter » ;
- vérifie le dépôt système et les dépôts suivis au démarrage, puis toutes les `check_interval_minutes` du `config.toml` (0 = jamais en périodique), à la demande, et dès que le fichier d'état de la mise à niveau change ;
- envoie une notification de bureau (`notify-send`, `core::notify`) quand une vérification révèle des mises à jour jamais signalées depuis son démarrage (la première vérification signale tout ce qui est en attente) ;
- surveille ce fichier d'état (sondage toutes les 5 s) : `nixos-flakes upgrade` (root) y écrit une `UpgradeRecord` (`outcome` : `up_to_date`, `applied`, `rolled_back`, `failed`). Une dernière mise à niveau en échec place l'état global en erreur jusqu'à la suivante ;
- garantit l'instance unique en réclamant sans file d'attente le nom de bus de session ci-dessous ; un second démon affiche un message et quitte avec le code 0.

### Interface D-Bus (bus de session)

| Élément | Valeur |
|---|---|
| Nom de bus | `io.github.untypequicode.NixosFlakes` |
| Chemin d'objet | `/io/github/untypequicode/NixosFlakes` |
| Interface | `io.github.untypequicode.NixosFlakes.Daemon` |
| Méthode `GetStatus() -> s` | état courant, JSON de `DaemonStatus` |
| Méthode `CheckNow() -> ()` | demande une vérification immédiate, retourne sans l'attendre (le résultat arrive par `StatusChanged`) |
| Signal `StatusChanged(s status)` | émis à chaque changement d'état (y compris `checking` qui passe à vrai puis faux) ; JSON de `DaemonStatus` |

JSON de `DaemonStatus` : `state` (`unknown`, `up_to_date`, `updates` ou `error`), `updates` (nombre d'inputs), `message` (détail d'erreur ou `null`), `checked_at` (Unix, secondes, ou `null`), `checking` (booléen), `upgrade` (`UpgradeRecord` : `outcome`, `finished_at`, `message`, ou `null`). Les clients Rust utilisent `nixos_flakes_core::daemon::DaemonProxy` ; si le nom de bus n'a pas de propriétaire, le démon n'est pas lancé.

## GUI `nixos-flakes-gui`

iced (Wayland/X11). Thème sombre/clair suivant le système, design moderne : barre latérale (Système / Suivis / Détectés / Ignorés) avec badges d'état, panneau de détail (diff par input, boutons Vérifier / Mettre à jour / Appliquer / Suivre / Ignorer), console de logs en direct, dialogue de commit (avant : modifs non commitées ; après : `flake.lock`), page Réglages, page Paquets (module `packages.rs` : état et plans de modification purs et testés, E/S bloquantes dans `iced::Task`, appels à `nixos-flakes-config` ; en `--demo`, données de `demo.rs` et écriture en mémoire seulement). Aucune icône ni option `--tray` : l'icône appartient au démon, fermer la fenêtre quitte la GUI. Lien au démon (`daemon_link.rs`, `DaemonProxy` sur le bus de session) : abonnement au signal `StatusChanged` (relecture de `GetStatus` toutes les 15 s pour détecter sa disparition, reconnexion toutes les 5 s), affichage de l'état dans la barre latérale et bouton « Vérifier maintenant » (`CheckNow`) ; démon absent = pas d'erreur, la GUI continue avec ses propres vérifications manuelles. Synchronisation (`sync.rs`, `core::git::{fetch,sync_status,pull_ff}`) du dépôt système : fetch à l'ouverture et sur demande uniquement, puis `pull --ff-only` refusé si arbre sale, historiques divergés ou pas d'amont. Terminal (`terminal.rs`, crate `iced_term` 0.8 sur `alacritty_terminal`, Rust pur donc sans dépendance native) : `$SHELL` lancé dans le dépôt système, touches non interceptées par les raccourcis de la GUI ; la GUI active la feature `tokio` d'iced pour piloter `zbus`. Les appels `core` se font dans des threads (`iced::Task::perform` + `std::thread`/canal pour les logs).

## Modules NixOS

Un module NixOS n'est pas un programme : c'est un fichier de configuration réutilisable qui déclare des options (`services.…`) et les traduit en services systemd, paquets installés, etc. Chaque outil a le sien, exposé par le flake sous `nixosModules.<outil>` : `nixos-flakes` (ci-dessous) et `image-downloader-visible` (extension Firefox). Il n'y a pas de module `default`, pour rester explicite. La GUI est un paquet, que le module `nixos-flakes` installe si `gui.enable = true`.

### nixosModules.nixos-flakes

Options à plat sous `services.nixos-flakes` :

- `enable` : démon + icône d'état (service utilisateur `nixos-flakes-daemon`, lié à `graphical-session.target`, restreint à `user` par `ConditionUser` s'il est défini) ;
- `interval` (`null` par défaut) : si défini (`OnCalendar`), active le service root + minuteur qui exécutent `nixos-flakes upgrade --notify` ; le service a un `StateDirectory` (`/var/lib/nixos-flakes`, lisible par tous) pour le fichier d'état ;
- `flakePath` (défaut `/etc/nixos` ; peut être n'importe où, y compris dans le dossier personnel de `user` ; écrit dans `/etc/nixos-flakes/config.toml`), `flakeAttr` (`null` : nom d'hôte ; passé au service par `--attr` et écrit dans ce fichier), `user` (utilisateur du démon et propriétaire de `flake.lock` après la mise à niveau) ;
- `gui.enable` : installe la GUI (nécessite `enable`) et la propose dans le menu de l'icône ;
- `systemRepo.{owner,path}` (`path` suit `flakePath` par défaut ; aucune ACL n'est posée quand `path` est dans le dossier personnel de `owner`, `safe.directory` l'est dès que `owner` ou `user` est défini), `githubTokenFile`. Le service root rend `flake.lock` et `.git/*` à leur propriétaire d'origine après chaque exécution.

Alias avec avertissement : `services.nixos-auto-upgrade-custom.interval` et `services.nixos-flakes.autoUpgrade.{interval,flakePath,user}` sont renommés vers `interval`, `flakePath`, `user`. Les anciens `…enable` sont acceptés avec un avertissement : `true` sans `interval` vaut `interval = "daily"`. Pas d'alias pour l'ancien `gui.tray`.
