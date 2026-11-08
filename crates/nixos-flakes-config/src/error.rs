//! Type d'erreur unique de la bibliothèque.

use std::path::PathBuf;

/// Erreur de `nixos-flakes-config`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Le Fichier de config n'est pas du Nix valide.
    #[error("Nix invalide : {0}")]
    Parse(String),
    /// Aucun Emplacement ne porte cet identifiant.
    #[error("Emplacement introuvable : {0}")]
    LocationNotFound(String),
    /// Le Paquet déclaré est déjà présent dans l'Emplacement.
    #[error("« {name} » est déjà déclaré dans {location}")]
    Duplicate { name: String, location: String },
    /// Le Paquet déclaré est absent de l'Emplacement.
    #[error("« {name} » n'est pas déclaré dans {location}")]
    NotFound { name: String, location: String },
    /// Cas que l'édition automatique refuse de toucher (liste ou entrée non littérale).
    #[error("modification non prise en charge : {0}")]
    Unsupported(String),
    /// Aucune source de recherche de paquets n'a répondu (et pas de cache).
    #[error("recherche de paquets impossible : {0}")]
    Search(String),
    /// Erreur d'entrée/sortie sur un fichier.
    #[error("erreur d'entrée/sortie sur {path} : {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Impossible de lancer un programme (autre qu'une absence de `nix-instantiate`).
    #[error("impossible de lancer « {program} » : {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}

/// `Result` de la bibliothèque.
pub type Result<T> = std::result::Result<T, Error>;
