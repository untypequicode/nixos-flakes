//! Raccourcis clavier : traduction pure d'une touche en intention, et navigation
//! dans la liste des dépôts.

use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use std::path::{Path, PathBuf};

/// Intention déduite d'une touche.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    OpenPalette,
    /// Ferme la palette, le dialogue ou la page de réglages (selon le contexte).
    Escape,
    /// Déplace la sélection (barre latérale ou palette selon le contexte).
    Move(i32),
    /// Entrée : ouvre le dépôt sélectionné, ou exécute l'élément de la palette.
    Enter,
    Check,
    Update,
}

/// Contexte courant, qui décide quelles touches sont actives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyContext {
    pub modal: bool,
    pub palette: bool,
    pub settings: bool,
}

/// Traduit une touche pressée. `captured` : un widget (champ de texte…) l'a déjà consommée.
pub fn resolve(key: &Key, mods: Modifiers, captured: bool, ctx: KeyContext) -> Option<Shortcut> {
    if let Key::Character(c) = key {
        if (mods.control() || mods.command()) && c.eq_ignore_ascii_case("k") {
            return Some(Shortcut::OpenPalette);
        }
    }
    if let Key::Named(Named::Escape) = key {
        return Some(Shortcut::Escape);
    }
    if ctx.palette {
        // Dans la palette, les flèches pilotent la liste même si le champ les a vues.
        return match key {
            Key::Named(Named::ArrowDown) => Some(Shortcut::Move(1)),
            Key::Named(Named::ArrowUp) => Some(Shortcut::Move(-1)),
            Key::Named(Named::Tab) => Some(Shortcut::Move(if mods.shift() { -1 } else { 1 })),
            Key::Named(Named::Enter) if !captured => Some(Shortcut::Enter),
            _ => None,
        };
    }
    if captured || ctx.modal || ctx.settings || mods.control() || mods.alt() || mods.logo() {
        return None;
    }
    match key {
        Key::Named(Named::ArrowDown) => Some(Shortcut::Move(1)),
        Key::Named(Named::ArrowUp) => Some(Shortcut::Move(-1)),
        Key::Named(Named::Enter) => Some(Shortcut::Enter),
        Key::Character(c) => match c.as_str() {
            "j" => Some(Shortcut::Move(1)),
            "k" => Some(Shortcut::Move(-1)),
            "r" => Some(Shortcut::Check),
            "u" => Some(Shortcut::Update),
            _ => None,
        },
        _ => None,
    }
}

