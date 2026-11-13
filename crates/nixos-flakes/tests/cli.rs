//! Tests d'intégration : lance le binaire compilé. Pas de nix requis.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

type TestResult = Result<(), String>;

fn expect(cond: bool, message: impl Into<String>) -> TestResult {
    if cond {
        Ok(())
    } else {
        Err(message.into())
    }
}

fn run(args: &[&str]) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_nixos-flakes"))
        .args(args)
        .output()
}

/// Dossier de travail propre, dans le répertoire cible fourni par cargo.
fn work_dir(name: &str) -> std::io::Result<PathBuf> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cli-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[test]
fn help_lists_subcommands() -> TestResult {
    let out = run(&["--help"]).map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    expect(out.status.success(), "--help doit réussir")?;
    for sub in [
        "list",
        "scan",
        "check",
        "update",
        "apply",
        "upgrade",
        "set-system",
    ] {
        expect(stdout.contains(sub), format!("« {sub} » absent de l'aide"))?;
    }
    Ok(())
}

#[test]
fn list_reads_temporary_config() -> TestResult {
    let dir = work_dir("list").map_err(|e| e.to_string())?;
    let system = dir.join("systeme");
    let tracked = dir.join("suivi");
    for d in [&system, &tracked] {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        std::fs::write(d.join("flake.nix"), "{}").map_err(|e| e.to_string())?;
    }
    let config = dir.join("config.toml");
    let text = format!(
        "system_repo = \"{}\"\ntracked = [\"{}\"]\n",
        system.display(),
        tracked.display()
    );
    std::fs::write(&config, text).map_err(|e| e.to_string())?;

    let out = run(&["--config", config.to_str().unwrap_or_default(), "list"])
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    expect(out.status.success(), format!("list a échoué : {stdout}"))?;
    expect(stdout.contains("Rôle"), "en-tête absent")?;
    expect(
        stdout.contains(&format!("système  {}", system.display())),
        format!("dépôt système absent :\n{stdout}"),
    )?;
    expect(
        stdout.contains(&format!("suivi    {}", tracked.display())),
        format!("dépôt suivi absent :\n{stdout}"),
    )?;
    expect(
        stdout.contains('—'),
        "sans --check, les mises à jour ne doivent pas être calculées",
    )?;
    Ok(())
}

#[test]
fn scan_without_roots_finds_nothing() -> TestResult {
    let dir = work_dir("scan").map_err(|e| e.to_string())?;
    let config = dir.join("config.toml");
    std::fs::write(&config, "scan_roots = []\n").map_err(|e| e.to_string())?;
    let out = run(&["--config", config.to_str().unwrap_or_default(), "scan"])
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    expect(out.status.success(), "scan doit réussir")?;
    expect(stdout.contains("Aucun dépôt flake trouvé."), stdout)?;
    Ok(())
}

#[test]
fn invalid_config_is_error_exit_one() -> TestResult {
    let dir = work_dir("invalid").map_err(|e| e.to_string())?;
    let config = dir.join("config.toml");
    std::fs::write(&config, "scan_depth = [").map_err(|e| e.to_string())?;
    let out = run(&["--config", config.to_str().unwrap_or_default(), "list"])
        .map_err(|e| e.to_string())?;
    expect(out.status.code() == Some(1), "code de sortie attendu : 1")?;
    expect(
        String::from_utf8_lossy(&out.stderr).starts_with("erreur : "),
        "message d'erreur attendu sur stderr",
    )?;
    Ok(())
}

#[test]
fn waybar_always_prints_json_and_exits_zero() -> TestResult {
    let dir = work_dir("waybar").map_err(|e| e.to_string())?;
    let config = dir.join("config.toml");
    std::fs::write(&config, "scan_depth = [").map_err(|e| e.to_string())?;
    let out = run(&[
        "--config",
        config.to_str().unwrap_or_default(),
        "check",
        "--waybar",
    ])
    .map_err(|e| e.to_string())?;
    expect(
        out.status.code() == Some(0),
        "check --waybar doit sortir avec 0",
    )?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    expect(
        stdout.contains("\"class\":\"error\""),
        format!("JSON d'erreur attendu :\n{stdout}"),
    )?;
    Ok(())
}

