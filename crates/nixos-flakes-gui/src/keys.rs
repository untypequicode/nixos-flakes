//! Raccourcis clavier : traduction pure d'une touche en intention, liste documentée
//! (aide des raccourcis) et navigation dans la liste des dépôts.

use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use std::path::{Path, PathBuf};

/// Intention déduite d'une touche.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    OpenPalette,
    /// Ferme la palette, l'aide, le dialogue ou la page courante (selon le contexte).
    Escape,
    /// Déplace la sélection (barre latérale ou palette selon le contexte).
    Move(i32),
    /// Entrée : confirme un dialogue, ouvre le dépôt sélectionné, ou exécute l'élément de la palette.
    Enter,
    Check,
    Update,
    /// Appliquer au système (avec confirmation).
    Apply,
    /// Ouvre ou ferme l'aide des raccourcis.
    Help,
    OpenSettings,
    /// Met le focus sur le champ de recherche de la page Paquets.
    FocusSearch,
    /// Onglet du panneau de détail (0 = Mises à jour, 1 = Git, 2 = Dépôt).
    Tab(usize),
}

/// Contexte courant, qui décide quelles touches sont actives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyContext {
    pub modal: bool,
    /// Le dialogue ouvert est une simple confirmation (Entrée la valide).
    pub confirm: bool,
    pub palette: bool,
    /// L'aide des raccourcis est affichée.
    pub help: bool,
    /// Une page autre que la principale est affichée (réglages, paquets, terminal).
    pub settings: bool,
    /// La page affichée est celle des paquets.
    pub packages: bool,
}

/// Un raccourci documenté dans l'aide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Doc {
    /// Touches, telles qu'affichées (« Ctrl K », « j / ↓ »).
    pub keys: &'static str,
    pub what: &'static str,
    /// Une touche simple qui doit se résoudre (vérifié par les tests) ; `None` pour les combinaisons.
    pub probe: Option<&'static str>,
}

const fn doc(keys: &'static str, what: &'static str, probe: Option<&'static str>) -> Doc {
    Doc { keys, what, probe }
}

