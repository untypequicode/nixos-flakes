//! Présentation : mode silencieux, couleur automatique, tableaux alignés.

use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

static QUIET: AtomicBool = AtomicBool::new(false);

/// Active ou non le mode silencieux (`--quiet`).
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

pub fn is_quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

/// Affiche une ligne d'information sur stdout, sauf en mode silencieux.
pub fn info(message: &str) {
    if !is_quiet() {
        println!("{message}");
    }
}

/// Affiche une ligne de progression sur stderr, sauf en mode silencieux.
pub fn progress(line: &str) {
    if !is_quiet() {
        eprintln!("{line}");
    }
}

/// Couleur d'une cellule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Green,
    Yellow,
    Red,
    Dim,
    Bold,
}

/// Vrai si la couleur est souhaitée : `NO_COLOR` absent ou vide, `TERM` différent de `dumb`.
pub fn color_allowed(no_color: Option<&str>, term: Option<&str>) -> bool {
    no_color.is_none_or(str::is_empty) && term != Some("dumb")
}

/// Couleur pour stdout : terminal interactif et `color_allowed`.
pub fn stdout_color() -> bool {
    std::io::stdout().is_terminal()
        && color_allowed(
            std::env::var("NO_COLOR").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
        )
}

/// Enveloppe `text` dans la séquence ANSI de `style` si `enabled`.
pub fn paint(text: &str, style: Style, enabled: bool) -> String {
    if !enabled {
        return text.to_string();
    }
    let code = match style {
        Style::Green => "32",
        Style::Yellow => "33",
        Style::Red => "31",
        Style::Dim => "2",
        Style::Bold => "1",
    };
    format!("\x1b[{code}m{text}\x1b[0m")
}

/// Tableau aligné : chaque colonne a la largeur de sa plus grande cellule.
pub fn format_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    format_table_styled(headers, rows, &|_, _| None, false)
}

/// Tableau aligné ; `styler(colonne, cellule)` choisit une couleur appliquée si `color`.
/// L'alignement se fait sur le texte nu : les séquences ANSI ne le perturbent pas.
pub fn format_table_styled(
    headers: &[&str],
    rows: &[Vec<String>],
    styler: &dyn Fn(usize, &str) -> Option<Style>,
    color: bool,
) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    let mut push_row = |cells: &[&str], header: bool| {
        let mut line = String::new();
        for (i, (cell, &w)) in cells.iter().zip(&widths).enumerate() {
            if i > 0 {
                line.push_str("  ");
            }
            let style = if header {
                Some(Style::Bold)
            } else {
                styler(i, cell)
            };
            match style {
                Some(s) => line.push_str(&paint(cell, s, color)),
                None => line.push_str(cell),
            }
            line.push_str(&" ".repeat(w.saturating_sub(cell.chars().count())));
        }
        out.push_str(line.trim_end());
        out.push('\n');
    };
    push_row(headers, true);
    for row in rows {
        let cells: Vec<&str> = row.iter().map(String::as_str).collect();
        push_row(&cells, false);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_rules() {
        assert!(color_allowed(None, Some("xterm")));
        assert!(color_allowed(Some(""), None));
        assert!(!color_allowed(Some("1"), Some("xterm")));
        assert!(!color_allowed(None, Some("dumb")));
    }

    #[test]
    fn paint_is_identity_when_disabled() {
        assert_eq!(paint("x", Style::Red, false), "x");
        assert_eq!(paint("x", Style::Red, true), "\x1b[31mx\x1b[0m");
    }

    #[test]
    fn styled_table_keeps_alignment() {
        let rows = vec![vec!["a".to_string(), "bb".to_string()]];
        let plain = format_table(&["H1", "H2"], &rows);
        let colored = format_table_styled(
            &["H1", "H2"],
            &rows,
            &|c, _| (c == 1).then_some(Style::Green),
            true,
        );
        let strip = |s: &str| {
            let mut out = String::new();
            let mut esc = false;
            for ch in s.chars() {
                match (esc, ch) {
                    (false, '\x1b') => esc = true,
                    (true, 'm') => esc = false,
                    (false, c) => out.push(c),
                    _ => {}
                }
            }
            out
        };
        assert_eq!(
            strip(&colored),
            strip(&format_table_styled(
                &["H1", "H2"],
                &rows,
                &|_, _| None,
                true
            ))
        );
        assert_eq!(plain, "H1  H2\na   bb\n");
    }
}
