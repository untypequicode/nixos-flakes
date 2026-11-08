//! Synchronisation du dépôt de config système avec son amont (`git fetch`, puis
//! `git pull --ff-only` à la demande). Jamais de fetch périodique : à l'ouverture et sur clic.

use nixos_flakes_core::git::{self, SyncStatus};
use std::path::Path;

/// État de la synchronisation, affiché dans la barre latérale.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SyncState {
    /// Pas encore de fetch (ou pas de dépôt système).
    #[default]
    Idle,
    Fetching,
    Pulling,
    Ready(SyncStatus),
    Failed(String),
}

impl SyncState {
    /// Une opération réseau est en cours.
    pub fn busy(&self) -> bool {
        matches!(self, SyncState::Fetching | SyncState::Pulling)
    }

    /// Texte court de l'état.
    pub fn label(&self) -> String {
        match self {
            SyncState::Idle => "Pas encore vérifié".to_string(),
            SyncState::Fetching => "Récupération des références…".to_string(),
            SyncState::Pulling => "Synchronisation…".to_string(),
            SyncState::Ready(s) => s.summary(),
            SyncState::Failed(e) => e.lines().next().unwrap_or_default().to_string(),
        }
    }
}

/// `git fetch` puis avance/retard.
pub fn fetch_and_status(repo: &Path) -> Result<SyncStatus, String> {
    git::fetch(repo, &mut |_| {}).map_err(|e| e.to_string())?;
    git::sync_status(repo).map_err(|e| e.to_string())
}

/// `git pull --ff-only` (refusé si arbre sale ou historiques divergés), puis nouvel état.
pub fn pull_and_status(repo: &Path) -> Result<SyncStatus, String> {
    git::pull_ff(repo, &mut |_| {}).map_err(|e| e.to_string())?;
    git::sync_status(repo).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_busy() {
        assert!(SyncState::Fetching.busy());
        assert!(!SyncState::Idle.busy());
        let s = SyncStatus {
            behind: 2,
            ..Default::default()
        };
        assert_eq!(SyncState::Ready(s).label(), "2 en retard (à récupérer)");
        assert_eq!(SyncState::Failed("a\nb".into()).label(), "a");
    }

    #[test]
    fn non_repo_fails_with_message() {
        let d = tempfile::tempdir().unwrap();
        assert!(fetch_and_status(d.path()).is_err());
        assert!(pull_and_status(d.path()).is_err());
    }
}
