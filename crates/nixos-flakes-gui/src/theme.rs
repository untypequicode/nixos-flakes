//! Jetons de design (clair/sombre) et styles des composants, façon Linear :
//! surfaces sombres en couches, filets à ~7 % de blanc, un seul accent indigo.
//!
//! Tout est dérivé de [`Tokens`] ; l'interpolation entre deux jeux de jetons
//! ([`Tokens::mix`]) anime le changement de thème.

use crate::anim::{lerp, lerp_color};
use crate::model::Tone;
use iced::widget::{button, container, scrollable, text_editor, text_input, toggler};
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

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color::from_rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a)
}

/// Jetons de couleur d'un thème.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    /// Fond de l'application (barre latérale).
    pub bg: Color,
    /// Panneau principal.
    pub panel: Color,
    /// Éléments surélevés (cartes, champs, menus).
    pub raised: Color,
    /// Voile de survol (translucide).
    pub hover: Color,
    /// Voile de sélection (translucide).
    pub selected: Color,
    /// Filet de 1 px.
    pub border: Color,
    /// Filet appuyé (champs, boutons).
    pub border_strong: Color,
    pub text: Color,
    pub text2: Color,
    pub text3: Color,
    pub accent: Color,
    pub accent_hover: Color,
    pub on_accent: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    /// Voile derrière les dialogues (alpha maximal).
    pub scrim: Color,
}

pub fn tokens(mode: Mode) -> Tokens {
    match mode {
        Mode::Dark => Tokens {
            bg: rgb(0x0F, 0x10, 0x11),
            panel: rgb(0x15, 0x16, 0x18),
            raised: rgb(0x1B, 0x1C, 0x1F),
            hover: rgba(255, 255, 255, 0.045),
            selected: rgba(255, 255, 255, 0.085),
            border: rgba(255, 255, 255, 0.07),
            border_strong: rgba(255, 255, 255, 0.12),
            text: rgb(0xED, 0xED, 0xEF),
            text2: rgb(0x8A, 0x8F, 0x98),
            text3: rgb(0x62, 0x66, 0x6D),
            accent: rgb(0x5E, 0x6A, 0xD2),
            accent_hover: rgb(0x6F, 0x7B, 0xE0),
            on_accent: rgb(0xFF, 0xFF, 0xFF),
            success: rgb(0x4C, 0xA8, 0x7A),
            warning: rgb(0xD1, 0xA1, 0x4A),
            danger: rgb(0xE0, 0x6C, 0x66),
            scrim: rgba(0, 0, 0, 0.60),
        },
        Mode::Light => Tokens {
            bg: rgb(0xF4, 0xF5, 0xF6),
            panel: rgb(0xFC, 0xFC, 0xFD),
            raised: rgb(0xFF, 0xFF, 0xFF),
            hover: rgba(0, 0, 0, 0.04),
            selected: rgba(0, 0, 0, 0.07),
            border: rgba(0, 0, 0, 0.08),
            border_strong: rgba(0, 0, 0, 0.14),
            text: rgb(0x1F, 0x20, 0x23),
            text2: rgb(0x62, 0x66, 0x6D),
            text3: rgb(0x8A, 0x8F, 0x98),
            accent: rgb(0x5E, 0x6A, 0xD2),
            accent_hover: rgb(0x4F, 0x5B, 0xC4),
            on_accent: rgb(0xFF, 0xFF, 0xFF),
            success: rgb(0x2E, 0x8B, 0x5E),
            warning: rgb(0xA8, 0x70, 0x0E),
            danger: rgb(0xC4, 0x44, 0x3E),
            scrim: rgba(20, 22, 30, 0.38),
        },
    }
}

impl Tokens {
    /// Interpolation champ par champ (animation du thème).
    pub fn mix(a: Tokens, b: Tokens, t: f32) -> Tokens {
        let m = |x: Color, y: Color| lerp_color(x, y, t);
        Tokens {
            bg: m(a.bg, b.bg),
            panel: m(a.panel, b.panel),
            raised: m(a.raised, b.raised),
            hover: m(a.hover, b.hover),
            selected: m(a.selected, b.selected),
            border: m(a.border, b.border),
            border_strong: m(a.border_strong, b.border_strong),
            text: m(a.text, b.text),
            text2: m(a.text2, b.text2),
            text3: m(a.text3, b.text3),
            accent: m(a.accent, b.accent),
            accent_hover: m(a.accent_hover, b.accent_hover),
            on_accent: m(a.on_accent, b.on_accent),
            success: m(a.success, b.success),
            warning: m(a.warning, b.warning),
            danger: m(a.danger, b.danger),
            scrim: m(a.scrim, b.scrim),
        }
    }

