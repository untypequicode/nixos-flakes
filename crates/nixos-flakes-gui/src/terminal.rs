//! Terminal intégré (crate `iced_term`) : le shell de l'utilisateur (`$SHELL`), lancé dans
//! le dépôt de config système.

use crate::theme::{self, Mode};
use iced_term::settings::{BackendSettings, Settings, ThemeSettings};
use iced_term::ColorPalette;
use std::collections::HashMap;
use std::path::Path;

/// Identifiant de l'unique instance de terminal.
pub const TERM_ID: u64 = 0;

/// Shell à lancer : `$SHELL`, sinon `/bin/sh`.
pub fn shell_program(env: Option<&str>) -> String {
    env.filter(|s| !s.trim().is_empty())
        .unwrap_or("/bin/sh")
        .to_string()
}

/// Dossier de départ : le dépôt de config s'il existe, sinon `$HOME`.
pub fn start_dir(repo: Option<&Path>, home: Option<&Path>) -> Option<std::path::PathBuf> {
    repo.filter(|p| p.is_dir())
        .or(home.filter(|p| p.is_dir()))
        .map(Path::to_path_buf)
}

/// Environnement imposé au shell : un `TERM` valide (l'application lancée depuis le
/// bureau n'en hérite pas toujours, et zsh/readline redessinent alors n'importe comment).
pub fn shell_env() -> HashMap<String, String> {
    HashMap::from([
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
    ])
}

fn hex(c: iced::Color) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c.r), b(c.g), b(c.b))
}

/// Palette du terminal dérivée des jetons du thème du GUI (fond du panneau, texte) ;
/// les couleurs ANSI sont adaptées à la clarté du fond.
pub fn palette(mode: Mode) -> ColorPalette {
    let tk = theme::tokens(mode);
    // Normales puis vives : noir, rouge, vert, jaune, bleu, magenta, cyan, blanc.
    let (n, b): ([&str; 8], [&str; 8]) = if matches!(mode, Mode::Dark) {
        (
            [
                "#2a2c31", "#e06c66", "#4ca87a", "#d1a14a", "#6f7be0", "#b583d6", "#4fb3bf",
                "#c9ccd2",
            ],
            [
                "#62666d", "#ec8b86", "#6cc79a", "#e3bb6e", "#8f99ee", "#c9a0e6", "#72cbd6",
                "#ededef",
            ],
        )
    } else {
        (
            [
                "#1f2023", "#c4443e", "#2e8b5e", "#a8700e", "#4f5bc4", "#8e4fb0", "#1f7f8c",
                "#8a8f98",
            ],
            [
                "#62666d", "#d85a54", "#3fa073", "#bd8420", "#5e6ad2", "#a368c6", "#2e97a5",
                "#b4b8bf",
            ],
        )
    };
    let s = String::from;
    ColorPalette {
        foreground: hex(tk.text),
        background: hex(tk.panel),
        black: s(n[0]),
        red: s(n[1]),
        green: s(n[2]),
        yellow: s(n[3]),
        blue: s(n[4]),
        magenta: s(n[5]),
        cyan: s(n[6]),
        white: s(n[7]),
        bright_black: s(b[0]),
        bright_red: s(b[1]),
        bright_green: s(b[2]),
        bright_yellow: s(b[3]),
        bright_blue: s(b[4]),
        bright_magenta: s(b[5]),
        bright_cyan: s(b[6]),
        bright_white: s(b[7]),
        bright_foreground: None,
        dim_foreground: hex(tk.text2),
        dim_black: s(n[0]),
        dim_red: s(n[1]),
        dim_green: s(n[2]),
        dim_yellow: s(n[3]),
        dim_blue: s(n[4]),
        dim_magenta: s(n[5]),
        dim_cyan: s(n[6]),
        dim_white: s(n[7]),
    }
}

/// Crée un terminal dans `repo` (repli : `home`), aux couleurs du thème `mode`.
pub fn spawn(
    repo: Option<&Path>,
    home: Option<&Path>,
    mode: Mode,
) -> Result<iced_term::Terminal, String> {
    let settings = Settings {
        backend: BackendSettings {
            program: shell_program(std::env::var("SHELL").ok().as_deref()),
            working_directory: start_dir(repo, home),
            env: shell_env(),
            ..Default::default()
        },
        theme: ThemeSettings::new(Box::new(palette(mode))),
        ..Default::default()
    };
    iced_term::Terminal::new(TERM_ID, settings).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_sets_a_valid_term() {
        assert_eq!(shell_env()["TERM"], "xterm-256color");
        assert_eq!(shell_env()["COLORTERM"], "truecolor");
    }

    #[test]
    fn palette_follows_the_theme() {
        let (d, l) = (palette(Mode::Dark), palette(Mode::Light));
        assert_ne!(d.background, l.background);
        assert_eq!(l.background, hex(theme::tokens(Mode::Light).panel));
        assert_eq!(d.foreground, hex(theme::tokens(Mode::Dark).text));
        assert_eq!(d.red.len(), 7);
    }

    #[test]
    fn shell_falls_back_to_sh() {
        assert_eq!(
            shell_program(Some("/run/current-system/sw/bin/zsh")),
            "/run/current-system/sw/bin/zsh"
        );
        assert_eq!(shell_program(Some("  ")), "/bin/sh");
        assert_eq!(shell_program(None), "/bin/sh");
    }

    #[test]
    fn start_dir_prefers_existing_repo() {
        let d = tempfile::tempdir().unwrap();
        let missing = d.path().join("absent");
        assert_eq!(
            start_dir(Some(d.path()), None),
            Some(d.path().to_path_buf())
        );
        assert_eq!(
            start_dir(Some(&missing), Some(d.path())),
            Some(d.path().to_path_buf())
        );
        assert_eq!(start_dir(Some(&missing), None), None);
    }
}
