//! Jeton d'accès GitHub (limite de débit de l'API pour `nix flake update`).
//!
//! Stocké dans `$XDG_CONFIG_HOME/nixos-flakes/github-token` (défaut
//! `~/.config`), fichier en 0600 dans un dossier en 0700. Jamais dans
//! `config.toml`, jamais en argument de commande, jamais journalisé : il est
//! transmis aux sous-processus `nix` uniquement par `NIX_CONFIG`.
//!
//! Si `NIXOS_FLAKES_GITHUB_TOKEN_FILE` est défini (option NixOS
//! `services.nixos-flakes.githubTokenFile`), le jeton est lu depuis ce fichier,
//! en lecture seule : l'interface ne peut ni l'enregistrer ni l'effacer.

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

/// Variable d'environnement désignant un fichier de jeton géré par Nix.
pub const ENV_FILE: &str = "NIXOS_FLAKES_GITHUB_TOKEN_FILE";

/// Origine du jeton.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Fichier configuré par Nix (lecture seule).
    Fichier(PathBuf),
    /// Fichier géré par l'interface (`~/.config/nixos-flakes/github-token`).
    Interface,
    /// Aucun jeton.
    Aucun,
}

/// Fichier de jeton imposé par l'environnement, s'il est défini et non vide.
fn env_file() -> Option<PathBuf> {
    std::env::var_os(ENV_FILE)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Origine du jeton courant.
pub fn source() -> Source {
    source_with(env_file(), path().ok().as_deref())
}

fn source_with(env: Option<PathBuf>, own: Option<&Path>) -> Source {
    match env {
        Some(p) => Source::Fichier(p),
        None if own.is_some_and(Path::is_file) => Source::Interface,
        None => Source::Aucun,
    }
}

/// Un jeton est-il disponible ? (fichier Nix présent, ou jeton de l'interface)
pub fn exists() -> bool {
    match env_file() {
        Some(p) => p.is_file(),
        None => path().is_ok_and(|p| p.is_file()),
    }
}

/// Lit le jeton ; `None` s'il est absent ou vide.
pub fn load() -> Result<Option<Token>> {
    match env_file() {
        Some(p) => load_file(&p),
        None => load_in(&default_dir()?),
    }
}

/// Enregistre le jeton (remplace l'existant).
pub fn save(token: &str) -> Result<()> {
    ensure_writable(env_file().as_deref())?;
    save_in(&default_dir()?, token)
}

/// Supprime le jeton (sans erreur s'il n'existe pas).
pub fn clear() -> Result<()> {
    ensure_writable(env_file().as_deref())?;
    clear_in(&default_dir()?)
}

fn ensure_writable(env: Option<&Path>) -> Result<()> {
    match env {
        Some(p) => Err(Error::Config(format!(
            "jeton configuré par la configuration Nix ({}) : lecture seule, \
             modifiez `services.nixos-flakes.githubTokenFile`",
            p.display()
        ))),
        None => Ok(()),
    }
}

/// Extrait le jeton d'un contenu : jeton brut, `github.com=JETON` ou
/// `access-tokens = github.com=JETON [autre.hote=…]`. Les lignes vides et les
/// commentaires (`#`) sont ignorés. Le contenu n'apparaît jamais dans les erreurs.
fn parse_token(raw: &str) -> Result<Option<Token>> {
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = match line.strip_prefix("access-tokens") {
            Some(rest) if rest.trim_start().starts_with('=') => rest.trim_start()[1..].trim(),
            _ => line,
        };
        for entry in line.split_whitespace() {
            match entry.split_once('=') {
                Some(("github.com", v)) => return validated(v).map(Some),
                Some(_) => {}
                None => return validated(entry).map(Some),
            }
        }
    }
    Ok(None)
}

fn load_file(file: &Path) -> Result<Option<Token>> {
    match fs::read_to_string(file) {
        Ok(s) => parse_token(&s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(file, e)),
    }
}

fn validated(token: &str) -> Result<Token> {
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
    Ok(Token(token.to_string()))
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
    let token = validated(token)?;
    let token = token.expose();
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
    fn parses_the_three_formats() {
        let ok = |raw: &str| parse_token(raw).unwrap().map(|t| t.expose().to_string());
        assert_eq!(ok("ghp_abc\n").as_deref(), Some("ghp_abc"));
        assert_eq!(ok("github.com=ghp_abc\n").as_deref(), Some("ghp_abc"));
        assert_eq!(
            ok("access-tokens = github.com=ghp_abc\n").as_deref(),
            Some("ghp_abc")
        );
        assert_eq!(
            ok("access-tokens=github.com=ghp_abc").as_deref(),
            Some("ghp_abc")
        );
        assert_eq!(
            ok("# note\n\naccess-tokens = gitlab.com=zzz github.com=ghp_abc\n").as_deref(),
            Some("ghp_abc")
        );
        assert_eq!(ok("  \n# rien\n"), None);
        assert_eq!(ok("access-tokens = gitlab.com=zzz"), None);
    }

    #[test]
    fn file_loading_and_invalid_content() {
        assert!(parse_token("github.com=")
            .unwrap_err()
            .to_string()
            .contains("vide"));
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("tok");
        assert!(load_file(&f).unwrap().is_none());
        fs::write(&f, "github.com=ghp_secret\n").unwrap();
        assert_eq!(load_file(&f).unwrap().unwrap().expose(), "ghp_secret");
    }

    #[test]
    fn nix_managed_file_is_read_only() {
        let p = PathBuf::from("/run/secrets/gh");
        let err = ensure_writable(Some(&p)).unwrap_err().to_string();
        assert!(err.contains("/run/secrets/gh") && err.contains("lecture seule"));
        assert!(ensure_writable(None).is_ok());
    }

    #[test]
    fn source_priority() {
        let d = tempfile::tempdir().unwrap();
        let own = d.path().join("github-token");
        assert_eq!(source_with(None, Some(&own)), Source::Aucun);
        fs::write(&own, "x").unwrap();
        assert_eq!(source_with(None, Some(&own)), Source::Interface);
        let nix = PathBuf::from("/run/secrets/gh");
        assert_eq!(
            source_with(Some(nix.clone()), Some(&own)),
            Source::Fichier(nix)
        );
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