    /// Couleur associée à un ton sémantique.
    pub fn tone(&self, tone: Tone) -> Color {
        match tone {
            Tone::Neutral => self.text3,
            Tone::Success => self.success,
            Tone::Info => self.accent,
            Tone::Warning => self.warning,
            Tone::Danger => self.danger,
        }
    }

    /// Teinte lisible d'un ton pour du texte (l'accent est éclairci sur fond sombre).
    pub fn tone_text(&self, tone: Tone) -> Color {
        match tone {
            Tone::Neutral => self.text2,
            Tone::Info => lerp_color(self.accent, self.text, 0.35),
            other => self.tone(other),
        }
    }

    /// Thème iced (palette) cohérent avec les jetons, pour les widgets non stylés.
    pub fn iced_theme(&self, name: &str) -> Theme {
        Theme::custom(
            name.to_string(),
            iced::theme::Palette {
                background: self.bg,
                text: self.text,
                primary: self.accent,
                success: self.success,
                warning: self.warning,
                danger: self.danger,
            },
        )
    }

    pub fn is_dark(&self) -> bool {
        (self.bg.r + self.bg.g + self.bg.b) / 3.0 < 0.5
    }
}

pub fn with_alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

/// Multiplie l'opacité d'une couleur.
pub fn fade(c: Color, k: f32) -> Color {
    Color { a: c.a * k, ..c }
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
// Styles de conteneurs
// ---------------------------------------------------------------------------

fn hairline(c: Color, radius: f32) -> Border {
    Border {
        color: c,
        width: 1.0,
        radius: radius.into(),
    }
}

/// Fond de l'application / barre latérale.
pub fn sidebar(tk: Tokens) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(tk.bg)),
        ..container::Style::default()
    }
}

/// Panneau principal, filet à gauche simulé par la bordure.
pub fn panel(tk: Tokens) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(tk.panel)),
        border: hairline(tk.border, 0.0),
        ..container::Style::default()
    }
}

/// Carte surélevée aux filets fins.
pub fn card(tk: Tokens) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(tk.raised)),
        border: hairline(tk.border, 10.0),
        ..container::Style::default()
    }
}

/// Bloc plat (tableaux) : fond du panneau, filet fin.
pub fn outlined(tk: Tokens) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: None,
        border: hairline(tk.border, 8.0),
        ..container::Style::default()
    }
}

/// Fenêtre flottante (dialogue, palette) : carte avec ombre portée.
pub fn floating(tk: Tokens) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(tk.raised)),
        border: hairline(tk.border_strong, 12.0),
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, if tk.is_dark() { 0.55 } else { 0.18 }),
            offset: Vector::new(0.0, 14.0),
            blur_radius: 36.0,
        },
        ..container::Style::default()
    }
}

/// Info-bulle.
pub fn tooltip(tk: Tokens) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(tk.raised)),
        text_color: Some(tk.text),
        border: hairline(tk.border_strong, 6.0),
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
            offset: Vector::new(0.0, 4.0),
            blur_radius: 12.0,
        },
        ..container::Style::default()
    }
}

/// Voile derrière un dialogue ; `k` ∈ [0, 1] règle l'opacité.
pub fn scrim(tk: Tokens, k: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(fade(tk.scrim, k))),
        ..container::Style::default()
    }
}

/// Aplat d'une couleur (voiles de fondu, séparateurs, pastilles).
pub fn flat(color: Color, radius: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(color)),
        border: border::rounded(radius),
        ..container::Style::default()
    }
}

/// Pastille (pilule) discrète teintée selon le ton : texte coloré, fond à ~12 %.
pub fn pill(tk: Tone, t: Tokens) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let c = t.tone(tk);
        container::Style {
            background: Some(Background::Color(with_alpha(c, 0.13))),
            text_color: Some(t.tone_text(tk)),
            border: Border {
                color: with_alpha(c, 0.22),
                width: 1.0,
                radius: 999.0.into(),
            },
            ..container::Style::default()
        }
    }
}

/// Chip de révision (monospace) ou raccourci clavier.
pub fn chip(tk: Tokens) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(tk.hover)),
        text_color: Some(tk.text2),
        border: hairline(tk.border, 5.0),
        ..container::Style::default()
    }
}

