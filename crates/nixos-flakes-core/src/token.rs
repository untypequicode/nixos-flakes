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
//!
//! Ordre de priorité : fichier Nix (`Fichier`) > jeton de l'interface >
//! fichier système `/etc/nix/access-tokens.conf` (`Systeme`, lecture seule,
//! chemin surchargeable par `NIXOS_FLAKES_SYSTEM_TOKEN_FILE`). Ce dernier est
//! souvent en root 600 : `nix` lancé par l'utilisateur ne le lit pas et son
//! jeton est alors ignoré (limite de 60 requêtes/h) ; `Source::Systeme
//! { lisible: false }` permet de le signaler.

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
    /// Fichier système `/etc/nix/access-tokens.conf` (lecture seule), utilisé
    /// quand ni le fichier Nix ni l'interface ne fournissent de jeton.
    /// `lisible: false` : le fichier existe mais l'utilisateur courant ne peut
    /// pas le lire (souvent root 600) ; `nix` lancé en utilisateur l'ignore donc
    /// aussi. Remède : le rendre lisible (`chmod 640`, groupe `users`) ou
    /// enregistrer un jeton dans l'interface.
    Systeme { path: PathBuf, lisible: bool },
    /// Aucun jeton.
    Aucun,
}

/// Variable d'environnement surchargeant le chemin du fichier système (tests).
pub const ENV_SYSTEM_FILE: &str = "NIXOS_FLAKES_SYSTEM_TOKEN_FILE";

/// Chemin par défaut du fichier de jetons système de Nix.
pub const DEFAULT_SYSTEM_FILE: &str = "/etc/nix/access-tokens.conf";

/// Chemin du fichier de jetons système.
pub fn system_path() -> PathBuf {
    std::env::var_os(ENV_SYSTEM_FILE)
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from(DEFAULT_SYSTEM_FILE), PathBuf::from)
}

/// État du fichier système : `None` s'il n'existe pas (ou ne contient aucun
/// jeton GitHub lisible) ; `Some(Systeme { .. })` sinon.
fn probe_system(path: &Path) -> Option<Source> {
    match fs::read_to_string(path) {
        Ok(s) => match parse_system(&s) {
            Ok(Some(_)) => Some(Source::Systeme {
                path: path.to_path_buf(),
                lisible: true,
            }),
            _ => None,
        },
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Some(Source::Systeme {
            path: path.to_path_buf(),
            lisible: false,
        }),
        Err(_) => None,
    }
}

/// Jeton du fichier système, s'il est lisible et en contient un.
fn load_system(path: &Path) -> Option<Token> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| parse_system(&s).ok().flatten())
}

/// Analyse stricte d'un fichier de configuration Nix : seules les lignes
/// `access-tokens`/`extra-access-tokens` comptent (pas de jeton brut).
fn parse_system(raw: &str) -> Result<Option<Token>> {
    let kept: Vec<&str> = raw
        .lines()
        .map(str::trim)
        .filter(|l| {
            [ACCESS_TOKENS, EXTRA_ACCESS_TOKENS].iter().any(|k| {
                l.strip_prefix(k)
                    .is_some_and(|r| r.trim_start().starts_with('='))
            })
        })
        .collect();
    parse_token(&kept.join("\n"))
}

const ACCESS_TOKENS: &str = "access-tokens";
const EXTRA_ACCESS_TOKENS: &str = "extra-access-tokens";

/// Fichier de jeton imposé par l'environnement, s'il est défini et non vide.
fn env_file() -> Option<PathBuf> {
    std::env::var_os(ENV_FILE)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Origine du jeton courant.
pub fn source() -> Source {
    source_with(env_file(), path().ok().as_deref(), &system_path())
}

/// Le fichier du jeton de l'interface existe-t-il et contient-il un jeton (non vide) ?
/// Cohérent avec `load_in` : un fichier vide ne compte pas comme jeton.
fn has_own_token(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|s| !s.trim().is_empty())
}

fn source_with(env: Option<PathBuf>, own: Option<&Path>, system: &Path) -> Source {
    match env {
        Some(p) => Source::Fichier(p),
        None if own.is_some_and(has_own_token) => Source::Interface,
        None => probe_system(system).unwrap_or(Source::Aucun),
    }
}

/// Un jeton utilisable est-il disponible ? (fichier Nix présent, jeton de
/// l'interface, ou fichier système lisible contenant un jeton GitHub)
pub fn exists() -> bool {
    match source() {
        Source::Fichier(p) => load_file(&p).is_ok_and(|t| t.is_some()),
        Source::Interface => true,
        Source::Systeme { lisible, .. } => lisible,
        Source::Aucun => false,
    }
}

/// Le fichier de jetons système existe-t-il sans être lisible par l'utilisateur
/// courant (et sans jeton prioritaire) ? Dans ce cas `nix` ignore son jeton.
pub fn system_unreadable() -> Option<PathBuf> {
    match source() {
        Source::Systeme {
            path,
            lisible: false,
        } => Some(path),
        _ => None,
    }
}