#[test]
fn update_rejects_unknown_input_before_running_nix() -> TestResult {
    let dir = work_dir("update-input").map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("flake.lock"),
        r#"{"nodes":{"root":{"inputs":{"nixpkgs":"nixpkgs"}},"nixpkgs":{"locked":{"type":"github","rev":"abc"}}},"root":"root","version":7}"#,
    )
    .map_err(|e| e.to_string())?;
    let config = dir.join("config.toml");
    std::fs::write(&config, "").map_err(|e| e.to_string())?;
    let out = run(&[
        "--config",
        &config.to_string_lossy(),
        "update",
        &dir.to_string_lossy(),
        "--input",
        "inconnu",
    ])
    .map_err(|e| e.to_string())?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    expect(!out.status.success(), "doit échouer")?;
    expect(
        stderr.contains("input inconnu"),
        format!("stderr : {stderr}"),
    )
}

/// Comme `run`, avec un bus de session inexistant (aucun démon réel n'est contacté).
fn run_isolated(args: &[&str]) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_nixos-flakes"))
        .args(args)
        .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/bus")
        .env("NIXOS_FLAKES_STATUS_FILE", "/nonexistent/status.json")
        .output()
}

fn flake_dir(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let d = dir.join(name);
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    std::fs::write(d.join("flake.nix"), "{}").map_err(|e| e.to_string())?;
    Ok(d)
}

#[test]
fn status_json_is_parseable_and_honors_config() -> TestResult {
    let dir = work_dir("status").map_err(|e| e.to_string())?;
    let system = flake_dir(&dir, "systeme")?;
    let config = dir.join("config.toml");
    std::fs::write(&config, format!("system_repo = \"{}\"\n", system.display()))
        .map_err(|e| e.to_string())?;
    let out = run_isolated(&["--config", &config.to_string_lossy(), "status", "--json"])
        .map_err(|e| e.to_string())?;
    expect(
        out.status.success(),
        format!(
            "status --json a échoué : {}",
            String::from_utf8_lossy(&out.stderr)
        ),
    )?;
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("JSON invalide : {e}"))?;
    expect(v["schema"] == 1, "schema attendu : 1")?;
    expect(
        v["system_repo"] == system.display().to_string(),
        format!("dépôt système : {v}"),
    )?;
    expect(v["repos"][0]["role"] == "system", format!("rôle : {v}"))?;
    expect(v["daemon"]["reachable"] == false, format!("démon : {v}"))?;
    Ok(())
}

#[test]
fn status_text_mentions_daemon() -> TestResult {
    let dir = work_dir("status-text").map_err(|e| e.to_string())?;
    let config = dir.join("config.toml");
    std::fs::write(&config, "").map_err(|e| e.to_string())?;
    let out = run_isolated(&["--config", &config.to_string_lossy(), "status"])
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    expect(out.status.success(), &stdout)?;
    expect(stdout.contains("Démon : injoignable"), stdout)
}

#[test]
fn doctor_runs_and_reports_broken_config() -> TestResult {
    let dir = work_dir("doctor").map_err(|e| e.to_string())?;
    let config = dir.join("config.toml");
    std::fs::write(&config, "scan_depth = [").map_err(|e| e.to_string())?;
    let out = run_isolated(&["--config", &config.to_string_lossy(), "doctor"])
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    expect(
        out.status.code() == Some(1),
        format!("config cassée : code 1 attendu\n{stdout}"),
    )?;
    expect(stdout.contains("✗ configuration"), &stdout)?;
    expect(stdout.contains("piste :"), &stdout)?;
    expect(stdout.contains("nix"), &stdout)
}

