//! Structures sérialisables (JSON stable) pour `list`, `scan` et `status`.

use nixos_flakes_core::config::Role;
use nixos_flakes_core::daemon::{DaemonStatus, UpgradeRecord};
use nixos_flakes_core::git::GitState;
use nixos_flakes_core::lock::InputChange;
use nixos_flakes_core::status::RepoStatus;
use serde::Serialize;

/// Version du schéma JSON de `status` ; à incrémenter en cas de changement incompatible.
pub const SCHEMA_VERSION: u32 = 1;

/// Nom stable (anglais, minuscules) d'un rôle dans le JSON.
pub fn role_key(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::Tracked => "tracked",
        Role::Detected => "detected",
        Role::Ignored => "ignored",
    }
}

#[derive(Debug, Serialize)]
pub struct GitJson {
    pub is_repo: bool,
    pub branch: Option<String>,
    pub dirty: Vec<String>,
    pub lock_dirty: bool,
}

impl From<&GitState> for GitJson {
    fn from(g: &GitState) -> Self {
        GitJson {
            is_repo: g.is_repo,
            branch: g.branch.clone(),
            dirty: g.dirty.clone(),
            lock_dirty: g.lock_dirty,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ChangeJson {
    pub name: String,
    pub old_rev: Option<String>,
    pub new_rev: Option<String>,
    pub old_date: Option<i64>,
    pub new_date: Option<i64>,
}

impl From<&InputChange> for ChangeJson {
    fn from(c: &InputChange) -> Self {
        ChangeJson {
            name: c.name.clone(),
            old_rev: c.old_rev.clone(),
            new_rev: c.new_rev.clone(),
            old_date: c.old_date,
            new_date: c.new_date,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RepoJson {
    pub path: String,
    pub role: &'static str,
    pub git: GitJson,
    /// `null` : mises à jour non vérifiées.
    pub updates: Option<Vec<ChangeJson>>,
    pub error: Option<String>,
}

impl From<&RepoStatus> for RepoJson {
    fn from(s: &RepoStatus) -> Self {
        RepoJson {
            path: s.path.display().to_string(),
            role: role_key(s.role),
            git: GitJson::from(&s.git),
            updates: s
                .pending
                .as_ref()
                .map(|p| p.iter().map(ChangeJson::from).collect()),
            error: s.error.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ScanJson {
    pub path: String,
    pub role: &'static str,
}

#[derive(Debug, Serialize)]
pub struct DaemonJson {
    pub reachable: bool,
    /// Cause quand le démon est injoignable.
    pub error: Option<String>,
    pub status: Option<DaemonStatus>,
}

/// Clé JSON stable de la provenance du dépôt système.
pub fn source_key(s: nixos_flakes_core::config::SystemRepoSource) -> &'static str {
    use nixos_flakes_core::config::SystemRepoSource as S;
    match s {
        S::UserConfig => "user",
        S::Env => "env",
        S::SystemDefault => "system-default",
        S::Legacy => "legacy",
    }
}

#[derive(Debug, Serialize)]
pub struct StatusJson {
    pub schema: u32,
    pub config_path: String,
    pub system_repo: Option<String>,
    /// Provenance : `user`, `env`, `system-default` ou `legacy`.
    pub system_repo_source: Option<&'static str>,
    pub system_attr: Option<String>,
    pub repos: Vec<RepoJson>,
    pub daemon: DaemonJson,
    pub last_upgrade: Option<UpgradeRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn repo_json_has_stable_keys() {
        let st = RepoStatus {
            path: PathBuf::from("/r"),
            role: Role::System,
            git: GitState::default(),
            pending: None,
            error: None,
        };
        let v = serde_json::to_value(RepoJson::from(&st)).unwrap();
        assert_eq!(v["path"], "/r");
        assert_eq!(v["role"], "system");
        assert!(v["updates"].is_null());
        assert_eq!(v["git"]["is_repo"], false);
    }
}
