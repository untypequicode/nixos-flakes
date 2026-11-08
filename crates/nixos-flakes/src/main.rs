//! CLI `nixos-flakes` : gestion des flakes Nix de la machine.

mod cli;
mod commands;
mod pkg;

use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    match commands::run(cli::Cli::parse()) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("erreur : {message}");
            ExitCode::FAILURE
        }
    }
}