#[test]
fn doctor_without_failure_exits_zero_or_one_never_panics() -> TestResult {
    let dir = work_dir("doctor-ok").map_err(|e| e.to_string())?;
    let system = flake_dir(&dir, "systeme")?;
    let config = dir.join("config.toml");
    std::fs::write(&config, format!("system_repo = \"{}\"\n", system.display()))
        .map_err(|e| e.to_string())?;
    let out = run_isolated(&["--config", &config.to_string_lossy(), "doctor"])
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    expect(stdout.contains("✓ configuration"), &stdout)?;
    expect(stdout.contains("✓ flake du dépôt système"), &stdout)?;
    expect(matches!(out.status.code(), Some(0 | 1)), &stdout)
}

#[test]
fn completions_bash_is_not_empty() -> TestResult {
    let out = run(&["completions", "bash"]).map_err(|e| e.to_string())?;
    expect(out.status.success(), "completions bash doit réussir")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    expect(
        stdout.contains("nixos-flakes") && stdout.len() > 200,
        "script de complétion attendu",
    )?;
    let bad = run(&["completions", "nimporte"]).map_err(|e| e.to_string())?;
    expect(bad.status.code() == Some(2), "shell inconnu : code 2")
}

#[test]
fn usage_error_exits_two_and_help_documents_exit_codes() -> TestResult {
    let out = run(&["frobnicate"]).map_err(|e| e.to_string())?;
    expect(out.status.code() == Some(2), "usage : code 2 attendu")?;
    let help = run(&["--help"]).map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&help.stdout).into_owned();
    for needle in ["Codes de sortie", "dépôt occupé", "délai dépassé", "10"] {
        expect(
            stdout.contains(needle),
            format!("« {needle} » absent de l'aide"),
        )?;
    }
    Ok(())
}

#[test]
fn busy_repo_exits_four_with_hint() -> TestResult {
    let dir = work_dir("busy").map_err(|e| e.to_string())?;
    let repo = flake_dir(&dir, "depot")?;
    let config = dir.join("config.toml");
    std::fs::write(&config, "").map_err(|e| e.to_string())?;
    // Le verrou est un flock sur le dossier du dépôt.
    let held = std::fs::File::open(&repo).map_err(|e| e.to_string())?;
    held.try_lock().map_err(|e| e.to_string())?;
    let out = run(&[
        "--config",
        &config.to_string_lossy(),
        "apply",
        "--yes",
        &repo.to_string_lossy(),
    ])
    .map_err(|e| e.to_string())?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    expect(
        out.status.code() == Some(4),
        format!("code 4 attendu : {stderr}"),
    )?;
    expect(stderr.contains("erreur : "), &stderr)?;
    expect(stderr.contains("piste : "), &stderr)
}

#[test]
fn config_flag_is_respected_by_list_json_and_scan_json() -> TestResult {
    let dir = work_dir("json").map_err(|e| e.to_string())?;
    let system = flake_dir(&dir, "systeme")?;
    let config = dir.join("config.toml");
    std::fs::write(
        &config,
        format!(
            "system_repo = \"{}\"\nscan_roots = [\"{}\"]\nscan_depth = 2\n",
            system.display(),
            dir.display()
        ),
    )
    .map_err(|e| e.to_string())?;
    let cfg = config.to_string_lossy().into_owned();
    let list = run(&["--config", &cfg, "list", "--json"]).map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_slice(&list.stdout).map_err(|e| e.to_string())?;
    expect(v[0]["role"] == "system", format!("list --json : {v}"))?;
    expect(v[0]["updates"].is_null(), "non vérifié : updates null")?;
    let scan = run(&["--config", &cfg, "scan", "--json"]).map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_slice(&scan.stdout).map_err(|e| e.to_string())?;
    expect(
        v.as_array().is_some_and(|a| !a.is_empty()),
        format!("scan --json : {v}"),
    )
}

