//! Petits composants visuels réutilisables.

use crate::model::Tone;
use crate::theme::{self, Btn};
use iced::font::Weight;
use iced::widget::{button, container, text};
use iced::{Element, Font, Theme};

pub const SPACE: f32 = 12.0;

pub fn bold() -> Font {
    Font {
        weight: Weight::Bold,
        ..Font::DEFAULT
    }
}

pub fn semibold() -> Font {
    Font {
        weight: Weight::Semibold,
        ..Font::DEFAULT
    }
}

/// Texte atténué.
pub fn muted<'a>(s: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(s).style(|t: &Theme| text::Style {
        color: Some(theme::muted(t)),
    })
}

/// Texte coloré selon un ton.
pub fn toned<'a>(s: impl text::IntoFragment<'a>, tone: Tone) -> text::Text<'a> {
    text(s).style(move |t: &Theme| text::Style {
        color: Some(theme::tone_color(t, tone)),
    })
}

/// Pastille arrondie.
pub fn chip<'a, M: 'a>(label: impl text::IntoFragment<'a>, tone: Tone) -> Element<'a, M> {
    container(text(label).size(11).font(semibold()))
        .padding([2, 8])
        .style(theme::badge(tone))
        .into()
}

/// Bouton à libellé texte ; désactivé si `msg` est `None`.
pub fn action<'a, M: Clone + 'a>(
    label: impl text::IntoFragment<'a>,
    kind: Btn,
    msg: Option<M>,
) -> Element<'a, M> {
    button(text(label).size(13))
        .padding([8, 14])
        .style(theme::button_style(kind))
        .on_press_maybe(msg)
        .into()
}
