//! Lancement de programmes externes avec délai maximal (curl, nix, nix-instantiate).

use std::io::Read;
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Attente maximale de la fin des tubes une fois le processus terminé (un petit-fils peut les
/// garder ouverts) ; même logique que `DRAIN_GRACE` de `nixos-flakes-core`.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Échec d'un lancement avec délai.
#[derive(Debug)]
pub enum RunError {
    /// Programme introuvable.
    NotFound,
    /// Autre échec de lancement ou de lecture.
    Io(std::io::Error),
    /// Délai dépassé ; le programme a été tué.
    Timeout,
}

/// Lit tout `pipe` dans un thread détaché (jamais joint) et envoie le contenu par `tx`.
fn drain<R: Read + Send + 'static>(
    pipe: Option<R>,
    is_stdout: bool,
    tx: mpsc::Sender<(bool, Vec<u8>)>,
) {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        let _ = tx.send((is_stdout, buf));
    });
}

/// Exécute `cmd` (stdin fermé, sorties capturées) ; le tue après `timeout`.
pub fn output_with_timeout(mut cmd: Command, timeout: Duration) -> Result<Output, RunError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // ETXTBSY : exécutable tout juste écrit, descripteur encore copié par un fork concurrent.
    let mut tries = 0;
    let mut child: Child = loop {
        match cmd.spawn() {
            Ok(c) => break c,
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 5 => {
                tries += 1;
                thread::sleep(Duration::from_millis(15));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(RunError::NotFound),
            Err(e) => return Err(RunError::Io(e)),
        }
    };
    let (tx, rx) = mpsc::channel();
    drain(child.stdout.take(), true, tx.clone());
    drain(child.stderr.take(), false, tx);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() >= deadline => {
                // Les lecteurs restent détachés : un petit-fils pourrait garder les tubes ouverts.
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::Timeout);
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(RunError::Io(e)),
        }
    };
    // Récupère les sorties sans attendre indéfiniment un petit-fils qui garderait les tubes.
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let grace = Instant::now() + DRAIN_GRACE;
    for _ in 0..2 {
        let Some(left) = grace.checked_duration_since(Instant::now()) else {
            break;
        };
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

/// `PATH` complété par `/run/wrappers/bin`.
pub fn path_with_wrappers(path: Option<String>) -> String {
    const W: &str = "/run/wrappers/bin";
    match path {
        Some(p) if p.split(':').any(|e| e == W) => p,
        Some(p) if !p.is_empty() => format!("{p}:{W}"),
        _ => W.to_string(),
    }
}

/// Commande avec le `PATH` complété.
pub fn command(program: &str) -> Command {
    let mut c = Command::new(program);
    c.env("PATH", path_with_wrappers(std::env::var("PATH").ok()));
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_output_and_times_out() {
        let mut c = Command::new("sh");
        c.args(["-c", "echo ok; echo err >&2"]);
        let o = output_with_timeout(c, Duration::from_secs(5)).unwrap();
        assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "ok");
        assert_eq!(String::from_utf8_lossy(&o.stderr).trim(), "err");
        let mut c = Command::new("sh");
        c.args(["-c", "sleep 5"]);
        assert!(matches!(
            output_with_timeout(c, Duration::from_millis(100)),
            Err(RunError::Timeout)
        ));
        assert!(matches!(
            output_with_timeout(Command::new("/nonexistent/x"), Duration::from_secs(1)),
            Err(RunError::NotFound)
        ));
    }

    #[test]
    fn grandchild_holding_pipes_does_not_block() {
        let mut c = Command::new("sh");
        c.args(["-c", "sleep 30 & echo ok"]);
        let t = Instant::now();
        let o = output_with_timeout(c, Duration::from_secs(10)).unwrap();
        assert!(t.elapsed() < Duration::from_secs(8));
        assert!(o.status.success());
    }
}
