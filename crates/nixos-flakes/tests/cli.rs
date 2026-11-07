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
