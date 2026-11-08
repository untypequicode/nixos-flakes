//! Exécution des sous-commandes et mise en forme de leur sortie.

use crate::cli::{Cli, Command};
use nixos_flakes_core::config::{Config, Role};
use nixos_flakes_core::daemon::{self as daemon_api, UpgradeOutcomeKind, UpgradeRecord};
use nixos_flakes_core::error::Error as CoreError;
use nixos_flakes_core::git::{self, GitState};
use nixos_flakes_core::lock::InputChange;
use nixos_flakes_core::nix::{self, Privilege, UpdateReport};
use nixos_flakes_core::scan;
use nixos_flakes_core::status::{self, RepoStatus};
use nixos_flakes_core::upgrade::{self, UpgradeOutcome};
use nixos_flakes_core::{notify, waybar};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Dépôt système de repli si la configuration n'en désigne aucun.
const DEFAULT_SYSTEM_REPO: &str = "/etc/nixos";

/// Exécute la commande demandée. `Err` = message d'erreur à afficher (code 1).
pub fn run(cli: Cli) -> Result<ExitCode, String> {
    let config_path = cli.config.unwrap_or_else(Config::default_path);

    // Waybar : toujours du JSON et code 0, même si la configuration est illisible.
    if let Command::Check { path, waybar: true } = &cli.command {
        return Ok(check_waybar(&config_path, path.as_deref()));
    }

    // Démon : relit lui-même la configuration à chaque vérification.
    if let Command::Daemon { gui_command } = &cli.command {
        return crate::daemon::run(gui_command.clone());
    }

    let mut config = Config::load(&config_path).map_err(|e| e.to_string())?;
    match cli.command {
        Command::List { all, check } => list(&config, all, check),
        Command::Scan => scan_cmd(&config),
        Command::Check { path, waybar: _ } => check(&config, path.as_deref()),
        Command::Update {
            path,
            commit,
            inputs,
        } => update(&config, path.as_deref(), commit, &inputs),
        Command::Apply { path } => apply(&config, path.as_deref()),
        Command::Upgrade { path, notify, sudo } => {
            upgrade_cmd(&config, path.as_deref(), notify, sudo)
        }
        Command::Sync { path, check } => sync_cmd(&config, path.as_deref(), check),
        Command::Daemon { .. } => unreachable!("traité plus haut"),
        Command::Pkg { command } => crate::pkg::run(command, &config),
        Command::Track { path } => set_role(&mut config, &config_path, &path, Role::Tracked),
        Command::Untrack { path } => set_role(&mut config, &config_path, &path, Role::Detected),
        Command::Ignore { path } => set_role(&mut config, &config_path, &path, Role::Ignored),
        Command::SetSystem { path } => set_role(&mut config, &config_path, &path, Role::System),
    }
}

/// Dépôt demandé, sinon dépôt système, sinon `/etc/nixos`.
pub(crate) fn resolve_repo(config: &Config, path: Option<&Path>) -> PathBuf {
    path.map(Path::to_path_buf)
        .or_else(|| config.system_repo.clone())
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SYSTEM_REPO))
}

/// `sync` : fetch, avance/retard, puis `pull --ff-only` (refusé si arbre sale ou historiques divergés).
fn sync_cmd(config: &Config, path: Option<&Path>, check_only: bool) -> Result<ExitCode, String> {
    let repo = resolve_repo(config, path);
    let err = |e: CoreError| e.to_string();
    git::fetch(&repo, &mut stream_to_stderr).map_err(err)?;
    let status = git::sync_status(&repo).map_err(err)?;
    println!(
        "{} : {}{}",
        repo.display(),
        status.summary(),
        status
            .upstream
            .as_deref()
            .map_or_else(String::new, |u| format!(" (amont : {u})"))
    );
    if check_only {
        return Ok(ExitCode::SUCCESS);
    }
    git::pull_ff(&repo, &mut stream_to_stderr).map_err(err)?;
    if status.behind > 0 {
        println!("Synchronisé : {} commit(s) récupéré(s).", status.behind);
    }
    Ok(ExitCode::SUCCESS)
}

fn stream_to_stderr(line: &str) {
    eprintln!("{line}");
}

fn check_waybar(config_path: &Path, path: Option<&Path>) -> ExitCode {
    let report: Result<UpdateReport, CoreError> =
        Config::load(config_path).and_then(|config| nix::check(&resolve_repo(&config, path)));
    println!("{}", waybar::waybar_json(report.as_ref()));
    ExitCode::SUCCESS
}

