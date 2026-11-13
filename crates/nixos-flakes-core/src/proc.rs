//! Lancement de sous-processus (privé à la bibliothèque).

use crate::error::{Error, Result};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const TAIL_LINES: usize = 30;

/// Variable d'environnement surchargeant les délais par défaut (en secondes).
pub(crate) const ENV_TIMEOUT: &str = "NIXOS_FLAKES_TIMEOUT_SECS";

/// Délai par défaut d'une commande longue (`nix`, `nixos-rebuild`, `git fetch`…).
const DEFAULT_RUN_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Délai par défaut d'une commande courte dont on lit la sortie (`git status`…).
const DEFAULT_OUTPUT_TIMEOUT: Duration = Duration::from_secs(60);

/// Attente maximale de la fin des tubes une fois le processus terminé (un
/// petit-fils peut les garder ouverts).
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Période de scrutation de la fin du processus.
const POLL: Duration = Duration::from_millis(20);

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

/// Délai issu de l'environnement (secondes > 0), sinon `default`.
fn timeout_from(env: Option<&str>, default: Duration) -> Duration {
    env.and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&s| s > 0)
        .map_or(default, Duration::from_secs)
}

fn default_timeout(default: Duration) -> Duration {
    timeout_from(std::env::var(ENV_TIMEOUT).ok().as_deref(), default)
}

fn spawn_error(program: &str, source: std::io::Error) -> Error {
    Error::Spawn {
        program: program.to_string(),
        source,
    }
}

fn timeout_error(program: &str, args: &[&str], timeout: Duration) -> Error {
    let mut command = program.to_string();
    for a in args {
        command.push(' ');
        command.push_str(a);
    }
    // Évite de recopier un script entier dans le message.
    if command.chars().count() > 160 {
        command = command.chars().take(160).collect::<String>() + "…";
    }
    Error::Timeout {
        command,
        secs: timeout.as_secs(),
    }
}

/// Lance la commande ; réessaie quelques fois sur `ETXTBSY` : un exécutable
/// tout juste écrit reste « occupé » tant qu'un fork concurrent (autre thread)
/// garde une copie du descripteur d'écriture avant son exec.
fn spawn_retry(cmd: &mut Command, program: &str) -> Result<Child> {
    let mut tries = 0;
    loop {
        match cmd.spawn() {
            Ok(c) => return Ok(c),
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 5 => {
                tries += 1;
                thread::sleep(Duration::from_millis(15));
            }
            Err(e) => return Err(spawn_error(program, e)),
        }
    }
}

/// Termine le processus (sans viser ses petits-fils) et récolte son statut.
fn kill_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Lance `program` et renvoie sa sortie brute (sans erreur si code non nul),
/// avec des variables d'environnement supplémentaires.
/// Délai : 60 s par défaut (`NIXOS_FLAKES_TIMEOUT_SECS` le surcharge).
pub(crate) fn output_env(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    env: &[(&str, &str)],
) -> Result<Output> {
    output_timeout(
        program,
        args,
        cwd,
        env,
        default_timeout(DEFAULT_OUTPUT_TIMEOUT),
    )
}

/// Comme `output`, avec variables d'environnement et délai explicites.
pub(crate) fn output_timeout(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    env: &[(&str, &str)],
    timeout: Duration,
) -> Result<Output> {
    let mut cmd = command(program);
    cmd.envs(env.iter().copied());
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let mut child = spawn_retry(&mut cmd, program)?;
    let (tx, rx) = mpsc::channel::<(bool, Vec<u8>)>();
    if let Some(s) = child.stdout.take() {
        slurp(s, true, tx.clone());
    }
    if let Some(s) = child.stderr.take() {
        slurp(s, false, tx.clone());
    }
    drop(tx);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {}
            Err(e) => return Err(spawn_error(program, e)),
        }
        if Instant::now() >= deadline {
            kill_child(&mut child);
            return Err(timeout_error(program, args, timeout));
        }
        thread::sleep(POLL);
    };
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let grace = Instant::now() + DRAIN_GRACE;
    while let Some(left) = grace.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok((true, b)) => stdout = b,
            Ok((false, b)) => stderr = b,
            Err(_) => break,
        }
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Lit tout `reader` puis envoie le contenu (thread jamais joint : il
/// s'arrête quand le tube se ferme).
fn slurp<R: std::io::Read + Send + 'static>(
    mut reader: R,
    is_stdout: bool,
    tx: mpsc::Sender<(bool, Vec<u8>)>,
) {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
        let _ = tx.send((is_stdout, buf));
    });
}

/// Lance `program`, envoie chaque ligne (stdout+stderr fusionnés) à `out`.
/// Échoue avec la fin de la sortie si le code de retour est non nul.
/// Délai : 30 min par défaut (`NIXOS_FLAKES_TIMEOUT_SECS` le surcharge).
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
    run_env_timeout(
        program,
        args,
        cwd,
        env,
        default_timeout(DEFAULT_RUN_TIMEOUT),
        out,
    )
}

