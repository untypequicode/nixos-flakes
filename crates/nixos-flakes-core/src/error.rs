//! Type d'erreur unique de la bibliothèque.

use std::path::PathBuf;

/// Erreur de `nixos-flakes-core`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Erreur d'entrée/sortie sur un fichier.
    #[error("erreur d'entrée/sortie sur {path} : {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Impossible de lancer un programme (introuvable, droits…).
    #[error("impossible de lancer « {program} » : {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    /// Un programme a terminé avec un code d'erreur ; `tail` contient la fin de sa sortie.
    #[error("« {program} » a échoué ({status}){}", format_tail(.tail))]
    Command {
        program: String,
        status: String,
        tail: String,
    },
    /// Un programme a dépassé son délai maximal et a été tué.
    #[error("délai dépassé après {secs} s : {command}")]
    Timeout { command: String, secs: u64 },
    /// `nixos-rebuild switch` a dépassé son délai : un processus orphelin peut encore tourner.
    #[error(
        "l'application (nixos-rebuild switch) a dépassé {secs} s et a été interrompue ; un processus orphelin peut encore tourner. Ni flake.lock ni la génération n'ont été restaurés (pas de retour arrière automatique)"
    )]
    ApplyTimedOut { secs: u64 },
    /// Le verrou du dépôt est tenu par une autre opération.
    #[error("une autre opération nixos-flakes est en cours sur ce dépôt")]
    RepoBusy,
    /// Échec d'une opération suivi de l'échec de la restauration de `flake.lock`.
    #[error("{original}\nRestauration de flake.lock impossible : {restore}")]
    RestoreFailed { original: String, restore: String },
    /// Opération git refusée ou impossible (message destiné à l'utilisateur).
    #[error("git : {0}")]
    Git(String),
    /// `flake.lock` illisible.
    #[error("flake.lock invalide : {0}")]
    Lock(String),
    /// Fichier de configuration illisible ou non sérialisable.
    #[error("configuration invalide : {0}")]
    Config(String),
}

fn format_tail(tail: &str) -> String {
    if tail.is_empty() {
        String::new()
    } else {
        format!(" :\n{tail}")
    }
}

/// Résultat de la bibliothèque.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}
