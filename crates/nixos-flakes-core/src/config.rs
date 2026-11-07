//! Configuration utilisateur (`config.toml`).

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Configuration de `nixos-flakes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Racines de scan (défaut : `~` et `/etc/nixos`).
    pub scan_roots: Vec<PathBuf>,
    /// Profondeur maximale du scan (défaut : 4).
    pub scan_depth: u8,
    /// Dépôt système (défaut : `/etc/nixos` si présent).
    pub system_repo: Option<PathBuf>,
    /// Dépôts suivis (hors dépôt système).
    pub tracked: BTreeSet<PathBuf>,
    /// Dépôts ignorés définitivement.
    pub ignored: BTreeSet<PathBuf>,
    /// Icône de zone de notification activée (défaut : non).
    pub tray_enabled: bool,
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
        if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
            scan_roots.push(PathBuf::from(home));
        }
        scan_roots.push(PathBuf::from("/etc/nixos"));
        let etc = Path::new("/etc/nixos");
        Config {
            scan_roots,
            scan_depth: 4,
            system_repo: etc.is_dir().then(|| etc.to_path_buf()),
            tracked: BTreeSet::new(),
            ignored: BTreeSet::new(),
            tray_enabled: false,
            check_interval_minutes: 60,
        }
    }
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
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(|h| PathBuf::from(h).join(".config"))
            })
            .unwrap_or_else(|| PathBuf::from(".config"));
        base.join("nixos-flakes").join("config.toml")
    }

    /// Charge la configuration ; fichier absent => valeurs par défaut.
    pub fn load(path: &Path) -> Result<Config> {
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
        std::fs::write(path, text).map_err(|e| Error::io(path, e))
    }

    /// Rôle d'un dépôt (par défaut : détecté).
    pub fn role(&self, repo: &Path) -> Role {
        let repo = norm(repo);
        if self.system_repo.as_deref().is_some_and(|s| norm(s) == repo) {
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
            ..Config::default()
        }
    }

    #[test]
    fn defaults() {
        let c = Config::default();
        assert_eq!(c.scan_depth, 4);
        assert_eq!(c.check_interval_minutes, 60);
        assert!(!c.tray_enabled);
        assert!(c.scan_roots.contains(&PathBuf::from("/etc/nixos")));
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
        c.tray_enabled = true;
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