/// Comme `run_env`, avec un délai maximal explicite. À l'échéance le processus
/// est tué (SIGKILL ; ses éventuels petits-fils ne sont pas visés) et
/// `Error::Timeout` est renvoyée.
pub(crate) fn run_env_timeout(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    env: &[(String, String)],
    timeout: Duration,
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
    let mut child = spawn_retry(&mut cmd, program)?;

    let (tx, rx) = mpsc::channel::<String>();
    if let Some(s) = child.stdout.take() {
        pump(s, tx.clone());
    }
    if let Some(s) = child.stderr.take() {
        pump(s, tx.clone());
    }
    drop(tx);

    let deadline = Instant::now() + timeout;
    let mut tail: VecDeque<String> = VecDeque::new();
    let mut push = |line: String, out: &mut dyn FnMut(&str)| {
        out(&line);
        if tail.len() == TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    };
    let mut open = true;
    let status = loop {
        if open {
            match rx.recv_timeout(POLL) {
                Ok(line) => push(line, out),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => open = false,
            }
        } else {
            thread::sleep(POLL);
        }
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {}
            Err(e) => return Err(spawn_error(program, e)),
        }
        if Instant::now() >= deadline {
            kill_child(&mut child);
            return Err(timeout_error(program, args, timeout));
        }
    };
    // Le processus a fini : on récupère le reste de la sortie, sans attendre
    // indéfiniment un petit-fils qui garderait les tubes ouverts.
    let grace = Instant::now() + DRAIN_GRACE;
    while open && Instant::now() < grace {
        let Some(left) = grace.checked_duration_since(Instant::now()) else {
            break;
        };
        match rx.recv_timeout(left) {
            Ok(line) => push(line, out),
            Err(_) => break,
        }
    }
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

/// Variables `GIT_CONFIG_*` déclarant chaque chemin de `repos` comme
/// `safe.directory` (chemins exacts, jamais `*`), à la suite d'un éventuel
/// `GIT_CONFIG_COUNT` existant.
pub(crate) fn git_safe_env(existing_count: Option<&str>, repos: &[&Path]) -> Vec<(String, String)> {
    let n: usize = existing_count
        .and_then(|c| c.trim().parse().ok())
        .unwrap_or(0);
    let mut v = vec![(
        "GIT_CONFIG_COUNT".to_string(),
        (n + repos.len()).to_string(),
    )];
    for (i, r) in repos.iter().enumerate() {
        v.push((format!("GIT_CONFIG_KEY_{}", n + i), "safe.directory".into()));
        v.push((
            format!("GIT_CONFIG_VALUE_{}", n + i),
            r.to_string_lossy().into_owned(),
        ));
    }
    v
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
    // Le flake peut être dans un sous-dossier d'un dépôt git : on déclare aussi le
    // chemin canonique et la racine de l'arbre de travail.
    let mut paths: Vec<PathBuf> = vec![
        repo.to_path_buf(),
        repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf()),
        crate::git::worktree_root(repo),
    ];
    paths.sort();
    paths.dedup();
    let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    let mut env = git_safe_env(std::env::var("GIT_CONFIG_COUNT").ok().as_deref(), &refs);
    if let Some(t) = token {
        env.push((
            "NIX_CONFIG".into(),
            nix_config_with_token(std::env::var("NIX_CONFIG").ok().as_deref(), t),
        ));
    }
    env
}

/// Thread de lecture ligne à ligne ; jamais joint (il s'arrête à la fermeture du tube).
fn pump<R: std::io::Read + Send + 'static>(reader: R, tx: mpsc::Sender<String>) {
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
    });
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
        let r = Path::new("/srv/nixos");
        let e = git_safe_env(None, &[r]);
        assert!(e.contains(&("GIT_CONFIG_COUNT".into(), "1".into())));
        assert!(e.contains(&("GIT_CONFIG_KEY_0".into(), "safe.directory".into())));
        assert!(e.contains(&("GIT_CONFIG_VALUE_0".into(), "/srv/nixos".into())));
        let e = git_safe_env(Some("2"), &[r, Path::new("/x")]);
        assert!(e.contains(&("GIT_CONFIG_VALUE_3".into(), "/x".into())));
        assert!(e.contains(&("GIT_CONFIG_COUNT".into(), "4".into())));
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
    fn timeout_kills_and_reports() {
        let t = Instant::now();
        let err = run_env_timeout(
            "sleep",
            &["30"],
            None,
            &[],
            Duration::from_millis(300),
            &mut |_| {},
        )
        .unwrap_err();
        assert!(t.elapsed() < Duration::from_secs(10));
        assert!(matches!(err, Error::Timeout { .. }));
        assert!(err
            .to_string()
            .contains("délai dépassé après 0 s : sleep 30"));
    }

    #[test]
    fn output_timeout_reports() {
        let err =
            output_timeout("sleep", &["30"], None, &[], Duration::from_millis(300)).unwrap_err();
        assert!(matches!(err, Error::Timeout { .. }));
    }

    #[test]
    fn output_collects_both_streams() {
        let o = output_env("sh", &["-c", "echo a; echo b >&2; exit 2"], None, &[]).unwrap();
        assert_eq!(o.stdout, b"a\n");
        assert_eq!(o.stderr, b"b\n");
        assert_eq!(o.status.code(), Some(2));
    }

    #[test]
    fn grandchild_holding_pipe_does_not_block() {
        let t = Instant::now();
        let mut lines = Vec::new();
        run("sh", &["-c", "echo fini; sleep 20 &"], None, &mut |l| {
            lines.push(l.to_string())
        })
        .unwrap();
        assert!(t.elapsed() < Duration::from_secs(10));
        assert_eq!(lines, vec!["fini"]);
    }

    #[test]
    fn timeout_env_parsing() {
        let d = Duration::from_secs(5);
        assert_eq!(timeout_from(Some("12"), d), Duration::from_secs(12));
        assert_eq!(timeout_from(Some("0"), d), d);
        assert_eq!(timeout_from(Some("abc"), d), d);
        assert_eq!(timeout_from(None, d), d);
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
