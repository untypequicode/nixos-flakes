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
    fn update(
        &mut self,
        repo: &Path,
        inputs: &[String],
        out: &mut dyn FnMut(&str),
    ) -> Result<UpdateReport>;
    fn apply(&mut self, repo: &Path, out: &mut dyn FnMut(&str)) -> Result<()>;
    fn rollback(&mut self, out: &mut dyn FnMut(&str)) -> Result<()>;
}

struct Real<'a>(Privilege, Option<&'a str>);

impl Backend for Real<'_> {
    fn update(
        &mut self,
        repo: &Path,
        inputs: &[String],
        out: &mut dyn FnMut(&str),
    ) -> Result<UpdateReport> {
        // Le verrou du dépôt est déjà tenu par `upgrade_inputs`.
        nix::update_inputs_unlocked(repo, inputs, out)
    }
    fn apply(&mut self, repo: &Path, out: &mut dyn FnMut(&str)) -> Result<()> {
        nix::apply_unlocked(repo, self.1, self.0, out)
    }
    fn rollback(&mut self, out: &mut dyn FnMut(&str)) -> Result<()> {
        nix::rollback(self.0, out)
    }
}

/// Met à jour `repo`, l'applique (configuration `attr`, sinon le nom d'hôte) si des inputs ont changé ; si l'application
/// échoue, restaure le `flake.lock` d'origine puis revient à la génération
/// précédente.
pub fn upgrade(
    repo: &Path,
    attr: Option<&str>,
    privilege: Privilege,
    out: &mut dyn FnMut(&str),
) -> Result<UpgradeOutcome> {
    upgrade_inputs(repo, &[], attr, privilege, out)
}

/// Comme `upgrade`, limité aux inputs `inputs` (vide = tous). En cas d'échec
/// de l'application, le `flake.lock` d'origine est restauré en entier.
pub fn upgrade_inputs(
    repo: &Path,
    inputs: &[String],
    attr: Option<&str>,
    privilege: Privilege,
    out: &mut dyn FnMut(&str),
) -> Result<UpgradeOutcome> {
    if let Some(a) = attr {
        crate::config::validate_attr(a)?;
    }
    // Verrou pris une seule fois pour toute la séquence ; `Real` appelle les
    // variantes sans verrou de `nix` (sinon auto-blocage).
    let _lock = crate::repo_lock::acquire(repo)?;
    // Les fichiers créés par root dans le dépôt reviennent à leur propriétaire d'origine.
    let _owner = crate::fsutil::OwnerGuard::new(repo);
    upgrade_with(&mut Real(privilege, attr), repo, inputs, out)
}

fn restore_lock(path: &Path, backup: &Option<Vec<u8>>) -> Result<()> {
    match backup {
        Some(bytes) => crate::fsutil::write_atomic(path, bytes, None),
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(Error::io(path, e)),
            _ => Ok(()),
        },
    }
}

/// Vrai si l'échec prouve que `nixos-rebuild switch` n'a pas été lancé
/// (programme introuvable, élévation de privilèges refusée ou fermée) : un
/// retour arrière serait alors inutile, voire nuisible.
fn switch_never_started(e: &Error) -> bool {
    match e {
        Error::Spawn { .. } => true,
        Error::Command {
            program, status, ..
        } => program == "pkexec" && (status == "code 126" || status == "code 127"),
        _ => false,
    }
}

