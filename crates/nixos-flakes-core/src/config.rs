//! Configuration utilisateur (`config.toml`).

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Configuration de `nixos-flakes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Racines de scan (défaut : `~` et le dépôt système par défaut s'il est hors de `~`).
    pub scan_roots: Vec<PathBuf>,
    /// Profondeur maximale du scan (défaut : 4).
    pub scan_depth: u8,
    /// Dépôt système choisi par l'utilisateur. Absent : voir
    /// [`Config::system_repo_resolved`] (variable d'environnement, défaut système…).
    pub system_repo: Option<PathBuf>,
    /// Attribut `nixosConfigurations.<attr>` à appliquer (défaut : nom d'hôte).
    pub system_attr: Option<String>,
    /// Dépôts suivis (hors dépôt système).
    pub tracked: BTreeSet<PathBuf>,
    /// Dépôts ignorés définitivement.
    pub ignored: BTreeSet<PathBuf>,
    /// Intervalle de vérification, en minutes (défaut : 60).
    pub check_interval_minutes: u32,
}

/// Rôle d'un dépôt flake vis-à-vis de l'utilisateur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    System,
    Tracked,
    Detected,
    Ignored,
}

impl Default for Config {
    fn default() -> Self {
        let mut scan_roots = Vec::new();
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from);
        if let Some(home) = &home {
            scan_roots.push(home.clone());
        }
        // Le dépôt système par défaut est scanné s'il n'est pas déjà sous `~`.
        if let Some(sys) = default_system_repo() {
            if !home.as_deref().is_some_and(|h| sys.path.starts_with(h)) {
                scan_roots.push(sys.path);
            }
        }
        Config {
            scan_roots,
            scan_depth: 4,
            system_repo: None,
            system_attr: None,
            tracked: BTreeSet::new(),
            ignored: BTreeSet::new(),
            check_interval_minutes: 60,
        }
    }
}

/// Variable d'environnement désignant le dépôt système.
pub const ENV_SYSTEM_REPO: &str = "NIXOS_FLAKES_SYSTEM_REPO";
/// Variable d'environnement désignant l'attribut de configuration associé.
pub const ENV_SYSTEM_ATTR: &str = "NIXOS_FLAKES_SYSTEM_ATTR";
/// Variable d'environnement surchargeant le chemin du fichier de défauts système.
pub const ENV_SYSTEM_DEFAULTS: &str = "NIXOS_FLAKES_SYSTEM_DEFAULTS";
/// Fichier de défauts système, écrit par le module NixOS.
pub const SYSTEM_DEFAULTS_PATH: &str = "/etc/nixos-flakes/config.toml";
/// Emplacement historique de la configuration NixOS (dernier recours).
const LEGACY_REPO: &str = "/etc/nixos";

/// Piste affichée quand aucun dépôt système n'est défini.
pub const SYSTEM_REPO_HINT: &str = "définissez-le avec « nixos-flakes set-system <chemin> », ou avec `services.nixos-flakes.flakePath` dans la configuration NixOS";

/// Origine du dépôt système résolu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SystemRepoSource {
    /// `system_repo` de la configuration utilisateur.
    UserConfig,
    /// Variable `NIXOS_FLAKES_SYSTEM_REPO`.
    Env,
    /// Fichier de défauts système (`/etc/nixos-flakes/config.toml`).
    SystemDefault,
    /// `/etc/nixos`, présent et contenant `flake.nix`.
    Legacy,
}

impl SystemRepoSource {
    /// Libellé lisible de la provenance.
    pub fn label(self) -> &'static str {
        match self {
            SystemRepoSource::UserConfig => "configuration utilisateur",
            SystemRepoSource::Env => "variable d'environnement",
            SystemRepoSource::SystemDefault => "défaut système",
            SystemRepoSource::Legacy => "défaut historique",
        }
    }
}

/// Dépôt système résolu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemRepo {
    pub path: PathBuf,
    /// Attribut `nixosConfigurations.<attr>` ; `None` : nom d'hôte.
    pub attr: Option<String>,
    pub source: SystemRepoSource,
}

/// Contenu du fichier de défauts système (mêmes clés que `config.toml`).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SystemDefaults {
    system_repo: Option<PathBuf>,
    system_attr: Option<String>,
}

fn non_empty(v: Option<std::ffi::OsString>) -> Option<std::ffi::OsString> {
    v.filter(|s| !s.is_empty())
}

/// Résolution pure : (b) env, (c) fichier de défauts, (d) `legacy`.
fn resolve_defaults(
    env_repo: Option<PathBuf>,
    env_attr: Option<String>,
    defaults_file: &Path,
    legacy: &Path,
) -> Option<SystemRepo> {
    if let Some(path) = env_repo {
        return Some(SystemRepo {
            path,
            attr: env_attr,
            source: SystemRepoSource::Env,
        });
    }
    if let Ok(text) = std::fs::read_to_string(defaults_file) {
        if let Ok(d) = toml::from_str::<SystemDefaults>(&text) {
            if let Some(path) = d.system_repo.filter(|p| !p.as_os_str().is_empty()) {
                return Some(SystemRepo {
                    path,
                    attr: d.system_attr.filter(|a| !a.is_empty()),
                    source: SystemRepoSource::SystemDefault,
                });
            }
        }
    }
    legacy.join("flake.nix").is_file().then(|| SystemRepo {
        path: legacy.to_path_buf(),
        attr: None,
        source: SystemRepoSource::Legacy,
    })
}

/// Dépôt système par défaut, hors configuration utilisateur : variable
/// d'environnement, puis fichier de défauts système, puis `/etc/nixos`
/// (seulement s'il contient `flake.nix`).
pub fn default_system_repo() -> Option<SystemRepo> {
    let defaults = non_empty(std::env::var_os(ENV_SYSTEM_DEFAULTS))
        .map_or_else(|| PathBuf::from(SYSTEM_DEFAULTS_PATH), PathBuf::from);
    resolve_defaults(
        non_empty(std::env::var_os(ENV_SYSTEM_REPO)).map(PathBuf::from),
        non_empty(std::env::var_os(ENV_SYSTEM_ATTR)).and_then(|a| a.into_string().ok()),
        &defaults,
        Path::new(LEGACY_REPO),
    )
}

/// Valide un attribut `nixosConfigurations.<attr>` : caractères sûrs seulement
/// (lettres, chiffres, `-`, `_`, `.`), sans début en `-` ni `.`.
pub fn validate_attr(attr: &str) -> Result<()> {
    let ok = !attr.is_empty()
        && attr.len() <= 253
        && !attr.starts_with(['-', '.'])
        && attr
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(Error::Config(format!(
            "attribut de configuration invalide : « {attr} » (lettres, chiffres, `-`, `_` et `.` seulement)"
        )))
    }
}

/// Refuse un chemin de dépôt qui, concaténé en `<chemin>#<attr>`, changerait de sens pour
/// `nixos-rebuild`/`nix` : `#` (début de l'attribut) et `?` (paramètres de la référence).
pub fn validate_repo_path(repo: &Path) -> Result<()> {
    let s = repo.to_string_lossy();
    if s.contains(['#', '?']) {
        Err(Error::Config(format!(
            "chemin de dépôt invalide : « {s} » (les caractères `#` et `?` sont interdits dans une référence de flake)"
        )))
    } else {
        Ok(())
    }
}

