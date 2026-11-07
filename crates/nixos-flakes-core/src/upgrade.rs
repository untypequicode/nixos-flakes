//! Mise à niveau automatique : mise à jour, application, retour arrière.

use crate::error::{Error, Result};
use crate::nix::{self, Privilege, UpdateReport};
use std::path::Path;

/// Issue d'une mise à niveau automatique.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeOutcome {
    AlreadyUpToDate,
    Applied(UpdateReport),
    RolledBack {
        report: UpdateReport,
        reason: String,
    },
}

/// Opérations nécessaires, abstraites pour pouvoir les tester.
pub(crate) trait Backend {
    fn update(&mut self, repo: &Path, out: &mut dyn FnMut(&str)) -> Result<UpdateReport>;
    fn apply(&mut self, repo: &Path, out: &mut dyn FnMut(&str)) -> Result<()>;
    fn rollback(&mut self, out: &mut dyn FnMut(&str)) -> Result<()>;
}

struct Real(Privilege);

impl Backend for Real {
    fn update(&mut self, repo: &Path, out: &mut dyn FnMut(&str)) -> Result<UpdateReport> {
        nix::update(repo, out)
    }
    fn apply(&mut self, repo: &Path, out: &mut dyn FnMut(&str)) -> Result<()> {
        nix::apply(repo, self.0, out)
    }
    fn rollback(&mut self, out: &mut dyn FnMut(&str)) -> Result<()> {
        nix::rollback(self.0, out)
    }
}

/// Met à jour `repo`, l'applique si des inputs ont changé ; si l'application
/// échoue, restaure le `flake.lock` d'origine puis revient à la génération
/// précédente.
pub fn upgrade(
    repo: &Path,
    privilege: Privilege,
    out: &mut dyn FnMut(&str),
) -> Result<UpgradeOutcome> {
    upgrade_with(&mut Real(privilege), repo, out)
}

fn restore_lock(path: &Path, backup: &Option<Vec<u8>>) -> Result<()> {
    match backup {
        Some(bytes) => std::fs::write(path, bytes).map_err(|e| Error::io(path, e)),
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(Error::io(path, e)),
            _ => Ok(()),
        },
    }
}

