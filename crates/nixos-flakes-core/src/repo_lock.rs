//! Verrou inter-processus par dépôt (`flock`), pour qu'une seule opération
//! modifiante (mise à jour du verrou, application, mise à niveau) tourne à la
//! fois sur un même dépôt, quel que soit l'utilisateur ou le processus
//! (CLI, GUI, démon, unité systemd).
//!
//! Le `flock` est pris directement sur le descripteur du DOSSIER du dépôt
//! (chemin canonique), ouvert en lecture seule : aucun fichier n'est créé (ni
//! dans le dépôt, ni dans un dossier partagé où root pourrait être trompé par
//! un lien symbolique), et le verrou est vu de la même façon par root et par
//! l'utilisateur. Il est libéré à la destruction du garde (ou à la mort du
//! processus).

use crate::error::{Error, Result};
use std::fs::{File, OpenOptions, TryLockError};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Durée pendant laquelle un verrou tenu est réessayé avant `RepoBusy`.
const RETRY_WINDOW: Duration = Duration::from_millis(250);
/// Pause entre deux essais.
const RETRY_STEP: Duration = Duration::from_millis(15);

/// `O_DIRECTORY` : la valeur dépend de l'architecture (Linux).
#[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
const O_DIRECTORY: i32 = 0o40000;
#[cfg(not(any(target_arch = "aarch64", target_arch = "arm")))]
const O_DIRECTORY: i32 = 0o200_000;

/// Garde RAII : le verrou du dépôt est tenu tant qu'il existe.
#[derive(Debug)]
pub struct RepoLock {
    _dir: File,
    path: PathBuf,
}

impl RepoLock {
    /// Dossier (canonique) du dépôt verrouillé (diagnostic).
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Prend le verrou de `repo` sans attendre. Erreur explicite s'il est déjà
/// tenu par une autre opération.
pub fn acquire(repo: &Path) -> Result<RepoLock> {
    let canon = repo.canonicalize().map_err(|e| Error::io(repo, e))?;
    // `std` ajoute O_CLOEXEC ; O_DIRECTORY refuse tout ce qui n'est pas un dossier.
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(O_DIRECTORY)
        .open(&canon)
        .map_err(|e| Error::io(&canon, e))?;
    // Un autre thread qui lance un processus (fork avant exec) détient
    // brièvement une copie du descripteur : le verrou d'un garde tout juste
    // détruit peut rester pris un instant. On réessaie donc un court moment.
    let deadline = Instant::now() + RETRY_WINDOW;
    loop {
        match dir.try_lock() {
            Ok(()) => {
                return Ok(RepoLock {
                    _dir: dir,
                    path: canon,
                })
            }
            Err(TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err(Error::RepoBusy);
                }
                std::thread::sleep(RETRY_STEP);
            }
            Err(TryLockError::Error(e)) => return Err(Error::io(canon, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn second_lock_refused_then_accepted_after_drop() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("depot");
        fs::create_dir(&repo).unwrap();
        let a = acquire(&repo).unwrap();
        assert!(a.path().ends_with("depot"));
        let err = acquire(&repo).unwrap_err();
        assert!(matches!(err, Error::RepoBusy));
        assert!(err
            .to_string()
            .contains("une autre opération nixos-flakes est en cours sur ce dépôt"));
        // Un autre dépôt n'est pas bloqué.
        let other = d.path().join("autre");
        fs::create_dir(&other).unwrap();
        let _b = acquire(&other).unwrap();
        drop(a);
        acquire(&repo).unwrap();
    }

    #[test]
    fn no_file_is_created_and_symlinks_share_the_lock() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("depot");
        fs::create_dir(&repo).unwrap();
        let link = d.path().join("lien");
        std::os::unix::fs::symlink(&repo, &link).unwrap();
        let _a = acquire(&repo).unwrap();
        assert!(matches!(acquire(&link), Err(Error::RepoBusy)));
        assert_eq!(fs::read_dir(&repo).unwrap().count(), 0);
    }

    #[test]
    fn missing_or_non_directory_is_an_io_error() {
        let d = tempfile::tempdir().unwrap();
        assert!(matches!(
            acquire(&d.path().join("absent")),
            Err(Error::Io { .. })
        ));
        let f = d.path().join("fichier");
        fs::write(&f, "x").unwrap();
        assert!(matches!(acquire(&f), Err(Error::Io { .. })));
    }
}
