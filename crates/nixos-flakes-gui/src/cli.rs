//! Analyse de la ligne de commande (sans dépendance, testable).

use std::path::PathBuf;

/// Options de la ligne de commande.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cli {
    /// `--tray` : active la zone de notification et démarre fenêtre masquée.
    pub tray: bool,
    /// `--config <fichier>` : fichier de configuration alternatif.
    pub config: Option<PathBuf>,
    /// `--help` / `-h`.
    pub help: bool,
    /// `--version` / `-V`.
    pub version: bool,
}

pub const USAGE: &str = "\
Usage : nixos-flakes-gui [OPTIONS]

Options :
      --tray            Active l'icône de notification (démarre masqué si elle est disponible)
      --config <FICHIER>  Utilise ce fichier de configuration
  -h, --help            Affiche cette aide
  -V, --version         Affiche la version";

/// Analyse les arguments (sans le nom du programme).
pub fn parse<I, S>(args: I) -> Result<Cli, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut cli = Cli::default();
    let mut it = args.into_iter().map(Into::into);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--tray" => cli.tray = true,
            "-h" | "--help" => cli.help = true,
            "-V" | "--version" => cli.version = true,
            "--config" => {
                let v = it
                    .next()
                    .ok_or_else(|| "--config attend un chemin".to_string())?;
                cli.config = Some(PathBuf::from(v));
            }
            other => {
                if let Some(v) = other.strip_prefix("--config=") {
                    cli.config = Some(PathBuf::from(v));
                } else {
                    return Err(format!("option inconnue : {other}"));
                }
            }
        }
    }
    Ok(cli)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_default() {
        assert_eq!(parse(Vec::<String>::new()).unwrap(), Cli::default());
    }

    #[test]
    fn tray_flag() {
        assert!(parse(["--tray"]).unwrap().tray);
        assert!(!parse(["--help"]).unwrap().tray);
    }

    #[test]
    fn help_and_version() {
        assert!(parse(["-h"]).unwrap().help);
        assert!(parse(["--version"]).unwrap().version);
    }

    #[test]
    fn config_forms() {
        assert_eq!(
            parse(["--config", "/x.toml"]).unwrap().config,
            Some(PathBuf::from("/x.toml"))
        );
        assert_eq!(
            parse(["--config=/y.toml"]).unwrap().config,
            Some(PathBuf::from("/y.toml"))
        );
        assert!(parse(["--config"]).is_err());
    }

    #[test]
    fn unknown_option_is_error() {
        let e = parse(["--tray", "--bof"]).unwrap_err();
        assert!(e.contains("--bof"));
    }
}
