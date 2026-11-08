//! Terminal intégré (crate `iced_term`) : le shell de l'utilisateur (`$SHELL`), lancé dans
//! le dépôt de config système.

use iced_term::settings::{BackendSettings, Settings};
use std::path::Path;

/// Identifiant de l'unique instance de terminal.
pub const TERM_ID: u64 = 0;

/// Shell à lancer : `$SHELL`, sinon `/bin/sh`.
pub fn shell_program(env: Option<&str>) -> String {
    env.filter(|s| !s.trim().is_empty())
        .unwrap_or("/bin/sh")
        .to_string()
}

/// Dossier de départ : le dépôt de config s'il existe, sinon `$HOME`.
pub fn start_dir(repo: Option<&Path>, home: Option<&Path>) -> Option<std::path::PathBuf> {
    repo.filter(|p| p.is_dir())
        .or(home.filter(|p| p.is_dir()))
        .map(Path::to_path_buf)
}

/// Crée un terminal dans `repo` (repli : `home`).
pub fn spawn(repo: Option<&Path>, home: Option<&Path>) -> Result<iced_term::Terminal, String> {
    let settings = Settings {
        backend: BackendSettings {
            program: shell_program(std::env::var("SHELL").ok().as_deref()),
            working_directory: start_dir(repo, home),
            ..Default::default()
        },
        ..Default::default()
    };
    iced_term::Terminal::new(TERM_ID, settings).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_falls_back_to_sh() {
        assert_eq!(
            shell_program(Some("/run/current-system/sw/bin/zsh")),
            "/run/current-system/sw/bin/zsh"
        );
        assert_eq!(shell_program(Some("  ")), "/bin/sh");
        assert_eq!(shell_program(None), "/bin/sh");
    }

    #[test]
    fn start_dir_prefers_existing_repo() {
        let d = tempfile::tempdir().unwrap();
        let missing = d.path().join("absent");
        assert_eq!(
            start_dir(Some(d.path()), None),
            Some(d.path().to_path_buf())
        );
        assert_eq!(
            start_dir(Some(&missing), Some(d.path())),
            Some(d.path().to_path_buf())
        );
        assert_eq!(start_dir(Some(&missing), None), None);
    }
}
