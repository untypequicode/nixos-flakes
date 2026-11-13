//! `nixos-flakes doctor` : diagnostic de l'installation.

use crate::dbus;
use crate::ui::{paint, Style};
use nixos_flakes_core::config::{self, Config, SystemRepo, SystemRepoSource, SYSTEM_REPO_HINT};
use nixos_flakes_core::token::{self, Source};
use std::path::{Path, PathBuf};

/// Gravité d'une ligne du diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

/// Une ligne du diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub level: Level,
    pub name: String,
    pub detail: String,
    pub hint: Option<String>,
}

impl Check {
    fn new(level: Level, name: &str, detail: impl Into<String>, hint: Option<&str>) -> Self {
        Check {
            level,
            name: name.to_string(),
            detail: detail.into(),
            hint: hint.map(str::to_string),
        }
    }
}

/// Dossiers où chercher un programme : `PATH` puis les emplacements NixOS usuels.
fn search_dirs(path_var: Option<&str>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = path_var
        .unwrap_or_default()
        .split(':')
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .collect();
    for extra in ["/run/wrappers/bin", "/run/current-system/sw/bin"] {
        let p = PathBuf::from(extra);
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    }
    dirs
}

/// Premier exécutable nommé `name` dans `dirs`.
fn find_program(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    dirs.iter().map(|d| d.join(name)).find(|p| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

fn tool_check(name: &str, dirs: &[PathBuf], missing: Level, hint: &str) -> Check {
    match find_program(name, dirs) {
        Some(p) => Check::new(Level::Ok, name, p.display().to_string(), None),
        None => Check::new(missing, name, "introuvable", Some(hint)),
    }
}

fn token_check(source: &Source) -> Check {
    let name = "jeton GitHub";
    match source {
        Source::Fichier(p) => Check::new(
            Level::Ok,
            name,
            format!("fichier géré par Nix ({})", p.display()),
            None,
        ),
        Source::Interface => Check::new(Level::Ok, name, "enregistré dans l'interface", None),
        Source::Systeme {
            path,
            lisible: true,
        } => Check::new(
            Level::Ok,
            name,
            format!("fichier système ({})", path.display()),
            None,
        ),
        Source::Systeme {
            path,
            lisible: false,
        } => Check::new(
            Level::Warn,
            name,
            format!("{} existe mais n'est pas lisible", path.display()),
            Some("rendez le fichier lisible (chmod 640, groupe users) ou enregistrez un jeton dans l'interface."),
        ),
        Source::Aucun => Check::new(
            Level::Warn,
            name,
            "absent (limite de requêtes GitHub anonyme)",
            Some("enregistrez un jeton dans l'interface, ou configurez access-tokens dans nix.conf."),
        ),
    }
}

/// Contexte des contrôles du dépôt système (injectable pour les tests).
struct RepoCtx {
    /// Utilisateur courant.
    uid: u32,
    hostname: Option<String>,
}

impl RepoCtx {
    fn current() -> Self {
        use std::os::unix::fs::MetadataExt;
        RepoCtx {
            uid: std::fs::metadata("/proc/self").map_or(u32::MAX, |m| m.uid()),
            hostname: std::fs::read_to_string("/proc/sys/kernel/hostname")
                .ok()
                .map(|h| h.trim().to_string())
                .filter(|h| !h.is_empty()),
        }
    }
}

/// Nom d'utilisateur pour un uid (`/etc/passwd`), sinon l'uid.
fn user_name(uid: u32) -> String {
    std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|p| {
            p.lines().find_map(|l| {
                let mut f = l.split(':');
                let name = f.next()?;
                let id: u32 = f.nth(1)?.parse().ok()?;
                (id == uid).then(|| name.to_string())
            })
        })
        .unwrap_or_else(|| format!("uid {uid}"))
}

/// Contrôles du dépôt système résolu (`system`), du défaut système hors
/// configuration utilisateur (`default`) et de leur cohérence.
fn repo_checks(
    system: Option<&SystemRepo>,
    default: Option<&SystemRepo>,
    ctx: &RepoCtx,
) -> Vec<Check> {
    use std::os::unix::fs::MetadataExt;
    let Some(sys) = system else {
        return vec![Check::new(
            Level::Warn,
            "dépôt système",
            "non défini",
            Some(SYSTEM_REPO_HINT),
        )];
    };
    let repo = sys.path.as_path();
    if !repo.is_dir() {
        return vec![Check::new(
            Level::Fail,
            "dépôt système",
            format!(
                "{} n'existe pas ({})",
                repo.display(),
                sys.source.label()
            ),
            Some("corrigez-le avec « nixos-flakes set-system <dossier> » (ou `services.nixos-flakes.flakePath`)."),
        )];
    }
    let attr_note = sys
        .attr
        .as_deref()
        .map_or_else(String::new, |a| format!(", configuration {a}"));
    let mut v = vec![Check::new(
        Level::Ok,
        "dépôt système",
        format!("{} ({}{attr_note})", repo.display(), sys.source.label()),
        None,
    )];
    let flake = repo.join("flake.nix");
    if flake.is_file() {
        v.push(Check::new(
            Level::Ok,
            "flake du dépôt système",
            "flake.nix présent",
            None,
        ));
    } else {
        v.push(Check::new(
            Level::Fail,
            "flake du dépôt système",
            format!("pas de flake.nix dans {}", repo.display()),
            Some("le dépôt système doit être un flake (flake.nix à sa racine)."),
        ));
    }

    // Git : facultatif, mais nécessaire à `sync` et `update --commit`.
    match nixos_flakes_core::git::state(repo) {
        Ok(g) if g.is_repo => v.push(Check::new(
            Level::Ok,
            "git du dépôt système",
            format!(
                "dépôt git ({})",
                g.branch.as_deref().unwrap_or("HEAD détachée")
            ),
            None,
        )),
        Ok(_) => v.push(Check::new(
            Level::Warn,
            "git du dépôt système",
            "pas un dépôt git : « sync » et « update --commit » sont indisponibles",
            Some("facultatif ; « git init » dans le dossier active le suivi."),
        )),
        Err(e) => v.push(Check::new(
            Level::Warn,
            "git du dépôt système",
            e.to_string(),
            None,
        )),
    }

    // Propriétaire : conditionne l'élévation de privilèges.
    if let Ok(meta) = std::fs::metadata(repo) {
        let owner = meta.uid();
        if owner == ctx.uid || ctx.uid == 0 {
            v.push(Check::new(
                Level::Ok,
                "propriétaire du dépôt",
                if owner == ctx.uid {
                    "votre utilisateur : mise à jour, validation et synchronisation sans élévation (seul « nixos-rebuild switch » élève)".to_string()
                } else {
                    format!("{} (vous êtes root)", user_name(owner))
                },
                None,
            ));
        } else {
            v.push(Check::new(
                Level::Warn,
                "propriétaire du dépôt",
                format!(
                    "{} (pas votre utilisateur) : git et nix peuvent refuser le dépôt, et l'écriture de flake.lock demande une élévation",
                    user_name(owner)
                ),
                Some(format!(
                    "ajoutez `services.nixos-flakes.systemRepo.owner = \"<utilisateur>\";` (avec `flakePath`/`systemRepo.path` = {}) ou changez le propriétaire : sudo chown -R $USER {}",
                    repo.display(),
                    repo.display()
                ).as_str()),
            ));
        }
    }

    // Configuration nixosConfigurations.<attr ou nom d'hôte> : repérée dans flake.nix
    // (une évaluation de nix serait lente et peut télécharger les inputs).
    let wanted = sys.attr.clone().or_else(|| ctx.hostname.clone());
    if let Some(name) = wanted {
        let text = std::fs::read_to_string(&flake).unwrap_or_default();
        if text.contains(&name) {
            v.push(Check::new(
                Level::Ok,
                "configuration NixOS",
                format!("« {name} » mentionnée dans flake.nix"),
                None,
            ));
        } else if flake.is_file() {
            v.push(Check::new(
                Level::Warn,
                "configuration NixOS",
                format!("« {name} » introuvable dans flake.nix (non vérifiable sans évaluer le flake)"),
                Some(format!(
                    "vérifiez avec : nix eval {}#nixosConfigurations --apply builtins.attrNames ; sinon indiquez la bonne avec « nixos-flakes set-system {} --attr <nom> ».",
                    repo.display(),
                    repo.display()
                ).as_str()),
            ));
        }
    }

    // Cohérence entre la configuration utilisateur et le défaut système.
    if sys.source == SystemRepoSource::UserConfig {
        if let Some(def) = default {
            let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
            if canon(&def.path) != canon(&sys.path) {
                v.push(Check::new(
                    Level::Warn,
                    "cohérence du dépôt système",
                    format!(
                        "la configuration utilisateur ({}) diffère du défaut {} ({}) : la CLI et l'interface utilisent le premier, la mise à niveau automatique le second",
                        sys.path.display(),
                        def.source.label(),
                        def.path.display()
                    ),
                    Some(format!(
                        "alignez-les : « nixos-flakes set-system {} » ou `services.nixos-flakes.flakePath`.",
                        def.path.display()
                    ).as_str()),
                ));
            } else if def.attr.is_some() && sys.attr != def.attr {
                v.push(Check::new(
                    Level::Warn,
                    "cohérence du dépôt système",
                    format!(
                        "la configuration utilisée diffère du défaut système ({})",
                        def.attr.as_deref().unwrap_or_default()
                    ),
                    Some("alignez `system_attr` (ou « set-system --attr ») et `services.nixos-flakes.flakeAttr`."),
                ));
            }
        }
    }
    v
}

/// Exécute tous les contrôles.
pub fn run(config_path: &Path) -> Vec<Check> {
    let dirs = search_dirs(std::env::var("PATH").ok().as_deref());
    let mut checks = vec![
        tool_check(
            "nix",
            &dirs,
            Level::Fail,
            "installez Nix (https://nixos.org/download).",
        ),
        tool_check(
            "git",
            &dirs,
            Level::Fail,
            "installez git (nécessaire à la lecture des flakes et à la synchronisation).",
        ),
    ];
    let has_pkexec = find_program("pkexec", &dirs).is_some();
    let has_sudo = find_program("sudo", &dirs).is_some();
    checks.push(match (has_pkexec, has_sudo) {
        (true, true) => Check::new(Level::Ok, "pkexec / sudo", "pkexec et sudo présents", None),
        (true, false) => Check::new(
            Level::Ok,
            "pkexec / sudo",
            "pkexec présent (sudo absent : « --sudo » indisponible)",
            None,
        ),
        (false, true) => Check::new(
            Level::Warn,
            "pkexec / sudo",
            "sudo présent, pkexec absent",
            Some("utilisez « --sudo » avec apply/upgrade, ou installez polkit."),
        ),
        (false, false) => Check::new(
            Level::Fail,
            "pkexec / sudo",
            "ni pkexec ni sudo",
            Some(
                "l'élévation de privilèges est nécessaire à « apply » : installez polkit ou sudo.",
            ),
        ),
    });
    checks.push(tool_check(
        "notify-send",
        &dirs,
        Level::Warn,
        "installez libnotify pour les notifications de bureau (facultatif).",
    ));

    match Config::load(config_path) {
        Ok(config) => {
            checks.push(Check::new(
                Level::Ok,
                "configuration",
                format!("{} lisible", config_path.display()),
                None,
            ));
            checks.extend(repo_checks(
                config.system_repo_resolved().as_ref(),
                config::default_system_repo().as_ref(),
                &RepoCtx::current(),
            ));
        }
        Err(e) => checks.push(Check::new(
            Level::Fail,
            "configuration",
            e.to_string(),
            Some("corrigez ou supprimez le fichier de configuration (--config pour en choisir un autre)."),
        )),
    }

    checks.push(token_check(&token::source()));

    checks.push(match dbus::query_daemon() {
        Ok(_) => Check::new(Level::Ok, "démon", "joignable sur D-Bus", None),
        Err(e) => Check::new(
            Level::Warn,
            "démon",
            e,
            Some("lancez « nixos-flakes daemon » (ou activez le service de session du module NixOS)."),
        ),
    });
    checks
}

/// Vrai si une ligne est en échec (code de sortie 1).
pub fn has_failure(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.level == Level::Fail)
}