/// Bandeau d'erreur.
pub fn error_banner(tk: Tokens) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(with_alpha(tk.danger, 0.10))),
        text_color: Some(tk.danger),
        border: hairline(with_alpha(tk.danger, 0.30), 8.0),
        ..container::Style::default()
    }
}

/// Console : plus profonde que le panneau.
pub fn console(tk: Tokens) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(tk.bg)),
        border: hairline(tk.border, 0.0),
        ..container::Style::default()
    }
}

// ---------------------------------------------------------------------------
// Boutons
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    Primary,
    Secondary,
    Danger,
    Ghost,
}

/// Style de bouton ; `hover` ∈ [0, 1] est le niveau animé de survol.
pub fn button_style(
    tk: Tokens,
    kind: Btn,
    hover: f32,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let (rest, over, fg, edge) = match kind {
            Btn::Primary => (tk.accent, tk.accent_hover, tk.on_accent, Color::TRANSPARENT),
            Btn::Secondary => (
                with_alpha(tk.raised, 1.0),
                lerp_color(tk.raised, tk.text, 0.07),
                tk.text,
                tk.border_strong,
            ),
            Btn::Danger => (
                with_alpha(tk.danger, 0.10),
                with_alpha(tk.danger, 0.20),
                tk.danger,
                with_alpha(tk.danger, 0.28),
            ),
            Btn::Ghost => (Color::TRANSPARENT, tk.hover, tk.text2, Color::TRANSPARENT),
        };
        let h = match status {
            button::Status::Pressed => 1.3,
            _ => hover,
        };
        let mut bg = lerp_color(rest, over, h.min(1.0));
        if h > 1.0 {
            // Pression : un cran plus marqué que le survol.
            bg = lerp_color(bg, tk.text, 0.05);
        }
        let fg = if kind == Btn::Ghost {
            lerp_color(fg, tk.text, hover.min(1.0))
        } else {
            fg
        };
        let base = button::Style {
            background: Some(Background::Color(bg)),
            text_color: fg,
            border: hairline(edge, 7.0),
            ..button::Style::default()
        };
        match status {
            button::Status::Disabled => button::Style {
                background: match kind {
                    Btn::Primary | Btn::Secondary => Some(Background::Color(tk.hover)),
                    _ => None,
                },
                text_color: tk.text3,
                border: hairline(with_alpha(edge, edge.a * 0.5), 7.0),
                ..base
            },
            _ => base,
        }
    }
}

/// Ligne cliquable (barre latérale, palette) : fond animé par survol et sélection.
pub fn row_style(
    tk: Tokens,
    hover: f32,
    selected: f32,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, _| {
        let bg = lerp_color(
            Color::TRANSPARENT,
            lerp_color(tk.hover, tk.selected, selected),
            hover.max(selected).min(1.0),
        );
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: lerp_color(tk.text2, tk.text, hover.max(selected).min(1.0)),
            border: border::rounded(6),
            ..button::Style::default()
        }
    }
}

/// Onglet : texte plus vif quand actif ou survolé, sans fond.
pub fn tab_style(
    tk: Tokens,
    active: bool,
    hover: f32,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, _| button::Style {
        background: Some(Background::Color(lerp_color(
            Color::TRANSPARENT,
            tk.hover,
            if active { 0.0 } else { hover },
        ))),
        text_color: if active {
            tk.text
        } else {
            lerp_color(tk.text2, tk.text, hover)
        },
        border: border::rounded(6),
        ..button::Style::default()
    }
}

// ---------------------------------------------------------------------------
// Champs, défilement, interrupteur
// ---------------------------------------------------------------------------

pub fn input_style(tk: Tokens) -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
    move |_, status| {
        let edge = match status {
            text_input::Status::Focused { .. } => tk.accent,
            text_input::Status::Hovered => tk.text3,
            _ => tk.border_strong,
        };
        text_input::Style {
            background: Background::Color(tk.panel),
            border: hairline(edge, 7.0),
            icon: tk.text2,
            placeholder: tk.text3,
            value: tk.text,
            selection: with_alpha(tk.accent, 0.35),
        }
    }
}

/// Champ de la palette : sans bordure, intégré à la fenêtre.
pub fn bare_input_style(tk: Tokens) -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
    move |_, _| text_input::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: border::rounded(0),
        icon: tk.text2,
        placeholder: tk.text3,
        value: tk.text,
        selection: with_alpha(tk.accent, 0.35),
    }
}

