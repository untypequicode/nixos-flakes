//! Composants visuels réutilisables : boutons, pastilles, raccourcis, indicateurs
//! animés (spinner, barre indéterminée, squelettes), états vides, infobulles.

use crate::anim::{ease_in_out_cubic, lerp, pulse, Fades};
use crate::app::Message;
use crate::model::{Badge, Tone};
use crate::theme::{self, fade, with_alpha, Btn, Tokens};
use iced::font::Weight;
use iced::widget::{
    button, column, container, mouse_area, row, space, stack, text, tooltip, Container, Text,
};
use iced::{Element, Fill, Font, Length};
use std::time::{Duration, Instant};

pub const INTER: Font = Font::with_name("Inter");
pub const MONO: Font = Font::MONOSPACE;

pub fn regular() -> Font {
    INTER
}

pub fn medium() -> Font {
    Font {
        weight: Weight::Medium,
        ..INTER
    }
}

pub fn semibold() -> Font {
    Font {
        weight: Weight::Semibold,
        ..INTER
    }
}

/// Période du spinner et de la pulsation des points d'état.
pub const SPIN_PERIOD: Duration = Duration::from_millis(1_100);
/// Période du balayage de la barre indéterminée.
pub const SWEEP_PERIOD: Duration = Duration::from_millis(1_500);

/// Contexte de rendu : jetons du thème, instant courant et transitions en cours.
#[derive(Clone, Copy)]
pub struct Ui<'a> {
    pub tk: Tokens,
    pub now: Instant,
    pub fades: &'a Fades,
    /// Phase `[0, 1)` des animations cycliques (spinner, pulsation).
    pub spin: f32,
    /// Phase `[0, 1)` du balayage.
    pub sweep: f32,
}

impl Ui<'_> {
    /// Niveau animé de survol d'un élément.
    pub fn hover(&self, key: &str) -> f32 {
        self.fades.level(&format!("hov:{key}"), self.now)
    }

    /// Niveau animé de sélection d'un élément.
    pub fn selection(&self, key: &str) -> f32 {
        self.fades.level(&format!("sel:{key}"), self.now)
    }

    pub fn pulse(&self) -> f32 {
        pulse(self.spin)
    }
}

// -- texte ---------------------------------------------------------------------

pub fn body<'a>(tk: Tokens, s: impl text::IntoFragment<'a>) -> Text<'a> {
    text(s).size(13).color(tk.text)
}

pub fn dim<'a>(tk: Tokens, s: impl text::IntoFragment<'a>) -> Text<'a> {
    text(s).size(13).color(tk.text2)
}

pub fn faint<'a>(tk: Tokens, s: impl text::IntoFragment<'a>) -> Text<'a> {
    text(s).size(12).color(tk.text3)
}

pub fn mono<'a>(tk: Tokens, s: impl text::IntoFragment<'a>) -> Text<'a> {
    text(s).size(12).font(MONO).color(tk.text)
}

pub fn title<'a>(tk: Tokens, s: impl text::IntoFragment<'a>, size: f32) -> Text<'a> {
    text(s).size(size).font(semibold()).color(tk.text)
}

/// Étiquette de section en petites capitales (« SYSTÈME »).
pub fn section_label<'a>(tk: Tokens, s: &str) -> Text<'a> {
    text(s.to_uppercase())
        .size(10.5)
        .font(semibold())
        .color(tk.text3)
}

/// Étiquette de section alignée sur le texte des rangées (6 px de retrait).
pub fn section_label_pad<'a>(tk: Tokens, s: &str) -> Container<'a, Message> {
    container(section_label(tk, s)).padding([4, 6])
}

// -- survol ----------------------------------------------------------------------

/// Enveloppe `content` pour suivre le survol de la clé `key`.
pub fn hoverable<'a>(
    key: String,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    mouse_area(content)
        .on_enter(Message::Hover(key.clone(), true))
        .on_exit(Message::Hover(key, false))
        .into()
}

