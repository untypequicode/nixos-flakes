//! Thème personnalisé (clair/sombre) et styles des composants.

use crate::model::Tone;
use iced::widget::{button, container, text_input};
use iced::{border, Background, Border, Color, Shadow, Theme, Vector};
use std::process::Command;

/// Mode d'apparence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Dark,
    Light,
}

/// Mode iced (`None` = indéterminé) vers notre mode.
pub fn from_iced(m: iced::theme::Mode) -> Option<Mode> {
    match m {
        iced::theme::Mode::Dark => Some(Mode::Dark),
        iced::theme::Mode::Light => Some(Mode::Light),
        iced::theme::Mode::None => None,
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
}

pub fn palette(mode: Mode) -> iced::theme::Palette {
    match mode {
        Mode::Dark => iced::theme::Palette {
            background: rgb(0x14, 0x16, 0x1b),
            text: rgb(0xe6, 0xe8, 0xee),
            primary: rgb(0x7c, 0x8c, 0xff),
            success: rgb(0x4a, 0xc2, 0x8a),
            warning: rgb(0xe8, 0xb2, 0x4a),
            danger: rgb(0xf0, 0x6a, 0x6a),
        },
        Mode::Light => iced::theme::Palette {
            background: rgb(0xf3, 0xf4, 0xf8),
            text: rgb(0x1c, 0x1f, 0x2a),
            primary: rgb(0x4a, 0x5b, 0xe0),
            success: rgb(0x1a, 0x8a, 0x5c),
            warning: rgb(0xb7, 0x7a, 0x12),
            danger: rgb(0xc7, 0x3a, 0x3a),
        },
    }
}

pub fn theme(mode: Mode) -> Theme {
    let name = match mode {
        Mode::Dark => "Flakes sombre",
        Mode::Light => "Flakes clair",
    };
    Theme::custom(name.to_string(), palette(mode))
}

// ---------------------------------------------------------------------------
// Détection du thème système (au mieux)
// ---------------------------------------------------------------------------

/// Interprète la sortie de `gsettings get … color-scheme` / `dconf read …`.
pub fn parse_color_scheme(output: &str) -> Option<Mode> {
    let v = output.trim().trim_matches('\'').trim_matches('"');
    match v {
        "prefer-dark" => Some(Mode::Dark),
        "prefer-light" => Some(Mode::Light),
        _ => None,
    }
}

/// Interprète un nom de thème GTK (`Adwaita-dark`…).
pub fn parse_gtk_theme(output: &str) -> Option<Mode> {
    let v = output.trim().trim_matches('\'').to_lowercase();
    if v.is_empty() {
        None
    } else if v.contains("dark") {
        Some(Mode::Dark)
    } else {
        Some(Mode::Light)
    }
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Préférence système : gsettings, puis dconf, puis nom du thème GTK ; repli sombre.
pub fn detect_system_mode() -> Mode {
    run(
        "gsettings",
        &["get", "org.gnome.desktop.interface", "color-scheme"],
    )
    .and_then(|o| parse_color_scheme(&o))
    .or_else(|| {
        run(
            "dconf",
            &["read", "/org/gnome/desktop/interface/color-scheme"],
        )
        .and_then(|o| parse_color_scheme(&o))
    })
    .or_else(|| {
        run(
            "gsettings",
            &["get", "org.gnome.desktop.interface", "gtk-theme"],
        )
        .and_then(|o| parse_gtk_theme(&o))
    })
    .unwrap_or(Mode::Dark)
}

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

pub fn with_alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

pub fn muted(theme: &Theme) -> Color {
    with_alpha(theme.palette().text, 0.6)
}

pub fn tone_color(theme: &Theme, tone: Tone) -> Color {
    let p = theme.palette();
    match tone {
        Tone::Neutral => with_alpha(p.text, 0.55),
        Tone::Success => p.success,
        Tone::Info => p.primary,
        Tone::Warning => p.warning,
        Tone::Danger => p.danger,
    }
}

fn outline(theme: &Theme) -> Color {
    with_alpha(theme.palette().text, 0.10)
}

pub fn surface(theme: &Theme) -> Color {
    theme.extended_palette().background.weak.color
}

/// Carte arrondie.
pub fn card(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(surface(theme))),
        border: Border {
            color: outline(theme),
            width: 1.0,
            radius: 14.0.into(),
        },
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.12),
            offset: Vector::new(0.0, 2.0),
            blur_radius: 8.0,
        },
        ..container::Style::default()
    }
}