/// Normalise un chemin saisi : `~` développé, relatif résolu par rapport au
/// dossier courant, liens symboliques résolus. Le chemin doit exister.
pub fn normalize_user_path(input: &str) -> Result<PathBuf> {
    let input = input.trim();
    if input.is_empty() {
        return Err(Error::Config("chemin vide".into()));
    }
    let home = || {
        std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| Error::Config("HOME n'est pas défini : `~` est inutilisable".into()))
    };
    let expanded = if input == "~" {
        home()?
    } else if let Some(rest) = input.strip_prefix("~/") {
        home()?.join(rest)
    } else {
        PathBuf::from(input)
    };
    let abs = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .map_err(|e| Error::io(".", e))?
            .join(expanded)
    };
    abs.canonicalize().map_err(|e| Error::io(&abs, e))
}

fn norm(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

fn contains(set: &BTreeSet<PathBuf>, repo: &Path) -> bool {
    set.contains(repo) || set.iter().any(|p| norm(p) == repo)
}

fn remove(set: &mut BTreeSet<PathBuf>, repo: &Path) {
    set.retain(|p| p != repo && norm(p) != repo);
}

impl Config {
    /// `$XDG_CONFIG_HOME/nixos-flakes/config.toml` (repli sur `~/.config`).
    /// Erreur si ni `XDG_CONFIG_HOME` ni `HOME` ne sont définis (jamais de
    /// repli sur un chemin relatif).
    pub fn default_path() -> Result<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(|h| PathBuf::from(h).join(".config"))
            })
            .ok_or_else(|| {
                Error::Config(
                    "impossible de localiser la configuration : ni XDG_CONFIG_HOME ni HOME ne sont définis"
                        .into(),
                )
            })?;
        Ok(base.join("nixos-flakes").join("config.toml"))
    }

    /// Charge la configuration ; fichier absent => valeurs par défaut.
    pub fn load(path: &Path) -> Result<Config> {
        if path.as_os_str().is_empty() {
            return Err(Error::Config("chemin de configuration vide".into()));
        }
        match std::fs::read_to_string(path) {
            Ok(s) => {
                toml::from_str(&s).map_err(|e| Error::Config(format!("{} : {e}", path.display())))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(Error::io(path, e)),
        }
    }

    /// Enregistre la configuration (crée les dossiers parents).
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        crate::fsutil::write_atomic(path, text.as_bytes(), None)
    }

    /// Dépôt système effectif, par priorité : configuration utilisateur,
    /// `NIXOS_FLAKES_SYSTEM_REPO`, fichier de défauts système, `/etc/nixos`
    /// (s'il contient `flake.nix`). `None` si rien n'est défini. L'attribut
    /// `system_attr` de l'utilisateur prime sur celui du niveau retenu.
    pub fn system_repo_resolved(&self) -> Option<SystemRepo> {
        let user_attr = self.system_attr.clone().filter(|a| !a.is_empty());
        match &self.system_repo {
            Some(path) => Some(SystemRepo {
                path: path.clone(),
                attr: user_attr,
                source: SystemRepoSource::UserConfig,
            }),
            None => default_system_repo().map(|mut r| {
                if user_attr.is_some() {
                    r.attr = user_attr;
                }
                r
            }),
        }
    }

    /// Comme [`Config::system_repo_resolved`], avec une erreur claire et une piste.
    pub fn require_system_repo(&self) -> Result<SystemRepo> {
        self.system_repo_resolved().ok_or_else(|| {
            Error::Config(format!("aucun dépôt système défini ({SYSTEM_REPO_HINT})"))
        })
    }

    /// Rôle d'un dépôt (par défaut : détecté). Les chemins sont comparés une
    /// fois canonicalisés : un lien (`/etc/nixos` → `~/nixos`) et sa cible
    /// désignent le même dépôt.
    pub fn role(&self, repo: &Path) -> Role {
        let repo = norm(repo);
        if self
            .system_repo_resolved()
            .is_some_and(|s| norm(&s.path) == repo)
        {
            Role::System
        } else if contains(&self.ignored, &repo) {
            Role::Ignored
        } else if contains(&self.tracked, &repo) {
            Role::Tracked
        } else {
            Role::Detected
        }
    }

    /// Change le rôle d'un dépôt. Le dépôt système est unique : l'ancien
    /// dépôt système devient suivi. Repasser un dépôt système en « détecté »
    /// retire le dépôt système.
    pub fn set_role(&mut self, repo: &Path, role: Role) {
        let repo = norm(repo);
        let was_system = self.system_repo.as_deref().is_some_and(|s| norm(s) == repo);
        if was_system {
            self.system_repo = None;
        }
        remove(&mut self.tracked, &repo);
        remove(&mut self.ignored, &repo);
        match role {
            Role::System => {
                if let Some(old) = self.system_repo.take() {
                    self.tracked.insert(norm(&old));
                }
                self.system_repo = Some(repo);
            }
            Role::Tracked => {
                self.tracked.insert(repo);
            }
            Role::Ignored => {
                self.ignored.insert(repo);
            }
            Role::Detected => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Config {
        Config {
            scan_roots: vec![],
            system_repo: None,
            system_attr: None,
            ..Config::default()
        }
    }

    fn write_defaults(dir: &Path, body: &str) -> PathBuf {
        let f = dir.join("defaults.toml");
        std::fs::write(&f, body).unwrap();
        f
    }

    #[test]
    fn resolution_priority_env_then_file_then_legacy() {
        let d = tempfile::tempdir().unwrap();
        let legacy = d.path().join("legacy");
        std::fs::create_dir(&legacy).unwrap();
        let none = d.path().join("absent.toml");
        // rien
        assert_eq!(resolve_defaults(None, None, &none, &legacy), None);
        // legacy sans flake.nix : ignoré ; avec : retenu
        std::fs::write(legacy.join("flake.nix"), "{}").unwrap();
        let r = resolve_defaults(None, None, &none, &legacy).unwrap();
        assert_eq!(r.source, SystemRepoSource::Legacy);
        assert_eq!(r.path, legacy);
        // fichier système > legacy
        let f = write_defaults(
            d.path(),
            "system_repo = \"/srv/nixos\"\nsystem_attr = \"laptop\"\n",
        );
        let r = resolve_defaults(None, None, &f, &legacy).unwrap();
        assert_eq!(r.source, SystemRepoSource::SystemDefault);
        assert_eq!(r.path, PathBuf::from("/srv/nixos"));
        assert_eq!(r.attr.as_deref(), Some("laptop"));
        // env > fichier système
        let r = resolve_defaults(Some("/env/repo".into()), Some("x".into()), &f, &legacy).unwrap();
        assert_eq!(r.source, SystemRepoSource::Env);
        assert_eq!(r.path, PathBuf::from("/env/repo"));
        assert_eq!(r.attr.as_deref(), Some("x"));
        // fichier illisible : on retombe sur legacy
        let bad = write_defaults(d.path(), "system_repo = [");
        let r = resolve_defaults(None, None, &bad, &legacy).unwrap();
        assert_eq!(r.source, SystemRepoSource::Legacy);
    }

    #[test]
    fn user_config_wins_and_attr_override() {
        let mut c = empty();
        c.system_repo = Some(PathBuf::from("/home/u/nixos"));
        c.system_attr = Some("desk".into());
        let r = c.system_repo_resolved().unwrap();
        assert_eq!(r.source, SystemRepoSource::UserConfig);
        assert_eq!(r.path, PathBuf::from("/home/u/nixos"));
        assert_eq!(r.attr.as_deref(), Some("desk"));
        assert!(c.require_system_repo().is_ok());
    }

    #[test]
    fn repo_path_validation() {
        assert!(validate_repo_path(Path::new("/srv/nixos")).is_ok());
        assert!(validate_repo_path(Path::new("/srv/a#b")).is_err());
        assert!(validate_repo_path(Path::new("/srv/a?b")).is_err());
    }

    #[test]
    fn attr_validation() {
        for ok in ["host", "my-host_1", "a.b"] {
            assert!(validate_attr(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-x", ".x", "a b", "a#b", "a;b", "a/b", "é", "$(x)"] {
            assert!(validate_attr(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn normalize_resolves_relative_and_symlinks() {
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("dossier avec espaces");
        std::fs::create_dir(&real).unwrap();
        let link = d.path().join("lien");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let canon = real.canonicalize().unwrap();
        assert_eq!(normalize_user_path(link.to_str().unwrap()).unwrap(), canon);
        assert_eq!(normalize_user_path(real.to_str().unwrap()).unwrap(), canon);
        assert!(normalize_user_path("").is_err());
        assert!(normalize_user_path(d.path().join("nope").to_str().unwrap()).is_err());
        // relatif : « . » = dossier courant
        assert_eq!(
            normalize_user_path(".").unwrap(),
            std::env::current_dir().unwrap().canonicalize().unwrap()
        );
    }

    #[test]
    fn system_role_through_symlink() {
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("nixos");
        std::fs::create_dir(&real).unwrap();
        let link = d.path().join("etc-nixos");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut cfg = empty();
        cfg.system_repo = Some(link.clone());
        assert_eq!(cfg.role(&real), Role::System);
        assert_eq!(cfg.role(&link), Role::System);
    }

    #[test]
    fn defaults() {
        let c = Config::default();
        assert_eq!(c.scan_depth, 4);
        assert_eq!(c.check_interval_minutes, 60);
        assert!(c.system_repo.is_none() && c.system_attr.is_none());
    }

    #[test]
    fn missing_file_gives_default() {
        let d = tempfile::tempdir().unwrap();
        let c = Config::load(&d.path().join("nope.toml")).unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn roundtrip_creates_parents() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a/b/config.toml");
        let mut c = empty();
        c.tracked.insert(PathBuf::from("/x/y"));
        c.system_repo = Some(PathBuf::from("/s"));
        c.save(&p).unwrap();
        assert_eq!(Config::load(&p).unwrap(), c);
    }

    #[test]
    fn partial_file_uses_defaults() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("c.toml");
        std::fs::write(&p, "scan_depth = 2\n").unwrap();
        let c = Config::load(&p).unwrap();
        assert_eq!(c.scan_depth, 2);
        assert_eq!(c.check_interval_minutes, 60);
    }

    #[test]
    fn invalid_file_is_error() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("c.toml");
        std::fs::write(&p, "scan_depth = [").unwrap();
        assert!(matches!(Config::load(&p), Err(Error::Config(_))));
    }

    #[test]
    fn roles() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let b = d.path().join("b");
        let c = d.path().join("c");
        for p in [&a, &b, &c] {
            std::fs::create_dir(p).unwrap();
        }
        let mut cfg = empty();
        assert_eq!(cfg.role(&a), Role::Detected);
        cfg.set_role(&a, Role::Tracked);
        assert_eq!(cfg.role(&a), Role::Tracked);
        cfg.set_role(&a, Role::Ignored);
        assert_eq!(cfg.role(&a), Role::Ignored);
        assert!(cfg.tracked.is_empty());
        cfg.set_role(&a, Role::System);
        assert_eq!(cfg.role(&a), Role::System);
        assert!(cfg.ignored.is_empty());
        // le système est unique : l'ancien devient suivi
        cfg.set_role(&b, Role::System);
        assert_eq!(cfg.role(&b), Role::System);
        assert_eq!(cfg.role(&a), Role::Tracked);
        // retour à détecté
        cfg.set_role(&b, Role::Detected);
        assert_eq!(cfg.role(&b), Role::Detected);
        assert_eq!(cfg.system_repo, None);
        cfg.set_role(&c, Role::Tracked);
        cfg.set_role(&c, Role::Detected);
        assert_eq!(cfg.role(&c), Role::Detected);
    }

    #[test]
    fn role_works_through_symlink() {
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = d.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut cfg = empty();
        cfg.set_role(&link, Role::Tracked);
        assert_eq!(cfg.role(&real), Role::Tracked);
    }
}