// -- boutons -----------------------------------------------------------------------

/// Chip de raccourci clavier (« R », « Ctrl K »).
pub fn kbd<'a>(ui: &Ui<'_>, label: &str, on_accent: bool) -> Element<'a, Message> {
    let tk = ui.tk;
    let fg = if on_accent {
        with_alpha(tk.on_accent, 0.85)
    } else {
        tk.text2
    };
    container(text(label.to_string()).size(10.5).font(medium()).color(fg))
        .padding([1, 5])
        .style(move |_: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(if on_accent {
                with_alpha(tk.on_accent, 0.16)
            } else {
                tk.hover
            })),
            border: iced::Border {
                color: if on_accent {
                    with_alpha(tk.on_accent, 0.22)
                } else {
                    tk.border
                },
                width: 1.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// Bouton avec libellé et raccourci optionnel ; désactivé si `msg` est `None`.
pub fn action<'a>(
    ui: &Ui<'_>,
    key: &str,
    label: &str,
    kind: Btn,
    shortcut: Option<&str>,
    msg: Option<Message>,
) -> Element<'a, Message> {
    let tk = ui.tk;
    let mut content = row![text(label.to_string())
        .size(13)
        .font(medium())
        .wrapping(text::Wrapping::None)]
    .spacing(8)
    .align_y(iced::Center);
    if let Some(s) = shortcut {
        content = content.push(kbd(ui, s, kind == Btn::Primary));
    }
    let enabled = msg.is_some();
    let b = button(content)
        .padding([6, 11])
        .style(theme::button_style(tk, kind, ui.hover(key)))
        .on_press_maybe(msg);
    if enabled {
        hoverable(key.to_string(), b)
    } else {
        b.into()
    }
}

/// Ligne de navigation pleine largeur de 28 px (pied de la barre latérale) ;
/// `active` : page courante, surlignée.
pub fn nav_item<'a>(
    ui: &Ui<'_>,
    key: &str,
    label: &str,
    active: bool,
    msg: Message,
) -> Element<'a, Message> {
    let tk = ui.tk;
    let content = row![
        text(label.to_string())
            .size(13)
            .font(if active { medium() } else { regular() })
            .wrapping(text::Wrapping::None),
        space::horizontal(),
    ]
    .height(Fill)
    .align_y(iced::Center);
    let b = button(content)
        .width(Fill)
        .height(28)
        .padding([0, 10])
        .style(theme::row_style(
            tk,
            ui.hover(key),
            if active { 1.0 } else { 0.0 },
        ))
        .on_press(msg);
    hoverable(key.to_string(), b)
}

/// Raccourcit `s` à `max` caractères avec une ellipse (le texte complet va en infobulle).
pub fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", head.trim_end())
}

/// Petit bouton fantôme de 24 px, aligné à droite d'une rangée d'état ;
/// `primary` : variante discrète à fond teinté (une seule par bloc).
pub fn mini_button<'a>(
    ui: &Ui<'_>,
    key: &str,
    label: &str,
    primary: bool,
    msg: Option<Message>,
) -> Element<'a, Message> {
    let tk = ui.tk;
    let enabled = msg.is_some();
    let hover = ui.hover(key);
    let b = button(
        text(label.to_string())
            .size(12)
            .font(medium())
            .wrapping(text::Wrapping::None),
    )
    .height(24)
    .padding([0, 8])
    .style(move |_: &iced::Theme, status| {
        let disabled = matches!(status, button::Status::Disabled);
        let (rest, over, fg) = if primary {
            (
                with_alpha(tk.accent, 0.14),
                with_alpha(tk.accent, 0.26),
                tk.tone_text(Tone::Info),
            )
        } else {
            (iced::Color::TRANSPARENT, tk.hover, tk.text2)
        };
        let bg = crate::anim::lerp_color(rest, over, hover.min(1.0));
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: if disabled {
                tk.text3
            } else if primary {
                fg
            } else {
                crate::anim::lerp_color(fg, tk.text, hover.min(1.0))
            },
            border: iced::border::rounded(5),
            ..button::Style::default()
        }
    })
    .on_press_maybe(msg);
    if enabled {
        hoverable(key.to_string(), b)
    } else {
        b.into()
    }
}