pub(crate) fn upgrade_with(
    backend: &mut dyn Backend,
    repo: &Path,
    inputs: &[String],
    out: &mut dyn FnMut(&str),
) -> Result<UpgradeOutcome> {
    let lock_path = repo.join("flake.lock");
    let backup = match std::fs::read(&lock_path) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(Error::io(&lock_path, e)),
    };

    out("Mise à jour des verrous…");
    let report = match backend.update(repo, inputs, out) {
        Ok(r) => r,
        Err(e) => {
            return Err(match restore_lock(&lock_path, &backup) {
                Ok(()) => e,
                Err(re) => Error::RestoreFailed {
                    original: e.to_string(),
                    restore: re.to_string(),
                },
            });
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
            // Délai dépassé : un `switch` orphelin peut encore tourner. Toucher à flake.lock ou
            // lancer un retour arrière en parallèle serait dangereux : on s'arrête là.
            if let Error::Timeout { secs, .. } = &apply_err {
                out(&format!("Échec de l'application : {apply_err}"));
                out("Pas de retour arrière ni de restauration de flake.lock : un switch interrompu peut encore tourner.");
                return Err(Error::ApplyTimedOut { secs: *secs });
            }
            let mut reason = apply_err.to_string();
            out(&format!("Échec de l'application : {apply_err}"));
            if let Err(e) = restore_lock(&lock_path, &backup) {
                reason.push_str(&format!("\nRestauration de flake.lock impossible : {e}"));
            } else {
                out("flake.lock d'origine restauré.");
            }
            if switch_never_started(&apply_err) {
                out("L'application n'a pas démarré : pas de retour arrière nécessaire.");
            } else {
                out("Retour à la génération précédente…");
                if let Err(e) = backend.rollback(out) {
                    reason.push_str(&format!("\nRetour arrière impossible : {e}"));
                }
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
        apply_err: Option<Error>,
        calls: Vec<&'static str>,
    }

    fn fake(changes: usize, apply_ok: bool) -> Fake {
        Fake {
            new_lock: "NOUVEAU",
            changes,
            apply_ok,
            rollback_ok: true,
            update_ok: true,
            apply_err: None,
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
        fn update(
            &mut self,
            repo: &Path,
            _: &[String],
            _: &mut dyn FnMut(&str),
        ) -> Result<UpdateReport> {
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
            if let Some(e) = self.apply_err.take() {
                return Err(e);
            }
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
        let r = upgrade_with(&mut f, d.path(), &[], &mut |_| {}).unwrap();
        assert_eq!(r, UpgradeOutcome::AlreadyUpToDate);
        assert_eq!(f.calls, ["update"]);
    }

    #[test]
    fn success_keeps_new_lock() {
        let d = repo_with_lock("ORIGINE");
        let mut f = fake(2, true);
        let r = upgrade_with(&mut f, d.path(), &[], &mut |_| {}).unwrap();
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
        let r = upgrade_with(&mut f, d.path(), &[], &mut |l| lines.push(l.to_string())).unwrap();
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
    fn apply_timeout_neither_rolls_back_nor_restores_lock() {
        let d = repo_with_lock("ORIGINE");
        let mut f = fake(1, false);
        f.apply_err = Some(Error::Timeout {
            command: "nixos-rebuild switch".into(),
            secs: 21600,
        });
        let r = upgrade_with(&mut f, d.path(), &[], &mut |_| {});
        assert!(matches!(r, Err(Error::ApplyTimedOut { secs: 21600 })));
        assert_eq!(f.calls, ["update", "apply"]);
        assert_eq!(
            fs::read_to_string(d.path().join("flake.lock")).unwrap(),
            "NOUVEAU"
        );
    }

    #[test]
    fn rollback_failure_is_reported_in_reason() {
        let d = repo_with_lock("ORIGINE");
        let mut f = fake(1, false);
        f.rollback_ok = false;
        let r = upgrade_with(&mut f, d.path(), &[], &mut |_| {}).unwrap();
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
        upgrade_with(&mut f, d.path(), &[], &mut |_| {}).unwrap();
        assert!(!d.path().join("flake.lock").exists());
    }

    #[test]
    fn update_failure_restores_lock_and_errors() {
        let d = repo_with_lock("ORIGINE");
        let mut f = fake(1, true);
        f.update_ok = false;
        assert!(upgrade_with(&mut f, d.path(), &[], &mut |_| {}).is_err());
        assert_eq!(f.calls, ["update"]);
        assert_eq!(
            fs::read_to_string(d.path().join("flake.lock")).unwrap(),
            "ORIGINE"
        );
    }
}
