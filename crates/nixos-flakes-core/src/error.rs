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