pub(crate) fn upgrade_with(
    backend: &mut dyn Backend,
    repo: &Path,
    out: &mut dyn FnMut(&str),
) -> Result<UpgradeOutcome> {
    let lock_path = repo.join("flake.lock");
    let backup = match std::fs::read(&lock_path) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(Error::io(&lock_path, e)),
    };

    out("Mise à jour des verrous…");
    let report = match backend.update(repo, out) {
        Ok(r) => r,
        Err(e) => {
            let _ = restore_lock(&lock_path, &backup);
            return Err(e);
        }
    };
    if report.changes.is_empty() {
        out("Tout est à jour.");
        return Ok(UpgradeOutcome::AlreadyUpToDate);
    }

    out("Application de la configuration…");
    match backend.apply(repo, out) {
        Ok(()) => Ok(UpgradeOutcome::Applied(report)),
        Err(apply_err) => {
            let mut reason = apply_err.to_string();
            out(&format!("Échec de l'application : {apply_err}"));
            if let Err(e) = restore_lock(&lock_path, &backup) {
                reason.push_str(&format!("\nRestauration de flake.lock impossible : {e}"));
            } else {
                out("flake.lock d'origine restauré.");
            }
            out("Retour à la génération précédente…");
            if let Err(e) = backend.rollback(out) {
                reason.push_str(&format!("\nRetour arrière impossible : {e}"));
            }
            Ok(UpgradeOutcome::RolledBack { report, reason })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::InputChange;
    use std::fs;

    struct Fake {
        new_lock: &'static str,
        changes: usize,
        apply_ok: bool,
        rollback_ok: bool,
        update_ok: bool,
        calls: Vec<&'static str>,
    }

    fn fake(changes: usize, apply_ok: bool) -> Fake {
        Fake {
            new_lock: "NOUVEAU",
            changes,
            apply_ok,
            rollback_ok: true,
            update_ok: true,
            calls: vec![],
        }
    }

    fn cmd_err() -> Error {
        Error::Command {
            program: "nixos-rebuild".into(),
            status: "code 1".into(),
            tail: "build cassé".into(),
        }
    }

    impl Backend for Fake {
        fn update(&mut self, repo: &Path, _: &mut dyn FnMut(&str)) -> Result<UpdateReport> {
            self.calls.push("update");
            fs::write(repo.join("flake.lock"), self.new_lock).unwrap();
            if !self.update_ok {
                return Err(cmd_err());
            }
            Ok(UpdateReport {
                repo: repo.to_path_buf(),
                changes: (0..self.changes)
                    .map(|i| InputChange {
                        name: format!("i{i}"),
                        old_rev: None,
                        new_rev: None,
                        old_date: None,
                        new_date: None,
                    })
                    .collect(),
            })
        }
        fn apply(&mut self, _: &Path, _: &mut dyn FnMut(&str)) -> Result<()> {
            self.calls.push("apply");
            if self.apply_ok {
                Ok(())
            } else {
                Err(cmd_err())
            }
        }
        fn rollback(&mut self, _: &mut dyn FnMut(&str)) -> Result<()> {
            self.calls.push("rollback");
            if self.rollback_ok {
                Ok(())
            } else {
                Err(cmd_err())
            }
        }
    }

    fn repo_with_lock(content: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("flake.lock"), content).unwrap();
        d
    }

    #[test]
    fn up_to_date_does_not_apply() {
        let d = repo_with_lock("ORIGINE");
        let mut f = fake(0, true);
        let r = upgrade_with(&mut f, d.path(), &mut |_| {}).unwrap();
        assert_eq!(r, UpgradeOutcome::AlreadyUpToDate);
        assert_eq!(f.calls, ["update"]);
    }

    #[test]
    fn success_keeps_new_lock() {
        let d = repo_with_lock("ORIGINE");
        let mut f = fake(2, true);
        let r = upgrade_with(&mut f, d.path(), &mut |_| {}).unwrap();
        assert!(matches!(r, UpgradeOutcome::Applied(ref rep) if rep.changes.len() == 2));
        assert_eq!(f.calls, ["update", "apply"]);
        assert_eq!(
            fs::read_to_string(d.path().join("flake.lock")).unwrap(),
            "NOUVEAU"
        );
    }

    #[test]
    fn apply_failure_restores_lock_then_rolls_back() {
        let d = repo_with_lock("ORIGINE");
        let mut f = fake(1, false);
        let mut lines = Vec::new();
        let r = upgrade_with(&mut f, d.path(), &mut |l| lines.push(l.to_string())).unwrap();
        match r {
            UpgradeOutcome::RolledBack { report, reason } => {
                assert_eq!(report.changes.len(), 1);
                assert!(reason.contains("build cassé"));
            }
            o => panic!("inattendu : {o:?}"),
        }
        assert_eq!(f.calls, ["update", "apply", "rollback"]);
        assert_eq!(
            fs::read_to_string(d.path().join("flake.lock")).unwrap(),
            "ORIGINE"
        );
        assert!(lines.iter().any(|l| l.contains("restauré")));
    }

    #[test]
    fn rollback_failure_is_reported_in_reason() {
        let d = repo_with_lock("ORIGINE");
        let mut f = fake(1, false);
        f.rollback_ok = false;
        let r = upgrade_with(&mut f, d.path(), &mut |_| {}).unwrap();
        match r {
            UpgradeOutcome::RolledBack { reason, .. } => {
                assert!(reason.contains("Retour arrière impossible"))
            }
            o => panic!("inattendu : {o:?}"),
        }
    }

    #[test]
    fn missing_lock_is_removed_on_rollback() {
        let d = tempfile::tempdir().unwrap();
        let mut f = fake(1, false);
        upgrade_with(&mut f, d.path(), &mut |_| {}).unwrap();
        assert!(!d.path().join("flake.lock").exists());
    }

    #[test]
    fn update_failure_restores_lock_and_errors() {
        let d = repo_with_lock("ORIGINE");
        let mut f = fake(1, true);
        f.update_ok = false;
        assert!(upgrade_with(&mut f, d.path(), &mut |_| {}).is_err());
        assert_eq!(f.calls, ["update"]);
        assert_eq!(
            fs::read_to_string(d.path().join("flake.lock")).unwrap(),
            "ORIGINE"
        );
    }
}