/// Dépôt atteint en se déplaçant de `delta` dans `order` depuis `current`
/// (sans retour à l'autre bout). Sans sélection, on part du début ou de la fin.
pub fn step_selection(order: &[PathBuf], current: Option<&Path>, delta: i32) -> Option<PathBuf> {
    if order.is_empty() {
        return None;
    }
    let last = order.len() - 1;
    let idx = match current.and_then(|c| order.iter().position(|p| p == c)) {
        Some(i) => {
            let moved = i64::try_from(i).unwrap_or(0) + i64::from(delta);
            usize::try_from(moved.clamp(0, i64::try_from(last).unwrap_or(0))).unwrap_or(0)
        }
        None if delta < 0 => last,
        None => 0,
    };
    Some(order[idx].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(s: &str) -> Key {
        Key::Character(s.into())
    }

    fn ctx() -> KeyContext {
        KeyContext::default()
    }

    #[test]
    fn letters_drive_the_main_view() {
        let m = Modifiers::empty();
        assert_eq!(resolve(&ch("j"), m, false, ctx()), Some(Shortcut::Move(1)));
        assert_eq!(resolve(&ch("k"), m, false, ctx()), Some(Shortcut::Move(-1)));
        assert_eq!(resolve(&ch("r"), m, false, ctx()), Some(Shortcut::Check));
        assert_eq!(resolve(&ch("u"), m, false, ctx()), Some(Shortcut::Update));
        assert_eq!(resolve(&ch("x"), m, false, ctx()), None);
        assert_eq!(
            resolve(&Key::Named(Named::ArrowDown), m, false, ctx()),
            Some(Shortcut::Move(1))
        );
        assert_eq!(
            resolve(&Key::Named(Named::Enter), m, false, ctx()),
            Some(Shortcut::Enter)
        );
    }

    #[test]
    fn typing_in_a_field_never_triggers_shortcuts() {
        let m = Modifiers::empty();
        assert_eq!(resolve(&ch("r"), m, true, ctx()), None);
        let settings = KeyContext {
            settings: true,
            ..ctx()
        };
        assert_eq!(resolve(&ch("j"), m, false, settings), None);
        let modal = KeyContext {
            modal: true,
            ..ctx()
        };
        assert_eq!(resolve(&ch("u"), m, false, modal), None);
    }

    #[test]
    fn ctrl_k_and_escape_work_everywhere() {
        let c = Modifiers::CTRL;
        for context in [
            ctx(),
            KeyContext {
                modal: true,
                ..ctx()
            },
            KeyContext {
                settings: true,
                ..ctx()
            },
        ] {
            assert_eq!(
                resolve(&ch("k"), c, true, context),
                Some(Shortcut::OpenPalette)
            );
            assert_eq!(
                resolve(
                    &Key::Named(Named::Escape),
                    Modifiers::empty(),
                    true,
                    context
                ),
                Some(Shortcut::Escape)
            );
        }
        assert_eq!(
            resolve(&ch("K"), c, false, ctx()),
            Some(Shortcut::OpenPalette)
        );
    }

    #[test]
    fn palette_context_captures_navigation() {
        let p = KeyContext {
            palette: true,
            ..ctx()
        };
        let m = Modifiers::empty();
        assert_eq!(
            resolve(&ch("j"), m, false, p),
            None,
            "la lettre va au champ"
        );
        assert_eq!(
            resolve(&Key::Named(Named::ArrowDown), m, true, p),
            Some(Shortcut::Move(1))
        );
        assert_eq!(
            resolve(&Key::Named(Named::ArrowUp), m, true, p),
            Some(Shortcut::Move(-1))
        );
        assert_eq!(
            resolve(&Key::Named(Named::Enter), m, false, p),
            Some(Shortcut::Enter)
        );
        assert_eq!(resolve(&Key::Named(Named::Enter), m, true, p), None);
        assert_eq!(
            resolve(&Key::Named(Named::Tab), Modifiers::SHIFT, false, p),
            Some(Shortcut::Move(-1))
        );
    }

    #[test]
    fn modified_letters_are_ignored() {
        assert_eq!(resolve(&ch("r"), Modifiers::CTRL, false, ctx()), None);
        assert_eq!(resolve(&ch("u"), Modifiers::ALT, false, ctx()), None);
    }

    fn paths(v: &[&str]) -> Vec<PathBuf> {
        v.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn selection_steps_and_clamps() {
        let o = paths(&["/a", "/b", "/c"]);
        assert_eq!(
            step_selection(&o, Some(Path::new("/a")), 1),
            Some("/b".into())
        );
        assert_eq!(
            step_selection(&o, Some(Path::new("/c")), 1),
            Some("/c".into())
        );
        assert_eq!(
            step_selection(&o, Some(Path::new("/a")), -1),
            Some("/a".into())
        );
        assert_eq!(
            step_selection(&o, Some(Path::new("/b")), -1),
            Some("/a".into())
        );
    }

    #[test]
    fn selection_without_current_starts_at_the_edges() {
        let o = paths(&["/a", "/b"]);
        assert_eq!(step_selection(&o, None, 1), Some("/a".into()));
        assert_eq!(step_selection(&o, None, -1), Some("/b".into()));
        assert_eq!(
            step_selection(&o, Some(Path::new("/gone")), 1),
            Some("/a".into())
        );
        assert_eq!(step_selection(&[], None, 1), None);
    }
}
