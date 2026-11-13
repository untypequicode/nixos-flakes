//! Échec d'une commande : message, piste éventuelle et code de sortie.

use nixos_flakes_core::error::Error as CoreError;
use std::process::ExitCode;

/// Codes de sortie (voir `cli::EXIT_HELP`).
pub const EXIT_ERROR: u8 = 1;
pub const EXIT_CANCELLED: u8 = 3;
pub const EXIT_BUSY: u8 = 4;
pub const EXIT_TIMEOUT: u8 = 5;
/// `check --exit-code` : des mises à jour sont disponibles.
pub const EXIT_UPDATES: u8 = 10;

/// Échec affiché sur stderr (`erreur : …`, puis `piste : …`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    pub hint: Option<String>,
    pub code: u8,
}

impl Failure {
    pub fn new(message: impl Into<String>) -> Self {
        Failure {
            message: message.into(),
            hint: None,
            code: EXIT_ERROR,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_code(mut self, code: u8) -> Self {
        self.code = code;
        self
    }

    /// Préfixe le message (« fichier : message »).
    pub fn prefixed(mut self, prefix: &str) -> Self {
        self.message = format!("{prefix} : {}", self.message);
        self
    }

    /// Code de sortie du processus.
    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.code)
    }

    /// Texte complet écrit sur stderr.
    pub fn render(&self) -> String {
        match &self.hint {
            Some(h) => format!("erreur : {}\npiste : {h}", self.message),
            None => format!("erreur : {}", self.message),
        }
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Failure::new(message)
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Failure::new(message)
    }
}

impl From<CoreError> for Failure {
    fn from(e: CoreError) -> Self {
        match &e {
            CoreError::RepoBusy => Failure::new(e.to_string())
                .with_hint(
                    "attendez la fin de l'opération en cours (interface, démon ou minuteur systemd), puis relancez.",
                )
                .with_code(EXIT_BUSY),
            CoreError::Timeout { .. } => Failure::new(e.to_string())
                .with_hint(
                    "vérifiez le réseau ; pour une opération légitimement longue, augmentez NIXOS_FLAKES_TIMEOUT_SECS (secondes).",
                )
                .with_code(EXIT_TIMEOUT),
            CoreError::ApplyTimedOut { .. } => Failure::new(e.to_string())
                .with_hint(
                    "vérifiez l'état du système (processus nixos-rebuild, « nixos-rebuild list-generations ») avant de relancer ; flake.lock contient les nouvelles versions.",
                )
                .with_code(EXIT_TIMEOUT),
            CoreError::Spawn { source, program }
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                Failure::new(e.to_string()).with_hint(format!(
                    "installez « {program} » ou ajoutez-le au PATH (« nixos-flakes doctor » vérifie l'installation)."
                ))
            }
            _ => Failure::new(e.to_string()),
        }
    }
}

impl From<nixos_flakes_config::Error> for Failure {
    fn from(e: nixos_flakes_config::Error) -> Self {
        match &e {
            nixos_flakes_config::Error::Timeout { .. } => Failure::new(e.to_string())
                .with_hint("vérifiez le réseau et l'état de Nix, puis réessayez.")
                .with_code(EXIT_TIMEOUT),
            _ => Failure::new(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_and_timeout_have_codes_and_hints() {
        let busy = Failure::from(CoreError::RepoBusy);
        assert_eq!(busy.code, EXIT_BUSY);
        assert!(busy.hint.is_some());
        let t = Failure::from(CoreError::Timeout {
            command: "nix".into(),
            secs: 3,
        });
        assert_eq!(t.code, EXIT_TIMEOUT);
        assert!(t.render().starts_with("erreur : délai dépassé"));
        assert!(t.render().contains("\npiste : "));
    }

    #[test]
    fn plain_errors_exit_one_without_hint() {
        let f = Failure::from("boum");
        assert_eq!((f.code, f.render()), (1, "erreur : boum".to_string()));
    }
}
