//! Agrégation de l'état d'un dépôt pour la GUI et la CLI.

use crate::config::Role;
use crate::git::{self, GitState};
use crate::lock::InputChange;
use crate::nix;
use std::path::{Path, PathBuf};

/// État d'un dépôt flake.
#[derive(Debug, Clone)]
pub struct RepoStatus {
    pub path: PathBuf,
    pub role: Role,
    pub git: GitState,
    /// `None` = pas encore vérifié.
    pub pending: Option<Vec<InputChange>>,
    pub error: Option<String>,
}

/// Rassemble l'état git et, si `with_check`, les mises à jour en attente.
pub fn status(path: &Path, role: Role, with_check: bool) -> RepoStatus {
    status_with(path, role, with_check, nix::check)
}

fn status_with(
    path: &Path,
    role: Role,
    with_check: bool,
    check: impl FnOnce(&Path) -> crate::Result<nix::UpdateReport>,
) -> RepoStatus {
    let mut error = None;
    let git = git::state(path).unwrap_or_else(|e| {
        error = Some(e.to_string());
        GitState::default()
    });
    let mut pending = None;
    if with_check {
        match check(path) {
            Ok(r) => pending = Some(r.changes),
            Err(e) => error = Some(e.to_string()),
        }
    }
    RepoStatus {
        path: path.to_path_buf(),
        role,
        git,
        pending,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::nix::UpdateReport;

    #[test]
    fn without_check_is_unverified() {
        let d = tempfile::tempdir().unwrap();
        let s = status_with(d.path(), Role::Tracked, false, |_| panic!("pas de check"));
        assert_eq!(s.pending, None);
        assert_eq!(s.error, None);
        assert!(!s.git.is_repo);
        assert_eq!(s.role, Role::Tracked);
    }

    #[test]
    fn check_ok_and_error() {
        let d = tempfile::tempdir().unwrap();
        let s = status_with(d.path(), Role::System, true, |p| {
            Ok(UpdateReport {
                repo: p.to_path_buf(),
                changes: vec![],
            })
        });
        assert_eq!(s.pending, Some(vec![]));
        let s = status_with(d.path(), Role::System, true, |_| {
            Err(Error::Lock("cassé".into()))
        });
        assert_eq!(s.pending, None);
        assert!(s.error.unwrap().contains("cassé"));
    }
}
