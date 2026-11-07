//! Jeton d'accès GitHub (limite de débit de l'API pour `nix flake update`).
//!
//! Stocké dans `$XDG_CONFIG_HOME/nixos-flakes/github-token` (défaut
//! `~/.config`), fichier en 0600 dans un dossier en 0700. Jamais dans
//! `config.toml`, jamais en argument de commande, jamais journalisé : il est
//! transmis aux sous-processus `nix` uniquement par `NIX_CONFIG`.

use crate::error::{Error, Result};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// Jeton secret : `Debug` et `Display` n'en révèlent jamais le contenu.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// Valeur en clair (à ne transmettre qu'à un sous-processus).
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(<masqué>)")
    }
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<jeton masqué>")
    }
}

/// Dossier de configuration de nixos-flakes.
fn default_dir() -> Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(x) if !x.is_empty() => PathBuf::from(x),
        _ => {
            let home = std::env::var_os("HOME")
                .filter(|h| !h.is_empty())
                .ok_or_else(|| Error::Config("HOME non défini".into()))?;
            PathBuf::from(home).join(".config")
        }
    };
    Ok(base.join("nixos-flakes"))
}

/// Chemin du fichier du jeton.
pub fn path() -> Result<PathBuf> {
    Ok(default_dir()?.join("github-token"))
}

/// Un jeton est-il enregistré ?
pub fn exists() -> bool {
    path().is_ok_and(|p| p.is_file())
}

/// Lit le jeton ; `None` s'il est absent ou vide.
pub fn load() -> Result<Option<Token>> {
    load_in(&default_dir()?)
}

/// Enregistre le jeton (remplace l'existant).
pub fn save(token: &str) -> Result<()> {
    save_in(&default_dir()?, token)
}

/// Supprime le jeton (sans erreur s'il n'existe pas).
pub fn clear() -> Result<()> {
    clear_in(&default_dir()?)
}

pub(crate) fn load_in(dir: &Path) -> Result<Option<Token>> {
    let file = dir.join("github-token");
    match fs::read_to_string(&file) {
        Ok(s) => {
            let t = s.trim();
            Ok((!t.is_empty()).then(|| Token(t.to_string())))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(file, e)),
    }
}

pub(crate) fn save_in(dir: &Path, token: &str) -> Result<()> {
    let token = token.trim();
    if token.is_empty() {
        return Err(Error::Config("jeton vide".into()));
    }
    // Le jeton finit dans `NIX_CONFIG` : pas d'espace ni de saut de ligne.
    if token.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Error::Config(
            "jeton invalide (espaces ou caractères de contrôle)".into(),
        ));
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| Error::io(dir, e))?;
    let tmp = dir.join(format!(".github-token.{}.tmp", std::process::id()));
    let dest = dir.join("github-token");
    let _ = fs::remove_file(&tmp);
    let res = (|| {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(token.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        fs::rename(&tmp, &dest)
    })();
    if let Err(e) = res {
        let _ = fs::remove_file(&tmp);
        return Err(Error::io(dest, e));
    }
    Ok(())
}

pub(crate) fn clear_in(dir: &Path) -> Result<()> {
    let file = dir.join("github-token");
    match fs::remove_file(&file) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(file, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn roundtrip_with_strict_permissions() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("nixos-flakes");
        assert!(load_in(&dir).unwrap().is_none());
        save_in(&dir, "  ghp_abc123 \n").unwrap();
        assert_eq!(load_in(&dir).unwrap().unwrap().expose(), "ghp_abc123");
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("github-token")), 0o600);
        // Remplacement : pas de fichier temporaire résiduel.
        save_in(&dir, "ghp_new").unwrap();
        assert_eq!(load_in(&dir).unwrap().unwrap().expose(), "ghp_new");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn clear_is_idempotent() {
        let d = tempfile::tempdir().unwrap();
        save_in(d.path(), "ghp_x").unwrap();
        clear_in(d.path()).unwrap();
        clear_in(d.path()).unwrap();
        assert!(load_in(d.path()).unwrap().is_none());
    }

    #[test]
    fn rejects_empty_and_multiline() {
        let d = tempfile::tempdir().unwrap();
        assert!(save_in(d.path(), "  ").is_err());
        assert!(save_in(d.path(), "a\nsubstituters = evil").is_err());
        assert!(save_in(d.path(), "a b").is_err());
        assert!(!d.path().join("github-token").exists());
    }

    #[test]
    fn debug_and_display_hide_the_secret() {
        let t = Token("ghp_secret".into());
        assert!(!format!("{t:?}").contains("ghp_secret"));
        assert!(!format!("{t}").contains("ghp_secret"));
        let err = save_in(Path::new("/proc/inexistant/x"), "ghp_secret").unwrap_err();
        assert!(!err.to_string().contains("ghp_secret"));
    }
}