#[test]
fn quiet_suppresses_info_messages() -> TestResult {
    let dir = work_dir("quiet").map_err(|e| e.to_string())?;
    let config = dir.join("config.toml");
    std::fs::write(&config, "scan_roots = []\n").map_err(|e| e.to_string())?;
    let out = run(&["--config", &config.to_string_lossy(), "--quiet", "scan"])
        .map_err(|e| e.to_string())?;
    expect(
        out.status.success() && out.stdout.is_empty(),
        "scan -q : sortie vide",
    )
}

#[test]
fn pkg_search_ignores_broken_config() -> TestResult {
    let dir = work_dir("pkg-search").map_err(|e| e.to_string())?;
    let config = dir.join("config.toml");
    std::fs::write(&config, "scan_depth = [").map_err(|e| e.to_string())?;
    // Hors réseau : curl/nix absents du PATH et cache vide => échec de recherche (pas de
    // config), jamais « configuration invalide ».
    let out = Command::new(env!("CARGO_BIN_EXE_nixos-flakes"))
        .args([
            "--config",
            &config.to_string_lossy(),
            "pkg",
            "search",
            "zzz-inexistant",
        ])
        .env("PATH", "/nonexistent")
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .output()
        .map_err(|e| e.to_string())?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    expect(!stderr.contains("configuration invalide"), &stderr)?;
    expect(stderr.contains("recherche de paquets impossible"), &stderr)?;
    expect(
        stderr.contains("piste :") || stderr.contains("Piste :"),
        &stderr,
    )
}

/// Commande isolée de l'environnement de la machine : pas de dépôt système implicite
/// (défauts système inexistants), pas de démon.
fn sys_cmd(dir: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_nixos-flakes"));
    c.env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/bus")
        .env("NIXOS_FLAKES_STATUS_FILE", "/nonexistent/status.json")
        .env(
            "NIXOS_FLAKES_SYSTEM_DEFAULTS",
            dir.join("absent-defaults.toml"),
        )
        .env_remove("NIXOS_FLAKES_SYSTEM_REPO")
        .env_remove("NIXOS_FLAKES_SYSTEM_ATTR")
        .env("NIXOS_FLAKES_LOCK_DIR", dir.join("locks"));
    c
}

fn status_json(c: &mut Command) -> Result<serde_json::Value, String> {
    let out = c.output().map_err(|e| e.to_string())?;
    expect(
        out.status.success(),
        format!("status a échoué : {}", String::from_utf8_lossy(&out.stderr)),
    )?;
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}

#[test]
fn set_system_accepts_relative_path_with_spaces_and_attr() -> TestResult {
    let dir = work_dir("setsys-rel").map_err(|e| e.to_string())?;
    let repo = flake_dir(&dir, "ma config nixos")?;
    let config = dir.join("config.toml");
    let out = sys_cmd(&dir)
        .current_dir(&dir)
        .args(["--config", &config.to_string_lossy()])
        .args(["set-system", "ma config nixos", "--attr", "portable"])
        .output()
        .map_err(|e| e.to_string())?;
    expect(
        out.status.success(),
        format!("set-system : {}", String::from_utf8_lossy(&out.stderr)),
    )?;
    let v = status_json(
        sys_cmd(&dir)
            .args(["--config", &config.to_string_lossy()])
            .args(["status", "--json"]),
    )?;
    let canon = repo.canonicalize().map_err(|e| e.to_string())?;
    expect(
        v["system_repo"] == canon.display().to_string(),
        format!("{v}"),
    )?;
    expect(v["system_attr"] == "portable", format!("{v}"))?;
    expect(v["system_repo_source"] == "user", format!("{v}"))
}