/// Fond de la barre latérale.
pub fn sidebar(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(
            theme.extended_palette().background.base.color,
        )),
        border: Border {
            color: outline(theme),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

/// Console : fond légèrement plus sombre que la base.
pub fn console(theme: &Theme) -> container::Style {
    let base = theme.extended_palette().background.base.color;
    let dark = crate::theme::is_dark(theme);
    let bg = if dark {
        Color::from_rgb(base.r * 0.7, base.g * 0.7, base.b * 0.7)
    } else {
        Color::from_rgb(base.r * 0.96, base.g * 0.96, base.b * 0.96)
    };
    container::Style {
        background: Some(Background::Color(bg)),
        border: Border {
            color: outline(theme),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn is_dark(theme: &Theme) -> bool {
    let b = theme.palette().background;
    (b.r + b.g + b.b) / 3.0 < 0.5
}

/// Pastille discrète colorée selon le ton.
pub fn badge(tone: Tone) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let c = tone_color(theme, tone);
        container::Style {
            background: Some(Background::Color(with_alpha(c, 0.16))),
            text_color: Some(c),
            border: border::rounded(999),
            ..container::Style::default()
        }
    }
}

/// Voile derrière un dialogue.
pub fn scrim(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.55))),
        ..container::Style::default()
    }
}

/// Bandeau d'erreur.
pub fn error_banner(theme: &Theme) -> container::Style {
    let c = theme.palette().danger;
    container::Style {
        background: Some(Background::Color(with_alpha(c, 0.12))),
        text_color: Some(c),
        border: Border {
            color: with_alpha(c, 0.4),
            width: 1.0,
            radius: 10.0.into(),
        },
        ..container::Style::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    Primary,
    Secondary,
    Danger,
    Ghost,
}

fn disabled_style(theme: &Theme, base: button::Style) -> button::Style {
    button::Style {
        background: base
            .background
            .map(|_| Background::Color(with_alpha(theme.palette().text, 0.06))),
        text_color: with_alpha(theme.palette().text, 0.3),
        ..base
    }
}

/// Style de bouton.
pub fn button_style(kind: Btn) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let p = theme.palette();
        let (bg, fg) = match kind {
            Btn::Primary => (Some(p.primary), readable_on(p.primary)),
            Btn::Secondary => (Some(with_alpha(p.text, 0.08)), p.text),
            Btn::Danger => (Some(with_alpha(p.danger, 0.14)), p.danger),
            Btn::Ghost => (None, p.text),
        };
        let base = button::Style {
            background: bg.map(Background::Color),
            text_color: fg,
            border: Border {
                color: if kind == Btn::Ghost {
                    outline(theme)
                } else {
                    Color::TRANSPARENT
                },
                width: 1.0,
                radius: 10.0.into(),
            },
            ..button::Style::default()
        };
        match status {
            button::Status::Active => base,
            button::Status::Hovered => button::Style {
                background: Some(Background::Color(match (kind, bg) {
                    (Btn::Primary, Some(c)) => with_alpha(c, 0.88),
                    (_, Some(c)) => with_alpha(c, (c.a + 0.08).min(1.0)),
                    (_, None) => with_alpha(p.text, 0.07),
                })),
                ..base
            },
            button::Status::Pressed => button::Style {
                background: Some(Background::Color(match bg {
                    Some(c) => with_alpha(c, (c.a + 0.16).min(1.0)),
                    None => with_alpha(p.text, 0.12),
                })),
                ..base
            },
            button::Status::Disabled => disabled_style(theme, base),
        }
    }
}

fn readable_on(c: Color) -> Color {
    if (c.r * 0.299 + c.g * 0.587 + c.b * 0.114) > 0.62 {
        Color::BLACK
    } else {
        Color::WHITE
    }
}

/// Ligne cliquable de la barre latérale.
pub fn row_style(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let p = theme.palette();
        let bg = match (selected, status) {
            (true, _) => Some(with_alpha(p.primary, 0.18)),
            (false, button::Status::Hovered | button::Status::Pressed) => {
                Some(with_alpha(p.text, 0.06))
            }
            _ => None,
        };
        button::Style {
            background: bg.map(Background::Color),
            text_color: p.text,
            border: Border {
                color: if selected {
                    with_alpha(p.primary, 0.5)
                } else {
                    Color::TRANSPARENT
                },
                width: 1.0,
                radius: 10.0.into(),
            },
            ..button::Style::default()
        }
    }
}

pub fn input_style(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let mut s = text_input::default(theme, status);
    s.border.radius = 10.0.into();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_scheme_parsing() {
        assert_eq!(parse_color_scheme("'prefer-dark'\n"), Some(Mode::Dark));
        assert_eq!(parse_color_scheme("'prefer-light'"), Some(Mode::Light));
        assert_eq!(parse_color_scheme("'default'"), None);
        assert_eq!(parse_color_scheme(""), None);
    }

    #[test]
    fn gtk_theme_parsing() {
        assert_eq!(parse_gtk_theme("'Adwaita-dark'"), Some(Mode::Dark));
        assert_eq!(parse_gtk_theme("'Adwaita'"), Some(Mode::Light));
        assert_eq!(parse_gtk_theme("''"), None);
    }

    #[test]
    fn system_mode_conversion() {
        assert_eq!(from_iced(iced::theme::Mode::Light), Some(Mode::Light));
        assert_eq!(from_iced(iced::theme::Mode::None), None);
    }

    #[test]
    fn themes_have_expected_darkness() {
        assert!(is_dark(&theme(Mode::Dark)));
        assert!(!is_dark(&theme(Mode::Light)));
    }
}
