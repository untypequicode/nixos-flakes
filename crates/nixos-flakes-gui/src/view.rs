//! Affichage : barre latérale, panneau de détail, console, dialogues.

use crate::app::{App, LogKind, Message, Page};
use crate::model::{
    self, badge, format_opt_date, group_entries, role_singular, role_title, short_rev, Modal,
    RepoEntry, RoleAction, Tone,
};
use crate::settings;
use crate::theme::{self, Btn};
use crate::widgets::{self, bold, chip, muted, semibold, toned, SPACE};
use iced::widget::{
    button, column, container, mouse_area, opaque, responsive, row, scrollable, space, stack, text,
    text_editor,
};
use iced::{Element, Fill, Font, Length, Size, Theme};
use nixos_flakes_core::config::Role;

/// Largeur à partir de laquelle la barre latérale reste visible.
pub const WIDE_BREAKPOINT: f32 = 820.0;
const SIDEBAR_WIDTH: f32 = 310.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    Wide,
    Narrow,
}

pub fn layout_mode(width: f32) -> LayoutMode {
    if width >= WIDE_BREAKPOINT {
        LayoutMode::Wide
    } else {
        LayoutMode::Narrow
    }
}

impl App {
    pub fn view(&self, _id: iced::window::Id) -> Element<'_, Message> {
        let body: Element<'_, Message> = match self.page {
            Page::Settings => settings::view(&self.cfg, &self.new_root, self.tray.is_some())
                .map(Message::Settings),
            Page::Main => responsive(|size| self.main_layout(size)).into(),
        };
        let page = column![self.top_bar(), body, self.console_view()];
        let base = container(page).width(Fill).height(Fill);
        match &self.modal {
            Some(m) => stack![base, self.modal_view(m)].into(),
            None => base.into(),
        }
    }

    fn top_bar(&self) -> Element<'_, Message> {
        let busy_scan: Element<'_, Message> = if self.scanning {
            muted("Recherche des dépôts…").size(12).into()
        } else {
            space::horizontal().width(0).into()
        };
        let in_settings = self.page == Page::Settings;
        container(
            row![
                text("❄").size(20).style(|t: &Theme| text::Style {
                    color: Some(t.palette().primary)
                }),
                text("Gestion des flakes").size(17).font(bold()),
                busy_scan,
                space::horizontal(),
                widgets::action(
                    "Tout vérifier",
                    Btn::Secondary,
                    (!in_settings).then_some(Message::CheckAll)
                ),
                widgets::action("↻ Rescanner", Btn::Ghost, Some(Message::Rescan)),
                widgets::action(
                    "⚙ Réglages",
                    Btn::Ghost,
                    (!in_settings).then_some(Message::OpenSettings)
                ),
            ]
            .spacing(SPACE)
            .align_y(iced::Center),
        )
        .padding([10, 16])
        .style(theme::sidebar)
        .width(Fill)
        .into()
    }

    fn main_layout(&self, size: Size) -> Element<'_, Message> {
        match layout_mode(size.width) {
            LayoutMode::Wide => row![
                container(self.sidebar_view()).width(SIDEBAR_WIDTH),
                self.detail_view(false),
            ]
            .height(Fill)
            .into(),
            LayoutMode::Narrow => {
                if self.sidebar_open {
                    container(self.sidebar_view())
                        .width(Fill)
                        .height(Fill)
                        .into()
                } else {
                    self.detail_view(true)
                }
            }
        }
    }

    // -- barre latérale -------------------------------------------------------

    fn sidebar_view(&self) -> Element<'_, Message> {
        let mut col = column![].spacing(4).padding(10);
        let groups = group_entries(&self.entries);
        if groups.is_empty() {
            col = col.push(
                container(
                    muted(if self.scanning {
                        "Recherche des dépôts flake…"
                    } else {
                        "Aucun dépôt flake trouvé. Ajoutez une racine de scan dans les réglages."
                    })
                    .size(13),
                )
                .padding(10),
            );
        }
        for (role, idx) in groups {
            col = col.push(
                container(
                    row![
                        muted(role_title(role).to_uppercase()).size(11).font(bold()),
                        chip(idx.len().to_string(), Tone::Neutral),
                    ]
                    .spacing(8)
                    .align_y(iced::Center),
                )
                .padding([10, 6]),
            );
            for i in idx {
                col = col.push(self.repo_row(&self.entries[i]));
            }
        }
        container(scrollable(col).height(Fill))
            .height(Fill)
            .style(theme::sidebar)
            .into()
    }

    fn repo_row<'a>(&'a self, e: &'a RepoEntry) -> Element<'a, Message> {
        let b = badge(e);
        let selected = self.selected.as_deref() == Some(e.path.as_path());
        let top = row![
            toned(b.glyph(), b.tone()).size(14),
            text(e.name()).size(15).font(semibold()).width(Fill),
        ]
        .spacing(8)
        .align_y(iced::Center);
        let sub = muted(model::display_path(&e.path, self.home.as_deref())).size(11);
        let badge_row = row![chip(b.label(), b.tone())];
        button(column![top, badge_row, sub].spacing(4))
            .width(Fill)
            .padding(10)
            .style(theme::row_style(selected))
            .on_press(Message::Select(e.path.clone()))
            .into()
    }

    // -- détail ---------------------------------------------------------------

    fn detail_view(&self, narrow: bool) -> Element<'_, Message> {
        let nav: Element<'_, Message> = if narrow {
            widgets::action("☰ Dépôts", Btn::Ghost, Some(Message::ToggleSidebar))
        } else {
            space::horizontal().width(0).into()
        };
        let inner: Element<'_, Message> = match self.selected_entry() {
            Some(e) => self.repo_detail(e),
            None => container(muted("Sélectionnez un dépôt dans la liste.").size(14))
                .center(Fill)
                .into(),
        };
        container(column![container(nav).padding([8, 16]), inner].spacing(0))
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn repo_detail<'a>(&'a self, e: &'a RepoEntry) -> Element<'a, Message> {
        let a = model::actions(e);
        let b = badge(e);

        let git_line = if e.git.is_repo {
            let branch = e.git.branch.as_deref().unwrap_or("(HEAD détachée)");
            let state = if e.git.dirty.is_empty() {
                "aucune modification".to_string()
            } else {
                format!("{} fichier(s) modifié(s)", e.git.dirty.len())
            };
            let lock = if e.git.lock_dirty {
                " · flake.lock modifié"
            } else {
                ""
            };
            format!("git · branche {branch} · {state}{lock}")
        } else {
            "Hors git".to_string()
        };

        let header = container(
            column![
                row![
                    text(e.name()).size(26).font(bold()),
                    chip(role_singular(e.role), role_tone(e.role)),
                    chip(b.label(), b.tone()),
                ]
                .spacing(10)
                .align_y(iced::Center)
                .wrap(),
                muted(model::display_path(&e.path, self.home.as_deref())).size(13),
                muted(git_line).size(13),
            ]
            .spacing(6),
        )
        .padding(18)
        .width(Fill)
        .style(theme::card);

        let p = e.path.clone();
        let msg = |on: bool, m: Message| on.then_some(m);
        let mut buttons = row![
            widgets::action(
                "Vérifier",
                Btn::Secondary,
                msg(a.check, Message::Check(p.clone()))
            ),
            widgets::action(
                "Mettre à jour",
                Btn::Primary,
                msg(a.update, Message::Update(p.clone()))
            ),
        ]
        .spacing(8);
        if e.role == Role::System {
            buttons = buttons.push(widgets::action(
                "Appliquer au système",
                Btn::Danger,
                msg(a.apply, Message::Apply(p.clone())),
            ));
        }
        if e.git.is_repo {
            buttons = buttons.push(widgets::action(
                "Push",
                Btn::Secondary,
                msg(a.push, Message::Push(p.clone())),
            ));
        }
        let role_btn =
            |label: &'static str, on: bool, act: RoleAction| -> Option<Element<'a, Message>> {
                on.then(|| widgets::action(label, Btn::Ghost, Some(Message::Role(p.clone(), act))))
            };
        let mut roles = row![].spacing(8);
        for el in [
            role_btn("Suivre", a.track, RoleAction::Track),
            role_btn("Ne plus suivre", a.untrack, RoleAction::Untrack),
            role_btn(
                "Définir comme dépôt système",
                a.set_system,
                RoleAction::SetSystem,
            ),
            role_btn("Ignorer", a.ignore, RoleAction::Ignore),
            role_btn("Ne plus ignorer", a.unignore, RoleAction::Unignore),
        ]
        .into_iter()
        .flatten()
        {
            roles = roles.push(el);
        }

        let actions = container(
            column![
                text("Actions").size(14).font(bold()),
                buttons.wrap(),
                roles.wrap(),
            ]
            .spacing(10),
        )
        .padding(16)
        .width(Fill)
        .style(theme::card);

        let mut col = column![header, actions].spacing(SPACE);
        if let Some(err) = &e.error {
            col = col.push(
                container(
                    column![
                        text("Erreur").font(bold()),
                        text(err.clone()).size(12).font(Font::MONOSPACE)
                    ]
                    .spacing(4),
                )
                .padding(14)
                .width(Fill)
                .style(theme::error_banner),
            );
        }
        col = col.push(self.changes_card(e));
        if e.git.is_repo && !e.git.dirty.is_empty() {
            col = col.push(dirty_card(e));
        }

        scrollable(
            container(col.max_width(980))
                .padding([0, 16])
                .center_x(Fill),
        )
        .height(Fill)
        .into()
    }

    fn changes_card<'a>(&'a self, e: &'a RepoEntry) -> Element<'a, Message> {
        let (title, changes, tone): (String, &[_], Tone) = match (&e.pending, &e.last_update) {
            (Some(p), _) if !p.is_empty() => (
                format!("Mises à jour disponibles ({})", p.len()),
                p.as_slice(),
                Tone::Info,
            ),
            (Some(_), Some(l)) if !l.is_empty() => (
                format!("Dernière mise à jour ({} input(s))", l.len()),
                l.as_slice(),
                Tone::Success,
            ),
            (Some(_), _) => (
                "Tous les inputs sont à jour".to_string(),
                &[],
                Tone::Success,
            ),
            (None, _) => (
                "Pas encore vérifié — cliquez sur « Vérifier »".to_string(),
                &[],
                Tone::Neutral,
            ),
        };
        let mut col = column![toned(title, tone).size(15).font(bold())].spacing(8);
        if !changes.is_empty() {
            col = col.push(table_row(
                ["Input", "Ancienne révision", "Nouvelle révision", "Date"],
                true,
            ));
            for c in changes {
                let date = format!(
                    "{} → {}",
                    format_opt_date(c.old_date),
                    format_opt_date(c.new_date)
                );
                col = col.push(table_row(
                    [
                        c.name.as_str(),
                        &short_rev(&c.old_rev),
                        &short_rev(&c.new_rev),
                        &date,
                    ],
                    false,
                ));
            }
        }
        container(col)
            .padding(16)
            .width(Fill)
            .style(theme::card)
            .into()
    }

    // -- console --------------------------------------------------------------

    fn console_view(&self) -> Element<'_, Message> {
        let header = row![
            text("Console").size(13).font(bold()),
            muted(format!("{} ligne(s)", self.log.len())).size(11),
            space::horizontal(),
            small_button(
                if self.console_open {
                    "▾ Réduire"
                } else {
                    "▴ Afficher"
                },
                Message::ToggleConsole
            ),
            small_button("Effacer", Message::ClearLog),
        ]
        .spacing(10)
        .align_y(iced::Center);
        let mut col = column![header].spacing(6);
        if self.console_open {
            let lines = self.log.iter().fold(column![].spacing(1), |c, l| {
                let kind = l.kind;
                c.push(text(l.text.clone()).size(12).font(Font::MONOSPACE).style(
                    move |t: &Theme| text::Style {
                        color: Some(match kind {
                            LogKind::Info => theme::with_alpha(t.palette().text, 0.8),
                            LogKind::Command => t.palette().primary,
                            LogKind::Error => t.palette().danger,
                            LogKind::Success => t.palette().success,
                        }),
                    },
                ))
            });
            col = col.push(
                scrollable(container(lines).padding([0, 6]).width(Fill))
                    .anchor_bottom()
                    .height(Length::Fixed(150.0))
                    .width(Fill),
            );
        }
        container(col)
            .padding([8, 14])
            .width(Fill)
            .style(theme::console)
            .into()
    }

    // -- dialogues ------------------------------------------------------------

    fn modal_view<'a>(&'a self, m: &'a Modal) -> Element<'a, Message> {
        let editor = text_editor(&self.editor)
            .on_action(Message::EditorAction)
            .height(Length::Fixed(130.0))
            .padding(10);
        let name = model::short_name(m.repo());
        let card: Element<'a, Message> = match m {
            Modal::CommitDirty { dirty, .. } => {
                let mut files = column![].spacing(2);
                for f in dirty.iter().take(8) {
                    files = files.push(text(f.clone()).size(12).font(Font::MONOSPACE));
                }
                if dirty.len() > 8 {
                    files =
                        files.push(muted(format!("… et {} autre(s)", dirty.len() - 8)).size(12));
                }
                column![
                    text("Modifications non commitées").size(19).font(bold()),
                    muted(format!(
                        "Le dépôt « {name} » contient {} fichier(s) modifié(s). \
                         Commitez-les avant de mettre à jour les verrous, pour garder un historique propre.",
                        dirty.len()
                    ))
                    .size(13),
                    files,
                    muted("Message du commit").size(12),
                    editor,
                    row![
                        widgets::action("Annuler", Btn::Ghost, Some(Message::ModalCancel)),
                        space::horizontal(),
                        widgets::action(
                            "Continuer quand même",
                            Btn::Secondary,
                            Some(Message::ModalContinue)
                        ),
                        widgets::action(
                            "Committer puis mettre à jour",
                            Btn::Primary,
                            Some(Message::ModalCommit)
                        ),
                    ]
                    .spacing(8),
                ]
                .spacing(10)
                .into()
            }
            Modal::CommitLock { .. } => column![
                text("Committer flake.lock ?").size(19).font(bold()),
                muted(format!(
                    "La mise à jour de « {name} » a modifié flake.lock. \
                     Seul ce fichier sera committé ; aucun push automatique."
                ))
                .size(13),
                muted("Message du commit").size(12),
                editor,
                row![
                    space::horizontal(),
                    widgets::action("Plus tard", Btn::Ghost, Some(Message::ModalCancel)),
                    widgets::action(
                        "Committer flake.lock",
                        Btn::Primary,
                        Some(Message::ModalCommit)
                    ),
                ]
                .spacing(8),
            ]
            .spacing(10)
            .into(),
        };
        opaque(
            mouse_area(
                container(
                    container(card)
                        .padding(22)
                        .max_width(560)
                        .style(theme::card),
                )
                .center(Fill)
                .padding(16)
                .style(theme::scrim),
            )
            .on_press(Message::Noop),
        )
    }
}