#[test]
fn set_system_through_symlink_designates_the_same_repo() -> TestResult {
    let dir = work_dir("setsys-link").map_err(|e| e.to_string())?;
    let repo = flake_dir(&dir, "reel")?;
    let link = dir.join("etc-nixos");
    std::os::unix::fs::symlink(&repo, &link).map_err(|e| e.to_string())?;
    let config = dir.join("config.toml");
    let out = sys_cmd(&dir)
        .args(["--config", &config.to_string_lossy()])
        .args(["set-system", &link.to_string_lossy()])
        .output()
        .map_err(|e| e.to_string())?;
    expect(out.status.success(), "set-system via lien")?;
    // Le chemin réel porte le rôle système, pas « suivi » ni « détecté ».
    let out = sys_cmd(&dir)
        .args(["--config", &config.to_string_lossy()])
        .args(["status", "--json", &repo.to_string_lossy()])
        .output()
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    expect(v["repos"][0]["role"] == "system", format!("{v}"))
}

#[test]
fn set_system_rejects_bad_attr_and_missing_flake() -> TestResult {
    let dir = work_dir("setsys-bad").map_err(|e| e.to_string())?;
    let repo = flake_dir(&dir, "depot")?;
    let config = dir.join("config.toml");
    let out = sys_cmd(&dir)
        .args(["--config", &config.to_string_lossy()])
        .args(["set-system", &repo.to_string_lossy(), "--attr", "a;rm -rf"])
        .output()
        .map_err(|e| e.to_string())?;
    expect(out.status.code() == Some(1), "attribut invalide : code 1")?;
    expect(!config.exists(), "rien ne doit être enregistré")?;
    let vide = dir.join("vide");
    std::fs::create_dir_all(&vide).map_err(|e| e.to_string())?;
    let out = sys_cmd(&dir)
        .args(["--config", &config.to_string_lossy()])
        .args(["set-system", &vide.to_string_lossy()])
        .output()
        .map_err(|e| e.to_string())?;
    expect(
        String::from_utf8_lossy(&out.stderr).contains("pas de flake.nix"),
        "message flake.nix attendu",
    )
}

#[test]
fn system_repo_resolution_env_then_system_defaults() -> TestResult {
    let dir = work_dir("resolve").map_err(|e| e.to_string())?;
    let config = dir.join("config.toml");
    let from_env = flake_dir(&dir, "depot-env")?;
    let from_file = flake_dir(&dir, "depot-defaut")?;
    let defaults = dir.join("defaults.toml");
    std::fs::write(
        &defaults,
        format!(
            "system_repo = \"{}\"\nsystem_attr = \"serveur\"\n",
            from_file.display()
        ),
    )
    .map_err(|e| e.to_string())?;
    // Défaut système seul.
    let v = status_json(
        sys_cmd(&dir)
            .env("NIXOS_FLAKES_SYSTEM_DEFAULTS", &defaults)
            .args(["--config", &config.to_string_lossy()])
            .args(["status", "--json"]),
    )?;
    expect(v["system_repo_source"] == "system-default", format!("{v}"))?;
    expect(v["system_attr"] == "serveur", format!("{v}"))?;
    // La variable d'environnement prime sur le défaut système.
    let v = status_json(
        sys_cmd(&dir)
            .env("NIXOS_FLAKES_SYSTEM_DEFAULTS", &defaults)
            .env("NIXOS_FLAKES_SYSTEM_REPO", &from_env)
            .args(["--config", &config.to_string_lossy()])
            .args(["status", "--json"]),
    )?;
    expect(v["system_repo_source"] == "env", format!("{v}"))?;
    expect(
        v["system_repo"] == from_env.display().to_string(),
        format!("{v}"),
    )?;
    // La configuration utilisateur prime sur tout, et le dépôt apparaît dans `list`.
    std::fs::write(
        &config,
        format!(
            "system_repo = \"{}\"\nscan_roots = []\n",
            from_file.display()
        ),
    )
    .map_err(|e| e.to_string())?;
    let out = sys_cmd(&dir)
        .env("NIXOS_FLAKES_SYSTEM_REPO", &from_env)
        .args(["--config", &config.to_string_lossy(), "list"])
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    expect(
        stdout.contains(&format!("système  {}", from_file.display())),
        stdout,
    )
}

