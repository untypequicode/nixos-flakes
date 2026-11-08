//! Définition des arguments de la ligne de commande (clap).

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// Gestion des flakes Nix de la machine.
#[derive(Debug, Parser, PartialEq, Eq)]
#[command(name = "nixos-flakes", version, about, long_about = None)]
pub struct Cli {
    /// Fichier de configuration (défaut : $XDG_CONFIG_HOME/nixos-flakes/config.toml).
    #[arg(long, global = true, value_name = "FICHIER")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

/// Sous-commandes.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Command {
    /// Liste les dépôts suivis (le dépôt système inclus).
    List {
        /// Ajoute les dépôts détectés par le scan.
        #[arg(long)]
        all: bool,
        /// Calcule les mises à jour en attente (lance nix, plus lent).
        #[arg(long)]
        check: bool,
    },
    /// Recherche les dépôts flake sous les racines de scan.
    Scan,
    /// Affiche les mises à jour en attente d'un dépôt, sans rien modifier.
    Check {
        /// Dépôt à vérifier (défaut : dépôt système).
        path: Option<PathBuf>,
        /// Sortie JSON pour Waybar ; sort toujours avec le code 0.
        #[arg(long)]
        waybar: bool,
    },
    /// Met à jour les verrous (flake.lock) d'un dépôt.
    Update {
        /// Dépôt à mettre à jour (défaut : dépôt système).
        path: Option<PathBuf>,
        /// Valide flake.lock dans git si des inputs ont changé.
        #[arg(long)]
        commit: bool,
        /// Ne met à jour que cet input (répétable) ; sans option : tous les inputs.
        #[arg(long = "input", value_name = "NOM")]
        inputs: Vec<String>,
    },
    /// Applique la configuration d'un dépôt à la machine (nixos-rebuild switch).
    Apply {
        /// Dépôt à appliquer (défaut : dépôt système).
        path: Option<PathBuf>,
    },
    /// Mise à niveau automatique : mise à jour, application, retour arrière en cas d'échec.
    Upgrade {
        /// Dépôt à mettre à niveau (défaut : dépôt système).
        path: Option<PathBuf>,
        /// Envoie une notification de bureau.
        #[arg(long)]
        notify: bool,
        /// Élève les privilèges avec sudo au lieu de pkexec.
        #[arg(long)]
        sudo: bool,
    },
    /// Marque un dépôt comme suivi.
    Track {
        /// Dossier du dépôt flake.
        path: PathBuf,
    },
    /// Retire un dépôt de la liste des dépôts suivis (il redevient détecté).
    Untrack {
        /// Dossier du dépôt flake.
        path: PathBuf,
    },
    /// Ignore définitivement un dépôt.
    Ignore {
        /// Dossier du dépôt flake.
        path: PathBuf,
    },
    /// Recherche et édition des paquets déclarés dans les Fichiers de config.
    Pkg {
        #[command(subcommand)]
        command: PkgCommand,
    },
    /// Désigne le dépôt système (l'ancien devient suivi).
    SetSystem {
        /// Dossier du dépôt flake.
        path: PathBuf,
    },
}

/// Options communes aux commandes qui modifient un Fichier de config.
#[derive(Debug, Args, PartialEq, Eq, Default)]
pub struct WriteOpts {
    /// Dépôt à modifier (défaut : dépôt système).
    #[arg(long, value_name = "CHEMIN")]
    pub repo: Option<PathBuf>,
    /// Écrit sans demander confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// Affiche le diff sans rien écrire.
    #[arg(long)]
    pub dry_run: bool,
    /// Élève les privilèges avec sudo au lieu de pkexec si le dossier n'est pas modifiable.
    #[arg(long)]
    pub sudo: bool,
}

/// Sous-commandes de `pkg`.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum PkgCommand {
    /// Cherche un paquet nixpkgs (search.nixos.org, puis nix search, puis cache).
    Search {
        /// Mot recherché.
        query: String,
        /// Nombre maximal de résultats.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Liste les Fichiers de config, leurs Emplacements et les paquets déclarés.
    List {
        /// Dépôt à lire (défaut : dépôt système).
        #[arg(long, value_name = "CHEMIN")]
        repo: Option<PathBuf>,
    },
    /// Déclare un paquet dans un Emplacement.
    Add {
        /// Nom du paquet (attribut nixpkgs).
        name: String,
        /// Fichier de config (relatif au dépôt).
        #[arg(long, value_name = "FICHIER")]
        file: PathBuf,
        /// Identifiant de l'Emplacement (voir `pkg list`).
        #[arg(long, value_name = "ID")]
        location: String,
        #[command(flatten)]
        opts: WriteOpts,
    },
    /// Retire un paquet déclaré d'un Emplacement.
    Remove {
        name: String,
        #[arg(long, value_name = "FICHIER")]
        file: PathBuf,
        #[arg(long, value_name = "ID")]
        location: String,
        #[command(flatten)]
        opts: WriteOpts,
    },
    /// Déplace un paquet déclaré d'un Emplacement vers un autre.
    Move {
        name: String,
        /// Origine, au format FICHIER:ID.
        #[arg(long, value_name = "FICHIER:ID")]
        from: String,
        /// Destination, au format FICHIER:ID.
        #[arg(long, value_name = "FICHIER:ID")]
        to: String,
        #[command(flatten)]
        opts: WriteOpts,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("nixos-flakes").chain(args.iter().copied()))
    }

    #[test]
    fn list_flags() {
        let cli = parse(&["list", "--all", "--check"]).unwrap();
        assert_eq!(
            cli.command,
            Command::List {
                all: true,
                check: true
            }
        );
        assert_eq!(cli.config, None);
    }

    #[test]
    fn global_config_before_and_after_subcommand() {
        let before = parse(&["--config", "/x.toml", "scan"]).unwrap();
        let after = parse(&["scan", "--config", "/x.toml"]).unwrap();
        assert_eq!(before, after);
        assert_eq!(before.config, Some(PathBuf::from("/x.toml")));
    }

    #[test]
    fn check_waybar_with_path() {
        let cli = parse(&["check", "/etc/nixos", "--waybar"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Check {
                path: Some(PathBuf::from("/etc/nixos")),
                waybar: true
            }
        );
    }

    #[test]
    fn defaults_without_path() {
        assert_eq!(
            parse(&["update"]).unwrap().command,
            Command::Update {
                path: None,
                commit: false,
                inputs: vec![]
            }
        );
        assert_eq!(
            parse(&["apply"]).unwrap().command,
            Command::Apply { path: None }
        );
    }

    #[test]
    fn update_input_is_repeatable() {
        let cli = parse(&[
            "update",
            "/etc/nixos",
            "--input",
            "nixpkgs",
            "--input",
            "home",
        ])
        .unwrap();
        assert_eq!(
            cli.command,
            Command::Update {
                path: Some(PathBuf::from("/etc/nixos")),
                commit: false,
                inputs: vec!["nixpkgs".into(), "home".into()]
            }
        );
    }

    #[test]
    fn upgrade_flags() {
        let cli = parse(&["upgrade", "--notify", "--sudo"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Upgrade {
                path: None,
                notify: true,
                sudo: true
            }
        );
    }

    #[test]
    fn role_commands_need_a_path() {
        assert!(parse(&["track"]).is_err());
        assert!(parse(&["set-system"]).is_err());
        assert_eq!(
            parse(&["set-system", "/a"]).unwrap().command,
            Command::SetSystem {
                path: PathBuf::from("/a")
            }
        );
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        assert!(parse(&["frobnicate"]).is_err());
        assert!(parse(&[]).is_err());
    }
}
