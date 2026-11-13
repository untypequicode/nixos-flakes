//! Exécution des sous-commandes et mise en forme de leur sortie.

use crate::cli::{Cli, Command, PkgCommand};
use crate::failure::{Failure, EXIT_CANCELLED, EXIT_UPDATES};
use crate::report::{self, DaemonJson, RepoJson, ScanJson, StatusJson};
use crate::ui::{self, format_table, format_table_styled, info, progress, Style};
use crate::{dbus, doctor};
use nixos_flakes_core::config::{self, Config, Role, SYSTEM_REPO_HINT};
use nixos_flakes_core::daemon::{self as daemon_api, UpgradeOutcomeKind, UpgradeRecord};
use nixos_flakes_core::error::Error as CoreError;
use nixos_flakes_core::git::{self, GitState};
use nixos_flakes_core::lock::InputChange;
use nixos_flakes_core::nix::{self, Privilege, UpdateReport};
use nixos_flakes_core::scan;
use nixos_flakes_core::status::{self, RepoStatus};
use nixos_flakes_core::upgrade::{self, UpgradeOutcome};
use nixos_flakes_core::{notify, waybar};
use std::io::{BufRead, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Résultat d'une commande.
pub type CmdResult = Result<ExitCode, Failure>;

/// Exécute la commande demandée.
pub fn run(cli: Cli) -> CmdResult {
    ui::set_quiet(cli.quiet);

    // Sans configuration : complétions et recherche de paquets (même si la config est cassée).
    match &cli.command {
        Command::Completions { shell } => {
            let mut cmd = crate::cli::command();
            clap_complete::generate(*shell, &mut cmd, "nixos-flakes", &mut std::io::stdout());
            return Ok(ExitCode::SUCCESS);
        }
        Command::Pkg {
            command: PkgCommand::Search { query, limit },
        } => return crate::pkg::search(query, *limit),
        _ => {}
    }

    // Les commandes qui ÉCRIVENT la configuration exigent un emplacement utilisateur ; les autres
    // (lecture seule, dont `upgrade` lancé par un service root sans HOME) retombent sur les
    // défauts système (Config::load d'un fichier absent donne les valeurs par défaut).
    let writes_config = matches!(
        cli.command,
        Command::Track { .. }
            | Command::Untrack { .. }
            | Command::Ignore { .. }
            | Command::SetSystem { .. }
    );
    let config_path = resolve_config_path(cli.config, writes_config)?;

    // Waybar : toujours du JSON et code 0, même si la configuration est illisible.
    if let Command::Check {
        path, waybar: true, ..
    } = &cli.command
    {
        return Ok(check_waybar(&config_path, path.as_deref()));
    }

    // Démon : relit lui-même la configuration (celle de `--config`) à chaque vérification.
    if let Command::Daemon { gui_command } = &cli.command {
        return Ok(crate::daemon::run(gui_command.clone(), config_path)?);
    }

    // Diagnostic : la configuration illisible est un résultat, pas une erreur fatale.
    if let Command::Doctor = &cli.command {
        return Ok(doctor_cmd(&config_path));
    }

    let mut config = Config::load(&config_path)?;
    match cli.command {
        Command::List { all, check, json } => list(&config, all, check, json),
        Command::Scan { json } => scan_cmd(&config, json),
        Command::Check {
            path,
            waybar: _,
            exit_code,
        } => check(&config, path.as_deref(), exit_code),
        Command::Update {
            path,
            commit,
            inputs,
            dry_run,
        } => update(&config, path.as_deref(), commit, &inputs, dry_run),
        Command::Apply { path, attr, yes } => apply(&config, path.as_deref(), attr.as_deref(), yes),
        Command::Upgrade {
            path,
            attr,
            notify,
            sudo,
            yes,
        } => upgrade_cmd(&config, path.as_deref(), attr.as_deref(), notify, sudo, yes),
        Command::Status { path, json } => status_cmd(&config, &config_path, path.as_deref(), json),
        Command::Sync { path, check } => sync_cmd(&config, path.as_deref(), check),
        Command::Pkg { command } => crate::pkg::run(command, &config),
        Command::Track { path } => set_role(&mut config, &config_path, &path, Role::Tracked),
        Command::Untrack { path } => set_role(&mut config, &config_path, &path, Role::Detected),
        Command::Ignore { path } => set_role(&mut config, &config_path, &path, Role::Ignored),
        Command::SetSystem { path, attr } => {
            set_system(&mut config, &config_path, &path, attr.as_deref())
        }
        Command::Daemon { .. } | Command::Doctor | Command::Completions { .. } => {
            unreachable!("traité plus haut")
        }
    }
}

/// Fichier de configuration : `--config`, sinon l'emplacement utilisateur. Sans HOME ni
/// XDG_CONFIG_HOME, c'est une erreur seulement si `writes` ; en lecture on utilise le fichier de
/// défauts système (`/etc/nixos-flakes/config.toml`, ou `NIXOS_FLAKES_SYSTEM_DEFAULTS`).
pub(crate) fn resolve_config_path(
    explicit: Option<PathBuf>,
    writes: bool,
) -> Result<PathBuf, Failure> {
    match explicit {
        Some(p) => Ok(p),
        None => match Config::default_path() {
            Ok(p) => Ok(p),
            Err(e) if writes => Err(Failure::from(e)),
            Err(_) => Ok(std::env::var_os(config::ENV_SYSTEM_DEFAULTS)
                .filter(|v| !v.is_empty())
                .map_or_else(
                    || PathBuf::from(config::SYSTEM_DEFAULTS_PATH),
                    PathBuf::from,
                )),
        },
    }
}

/// Échec « aucun dépôt système » avec la piste de configuration.
pub(crate) fn no_system_repo() -> Failure {
    Failure::new("aucun dépôt système défini").with_hint(SYSTEM_REPO_HINT)
}

/// Dépôt demandé, sinon dépôt système résolu (configuration, environnement,
/// défaut système, `/etc/nixos`) ; erreur claire si aucun n'existe.
pub(crate) fn resolve_repo(config: &Config, path: Option<&Path>) -> Result<PathBuf, Failure> {
    match path {
        Some(p) => Ok(p.to_path_buf()),
        None => config
            .system_repo_resolved()
            .map(|r| r.path)
            .ok_or_else(no_system_repo),
    }
}

/// Attribut `nixosConfigurations` à utiliser : celui demandé, sinon celui du
/// dépôt système (seulement si `repo` est ce dépôt), sinon aucun (nom d'hôte).
pub(crate) fn resolve_attr(
    config: &Config,
    repo: &Path,
    requested: Option<&str>,
) -> Option<String> {
    if let Some(a) = requested {
        return Some(a.to_string());
    }
    if config.role(repo) == Role::System {
        return config.system_repo_resolved().and_then(|r| r.attr);
    }
    None
}

/// Affiche un objet sérialisable en JSON indenté.
fn print_json<T: serde::Serialize>(value: &T) -> CmdResult {
    let text =
        serde_json::to_string_pretty(value).map_err(|e| Failure::new(format!("JSON : {e}")))?;
    println!("{text}");
    Ok(ExitCode::SUCCESS)
}

/// Pose la question sur un terminal ; `Ok(true)` sans question si `yes` ou si l'entrée
/// n'est pas un terminal (minuteur systemd, scripts : comportement inchangé).
fn confirm_action(yes: bool, question: &str) -> bool {
    if yes || !std::io::stdin().is_terminal() {
        return true;
    }
    confirm_from(question, &mut std::io::stdin().lock())
}

fn confirm_from(question: &str, input: &mut dyn BufRead) -> bool {
    eprint!("{question} [o/N] ");
    let mut line = String::new();
    input.read_line(&mut line).is_ok() && crate::pkg::is_yes(&line)
}

fn cancelled() -> ExitCode {
    println!("Annulé : rien n'a été fait.");
    ExitCode::from(EXIT_CANCELLED)
}

/// `sync` : fetch, avance/retard, puis `pull --ff-only` (refusé si arbre sale ou historiques divergés).
fn sync_cmd(config: &Config, path: Option<&Path>, check_only: bool) -> CmdResult {
    let repo = resolve_repo(config, path)?;
    git::fetch(&repo, &mut stream_to_stderr)?;
    let status = git::sync_status(&repo)?;
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
    git::pull_ff(&repo, &mut stream_to_stderr)?;
    if status.behind > 0 {
        info(&format!(
            "Synchronisé : {} commit(s) récupéré(s).",
            status.behind
        ));
    }
    Ok(ExitCode::SUCCESS)
}

fn stream_to_stderr(line: &str) {
    progress(line);
}

fn check_waybar(config_path: &Path, path: Option<&Path>) -> ExitCode {
    let report: Result<UpdateReport, CoreError> = Config::load(config_path).and_then(|config| {
        let repo = resolve_repo(&config, path).map_err(|f| CoreError::Config(f.message))?;
        nix::check(&repo)
    });
    println!("{}", waybar::waybar_json(report.as_ref()));
    ExitCode::SUCCESS
}

/// Dépôts à lister : système, suivis, et détectés si `all`.
fn list_repos(config: &Config, all: bool) -> Vec<(PathBuf, Role)> {
    let mut repos: Vec<(PathBuf, Role)> = Vec::new();
    // Toujours présent, même hors des racines de scan.
    if let Some(system) = config.system_repo_resolved() {
        repos.push((system.path, Role::System));
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
    repos
}

/// Couleur d'une cellule du tableau des dépôts (colonnes : rôle, chemin, git, mises à jour).
fn repo_cell_style(col: usize, cell: &str) -> Option<Style> {
    match col {
        0 if cell == role_label(Role::System) => Some(Style::Bold),
        2 if cell == "hors git" => Some(Style::Dim),
        2 if cell.ends_with("propre") => Some(Style::Green),
        2 => Some(Style::Yellow),
        3 if cell == "à jour" => Some(Style::Green),
        3 if cell.ends_with("en attente") => Some(Style::Yellow),
        3 if cell.starts_with("erreur") => Some(Style::Red),
        3 => Some(Style::Dim),
        _ => None,
    }
}

fn list(config: &Config, all: bool, check: bool, json: bool) -> CmdResult {
    let repos = list_repos(config, all);
    let statuses: Vec<RepoStatus> = repos
        .iter()
        .map(|(path, role)| status::status(path, *role, check))
        .collect();

    if json {
        let items: Vec<RepoJson> = statuses.iter().map(RepoJson::from).collect();
        return print_json(&items);
    }
    if repos.is_empty() {
        info("Aucun dépôt suivi. Lancez « nixos-flakes scan » pour voir les dépôts détectés.");
        return Ok(ExitCode::SUCCESS);
    }
    let rows: Vec<Vec<String>> = statuses
        .iter()
        .map(|st| {
            vec![
                role_label(st.role).to_string(),
                st.path.display().to_string(),
                git_label(&st.git),
                pending_label(st, check),
            ]
        })
        .collect();
    print!(
        "{}",
        format_table_styled(
            &["Rôle", "Chemin", "Git", "Mises à jour"],
            &rows,
            &repo_cell_style,
            ui::stdout_color()
        )
    );
    Ok(ExitCode::SUCCESS)
}

fn scan_cmd(config: &Config, json: bool) -> CmdResult {
    let found = scan::scan(&config.scan_roots, config.scan_depth);
    if json {
        let items: Vec<ScanJson> = found
            .iter()
            .map(|p| ScanJson {
                path: p.display().to_string(),
                role: report::role_key(config.role(p)),
            })
            .collect();
        return print_json(&items);
    }
    if found.is_empty() {
        info("Aucun dépôt flake trouvé.");
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

fn check(config: &Config, path: Option<&Path>, exit_code: bool) -> CmdResult {
    let repo = resolve_repo(config, path)?;
    let report = nix::check(&repo)?;
    println!("Dépôt : {}", repo.display());
    if report.changes.is_empty() {
        println!("Aucune mise à jour en attente.");
    } else {
        println!("{} input(s) à mettre à jour :", report.changes.len());
        for c in &report.changes {
            println!("  {}", change_line(c));
        }
    }
    if exit_code && !report.changes.is_empty() {
        return Ok(ExitCode::from(EXIT_UPDATES));
    }
    Ok(ExitCode::SUCCESS)
}

fn update(
    config: &Config,
    path: Option<&Path>,
    commit: bool,
    inputs: &[String],
    dry_run: bool,
) -> CmdResult {
    let repo = resolve_repo(config, path)?;
    if dry_run {
        return update_dry_run(&repo, inputs, commit);
    }
    let report = nix::update_inputs(&repo, inputs, &mut stream_to_stderr)?;
    print_summary(&repo, &report);

    if commit {
        if report.changes.is_empty() {
            info("Aucun input n'a changé : rien à valider.");
        } else if git::state(&repo)?.is_repo {
            let message = git::lock_commit_message(&report.changes);
            git::commit_lock(&repo, &message, &mut stream_to_stderr)?;
            info("flake.lock validé dans git.");
        } else {
            info("Ce dépôt n'est pas un dépôt git : aucun commit.");
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `update --dry-run` : réutilise la simulation de `nix::check`, sans rien écrire.
fn update_dry_run(repo: &Path, inputs: &[String], commit: bool) -> CmdResult {
    if !inputs.is_empty() {
        nix::validate_inputs(repo, inputs)?;
    }
    let mut report = nix::check(repo)?;
    if !inputs.is_empty() {
        report.changes.retain(|c| inputs.contains(&c.name));
    }
    println!("Simulation (--dry-run) pour {} :", repo.display());
    if report.changes.is_empty() {
        println!("Aucun input ne changerait.");
    } else {
        println!("{} input(s) changeraient :", report.changes.len());
        for c in &report.changes {
            println!("  {}", change_line(c));
        }
    }
    if commit {
        info("--commit ignoré en simulation.");
    }
    info("Rien n'a été écrit.");
    Ok(ExitCode::SUCCESS)
}

fn apply(config: &Config, path: Option<&Path>, attr: Option<&str>, yes: bool) -> CmdResult {
    let repo = resolve_repo(config, path)?;
    let attr = resolve_attr(config, &repo, attr);
    warn_if_not_system(config, &repo);
    let target = nix::flake_ref(&repo, attr.as_deref());
    if !confirm_action(yes, &format!("Appliquer la configuration de {target} ?")) {
        return Ok(cancelled());
    }
    nix::apply(
        &repo,
        attr.as_deref(),
        Privilege::Pkexec,
        &mut stream_to_stderr,
    )?;
    info(&format!(
        "Configuration appliquée depuis {}.",
        repo.display()
    ));
    Ok(ExitCode::SUCCESS)
}

/// Mémorise l'issue pour le démon de session ; un échec d'écriture est signalé, pas ignoré.
fn write_record(outcome: UpgradeOutcomeKind, message: String) {
    let record = UpgradeRecord {
        outcome,
        finished_at: daemon_api::now_unix(),
        message,
    };
    let path = daemon_api::status_file_path();
    if let Err(e) = daemon_api::write_upgrade_record(&path, &record) {
        eprintln!(
            "attention : état de la mise à niveau non enregistré dans {} : {e}",
            path.display()
        );
    }
}

fn upgrade_cmd(
    config: &Config,
    path: Option<&Path>,
    attr: Option<&str>,
    notify_desktop: bool,
    sudo: bool,
    yes: bool,
) -> CmdResult {
    let repo = resolve_repo(config, path)?;
    let attr = resolve_attr(config, &repo, attr);
    warn_if_not_system(config, &repo);
    let privilege = if sudo {
        Privilege::Sudo
    } else {
        Privilege::Pkexec
    };
    if !confirm_action(
        yes,
        &format!(
            "Mettre à niveau et appliquer {} ?",
            nix::flake_ref(&repo, attr.as_deref())
        ),
    ) {
        return Ok(cancelled());
    }
    let notify_failure = |body: &str| {
        if notify_desktop {
            notify::desktop("Mise à niveau automatique échouée", body, true);
        }
    };

    match upgrade::upgrade(&repo, attr.as_deref(), privilege, &mut stream_to_stderr) {
        Ok(UpgradeOutcome::AlreadyUpToDate) => {
            write_record(UpgradeOutcomeKind::UpToDate, "tout est à jour".into());
            info("Tout est à jour.");
            Ok(ExitCode::SUCCESS)
        }
        Ok(UpgradeOutcome::Applied(report)) => {
            let body = upgrade_body(&report.changes);
            info(&format!("Mise à niveau appliquée : {body}."));
            write_record(UpgradeOutcomeKind::Applied, body.clone());
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
            write_record(UpgradeOutcomeKind::RolledBack, message.clone());
            notify_failure("Retour à la génération précédente effectué.");
            Err(Failure::new(message).with_hint(
                "le flake.lock d'origine est restauré et la génération précédente réactivée ; corrigez la cause ci-dessus (sortie de nixos-rebuild) puis relancez.",
            ))
        }
        Err(e) => {
            write_record(UpgradeOutcomeKind::Failed, e.to_string());
            notify_failure(&e.to_string());
            Err(e.into())
        }
    }
}

/// Chemin saisi (relatif, `~`, lien symbolique) ramené à un dépôt flake existant.
fn flake_dir(path: &Path) -> Result<PathBuf, Failure> {
    let canon = config::normalize_user_path(&path.to_string_lossy())?;
    if !canon.join("flake.nix").is_file() {
        return Err(Failure::new(format!("pas de flake.nix dans {}", canon.display()))
            .with_hint("indiquez le dossier qui contient flake.nix (il peut être un sous-dossier d'un dépôt git)"));
    }
    Ok(canon)
}

fn set_role(config: &mut Config, config_path: &Path, path: &Path, role: Role) -> CmdResult {
    let path = flake_dir(path)?;
    config.set_role(&path, role);
    config.save(config_path)?;
    info(&format!("{} : {}.", path.display(), role_label(role)));
    Ok(ExitCode::SUCCESS)
}

/// `set-system` : dépôt système (et attribut) enregistrés dans la configuration utilisateur.
fn set_system(
    config: &mut Config,
    config_path: &Path,
    path: &Path,
    attr: Option<&str>,
) -> CmdResult {
    let path = flake_dir(path)?;
    config::validate_repo_path(&path)?;
    if let Some(a) = attr {
        config::validate_attr(a)?;
    }
    let previous = config.system_repo.clone();
    config.set_role(&path, Role::System);
    config.system_attr = attr.map(str::to_string);
    config.save(config_path)?;

    info(&format!(
        "{} : {}{}.",
        path.display(),
        role_label(Role::System),
        attr.map_or_else(String::new, |a| format!(" (configuration {a})"))
    ));
    if let Some(old) = previous.filter(|old| {
        config::normalize_user_path(&old.to_string_lossy())
            .ok()
            .as_deref()
            != Some(path.as_path())
    }) {
        info(&format!(
            "L'ancien dépôt système ({}) est devenu dépôt suivi.",
            old.display()
        ));
    }
    Ok(ExitCode::SUCCESS)
}

/// `doctor` : code 1 si une ligne est en échec.
fn doctor_cmd(config_path: &Path) -> ExitCode {
    let checks = doctor::run(config_path);
    print!(
        "{}",
        doctor::render(&checks, ui::stdout_color(), ui::is_quiet())
    );
    if doctor::has_failure(&checks) {
        ExitCode::from(crate::failure::EXIT_ERROR)
    } else {
        ExitCode::SUCCESS
    }
}

/// Durée écoulée lisible : « il y a 5 min ».
pub fn ago(now: i64, then: i64) -> String {
    let secs = now.saturating_sub(then).max(0);
    match secs {
        0..=59 => "à l'instant".to_string(),
        60..=3599 => format!("il y a {} min", secs / 60),
        3600..=86399 => format!("il y a {} h", secs / 3600),
        _ => format!("il y a {} j", secs / 86400),
    }
}

fn upgrade_label(r: &UpgradeRecord) -> &'static str {
    match r.outcome {
        UpgradeOutcomeKind::UpToDate => "à jour",
        UpgradeOutcomeKind::Applied => "appliquée",
        UpgradeOutcomeKind::RolledBack => "échouée (retour arrière)",
        UpgradeOutcomeKind::Failed => "échouée",
    }
}

/// `status` : résumé lisible ou JSON stable.
fn status_cmd(config: &Config, config_path: &Path, path: Option<&Path>, json: bool) -> CmdResult {
    let system = config.system_repo_resolved();
    let repos: Vec<(PathBuf, Role)> = match path {
        Some(p) => vec![(p.to_path_buf(), config.role(p))],
        None => list_repos(config, false),
    };
    let statuses: Vec<RepoStatus> = repos
        .iter()
        .map(|(p, role)| status::status(p, *role, false))
        .collect();
    let daemon = dbus::query_daemon();
    let last_upgrade = daemon_api::read_upgrade_record(&daemon_api::status_file_path());

    if json {
        return print_json(&StatusJson {
            schema: report::SCHEMA_VERSION,
            config_path: config_path.display().to_string(),
            system_repo: system.as_ref().map(|r| r.path.display().to_string()),
            system_repo_source: system.as_ref().map(|r| report::source_key(r.source)),
            system_attr: system.as_ref().and_then(|r| r.attr.clone()),
            repos: statuses.iter().map(RepoJson::from).collect(),
            daemon: match &daemon {
                Ok(s) => DaemonJson {
                    reachable: true,
                    error: None,
                    status: Some(s.clone()),
                },
                Err(e) => DaemonJson {
                    reachable: false,
                    error: Some(e.clone()),
                    status: None,
                },
            },
            last_upgrade,
        });
    }

    let now = daemon_api::now_unix();
    println!("Configuration : {}", config_path.display());
    match &system {
        Some(r) => println!(
            "Dépôt système : {} ({}{})",
            r.path.display(),
            r.source.label(),
            r.attr
                .as_deref()
                .map_or_else(String::new, |a| format!(", configuration {a}"))
        ),
        None => println!("Dépôt système : non défini ({SYSTEM_REPO_HINT})"),
    }
    if statuses.is_empty() {
        println!("Dépôts suivis : aucun");
    } else {
        let rows: Vec<Vec<String>> = statuses
            .iter()
            .map(|st| {
                vec![
                    role_label(st.role).to_string(),
                    st.path.display().to_string(),
                    git_label(&st.git),
                ]
            })
            .collect();
        print!(
            "{}",
            format_table_styled(
                &["Rôle", "Chemin", "Git"],
                &rows,
                &repo_cell_style,
                ui::stdout_color()
            )
        );
        for st in statuses.iter().filter(|s| s.error.is_some()) {
            println!(
                "  {} : {}",
                st.path.display(),
                st.error.as_deref().unwrap_or_default()
            );
        }
    }
    match &daemon {
        Ok(s) => {
            let when = s
                .checked_at
                .map_or_else(|| "pas encore vérifié".to_string(), |t| ago(now, t));
            println!(
                "Démon : joignable ({} mise(s) à jour connue(s), {when})",
                s.updates
            );
        }
        Err(e) => println!("Démon : injoignable ({e})"),
    }
    match &last_upgrade {
        Some(r) => println!(
            "Dernière mise à niveau : {}, {} ({})",
            upgrade_label(r),
            ago(now, r.finished_at),
            r.message.lines().next().unwrap_or_default()
        ),
        None => println!("Dernière mise à niveau : inconnue"),
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
        info(&format!(
            "Verrous de {} à jour : aucun input n'a changé.",
            repo.display()
        ));
    } else {
        info(&format!(
            "Verrous de {} mis à jour : {} input(s) changé(s) :",
            repo.display(),
            report.changes.len()
        ));
        for c in &report.changes {
            info(&format!("  {}", change_line(c)));
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
    fn resolve_repo_prefers_argument_then_system() {
        let config = Config {
            system_repo: Some(PathBuf::from("/sys")),
            system_attr: Some("laptop".into()),
            ..Config::default()
        };
        assert_eq!(
            resolve_repo(&config, Some(Path::new("/x"))).unwrap(),
            PathBuf::from("/x")
        );
        assert_eq!(resolve_repo(&config, None).unwrap(), PathBuf::from("/sys"));
        // L'attribut du dépôt système ne s'applique qu'à ce dépôt ; `--attr` prime.
        assert_eq!(
            resolve_attr(&config, Path::new("/sys"), None).as_deref(),
            Some("laptop")
        );
        assert_eq!(resolve_attr(&config, Path::new("/x"), None), None);
        assert_eq!(
            resolve_attr(&config, Path::new("/sys"), Some("a")).as_deref(),
            Some("a")
        );
    }

    #[test]
    fn ago_labels() {
        assert_eq!(ago(100, 100), "à l'instant");
        assert_eq!(ago(200, 100), "il y a 1 min");
        assert_eq!(ago(100 + 7200, 100), "il y a 2 h");
        assert_eq!(ago(100 + 3 * 86400, 100), "il y a 3 j");
        assert_eq!(ago(10, 100), "à l'instant");
    }

    #[test]
    fn confirmation_answers() {
        assert!(confirm_from("?", &mut "oui\n".as_bytes()));
        assert!(!confirm_from("?", &mut "\n".as_bytes()));
        assert!(!confirm_from("?", &mut "".as_bytes()));
    }
}
