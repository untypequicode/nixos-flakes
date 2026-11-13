//! Définition des arguments de la ligne de commande (clap).

use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use std::path::PathBuf;

/// Codes de sortie, documentés dans `--help`.
pub const EXIT_HELP: &str = "\
Codes de sortie :
  0   succès
  1   erreur
  2   usage incorrect (arguments invalides)
  3   annulé par l'utilisateur
  4   dépôt occupé (une autre opération est en cours sur ce dépôt)
  5   délai dépassé
  10  « check --exit-code » : des mises à jour sont disponibles";

/// Gestion des flakes Nix de la machine.
#[derive(Debug, Parser, PartialEq, Eq)]
#[command(name = "nixos-flakes", version, about, long_about = None, after_help = EXIT_HELP)]
pub struct Cli {
    /// Fichier de configuration (défaut : $XDG_CONFIG_HOME/nixos-flakes/config.toml).
    #[arg(long, global = true, value_name = "FICHIER")]
    pub config: Option<PathBuf>,

    /// Réduit la sortie : n'affiche que les résultats et les erreurs.
    #[arg(long, short = 'q', global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Command,
}

/// Commande clap complète, avec l'aide intégrée en français.
pub fn command() -> clap::Command {
    let mut cmd = Cli::command();
    cmd.build();
    localize(cmd)
}

