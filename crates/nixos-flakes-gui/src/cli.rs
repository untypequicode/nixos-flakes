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
    /// `--demo` : faux dépôts et commandes simulées (aucun dépôt réel n'est touché).
    pub demo: bool,
    /// `--demo-view <vues>` : états à ouvrir au démarrage en mode démo (séparés par des virgules).
    pub demo_view: Vec<String>,
}

pub const USAGE: &str = "\
Usage : nixos-flakes-gui [OPTIONS]

Options :
      --tray              Active l'icône de notification (démarre masqué si elle est disponible)
      --config <FICHIER>  Utilise ce fichier de configuration
      --demo              Mode démonstration : faux dépôts et commandes simulées, rien n'est modifié
      --demo-view <VUES>  Avec --demo, ouvre au démarrage des vues (séparées par des virgules) :
                          palette, recherche, modal, reglages, verif, git, erreur, site,
                          replie, toasts, clair
  -h, --help              Affiche cette aide
  -V, --version           Affiche la version";

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
            "--demo" => cli.demo = true,
            "--demo-view" => {
                let v = it
                    .next()
                    .ok_or_else(|| "--demo-view attend une liste de vues".to_string())?;
                cli.demo = true;
                cli.demo_view = v.split(',').map(str::to_string).collect();
            }
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
    fn demo_flags() {
        assert!(parse(["--demo"]).unwrap().demo);
        let c = parse(["--demo-view", "palette,clair"]).unwrap();
        assert!(c.demo);
        assert_eq!(c.demo_view, ["palette", "clair"]);
        assert!(parse(["--demo-view"]).is_err());
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