/// Tous les raccourcis, par groupe : source unique de l'aide et des tests.
pub const SHORTCUT_GROUPS: &[(&str, &[Doc])] = &[
    (
        "Navigation",
        &[
            doc("j / ↓", "Dépôt suivant", Some("j")),
            doc("k / ↑", "Dépôt précédent", Some("k")),
            doc("1 · 2 · 3", "Onglet Mises à jour · Git · Dépôt", Some("1")),
            doc("Ctrl K", "Palette de commandes", None),
            doc(",", "Ouvrir les réglages", Some(",")),
            doc("/", "Rechercher (page Paquets)", None),
            doc("?", "Aide des raccourcis", Some("?")),
            doc("Échap", "Fermer, annuler ou revenir", None),
        ],
    ),
    (
        "Actions sur le dépôt sélectionné",
        &[
            doc("r", "Vérifier", Some("r")),
            doc("u", "Mettre à jour", Some("u")),
            doc("a", "Appliquer au système (avec confirmation)", Some("a")),
        ],
    ),
    (
        "Dialogues",
        &[
            doc("Entrée", "Confirmer une confirmation", None),
            doc("Échap", "Annuler", None),
        ],
    ),
];

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
    if mods.control() || mods.alt() || mods.logo() {
        return None;
    }
    if ctx.confirm && !captured {
        // Une confirmation n'a pas de champ : Entrée la valide.
        return matches!(key, Key::Named(Named::Enter)).then_some(Shortcut::Enter);
    }
    if captured || ctx.modal {
        return None;
    }
    if let Key::Character(c) = key {
        // Actives sur toutes les pages (hors champ de saisie).
        match c.as_str() {
            "?" => return Some(Shortcut::Help),
            _ if ctx.help => return None,
            "," => return Some(Shortcut::OpenSettings),
            "/" if ctx.packages => return Some(Shortcut::FocusSearch),
            _ => {}
        }
    }
    if ctx.help || ctx.settings {
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
            "a" => Some(Shortcut::Apply),
            "1" => Some(Shortcut::Tab(0)),
            "2" => Some(Shortcut::Tab(1)),
            "3" => Some(Shortcut::Tab(2)),
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

    #[test]
    fn new_shortcuts_resolve() {
        let m = Modifiers::SHIFT;
        assert_eq!(
            resolve(&ch("a"), Modifiers::empty(), false, ctx()),
            Some(Shortcut::Apply)
        );
        assert_eq!(resolve(&ch("?"), m, false, ctx()), Some(Shortcut::Help));
        assert_eq!(
            resolve(&ch(","), Modifiers::empty(), false, ctx()),
            Some(Shortcut::OpenSettings)
        );
        for (c, i) in [("1", 0), ("2", 1), ("3", 2)] {
            assert_eq!(
                resolve(&ch(c), Modifiers::empty(), false, ctx()),
                Some(Shortcut::Tab(i))
            );
        }
        // « / » ne sert que sur la page Paquets.
        assert_eq!(resolve(&ch("/"), Modifiers::empty(), false, ctx()), None);
        let pk = KeyContext {
            settings: true,
            packages: true,
            ..ctx()
        };
        assert_eq!(
            resolve(&ch("/"), Modifiers::empty(), false, pk),
            Some(Shortcut::FocusSearch)
        );
        // Ni dans un champ de saisie, ni dans un dialogue.
        assert_eq!(resolve(&ch("/"), Modifiers::empty(), true, pk), None);
        assert_eq!(resolve(&ch("?"), m, true, ctx()), None);
        let modal = KeyContext {
            modal: true,
            ..ctx()
        };
        assert_eq!(resolve(&ch("?"), m, false, modal), None);
        assert_eq!(resolve(&ch("a"), Modifiers::empty(), false, modal), None);
    }

    #[test]
    fn enter_confirms_a_confirmation_dialog_only() {
        let enter = Key::Named(Named::Enter);
        let m = Modifiers::empty();
        let confirm = KeyContext {
            modal: true,
            confirm: true,
            ..ctx()
        };
        assert_eq!(resolve(&enter, m, false, confirm), Some(Shortcut::Enter));
        assert_eq!(resolve(&ch("r"), m, false, confirm), None);
        // Dialogue de commit : Entrée appartient à l'éditeur de texte.
        let commit = KeyContext {
            modal: true,
            ..ctx()
        };
        assert_eq!(resolve(&enter, m, false, commit), None);
        assert_eq!(resolve(&enter, m, true, confirm), None);
    }

    #[test]
    fn help_overlay_swallows_other_shortcuts() {
        let help = KeyContext {
            help: true,
            ..ctx()
        };
        let m = Modifiers::empty();
        assert_eq!(resolve(&ch("j"), m, false, help), None);
        assert_eq!(resolve(&ch("a"), m, false, help), None);
        assert_eq!(resolve(&ch(","), m, false, help), None);
        assert_eq!(
            resolve(&ch("?"), Modifiers::SHIFT, false, help),
            Some(Shortcut::Help)
        );
        assert_eq!(
            resolve(&Key::Named(Named::Escape), m, false, help),
            Some(Shortcut::Escape)
        );
    }

    #[test]
    fn every_documented_shortcut_exists() {
        let mut n = 0;
        for (_, docs) in SHORTCUT_GROUPS {
            for d in *docs {
                assert!(!d.keys.is_empty() && !d.what.is_empty());
                if let Some(k) = d.probe {
                    n += 1;
                    let mods = if k == "?" {
                        Modifiers::SHIFT
                    } else {
                        Modifiers::empty()
                    };
                    assert!(
                        resolve(&ch(k), mods, false, ctx()).is_some(),
                        "raccourci documenté sans effet : {}",
                        d.keys
                    );
                }
            }
        }
        assert!(n >= 6);
        // L'aide mentionne la palette et Échap (combinaisons sans sonde).
        let all: Vec<&str> = SHORTCUT_GROUPS
            .iter()
            .flat_map(|(_, d)| d.iter().map(|d| d.keys))
            .collect();
        assert!(all.contains(&"Ctrl K") && all.contains(&"Échap"));
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
