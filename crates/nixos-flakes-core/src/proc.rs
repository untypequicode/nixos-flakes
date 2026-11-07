//! Lancement de sous-processus (privé à la bibliothèque).

use crate::error::{Error, Result};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;

const TAIL_LINES: usize = 30;

/// Commande dont le `PATH` se termine par `/run/wrappers/bin`.
pub(crate) fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.env("PATH", path_with_wrappers(std::env::var("PATH").ok()));
    cmd
}

pub(crate) fn path_with_wrappers(path: Option<String>) -> String {
    const W: &str = "/run/wrappers/bin";
    match path {
        Some(p) if p.split(':').any(|e| e == W) => p,
        Some(p) if !p.is_empty() => format!("{p}:{W}"),
        _ => W.to_string(),
    }
}

/// Lance `program` et renvoie sa sortie brute (sans erreur si code non nul).
pub(crate) fn output(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<Output> {
    let mut cmd = command(program);
    cmd.args(args).stdin(Stdio::null());
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    cmd.output().map_err(|source| Error::Spawn {
        program: program.to_string(),
        source,
    })
}

/// Lance `program`, envoie chaque ligne (stdout+stderr fusionnés) à `out`.
/// Échoue avec la fin de la sortie si le code de retour est non nul.
pub(crate) fn run(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    out: &mut dyn FnMut(&str),
) -> Result<()> {
    run_env(program, args, cwd, &[], out)
}

/// Comme `run`, avec des variables d'environnement supplémentaires.
/// Les valeurs ne sont jamais journalisées ni placées dans les erreurs.
pub(crate) fn run_env(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    env: &[(String, String)],
    out: &mut dyn FnMut(&str),
) -> Result<()> {
    let mut cmd = command(program);
    cmd.envs(env.iter().map(|(k, v)| (k, v)));
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let mut child = cmd.spawn().map_err(|source| Error::Spawn {
        program: program.to_string(),
        source,
    })?;

    let (tx, rx) = mpsc::channel::<String>();
    let mut handles = Vec::new();
    if let Some(s) = child.stdout.take() {
        handles.push(pump(s, tx.clone()));
    }
    if let Some(s) = child.stderr.take() {
        handles.push(pump(s, tx.clone()));
    }
    drop(tx);

    let mut tail: VecDeque<String> = VecDeque::new();
    for line in rx {
        out(&line);
        if tail.len() == TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    }
    for h in handles {
        let _ = h.join();
    }
    let status = child.wait().map_err(|source| Error::Spawn {
        program: program.to_string(),
        source,
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Command {
            program: program.to_string(),
            status: status
                .code()
                .map_or_else(|| status.to_string(), |c| format!("code {c}")),
            tail: Vec::from(tail).join("\n"),
        })
    }
}

/// Variables `GIT_CONFIG_*` déclarant `repo` comme `safe.directory` (chemin
/// exact, jamais `*`), à la suite d'un éventuel `GIT_CONFIG_COUNT` existant.
pub(crate) fn git_safe_env(existing_count: Option<&str>, repo: &Path) -> Vec<(String, String)> {
    let n: usize = existing_count
        .and_then(|c| c.trim().parse().ok())
        .unwrap_or(0);
    vec![
        ("GIT_CONFIG_COUNT".into(), (n + 1).to_string()),
        (format!("GIT_CONFIG_KEY_{n}"), "safe.directory".into()),
        (
            format!("GIT_CONFIG_VALUE_{n}"),
            repo.to_string_lossy().into_owned(),
        ),
    ]
}

/// Valeur de `NIX_CONFIG` portant le jeton GitHub, ajoutée après `existing`.
pub(crate) fn nix_config_with_token(existing: Option<&str>, token: &str) -> String {
    let line = format!("access-tokens = github.com={token}");
    match existing {
        Some(e) if !e.trim().is_empty() => format!("{}\n{line}\n", e.trim_end_matches('\n')),
        _ => format!("{line}\n"),
    }
}

/// Environnement des sous-processus `nix` : `safe.directory` pour `repo` et,
/// si fourni, le jeton GitHub via `NIX_CONFIG`.
pub(crate) fn nix_env(repo: &Path, token: Option<&str>) -> Vec<(String, String)> {
    let mut env = git_safe_env(std::env::var("GIT_CONFIG_COUNT").ok().as_deref(), repo);
    if let Some(t) = token {
        env.push((
            "NIX_CONFIG".into(),
            nix_config_with_token(std::env::var("NIX_CONFIG").ok().as_deref(), t),
        ));
    }
    env
}

fn pump<R: std::io::Read + Send + 'static>(
    reader: R,
    tx: mpsc::Sender<String>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut r = BufReader::new(reader);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match r.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let s = String::from_utf8_lossy(&buf);
                    let line = s.trim_end_matches(['\n', '\r']);
                    if tx.send(line.to_string()).is_err() {
                        break;
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_gets_wrappers_appended_once() {
        assert_eq!(
            path_with_wrappers(Some("/a:/b".into())),
            "/a:/b:/run/wrappers/bin"
        );
        assert_eq!(
            path_with_wrappers(Some("/a:/run/wrappers/bin".into())),
            "/a:/run/wrappers/bin"
        );
        assert_eq!(path_with_wrappers(None), "/run/wrappers/bin");
    }

    #[test]
    fn git_safe_env_starts_at_zero_and_appends() {
        let r = Path::new("/etc/nixos");
        let e = git_safe_env(None, r);
        assert!(e.contains(&("GIT_CONFIG_COUNT".into(), "1".into())));
        assert!(e.contains(&("GIT_CONFIG_KEY_0".into(), "safe.directory".into())));
        assert!(e.contains(&("GIT_CONFIG_VALUE_0".into(), "/etc/nixos".into())));
        let e = git_safe_env(Some("2"), r);
        assert!(e.contains(&("GIT_CONFIG_COUNT".into(), "3".into())));
        assert!(e.contains(&("GIT_CONFIG_KEY_2".into(), "safe.directory".into())));
        assert!(e.iter().all(|(_, v)| v != "*"));
    }

    #[test]
    fn nix_config_preserves_existing() {
        assert_eq!(
            nix_config_with_token(None, "tok"),
            "access-tokens = github.com=tok\n"
        );
        assert_eq!(
            nix_config_with_token(Some("a = b"), "tok"),
            "a = b\naccess-tokens = github.com=tok\n"
        );
    }

    #[test]
    fn env_reaches_subprocess() {
        let mut lines = Vec::new();
        let env = vec![("NF_TEST".to_string(), "valeur".to_string())];
        run_env("sh", &["-c", "echo $NF_TEST"], None, &env, &mut |l| {
            lines.push(l.to_string())
        })
        .unwrap();
        assert_eq!(lines, vec!["valeur"]);
    }

    #[test]
    fn merges_stdout_and_stderr() {
        let mut lines = Vec::new();
        run("sh", &["-c", "echo un; echo deux >&2"], None, &mut |l| {
            lines.push(l.to_string())
        })
        .unwrap();
        lines.sort();
        assert_eq!(lines, vec!["deux", "un"]);
    }

    #[test]
    fn failure_carries_tail() {
        let err = run("sh", &["-c", "echo boom >&2; exit 3"], None, &mut |_| {}).unwrap_err();
        match &err {
            Error::Command { status, tail, .. } => {
                assert_eq!(status, "code 3");
                assert_eq!(tail, "boom");
            }
            e => panic!("erreur inattendue : {e}"),
        }
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn missing_program_is_spawn_error() {
        let err = run("nixos-flakes-inexistant", &[], None, &mut |_| {}).unwrap_err();
        assert!(matches!(err, Error::Spawn { .. }));
    }

    #[test]
    fn subprocess_sees_wrappers_in_path() {
        let mut lines = Vec::new();
        run("sh", &["-c", "echo $PATH"], None, &mut |l| {
            lines.push(l.to_string())
        })
        .unwrap();
        assert!(lines[0].split(':').any(|e| e == "/run/wrappers/bin"));
    }
}
