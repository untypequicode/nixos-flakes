//! Contrat entre le démon `nixos-flakes daemon`, le service de mise à niveau (root) et la GUI.
//!
//! - Fichier d'état écrit par `nixos-flakes upgrade` (root) : [`UpgradeRecord`], lu par le démon.
//! - Interface D-Bus de session exposée par le démon : constantes [`BUS_NAME`], [`OBJECT_PATH`],
//!   [`INTERFACE`], état sérialisé en JSON ([`DaemonStatus`]) et mandataire [`DaemonProxy`]
//!   pour les clients (GUI). Voir `docs/ARCHITECTURE.md`.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Nom de bus de session réclamé par le démon (instance unique).
pub const BUS_NAME: &str = "io.github.untypequicode.NixosFlakes";
/// Chemin de l'objet D-Bus du démon.
pub const OBJECT_PATH: &str = "/io/github/untypequicode/NixosFlakes";
/// Nom de l'interface D-Bus du démon.
pub const INTERFACE: &str = "io.github.untypequicode.NixosFlakes.Daemon";

/// Variable d'environnement qui remplace le chemin du fichier d'état.
pub const STATUS_FILE_ENV: &str = "NIXOS_FLAKES_STATUS_FILE";
/// Chemin par défaut du fichier d'état (répertoire d'état du service root).
pub const DEFAULT_STATUS_FILE: &str = "/var/lib/nixos-flakes/status.json";

/// Chemin du fichier d'état de la mise à niveau automatique.
pub fn status_file_path() -> PathBuf {
    std::env::var_os(STATUS_FILE_ENV)
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from(DEFAULT_STATUS_FILE), PathBuf::from)
}

/// Issue de la dernière mise à niveau automatique.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpgradeOutcomeKind {
    /// Rien à mettre à jour.
    UpToDate,
    /// Mise à jour appliquée.
    Applied,
    /// Application échouée, retour à la génération précédente.
    RolledBack,
    /// Échec avant l'application (mise à jour impossible…).
    Failed,
}

/// Contenu du fichier d'état, écrit par `nixos-flakes upgrade`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpgradeRecord {
    pub outcome: UpgradeOutcomeKind,
    /// Horodatage Unix (secondes) de la fin de l'exécution.
    pub finished_at: i64,
    /// Détail lisible (inputs changés, cause de l'échec…).
    pub message: String,
}

impl UpgradeRecord {
    /// Vrai si la dernière mise à niveau a échoué.
    pub fn is_failure(&self) -> bool {
        matches!(
            self.outcome,
            UpgradeOutcomeKind::RolledBack | UpgradeOutcomeKind::Failed
        )
    }
}

/// Horodatage Unix courant, en secondes.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Écrit le fichier d'état de façon atomique, lisible par tous les utilisateurs.
pub fn write_upgrade_record(path: &Path, record: &UpgradeRecord) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let json = serde_json::to_string_pretty(record).map_err(|e| Error::Config(e.to_string()))?;
    crate::fsutil::write_atomic(path, json.as_bytes(), Some(0o644))
}

/// Lit le fichier d'état ; absent ou illisible => `None`.
pub fn read_upgrade_record(path: &Path) -> Option<UpgradeRecord> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Résumé de l'état global.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateKind {
    /// Pas encore de vérification terminée.
    #[default]
    Unknown,
    UpToDate,
    Updates,
    Error,
}

/// État exposé par le démon (JSON renvoyé par `GetStatus` et porté par `StatusChanged`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub state: StateKind,
    /// Nombre d'inputs à mettre à jour (dépôt système et dépôts suivis).
    pub updates: u32,
    /// Détail d'une erreur, le cas échéant.
    pub message: Option<String>,
    /// Horodatage Unix de la dernière vérification terminée.
    pub checked_at: Option<i64>,
    /// Une vérification est en cours.
    pub checking: bool,
    /// Dernière mise à niveau automatique connue.
    pub upgrade: Option<UpgradeRecord>,
}

/// Résultat d'une vérification des dépôts surveillés.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CheckSummary {
    /// Nombre total d'inputs à mettre à jour.
    pub updates: u32,
    /// Première erreur rencontrée.
    pub error: Option<String>,
    pub checked_at: i64,
}