#[test]
fn missing_system_repo_gives_clear_error_with_hint() -> TestResult {
    if Path::new("/etc/nixos/flake.nix").exists() {
        return Ok(()); // le repli historique s'appliquerait sur cette machine
    }
    let dir = work_dir("nosys").map_err(|e| e.to_string())?;
    let config = dir.join("config.toml");
    let out = sys_cmd(&dir)
        .args(["--config", &config.to_string_lossy(), "check"])
        .output()
        .map_err(|e| e.to_string())?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    expect(out.status.code() == Some(1), &stderr)?;
    expect(stderr.contains("aucun dépôt système"), &stderr)?;
    expect(
        stderr.contains("set-system") && stderr.contains("flakePath"),
        &stderr,
    )
}

#[test]
fn doctor_reports_custom_system_repo_provenance() -> TestResult {
    let dir = work_dir("doctor-sys").map_err(|e| e.to_string())?;
    let repo = flake_dir(&dir, "depot")?;
    let config = dir.join("config.toml");
    std::fs::write(&config, format!("system_repo = \"{}\"\n", repo.display()))
        .map_err(|e| e.to_string())?;
    let out = sys_cmd(&dir)
        .args(["--config", &config.to_string_lossy(), "doctor"])
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    expect(
        stdout.contains("configuration utilisateur"),
        format!("provenance absente :\n{stdout}"),
    )?;
    expect(
        !stdout.contains("/etc/nixos n'existe pas"),
        "doctor ne doit pas échouer sur /etc/nixos",
    )
}

/// Faux `nix`/`git` qui réussissent sans rien faire, dans un dossier à mettre dans le PATH.
fn fake_bin(dir: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    for name in ["nix", "git", "nixos-rebuild"] {
        let p = bin.join(name);
        std::fs::write(&p, "#!/bin/sh\nexit 0\n").map_err(|e| e.to_string())?;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    Ok(bin)
}

/// Service root sans HOME ni XDG (unité systemd) : l'absence de configuration utilisateur ne doit
/// pas faire échouer les commandes en lecture (`upgrade`, `status`, `check`, `list`).
#[test]
fn read_commands_work_without_home_or_xdg() -> TestResult {
    let dir = work_dir("nohome").map_err(|e| e.to_string())?;
    let repo = flake_dir(&dir, "depot")?;
    let bin = fake_bin(&dir)?;
    let repo_s = repo.to_string_lossy().into_owned();
    let cases: [&[&str]; 4] = [
        &["upgrade", "--yes", &repo_s],
        &["status", &repo_s],
        &["check", &repo_s],
        &["list"],
    ];
    for args in cases {
        let out = Command::new(env!("CARGO_BIN_EXE_nixos-flakes"))
            .args(args)
            .env_clear()
            .env("PATH", &bin)
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/bus")
            .env("NIXOS_FLAKES_STATUS_FILE", dir.join("status.json"))
            .env("NIXOS_FLAKES_SYSTEM_DEFAULTS", dir.join("absent.toml"))
            .env("NIXOS_FLAKES_LOCK_DIR", dir.join("locks"))
            .output()
            .map_err(|e| e.to_string())?;
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        expect(
            !stderr.contains("XDG_CONFIG_HOME") && !stderr.contains("HOME"),
            format!("{args:?} : {stderr}"),
        )?;
        expect(
            out.status.success(),
            format!("{args:?} a échoué : {stderr}"),
        )?;
    }
    // Une commande qui écrit la configuration, elle, échoue clairement.
    let out = Command::new(env!("CARGO_BIN_EXE_nixos-flakes"))
        .args(["track", &repo_s])
        .env_clear()
        .env("PATH", &bin)
        .output()
        .map_err(|e| e.to_string())?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    expect(
        out.status.code() == Some(1) && stderr.contains("HOME"),
        stderr,
    )
}
