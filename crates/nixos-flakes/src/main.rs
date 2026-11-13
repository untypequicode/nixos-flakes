//! CLI `nixos-flakes` : gestion des flakes Nix de la machine.

mod cli;
mod commands;
mod daemon;
mod dbus;
mod doctor;
mod failure;
mod pkg;
mod report;
mod ui;

use clap::FromArgMatches;
use std::process::ExitCode;

fn main() -> ExitCode {
    // Erreur d'usage : clap affiche le message et sort avec le code 2.
    let matches = cli::command().get_matches();
    let cli = cli::Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    match commands::run(cli) {
        Ok(code) => code,
        Err(failure) => {
            eprintln!("{}", failure.render());
            failure.exit_code()
        }
    }
}