impl DaemonStatus {
    /// Combine la dernière vérification et la dernière mise à niveau.
    ///
    /// Erreur si la vérification a échoué ou si la dernière mise à niveau a échoué ;
    /// sinon mises à jour si `updates > 0` ; sinon à jour dès qu'une vérification a abouti.
    pub fn combine(
        check: Option<&CheckSummary>,
        upgrade: Option<UpgradeRecord>,
        checking: bool,
    ) -> Self {
        let upgrade_error = upgrade
            .as_ref()
            .filter(|u| u.is_failure())
            .map(|u| format!("Mise à niveau automatique échouée : {}", u.message));
        let check_error = check.and_then(|c| c.error.clone());
        let message = check_error.or(upgrade_error);
        let updates = check.map_or(0, |c| c.updates);
        let state = if message.is_some() {
            StateKind::Error
        } else if updates > 0 {
            StateKind::Updates
        } else if check.is_some() {
            StateKind::UpToDate
        } else {
            StateKind::Unknown
        };
        DaemonStatus {
            state,
            updates,
            message,
            checked_at: check.map(|c| c.checked_at),
            checking,
            upgrade,
        }
    }

    /// Sérialise en JSON (format du fil D-Bus).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    /// Désérialise depuis le JSON du fil D-Bus.
    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).map_err(|e| Error::Config(format!("état du démon : {e}")))
    }
}

/// Mandataire D-Bus (asynchrone) vers le démon, pour la GUI et les autres clients.
#[zbus::proxy(
    interface = "io.github.untypequicode.NixosFlakes.Daemon",
    default_service = "io.github.untypequicode.NixosFlakes",
    default_path = "/io/github/untypequicode/NixosFlakes"
)]
pub trait Daemon {
    /// État courant, JSON de [`DaemonStatus`].
    fn get_status(&self) -> zbus::Result<String>;
    /// Demande une vérification immédiate (retourne sans l'attendre).
    fn check_now(&self) -> zbus::Result<()>;
    /// Émis à chaque changement d'état ; le paramètre est le JSON de [`DaemonStatus`].
    #[zbus(signal)]
    fn status_changed(&self, status: String) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(outcome: UpgradeOutcomeKind) -> UpgradeRecord {
        UpgradeRecord {
            outcome,
            finished_at: 10,
            message: "m".into(),
        }
    }

    #[test]
    fn record_roundtrip_on_disk() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/status.json");
        let r = rec(UpgradeOutcomeKind::RolledBack);
        write_upgrade_record(&p, &r).unwrap();
        assert_eq!(read_upgrade_record(&p), Some(r));
        assert_eq!(read_upgrade_record(&d.path().join("absent")), None);
    }

    #[test]
    fn combine_states() {
        let ok = CheckSummary {
            updates: 0,
            error: None,
            checked_at: 5,
        };
        let upd = CheckSummary {
            updates: 2,
            ..ok.clone()
        };
        let err = CheckSummary {
            error: Some("x".into()),
            ..ok.clone()
        };
        assert_eq!(
            DaemonStatus::combine(None, None, false).state,
            StateKind::Unknown
        );
        assert_eq!(
            DaemonStatus::combine(Some(&ok), None, false).state,
            StateKind::UpToDate
        );
        let s = DaemonStatus::combine(Some(&upd), Some(rec(UpgradeOutcomeKind::Applied)), true);
        assert_eq!(
            (s.state, s.updates, s.checking),
            (StateKind::Updates, 2, true)
        );
        assert_eq!(
            DaemonStatus::combine(Some(&err), None, false).state,
            StateKind::Error
        );
        let s = DaemonStatus::combine(Some(&ok), Some(rec(UpgradeOutcomeKind::Failed)), false);
        assert_eq!(s.state, StateKind::Error);
        assert!(s.message.unwrap().contains("échouée"));
    }

    #[test]
    fn json_roundtrip() {
        let s = DaemonStatus::combine(None, Some(rec(UpgradeOutcomeKind::Applied)), false);
        assert_eq!(DaemonStatus::from_json(&s.to_json()).unwrap(), s);
        assert!(DaemonStatus::from_json("pas du json").is_err());
    }
}