fn list(config: &Config, all: bool, check: bool) -> Result<ExitCode, String> {
    let mut repos: Vec<(PathBuf, Role)> = Vec::new();
    if let Some(system) = &config.system_repo {
        repos.push((system.clone(), Role::System));
    }
    for tracked in &config.tracked {
        if config.role(tracked) == Role::Tracked {
            repos.push((tracked.clone(), Role::Tracked));
        }
    }
    if all {
        for found in scan::scan(&config.scan_roots, config.scan_depth) {
            if config.role(&found) == Role::Detected {
                repos.push((found, Role::Detected));
            }
        }
    }

    if repos.is_empty() {
        println!("Aucun dépôt suivi. Lancez « nixos-flakes scan » pour voir les dépôts détectés.");
        return Ok(ExitCode::SUCCESS);
    }

    let rows: Vec<Vec<String>> = repos
        .iter()
        .map(|(path, role)| {
            let st = status::status(path, *role, check);
            vec![
                role_label(st.role).to_string(),
                st.path.display().to_string(),
                git_label(&st.git),
                pending_label(&st, check),
            ]
        })
        .collect();
    print!(
        "{}",
        format_table(&["Rôle", "Chemin", "Git", "Mises à jour"], &rows)
    );
    Ok(ExitCode::SUCCESS)
}

fn scan_cmd(config: &Config) -> Result<ExitCode, String> {
    let found = scan::scan(&config.scan_roots, config.scan_depth);
    if found.is_empty() {
        println!("Aucun dépôt flake trouvé.");
        return Ok(ExitCode::SUCCESS);
    }
    let rows: Vec<Vec<String>> = found
        .iter()
        .map(|p| {
            vec![
                role_label(config.role(p)).to_string(),
                p.display().to_string(),
            ]
        })
        .collect();
    print!("{}", format_table(&["Rôle", "Chemin"], &rows));
    Ok(ExitCode::SUCCESS)
}