fn role_tone(role: Role) -> Tone {
    match role {
        Role::System => Tone::Warning,
        Role::Tracked => Tone::Info,
        Role::Detected => Tone::Neutral,
        Role::Ignored => Tone::Neutral,
    }
}

fn small_button(label: &'static str, msg: Message) -> Element<'static, Message> {
    button(text(label).size(11))
        .padding([3, 9])
        .style(theme::button_style(Btn::Ghost))
        .on_press(msg)
        .into()
}

fn table_row<'a>(cells: [&str; 4], header: bool) -> Element<'a, Message> {
    let portions = [3u16, 2, 2, 3];
    let mut r = row![].spacing(10).padding([4, 6]);
    for (c, w) in cells.iter().zip(portions) {
        let t = if header {
            muted(c.to_string()).size(11).font(bold())
        } else {
            text(c.to_string()).size(13).font(Font::MONOSPACE)
        };
        r = r.push(container(t).width(Length::FillPortion(w)));
    }
    if header {
        r.into()
    } else {
        container(r)
            .style(|t: &Theme| container::Style {
                background: Some(iced::Background::Color(theme::with_alpha(
                    t.palette().text,
                    0.04,
                ))),
                border: iced::border::rounded(8),
                ..container::Style::default()
            })
            .into()
    }
}

fn dirty_card<'a>(e: &'a RepoEntry) -> Element<'a, Message> {
    let mut files = column![].spacing(2);
    for f in e.git.dirty.iter().take(12) {
        files = files.push(text(f.clone()).size(12).font(Font::MONOSPACE));
    }
    if e.git.dirty.len() > 12 {
        files = files.push(muted(format!("… et {} autre(s)", e.git.dirty.len() - 12)).size(12));
    }
    container(
        column![
            toned("Modifications non commitées", Tone::Warning)
                .size(15)
                .font(bold()),
            files
        ]
        .spacing(8),
    )
    .padding(16)
    .width(Fill)
    .style(theme::card)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_breakpoint() {
        assert_eq!(layout_mode(1200.0), LayoutMode::Wide);
        assert_eq!(layout_mode(WIDE_BREAKPOINT), LayoutMode::Wide);
        assert_eq!(layout_mode(600.0), LayoutMode::Narrow);
    }

    #[test]
    fn role_tones() {
        assert_eq!(role_tone(Role::System), Tone::Warning);
        assert_eq!(role_tone(Role::Tracked), Tone::Info);
    }
}
