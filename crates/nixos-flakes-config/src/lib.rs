//! Édition sans perte des Fichiers de config Nix : repérer les Emplacements de
//! Paquets déclarés (`environment.systemPackages`, `home.packages`…) et y ajouter,
//! retirer ou déplacer des paquets en préservant commentaires et mise en forme.
//!
//! Les fonctions travaillent sur des `&str` ; seules `write_atomic` et
//! `validate_with_nix` touchent au système.

mod edit;
mod error;
mod fsio;
mod locate;
mod search;
#[cfg(test)]
mod tests;

pub use edit::{add_package, move_package, remove_package};
pub use error::{Error, Result};
pub use fsio::{validate_with_nix, write_atomic};
pub use locate::{list_locations, Entry, Location};
pub use search::{search, PackageHit};

/// Vérifie que `src` est du Nix syntaxiquement valide (analyseur `rnix`).
pub fn validate(src: &str) -> Result<()> {
    let errors = rnix::Root::parse(src).errors().to_vec();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(Error::Parse(
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ; "),
        ))
    }
}

/// Différences entre `old` et `new` au format diff unifié (3 lignes de contexte).
/// Chaîne vide si les textes sont identiques.
pub fn diff(old: &str, new: &str) -> String {
    if old == new {
        return String::new();
    }
    similar::TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header("avant", "après")
        .to_string()
}
