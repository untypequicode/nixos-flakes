# Architecture

`nixos-flakes` est un recueil d'outils Nix/NixOS écrits en Rust. Le vocabulaire est défini dans [`GLOSSARY.md`](../GLOSSARY.md) : l'utiliser tel quel dans le code, les messages et l'UI (français).

## Disposition

```
Cargo.toml                    workspace (members = crates/*)
crates/
  nixos-flakes-core/          lib : toute la logique (synchrone, sans async)
  nixos-flakes/               bin `nixos-flakes` : CLI (clap)
  nixos-flakes-gui/           bin `nixos-flakes-gui` : GUI iced + tray optionnel
  <futur-outil>/              un crate par nouvel outil
modules/                      modules NixOS (un fichier par outil)
nix/                          packaging (auto-découverte des crates)
docs/                         documentation
```

Règle : un crate binaire de `crates/` devient automatiquement un `package` et une `app` du flake. Ajouter un outil = `cargo new crates/mon-outil`.

## Contrat de `nixos-flakes-core`

Tout est synchrone. Les commandes longues prennent un callback `&mut dyn FnMut(&str)` qui reçoit chaque ligne de sortie (stdout+stderr fusionnés) ; la GUI les appelle depuis un thread. Erreurs : `thiserror` dans un type `Error` unique + `type Result<T>`. Chaque module est testé avec des dépôts temporaires (`tempfile`).

### `config`
```rust
pub struct Config {
    pub scan_roots: Vec<PathBuf>,        // défaut: [~, /etc/nixos]
    pub scan_depth: u8,                  // défaut: 4
    pub system_repo: Option<PathBuf>,    // défaut: /etc/nixos si présent
    pub tracked: BTreeSet<PathBuf>,      // dépôts suivis (hors système)
    pub ignored: BTreeSet<PathBuf>,      // ignorés définitivement
    pub tray_enabled: bool,              // défaut: false
    pub check_interval_minutes: u32,     // défaut: 60
}
pub enum Role { System, Tracked, Detected, Ignored }
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

## CLI `nixos-flakes`

```
nixos-flakes list [--all]                 dépôts (suivis par défaut ; --all = + détectés)
nixos-flakes scan                         met à jour le cache des dépôts détectés
nixos-flakes check [PATH] [--waybar]      défaut: dépôt système
nixos-flakes update [PATH] [--commit]     défaut: dépôt système
nixos-flakes apply [PATH]
nixos-flakes upgrade [PATH] [--notify]    mise à niveau automatique (utilisée par le timer systemd)
nixos-flakes track|untrack|ignore PATH    gestion des rôles
nixos-flakes set-system PATH
```
Codes de sortie : 0 ok, 1 erreur. `check --waybar` sort toujours 0 avec un JSON.

## GUI `nixos-flakes-gui`

iced (Wayland/X11). Thème sombre/clair suivant le système, design moderne : barre latérale (Système / Suivis / Détectés / Ignorés) avec badges d'état, panneau de détail (diff par input, boutons Vérifier / Mettre à jour / Appliquer / Suivre / Ignorer), console de logs en direct, dialogue de commit (avant : modifs non commitées ; après : `flake.lock`), page Réglages. Tray optionnel (`ksni`) : états à jour / mises à jour / erreur, menu Ouvrir / Vérifier / Quitter ; fermer la fenêtre réduit dans le tray si activé. Les appels `core` se font dans des threads (`iced::Task::perform` + `std::thread`/canal pour les logs).

## Module NixOS

`services.nixos-flakes.autoUpgrade.{enable,interval,flakePath}` (service oneshot + timer, exécute `nixos-flakes upgrade --notify`), `services.nixos-flakes.gui.{enable,tray}` (installe la GUI, autostart tray). Alias `mkRenamedOptionModule` depuis `services.nixos-auto-upgrade-custom`.