pub fn editor_style(tk: Tokens) -> impl Fn(&Theme, text_editor::Status) -> text_editor::Style {
    move |_, status| {
        let edge = match status {
            text_editor::Status::Focused { .. } => tk.accent,
            text_editor::Status::Hovered => tk.text3,
            _ => tk.border_strong,
        };
        text_editor::Style {
            background: Background::Color(tk.panel),
            border: hairline(edge, 7.0),
            placeholder: tk.text3,
            value: tk.text,
            selection: with_alpha(tk.accent, 0.35),
        }
    }
}

pub fn scroll_style(tk: Tokens) -> impl Fn(&Theme, scrollable::Status) -> scrollable::Style {
    move |theme, status| {
        let mut s = scrollable::default(theme, status);
        let active = match status {
            scrollable::Status::Active { .. } => 0.0,
            scrollable::Status::Hovered { .. } => 0.5,
            scrollable::Status::Dragged { .. } => 1.0,
        };
        let rail = scrollable::Rail {
            background: None,
            border: border::rounded(4),
            scroller: scrollable::Scroller {
                background: Background::Color(lerp_color(
                    with_alpha(tk.text, 0.10),
                    with_alpha(tk.text, 0.30),
                    active,
                )),
                border: border::rounded(4),
            },
        };
        s.container = container::Style::default();
        s.vertical_rail = rail;
        s.horizontal_rail = rail;
        s.gap = None;
        s
    }
}

pub fn toggler_style(tk: Tokens) -> impl Fn(&Theme, toggler::Status) -> toggler::Style {
    move |_, status| {
        let on = match status {
            toggler::Status::Active { is_toggled }
            | toggler::Status::Hovered { is_toggled }
            | toggler::Status::Disabled { is_toggled } => is_toggled,
        };
        toggler::Style {
            background: Background::Color(if on { tk.accent } else { tk.selected }),
            background_border_width: 1.0,
            background_border_color: if on {
                Color::TRANSPARENT
            } else {
                tk.border_strong
            },
            foreground: Background::Color(if on { tk.on_accent } else { tk.text2 }),
            foreground_border_width: 0.0,
            foreground_border_color: Color::TRANSPARENT,
            text_color: Some(tk.text),
            border_radius: None,
            padding_ratio: 0.2,
        }
    }
}

/// Opacité animée d'un voile de fondu : `1 - k`.
pub fn veil(k: f32) -> f32 {
    lerp(1.0, 0.0, k.clamp(0.0, 1.0))
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
        assert!(tokens(Mode::Dark).is_dark());
        assert!(!tokens(Mode::Light).is_dark());
    }

    #[test]
    fn token_mix_hits_both_ends() {
        let (d, l) = (tokens(Mode::Dark), tokens(Mode::Light));
        assert_eq!(Tokens::mix(d, l, 0.0), d);
        assert_eq!(Tokens::mix(d, l, 1.0), l);
        let mid = Tokens::mix(d, l, 0.5);
        assert!(mid.bg.r > d.bg.r && mid.bg.r < l.bg.r);
    }

    #[test]
    fn dark_surfaces_are_layered() {
        let t = tokens(Mode::Dark);
        let lum = |c: Color| c.r + c.g + c.b;
        assert!(lum(t.bg) < lum(t.panel) && lum(t.panel) < lum(t.raised));
        assert!(lum(t.text3) < lum(t.text2) && lum(t.text2) < lum(t.text));
    }

    #[test]
    fn text_hierarchy_has_readable_contrast() {
        // Le texte principal et secondaire restent lisibles sur le panneau (WCAG AA ≥ 4,5).
        fn lin(c: f32) -> f32 {
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        fn lum(c: Color) -> f32 {
            0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
        }
        fn ratio(a: Color, b: Color) -> f32 {
            let (x, y) = (lum(a), lum(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        }
        for mode in [Mode::Dark, Mode::Light] {
            let t = tokens(mode);
            assert!(ratio(t.text, t.panel) >= 7.0, "{mode:?} texte");
            assert!(ratio(t.text2, t.panel) >= 4.5, "{mode:?} texte secondaire");
            assert!(
                ratio(t.on_accent, t.accent) >= 4.0,
                "{mode:?} bouton primaire"
            );
        }
    }

    #[test]
    fn veil_goes_from_opaque_to_clear() {
        assert_eq!(veil(0.0), 1.0);
        assert_eq!(veil(1.0), 0.0);
    }
}