fn localize(cmd: clap::Command) -> clap::Command {
    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .collect();
    let mut cmd = cmd
        .help_template("{about-with-newline}\nUtilisation : {usage}\n\n{all-args}{after-help}")
        .subcommand_help_heading("Commandes")
        .mut_arg("help", |a| a.help("Affiche l'aide"));
    if cmd.get_arguments().any(|a| a.get_id() == "version") {
        cmd = cmd.mut_arg("version", |a| a.help("Affiche la version"));
    }
    for name in names {
        if name == "help" {
            cmd = cmd.mut_subcommand(name, |s| {
                s.about("Affiche l'aide d'une commande")
                    .help_template("{about}\n\nUtilisation : {usage}")
            });
        } else {
            cmd = cmd.mut_subcommand(name, localize);
        }
    }
    cmd
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
        /// Sortie JSON (tableau de dépôts).
        #[arg(long)]
        json: bool,
    },
    /// Recherche les dépôts flake sous les racines de scan.
    Scan {
        /// Sortie JSON (tableau de dépôts).
        #[arg(long)]
        json: bool,
    },
    /// Affiche les mises à jour en attente d'un dépôt, sans rien modifier.
    Check {
        /// Dépôt à vérifier (défaut : dépôt système).
        path: Option<PathBuf>,
        /// Sortie JSON pour Waybar ; sort toujours avec le code 0.
        #[arg(long)]
        waybar: bool,
        /// Sort avec le code 10 si des mises à jour sont disponibles (0 sinon).
        #[arg(long)]
        exit_code: bool,
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
        /// Montre ce qui changerait sans rien écrire (simulation).
        #[arg(long)]
        dry_run: bool,
    },
    /// Applique la configuration d'un dépôt à la machine (nixos-rebuild switch).
    Apply {
        /// Dépôt à appliquer (défaut : dépôt système).
        path: Option<PathBuf>,
        /// Configuration `nixosConfigurations.<NOM>` à appliquer (défaut : attribut enregistré, sinon nom d'hôte).
        #[arg(long, value_name = "NOM")]
        attr: Option<String>,
        /// Ne demande pas de confirmation (la question n'est posée que sur un terminal).
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Mise à niveau automatique : mise à jour, application, retour arrière en cas d'échec.
    Upgrade {
        /// Dépôt à mettre à niveau (défaut : dépôt système).
        path: Option<PathBuf>,
        /// Configuration `nixosConfigurations.<NOM>` à appliquer (défaut : attribut enregistré, sinon nom d'hôte).
        #[arg(long, value_name = "NOM")]
        attr: Option<String>,
        /// Envoie une notification de bureau.
        #[arg(long)]
        notify: bool,
        /// Élève les privilèges avec sudo au lieu de pkexec.
        #[arg(long)]
        sudo: bool,
        /// Ne demande pas de confirmation (la question n'est posée que sur un terminal).
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Résumé : dépôts suivis, état git, mises à jour connues, démon, dernière mise à niveau.
    Status {
        /// Se limite à ce dépôt.
        path: Option<PathBuf>,
        /// Sortie JSON stable.
        #[arg(long)]
        json: bool,
    },
    /// Diagnostique l'installation (outils, configuration, dépôt système, jeton, démon).
    Doctor,
    /// Génère le script de complétion pour un interpréteur de commandes.
    Completions {
        /// Interpréteur cible (bash, zsh, fish, elvish, powershell).
        shell: Shell,
    },
    /// Synchronise un dépôt avec son amont : fetch, avance/retard, puis pull en avance rapide.
    Sync {
        /// Dépôt à synchroniser (défaut : dépôt système).
        path: Option<PathBuf>,
        /// Se contente du fetch et de l'affichage, sans récupérer les commits.
        #[arg(long)]
        check: bool,
    },
    /// Démon de session : icône d'état, vérifications et interface D-Bus (instance unique).
    Daemon {
        /// Commande qui ouvre l'interface graphique (absente : pas d'entrée « Ouvrir l'interface »).
        #[arg(long, env = "NIXOS_FLAKES_GUI_COMMAND", value_name = "COMMANDE")]
        gui_command: Option<String>,
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
    /// Désigne le dépôt système (l'ancien devient suivi) ; chemin relatif, `~` et liens acceptés.
    SetSystem {
        /// Dossier du dépôt flake (contenant flake.nix).
        path: PathBuf,
        /// Configuration `nixosConfigurations.<NOM>` à appliquer (défaut : nom d'hôte).
        #[arg(long, value_name = "NOM")]
        attr: Option<String>,
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
                check: true,
                json: false
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
        let cli = parse(&["check", "/srv/nixos", "--waybar"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Check {
                path: Some(PathBuf::from("/srv/nixos")),
                waybar: true,
                exit_code: false
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
                inputs: vec![],
                dry_run: false
            }
        );
        assert_eq!(
            parse(&["apply"]).unwrap().command,
            Command::Apply {
                path: None,
                attr: None,
                yes: false
            }
        );
    }

    #[test]
    fn update_input_is_repeatable() {
        let cli = parse(&[
            "update",
            "/srv/nixos",
            "--input",
            "nixpkgs",
            "--input",
            "home",
        ])
        .unwrap();
        assert_eq!(
            cli.command,
            Command::Update {
                path: Some(PathBuf::from("/srv/nixos")),
                commit: false,
                inputs: vec!["nixpkgs".into(), "home".into()],
                dry_run: false
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
                attr: None,
                notify: true,
                sudo: true,
                yes: false
            }
        );
    }

    #[test]
    fn sync_flags() {
        let cli = parse(&["sync", "--check", "/x"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Sync {
                path: Some(PathBuf::from("/x")),
                check: true
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
                path: PathBuf::from("/a"),
                attr: None
            }
        );
        assert_eq!(
            parse(&["set-system", "~/nix", "--attr", "laptop"])
                .unwrap()
                .command,
            Command::SetSystem {
                path: PathBuf::from("~/nix"),
                attr: Some("laptop".into())
            }
        );
        assert_eq!(
            parse(&["apply", "--attr", "x"]).unwrap().command,
            Command::Apply {
                path: None,
                attr: Some("x".into()),
                yes: false
            }
        );
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        assert!(parse(&["frobnicate"]).is_err());
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn new_subcommands_and_flags() {
        assert_eq!(
            parse(&["status", "--json", "/x"]).unwrap().command,
            Command::Status {
                path: Some(PathBuf::from("/x")),
                json: true
            }
        );
        assert_eq!(parse(&["doctor"]).unwrap().command, Command::Doctor);
        assert_eq!(
            parse(&["completions", "bash"]).unwrap().command,
            Command::Completions { shell: Shell::Bash }
        );
        assert!(parse(&["completions", "nimporte"]).is_err());
        assert!(matches!(
            parse(&["check", "--exit-code"]).unwrap().command,
            Command::Check {
                exit_code: true,
                ..
            }
        ));
        assert!(matches!(
            parse(&["update", "--dry-run"]).unwrap().command,
            Command::Update { dry_run: true, .. }
        ));
        assert!(matches!(
            parse(&["apply", "-y"]).unwrap().command,
            Command::Apply { yes: true, .. }
        ));
        assert!(matches!(
            parse(&["upgrade", "--yes"]).unwrap().command,
            Command::Upgrade { yes: true, .. }
        ));
        assert!(parse(&["scan", "--json"]).is_ok());
        let q = parse(&["-q", "list"]).unwrap();
        assert!(q.quiet);
        assert!(parse(&["list", "--quiet"]).unwrap().quiet);
    }

    #[test]
    fn localized_command_builds_and_documents_exit_codes() {
        let mut c = command();
        let help = c.render_long_help().to_string();
        assert!(help.contains("Codes de sortie"), "{help}");
        assert!(help.contains("Utilisation"), "{help}");
        c.debug_assert();
    }
}