/// Lit le jeton ; `None` s'il est absent ou vide.
pub fn load() -> Result<Option<Token>> {
    match env_file() {
        Some(p) => load_file(&p),
        // HOME/XDG absents : pas de jeton d'interface, mais le fichier système reste utilisable.
        None => match default_dir() {
            Ok(dir) => match load_in(&dir)? {
                Some(t) => Ok(Some(t)),
                None => Ok(load_system(&system_path())),
            },
            Err(_) => Ok(load_system(&system_path())),
        },
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
        let line = [EXTRA_ACCESS_TOKENS, ACCESS_TOKENS]
            .iter()
            .find_map(|k| {
                line.strip_prefix(k)
                    .filter(|rest| rest.trim_start().starts_with('='))
                    .map(|rest| rest.trim_start()[1..].trim())
            })
            .unwrap_or(line);
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
            if t.is_empty() {
                Ok(None)
            } else {
                // Même validation qu'à l'enregistrement (pas de saut de ligne dans NIX_CONFIG).
                validated(t).map(Some)
            }
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
    fn empty_token_file_is_not_a_token() {
        let d = tempfile::tempdir().unwrap();
        let own = d.path().join("github-token");
        let sys = d.path().join("access-tokens.conf");
        fs::write(&own, "  \n").unwrap();
        assert!(load_in(d.path()).unwrap().is_none());
        // Un fichier vide ne masque ni ne simule un jeton : source() retombe sur le système.
        assert_eq!(source_with(None, Some(&own), &sys), Source::Aucun);
        fs::write(&sys, "access-tokens = github.com=ghp_sys\n").unwrap();
        assert!(matches!(
            source_with(None, Some(&own), &sys),
            Source::Systeme { lisible: true, .. }
        ));
        fs::write(&own, "ghp_x\n").unwrap();
        assert_eq!(source_with(None, Some(&own), &sys), Source::Interface);
    }

    #[test]
    fn load_validates_like_save() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("github-token"), "a\nsubstituters = evil\n").unwrap();
        assert!(load_in(d.path()).is_err());
        fs::write(d.path().join("github-token"), "a b\n").unwrap();
        assert!(load_in(d.path()).is_err());
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
        let sys = d.path().join("access-tokens.conf");
        assert_eq!(source_with(None, Some(&own), &sys), Source::Aucun);
        fs::write(&sys, "extra-access-tokens = github.com=ghp_sys\n").unwrap();
        assert_eq!(
            source_with(None, Some(&own), &sys),
            Source::Systeme {
                path: sys.clone(),
                lisible: true
            }
        );
        fs::write(&own, "x").unwrap();
        assert_eq!(source_with(None, Some(&own), &sys), Source::Interface);
        let nix = PathBuf::from("/run/secrets/gh");
        assert_eq!(
            source_with(Some(nix.clone()), Some(&own), &sys),
            Source::Fichier(nix)
        );
    }

    #[test]
    fn parses_extra_access_tokens_and_multiple_hosts() {
        let ok = |raw: &str| parse_token(raw).unwrap().map(|t| t.expose().to_string());
        assert_eq!(
            ok("extra-access-tokens = github.com=ghp_x\n").as_deref(),
            Some("ghp_x")
        );
        assert_eq!(
            ok("extra-access-tokens=gitlab.com=zz github.com=ghp_y other=1").as_deref(),
            Some("ghp_y")
        );
        assert_eq!(ok("extra-access-tokens = gitlab.com=zz"), None);
    }

    #[test]
    fn system_file_is_parsed_strictly() {
        let ok = |raw: &str| parse_system(raw).unwrap().map(|t| t.expose().to_string());
        assert_eq!(
            ok("substituters = https://c\nextra-access-tokens = github.com=ghp_s\n").as_deref(),
            Some("ghp_s")
        );
        // Pas de jeton brut ni de réglage étranger pris pour un jeton.
        assert_eq!(ok("substituters = https://c\n"), None);
        assert_eq!(ok("ghp_brut\n"), None);
    }

    #[test]
    fn system_file_states() {
        let d = tempfile::tempdir().unwrap();
        let sys = d.path().join("access-tokens.conf");
        assert_eq!(probe_system(&sys), None);
        assert!(load_system(&sys).is_none());
        fs::write(&sys, "extra-access-tokens = github.com=ghp_sys\n").unwrap();
        assert_eq!(load_system(&sys).unwrap().expose(), "ghp_sys");
        fs::set_permissions(&sys, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&sys).is_ok() {
            return; // root : les permissions ne s'appliquent pas
        }
        assert_eq!(
            probe_system(&sys),
            Some(Source::Systeme {
                path: sys.clone(),
                lisible: false
            })
        );
        assert!(load_system(&sys).is_none());
        // Le jeton de l'interface reste prioritaire sur un fichier illisible.
        let own = d.path().join("github-token");
        fs::write(&own, "x").unwrap();
        assert_eq!(source_with(None, Some(&own), &sys), Source::Interface);
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