/// Rangée d'état de 28 px : pastille, texte court sur une ligne (infobulle `detail`),
/// puis actions alignées à droite.
pub fn status_row<'a>(
    ui: &Ui<'_>,
    color: iced::Color,
    label: &str,
    detail: &str,
    max_chars: usize,
    actions: Vec<Element<'a, Message>>,
) -> Element<'a, Message> {
    let tk = ui.tk;
    let line = row![
        container(dot(color, 7.0)).center_x(12),
        text(ellipsize(label, max_chars))
            .size(12)
            .color(tk.text2)
            .wrapping(text::Wrapping::None),
    ]
    .spacing(6)
    .align_y(iced::Center);
    let mut r = row![
        tip(ui, line, detail, tooltip::Position::Top),
        space::horizontal()
    ]
    .spacing(6)
    .align_y(iced::Center);
    for a in actions {
        r = r.push(a);
    }
    container(r).height(28).padding([0, 6]).center_y(28).into()
}

/// Info-bulle sous l'élément.
pub fn tip<'a>(
    ui: &Ui<'_>,
    content: impl Into<Element<'a, Message>>,
    label: &str,
    position: tooltip::Position,
) -> Element<'a, Message> {
    let tk = ui.tk;
    tooltip(
        content,
        container(text(label.to_string()).size(12).color(tk.text))
            .padding([4, 8])
            .style(theme::tooltip(tk)),
        position,
    )
    .gap(6)
    .delay(Duration::from_millis(350))
    .into()
}

// -- pastilles ---------------------------------------------------------------------

/// Pilule de texte teintée (rôle, état).
pub fn pill<'a>(
    ui: &Ui<'_>,
    label: impl text::IntoFragment<'a>,
    tone: Tone,
) -> Element<'a, Message> {
    container(text(label).size(11).font(medium()))
        .padding([1, 8])
        .style(theme::pill(tone, ui.tk))
        .into()
}

/// Chip monospace (révision git).
pub fn rev_chip<'a>(
    ui: &Ui<'_>,
    label: impl text::IntoFragment<'a>,
    new: bool,
) -> Element<'a, Message> {
    let tk = ui.tk;
    let fg = if new { tk.text } else { tk.text2 };
    container(text(label).size(12).font(MONO).color(fg))
        .padding([1, 6])
        .style(theme::chip(tk))
        .into()
}

/// Point plein.
pub fn dot<'a>(color: iced::Color, size: f32) -> Container<'a, Message> {
    container(space::horizontal())
        .width(size)
        .height(size)
        .style(theme::flat(color, size / 2.0))
}

/// Point d'état d'un dépôt ; « respire » pendant une commande.
pub fn status_dot<'a>(ui: &Ui<'_>, badge: Badge) -> Element<'a, Message> {
    let tk = ui.tk;
    let busy = matches!(badge, Badge::Busy(_));
    let color = tk.tone(badge.tone());
    let (size, color) = if busy {
        let p = ui.pulse();
        (lerp(5.0, 8.0, p), with_alpha(color, lerp(0.45, 1.0, p)))
    } else if matches!(badge, Badge::Unchecked) {
        (6.0, with_alpha(color, 0.7))
    } else {
        (7.0, color)
    };
    container(dot(color, size))
        .width(12)
        .height(12)
        .center(12)
        .into()
}

