//! Logique métier partagée entre la CLI et la GUI.
//!
//! Tout est synchrone. Les commandes longues prennent un callback
//! `&mut dyn FnMut(&str)` qui reçoit chaque ligne de sortie (stdout et stderr
//! fusionnés). Voir `docs/ARCHITECTURE.md` et `GLOSSARY.md`.

pub mod config;
pub mod daemon;
pub mod error;
mod fsutil;
pub mod git;
pub mod icon;
pub mod lock;
pub mod nix;
pub mod notify;
mod proc;
pub mod repo_lock;
pub mod scan;
pub mod status;
pub mod token;
pub mod upgrade;
pub mod waybar;

pub use error::{Error, Result};

#[cfg(test)]
mod testutil;