/// Texte du diagnostic ; `quiet` ne garde que les avertissements et échecs.
pub fn render(checks: &[Check], color: bool, quiet: bool) -> String {
    let mut out = String::new();
    for c in checks {
        if quiet && c.level == Level::Ok {
            continue;
        }
        let (mark, style) = match c.level {
            Level::Ok => ("✓", Style::Green),
            Level::Warn => ("⚠", Style::Yellow),
            Level::Fail => ("✗", Style::Red),
        };
        out.push_str(&format!(
            "{} {} : {}\n",
            paint(mark, style, color),
            c.name,
            c.detail
        ));
        if let Some(h) = &c.hint {
            if c.level != Level::Ok {
                out.push_str(&format!("    piste : {h}\n"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_programs_in_given_dirs() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("outil");
        std::fs::write(&f, "#!/bin/sh\n").unwrap();
        let dirs = vec![d.path().to_path_buf()];
        assert_eq!(find_program("outil", &dirs), None, "non exécutable");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find_program("outil", &dirs), Some(f));
        assert_eq!(find_program("absent", &dirs), None);
    }

    fn sys(path: &Path, source: SystemRepoSource, attr: Option<&str>) -> SystemRepo {
        SystemRepo {
            path: path.to_path_buf(),
            attr: attr.map(Into::into),
            source,
        }
    }

    fn ctx(uid: u32) -> RepoCtx {
        RepoCtx {
            uid,
            hostname: Some("machine".into()),
        }
    }

    fn my_uid() -> u32 {
        RepoCtx::current().uid
    }

    #[test]
    fn repo_checks_levels() {
        let none = repo_checks(None, None, &ctx(1000));
        assert_eq!(none[0].level, Level::Warn);
        assert!(none[0].hint.as_deref().unwrap().contains("set-system"));
        let d = tempfile::tempdir().unwrap();
        let missing = sys(&d.path().join("x"), SystemRepoSource::UserConfig, None);
        assert_eq!(
            repo_checks(Some(&missing), None, &ctx(1000))[0].level,
            Level::Fail
        );
        let here = sys(d.path(), SystemRepoSource::UserConfig, None);
        let c = repo_checks(Some(&here), None, &ctx(my_uid()));
        assert_eq!(c[1].level, Level::Fail, "pas de flake.nix");
        std::fs::write(
            d.path().join("flake.nix"),
            "{ nixosConfigurations.machine = 1; }",
        )
        .unwrap();
        let c = repo_checks(Some(&here), None, &ctx(my_uid()));
        assert!(!has_failure(&c), "{c:?}");
        // propriétaire = utilisateur courant : pas d'avertissement de droits ; git absent = ⚠ informatif
        assert!(c
            .iter()
            .any(|x| x.name == "propriétaire du dépôt" && x.level == Level::Ok));
        assert!(c
            .iter()
            .any(|x| x.name == "configuration NixOS" && x.level == Level::Ok));
    }

    #[test]
    fn repo_checks_foreign_owner_and_missing_attr_warn() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("flake.nix"), "{}").unwrap();
        let here = sys(d.path(), SystemRepoSource::Env, Some("laptop"));
        // un autre uid que le propriétaire réel et non root
        let other = if my_uid() == 4242 { 4243 } else { 4242 };
        let c = repo_checks(Some(&here), None, &ctx(other));
        let owner = c
            .iter()
            .find(|x| x.name == "propriétaire du dépôt")
            .unwrap();
        assert_eq!(owner.level, Level::Warn);
        assert!(owner
            .hint
            .as_deref()
            .unwrap()
            .contains(&d.path().display().to_string()));
        let cfg = c.iter().find(|x| x.name == "configuration NixOS").unwrap();
        assert_eq!(cfg.level, Level::Warn);
        assert!(cfg.detail.contains("laptop"));
    }

    #[test]
    fn repo_checks_flag_user_vs_default_mismatch() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        for d in [&a, &b] {
            std::fs::write(d.path().join("flake.nix"), "machine").unwrap();
        }
        let user = sys(a.path(), SystemRepoSource::UserConfig, None);
        let def = sys(b.path(), SystemRepoSource::SystemDefault, None);
        let c = repo_checks(Some(&user), Some(&def), &ctx(my_uid()));
        assert!(c
            .iter()
            .any(|x| x.name == "cohérence du dépôt système" && x.level == Level::Warn));
        let same = sys(a.path(), SystemRepoSource::SystemDefault, None);
        let c = repo_checks(Some(&user), Some(&same), &ctx(my_uid()));
        assert!(!c.iter().any(|x| x.name == "cohérence du dépôt système"));
    }

    #[test]
    fn render_marks_and_hints() {
        let checks = vec![
            Check::new(Level::Ok, "a", "bon", None),
            Check::new(Level::Fail, "b", "cassé", Some("réparez")),
        ];
        let text = render(&checks, false, false);
        assert!(text.contains("✓ a : bon"));
        assert!(text.contains("✗ b : cassé\n    piste : réparez"));
        assert!(!render(&checks, false, true).contains("✓"));
        assert!(has_failure(&checks));
    }

    #[test]
    fn token_levels() {
        assert_eq!(token_check(&Source::Aucun).level, Level::Warn);
        assert_eq!(token_check(&Source::Interface).level, Level::Ok);
    }
}