fn check(config: &Config, path: Option<&Path>) -> Result<ExitCode, String> {
    let repo = resolve_repo(config, path);
    let report = nix::check(&repo).map_err(|e| e.to_string())?;
    println!("Dépôt : {}", repo.display());
    if report.changes.is_empty() {
        println!("Aucune mise à jour en attente.");
    } else {
        println!("{} input(s) à mettre à jour :", report.changes.len());
        for c in &report.changes {
            println!("  {}", change_line(c));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn update(
    config: &Config,
    path: Option<&Path>,
    commit: bool,
    inputs: &[String],
) -> Result<ExitCode, String> {
    let repo = resolve_repo(config, path);
    let report =
        nix::update_inputs(&repo, inputs, &mut stream_to_stderr).map_err(|e| e.to_string())?;
    print_summary(&repo, &report);

    if commit {
        if report.changes.is_empty() {
            println!("Aucun input n'a changé : rien à valider.");
        } else if git::state(&repo).map_err(|e| e.to_string())?.is_repo {
            let message = git::lock_commit_message(&report.changes);
            git::commit_lock(&repo, &message, &mut stream_to_stderr).map_err(|e| e.to_string())?;
            println!("flake.lock validé dans git.");
        } else {
            println!("Ce dépôt n'est pas un dépôt git : aucun commit.");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn apply(config: &Config, path: Option<&Path>) -> Result<ExitCode, String> {
    let repo = resolve_repo(config, path);
    warn_if_not_system(config, &repo);
    nix::apply(&repo, Privilege::Pkexec, &mut stream_to_stderr).map_err(|e| e.to_string())?;
    println!("Configuration appliquée depuis {}.", repo.display());
    Ok(ExitCode::SUCCESS)
}

fn upgrade_cmd(
    config: &Config,
    path: Option<&Path>,
    notify_desktop: bool,
    sudo: bool,
) -> Result<ExitCode, String> {
    let repo = resolve_repo(config, path);
    warn_if_not_system(config, &repo);
    let privilege = if sudo {
        Privilege::Sudo
    } else {
        Privilege::Pkexec
    };
    let notify_failure = |body: &str| {
        if notify_desktop {
            notify::desktop("Mise à niveau automatique échouée", body, true);
        }
    };

    // Mémorise l'issue pour le démon de session ; sans droit d'écriture, on l'ignore.
    let record = |outcome, message: String| {
        let record = UpgradeRecord {
            outcome,
            finished_at: daemon_api::now_unix(),
            message,
        };
        let _ = daemon_api::write_upgrade_record(&daemon_api::status_file_path(), &record);
    };

    match upgrade::upgrade(&repo, privilege, &mut stream_to_stderr) {
        Ok(UpgradeOutcome::AlreadyUpToDate) => {
            record(UpgradeOutcomeKind::UpToDate, "tout est à jour".into());
            println!("Tout est à jour.");
            Ok(ExitCode::SUCCESS)
        }
        Ok(UpgradeOutcome::Applied(report)) => {
            let body = upgrade_body(&report.changes);
            println!("Mise à niveau appliquée : {body}.");
            record(UpgradeOutcomeKind::Applied, body.clone());
            if notify_desktop {
                notify::desktop("Mise à niveau automatique réussie", &body, false);
            }
            Ok(ExitCode::SUCCESS)
        }
        Ok(UpgradeOutcome::RolledBack { report, reason }) => {
            let message = format!(
                "Échec de l'application ; retour à la génération précédente ({}).\n{reason}",
                upgrade_body(&report.changes)
            );
            eprintln!("{message}");
            record(UpgradeOutcomeKind::RolledBack, message.clone());
            notify_failure("Retour à la génération précédente effectué.");
            Ok(ExitCode::FAILURE)
        }
        Err(e) => {
            record(UpgradeOutcomeKind::Failed, e.to_string());
            notify_failure(&e.to_string());
            Err(e.to_string())
        }
    }
}

fn set_role(
    config: &mut Config,
    config_path: &Path,
    path: &Path,
    role: Role,
) -> Result<ExitCode, String> {
    if !path.join("flake.nix").is_file() {
        return Err(format!("pas de flake.nix dans {}", path.display()));
    }
    let previous_system = config.system_repo.clone();
    config.set_role(path, role);
    config.save(config_path).map_err(|e| e.to_string())?;

    println!("{} : {}.", path.display(), role_label(role));
    if role == Role::System {
        if let Some(old) = previous_system.filter(|old| old != path) {
            println!(
                "L'ancien dépôt système ({}) est devenu dépôt suivi.",
                old.display()
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn warn_if_not_system(config: &Config, repo: &Path) {
    if config.role(repo) != Role::System {
        eprintln!(
            "attention : {} n'est pas le dépôt système ; seule la mise à jour du dépôt système est censée être appliquée.",
            repo.display()
        );
    }
}

/// Affiche le résultat d'une mise à jour.
fn print_summary(repo: &Path, report: &UpdateReport) {
    if report.changes.is_empty() {
        println!(
            "Verrous de {} à jour : aucun input n'a changé.",
            repo.display()
        );
    } else {
        println!(
            "Verrous de {} mis à jour : {} input(s) changé(s) :",
            repo.display(),
            report.changes.len()
        );
        for c in &report.changes {
            println!("  {}", change_line(c));
        }
    }
}

/// Libellé français d'un rôle.
pub fn role_label(role: Role) -> &'static str {
    match role {
        Role::System => "système",
        Role::Tracked => "suivi",
        Role::Detected => "détecté",
        Role::Ignored => "ignoré",
    }
}

/// `propre`, `N modifié(s)` ou `hors git`, avec la branche si connue.
pub fn git_label(git: &GitState) -> String {
    if !git.is_repo {
        return "hors git".to_string();
    }
    let branch = git.branch.as_deref().unwrap_or("HEAD détachée");
    let modified = git.dirty.len() + usize::from(git.lock_dirty);
    if modified == 0 {
        format!("{branch}, propre")
    } else {
        format!("{branch}, {modified} modifié(s)")
    }
}

/// Colonne « Mises à jour » : nombre d'inputs en attente, erreur, ou tiret si non vérifié.
pub fn pending_label(st: &RepoStatus, checked: bool) -> String {
    match &st.pending {
        Some(changes) if changes.is_empty() => "à jour".to_string(),
        Some(changes) => format!("{} en attente", changes.len()),
        None if checked => match &st.error {
            Some(e) => format!("erreur : {}", e.lines().next().unwrap_or_default()),
            None => "—".to_string(),
        },
        None => "—".to_string(),
    }
}

/// Une ligne lisible décrivant le changement d'un input (révisions raccourcies).
pub fn change_line(c: &InputChange) -> String {
    match (&c.old_rev, &c.new_rev) {
        (None, Some(new)) => format!("{} : ajouté ({})", c.name, short_rev(new)),
        (Some(old), None) => format!("{} : retiré (était {})", c.name, short_rev(old)),
        (old, new) => format!(
            "{} : {} -> {}",
            c.name,
            old.as_deref().map_or("?".to_string(), short_rev),
            new.as_deref().map_or("?".to_string(), short_rev)
        ),
    }
}

fn short_rev(rev: &str) -> String {
    rev.chars().take(7).collect()
}

/// Noms des inputs changés, séparés par des virgules (pour les notifications).
pub fn upgrade_body(changes: &[InputChange]) -> String {
    match changes.len() {
        0 => "aucun input changé".to_string(),
        _ => changes
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// Tableau aligné : chaque colonne a la largeur de sa plus grande cellule.
pub fn format_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    let mut push_row = |cells: &[&str]| {
        let line: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|(cell, &w)| format!("{cell:<w$}"))
            .collect();
        out.push_str(line.join("  ").trim_end());
        out.push('\n');
    };
    push_row(headers);
    for row in rows {
        let cells: Vec<&str> = row.iter().map(String::as_str).collect();
        push_row(&cells);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(name: &str, old: Option<&str>, new: Option<&str>) -> InputChange {
        InputChange {
            name: name.into(),
            old_rev: old.map(Into::into),
            new_rev: new.map(Into::into),
            old_date: None,
            new_date: None,
        }
    }

    fn status_with(pending: Option<Vec<InputChange>>, error: Option<&str>) -> RepoStatus {
        RepoStatus {
            path: PathBuf::from("/r"),
            role: Role::Tracked,
            git: GitState::default(),
            pending,
            error: error.map(Into::into),
        }
    }

    #[test]
    fn role_labels() {
        assert_eq!(role_label(Role::System), "système");
        assert_eq!(role_label(Role::Tracked), "suivi");
        assert_eq!(role_label(Role::Detected), "détecté");
        assert_eq!(role_label(Role::Ignored), "ignoré");
    }

    #[test]
    fn git_labels() {
        let hors = GitState::default();
        assert_eq!(git_label(&hors), "hors git");
        let propre = GitState {
            is_repo: true,
            branch: Some("main".into()),
            ..GitState::default()
        };
        assert_eq!(git_label(&propre), "main, propre");
        let sale = GitState {
            is_repo: true,
            branch: None,
            dirty: vec!["a.nix".into()],
            lock_dirty: true,
        };
        assert_eq!(git_label(&sale), "HEAD détachée, 2 modifié(s)");
    }

    #[test]
    fn pending_labels() {
        assert_eq!(pending_label(&status_with(None, None), false), "—");
        assert_eq!(pending_label(&status_with(None, None), true), "—");
        assert_eq!(
            pending_label(&status_with(None, Some("boum\ndétail")), true),
            "erreur : boum"
        );
        assert_eq!(
            pending_label(&status_with(Some(vec![]), None), true),
            "à jour"
        );
        assert_eq!(
            pending_label(
                &status_with(Some(vec![change("a", None, None)]), None),
                true
            ),
            "1 en attente"
        );
    }

    #[test]
    fn change_lines() {
        assert_eq!(
            change_line(&change("nixpkgs", Some("abcdef0123"), Some("1234567890"))),
            "nixpkgs : abcdef0 -> 1234567"
        );
        assert_eq!(
            change_line(&change("neuf", None, Some("cafe"))),
            "neuf : ajouté (cafe)"
        );
        assert_eq!(
            change_line(&change("vieux", Some("cafe"), None)),
            "vieux : retiré (était cafe)"
        );
    }

    #[test]
    fn upgrade_body_lists_names() {
        assert_eq!(upgrade_body(&[]), "aucun input changé");
        let changes = [change("a", None, None), change("b", None, None)];
        assert_eq!(upgrade_body(&changes), "a, b");
    }

    #[test]
    fn table_is_aligned_and_trimmed() {
        let rows = vec![
            vec!["suivi".to_string(), "/a".to_string(), "x".to_string()],
            vec![
                "système".to_string(),
                "/longue/chemin".to_string(),
                "".to_string(),
            ],
        ];
        let text = format_table(&["Rôle", "Chemin", "Git"], &rows);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        // colonnes : Rôle = 7, Chemin = 14, Git = 3 ; séparateur = 2 espaces
        assert_eq!(
            lines[0],
            format!("Rôle{}Chemin{}Git", " ".repeat(5), " ".repeat(10))
        );
        assert_eq!(
            lines[1],
            format!("suivi{}/a{}x", " ".repeat(4), " ".repeat(14))
        );
        assert_eq!(lines[2], "système  /longue/chemin");
        assert!(lines.iter().all(|l| !l.ends_with(' ')));
    }

    #[test]
    fn resolve_repo_prefers_argument_then_system_then_default() {
        let mut config = Config {
            system_repo: None,
            ..Config::default()
        };
        assert_eq!(
            resolve_repo(&config, Some(Path::new("/x"))),
            PathBuf::from("/x")
        );
        assert_eq!(
            resolve_repo(&config, None),
            PathBuf::from(DEFAULT_SYSTEM_REPO)
        );
        config.system_repo = Some(PathBuf::from("/sys"));
        assert_eq!(resolve_repo(&config, None), PathBuf::from("/sys"));
    }
}
