//! Palette de commandes : éléments, filtrage flou, sélection. Logique pure.

use std::path::PathBuf;

/// Action déclenchée par un élément de la palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Goto(PathBuf),
    Check,
    CheckAll,
    Update,
    Apply,
    Settings,
    Rescan,
    ToggleConsole,
}

/// Une ligne de la palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub command: Command,
    pub label: String,
    /// Complément discret à droite (rôle d'un dépôt, chemin…).
    pub detail: String,
    /// Raccourci clavier affiché.
    pub kbd: Option<&'static str>,
    /// Vrai pour les dépôts (section « Aller à »), faux pour les actions.
    pub is_repo: bool,
}

/// Minuscules et sans accents, pour que « verifier » trouve « Vérifier ».
pub fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

/// Score d'une recherche « sous-séquence » : `None` si `query` ne s'y trouve pas.
/// Favorise le début de texte, les débuts de mot et les caractères contigus.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = fold(query).chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = fold(text).chars().collect();
    let mut qi = 0;
    let mut total = 0;
    let mut last: Option<usize> = None;
    for (i, &c) in t.iter().enumerate() {
        if qi < q.len() && c == q[qi] {
            let mut s = 1;
            if i == 0 {
                s += 8;
            } else if !t[i - 1].is_alphanumeric() {
                s += 5;
            }
            if last.is_some_and(|l| l + 1 == i) {
                s += 4;
            }
            total += s;
            last = Some(i);
            qi += 1;
        }
    }
    // Les textes courts qui contiennent la requête passent devant les longs.
    (qi == q.len()).then(|| total * 4 - i32::try_from(t.len()).unwrap_or(0).min(60))
}

/// Éléments correspondant à `query`, du meilleur au moins bon (ordre initial conservé à égalité).
pub fn filter(items: &[Item], query: &str) -> Vec<Item> {
    let mut scored: Vec<(i32, usize, &Item)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| {
            let s = score(query, &it.label)
                .or_else(|| score(query, &format!("{} {}", it.label, it.detail)).map(|s| s - 20))?;
            Some((s, i, it))
        })
        .collect();
    if !query.trim().is_empty() {
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    scored.into_iter().map(|(_, _, it)| it.clone()).collect()
}

/// État de saisie de la palette.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaletteState {
    pub query: String,
    pub selected: usize,
}

impl PaletteState {
    pub fn set_query(&mut self, q: String) {
        self.query = q;
        self.selected = 0;
    }

    /// Déplace la sélection de `delta` avec retour à l'autre bout de la liste.
    pub fn move_by(&mut self, delta: i32, len: usize) {
        if len == 0 {
            self.selected = 0;
            return;
        }
        let len_i = i64::try_from(len).unwrap_or(i64::MAX);
        let cur = i64::try_from(self.selected.min(len - 1)).unwrap_or(0);
        self.selected = usize::try_from((cur + i64::from(delta)).rem_euclid(len_i)).unwrap_or(0);
    }

    pub fn reset(&mut self) {
        *self = PaletteState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(cmd: Command, label: &str, detail: &str) -> Item {
        Item {
            command: cmd,
            label: label.into(),
            detail: detail.into(),
            kbd: None,
            is_repo: false,
        }
    }

    fn sample() -> Vec<Item> {
        vec![
            item(Command::Check, "Vérifier", ""),
            item(Command::CheckAll, "Tout vérifier", ""),
            item(Command::Update, "Mettre à jour", ""),
            item(Command::Apply, "Appliquer au système", ""),
            item(Command::Settings, "Réglages", ""),
            item(Command::Rescan, "Rescanner", ""),
            item(
                Command::Goto("/home/u/nixos-config".into()),
                "nixos-config",
                "Dépôt système",
            ),
        ]
    }

    #[test]
    fn fold_removes_accents_and_case() {
        assert_eq!(fold("Vérifier À JOUR çà"), "verifier a jour ca");
    }

    #[test]
    fn empty_query_keeps_everything_in_order() {
        let r = filter(&sample(), "  ");
        assert_eq!(r.len(), 7);
        assert_eq!(r[0].command, Command::Check);
    }

    #[test]
    fn matches_without_accents_and_as_subsequence() {
        let r = filter(&sample(), "verif");
        let cmds: Vec<_> = r.iter().map(|i| i.command.clone()).collect();
        assert_eq!(cmds[0], Command::Check, "début de texte d'abord");
        assert!(cmds.contains(&Command::CheckAll));
        assert!(!cmds.contains(&Command::Settings));
        // Sous-séquence éparse.
        let r = filter(&sample(), "rglg");
        assert_eq!(r[0].command, Command::Settings);
    }

    #[test]
    fn detail_text_is_searchable_but_ranks_lower() {
        let r = filter(&sample(), "systeme");
        let cmds: Vec<_> = r.iter().map(|i| i.command.clone()).collect();
        assert!(cmds.contains(&Command::Apply));
        assert!(cmds.contains(&Command::Goto("/home/u/nixos-config".into())));
    }

    #[test]
    fn no_match_is_empty() {
        assert!(filter(&sample(), "zzz").is_empty());
    }

    #[test]
    fn word_start_beats_scattered_match() {
        let a = score("mj", "Mettre à jour").unwrap();
        let b = score("mj", "Aimerajouter").unwrap_or(i32::MIN);
        assert!(a > b);
    }

    #[test]
    fn selection_wraps_and_resets_on_query() {
        let mut p = PaletteState::default();
        p.move_by(-1, 3);
        assert_eq!(p.selected, 2);
        p.move_by(1, 3);
        assert_eq!(p.selected, 0);
        p.move_by(1, 3);
        p.set_query("x".into());
        assert_eq!(p.selected, 0);
        p.move_by(1, 0);
        assert_eq!(p.selected, 0);
        p.selected = 9;
        p.move_by(1, 3);
        assert_eq!(p.selected, 0, "sélection hors bornes ramenée dans la liste");
    }
}