/// Spinner à trois points qui s'allument en cascade.
pub fn spinner<'a>(ui: &Ui<'_>, color: iced::Color) -> Element<'a, Message> {
    let mut r = row![].spacing(3).align_y(iced::Center);
    for i in 0..3 {
        let ph = (ui.spin * 1.0 + 1.0 - i as f32 * 0.16).fract();
        let a = lerp(0.25, 1.0, pulse(ph));
        r = r.push(dot(with_alpha(color, a), 4.0));
    }
    r.into()
}

/// Barre de progression indéterminée de 2 px (invisible au repos, hauteur constante).
pub fn progress_bar<'a>(ui: &Ui<'_>, active: bool) -> Element<'a, Message> {
    const SEG: f32 = 0.28;
    let tk = ui.tk;
    if !active {
        return container(space::horizontal()).width(Fill).height(2).into();
    }
    let p = ease_in_out_cubic(ui.sweep);
    let left = (p * (1.0 - SEG) * 1000.0) as u16;
    let mid = (SEG * 1000.0) as u16;
    let right = ((1.0 - SEG) * 1000.0) as u16 - left;
    container(
        row![
            space::horizontal().width(Length::FillPortion(left.max(1))),
            container(space::horizontal())
                .width(Length::FillPortion(mid))
                .height(2)
                .style(theme::flat(tk.accent, 1.0)),
            space::horizontal().width(Length::FillPortion(right.max(1))),
        ]
        .height(2),
    )
    .width(Fill)
    .height(2)
    .style(theme::flat(with_alpha(tk.accent, 0.12), 0.0))
    .into()
}

/// Barre « squelette » pulsante, pendant un chargement.
pub fn skeleton<'a>(ui: &Ui<'_>, width: impl Into<Length>, height: f32) -> Element<'a, Message> {
    let a = lerp(0.05, 0.12, ui.pulse());
    container(space::horizontal())
        .width(width)
        .height(height)
        .style(theme::flat(with_alpha(ui.tk.text, a), 5.0))
        .into()
}

/// Chevron de groupe : « › » qui se fond en « ⌄ » selon l'ouverture `k` ∈ [0, 1].
pub fn chevron<'a>(ui: &Ui<'_>, k: f32) -> Element<'a, Message> {
    let tk = ui.tk;
    let glyph = |g: &'static str, a: f32| {
        container(text(g).size(13).font(medium()).color(fade(tk.text3, a)))
            .center(14)
            .width(14)
            .height(14)
    };
    stack![glyph("›", 1.0 - k), glyph("⌄", k)]
        .width(14)
        .height(14)
        .into()
}

/// Fait apparaître `content` en fondu depuis la couleur de fond `bg` (`k` : 0 → 1).
pub fn fade_in<'a>(
    content: impl Into<Element<'a, Message>>,
    bg: iced::Color,
    k: f32,
) -> Element<'a, Message> {
    if k >= 0.999 {
        return content.into();
    }
    stack![
        content.into(),
        container(space::horizontal())
            .width(Fill)
            .height(Fill)
            .style(theme::flat(with_alpha(bg, theme::veil(k)), 0.0)),
    ]
    .width(Fill)
    .height(Fill)
    .into()
}

/// État vide : message amical et appel à l'action.
pub fn empty_state<'a>(
    ui: &Ui<'_>,
    heading: &str,
    detail: &str,
    cta: Option<(&str, &str, Message)>,
) -> Element<'a, Message> {
    let tk = ui.tk;
    let mut col = column![
        container(dot(with_alpha(tk.accent, 0.9), 8.0))
            .padding(10)
            .style(theme::card(tk)),
        title(tk, heading.to_string(), 15.0),
        container(dim(tk, detail.to_string()).size(13).center()).max_width(380),
    ]
    .spacing(10)
    .align_x(iced::Center);
    if let Some((key, label, msg)) = cta {
        col = col.push(action(ui, key, label, Btn::Primary, None, Some(msg)));
    }
    container(col).center(Fill).padding(24).into()
}
