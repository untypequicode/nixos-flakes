//! Affichage : barre latérale, panneau de détail, console, dialogue, palette, toasts.

use crate::anim;
use crate::app::{App, InputSelect, LogKind, Message, Page, Tab, CONSOLE_HEIGHT};
use crate::model::{
    self, badge, format_opt_date, format_relative, group_entries, role_singular, role_title,
    short_rev, Badge, Modal, RepoEntry, RoleAction, Tone,
};
use crate::palette::Item;
use crate::settings;
use crate::theme::{self, with_alpha, Btn};
use crate::toast::{ToastKind, Toasts};
use crate::widgets::{
    self, action, body, chevron, dim, dot, empty_state, fade_in, faint, hoverable, kbd, medium,
    mono, nav_item, pill, progress_bar, rev_chip, section_label, semibold, skeleton, spinner,
    status_dot, tip, title, Ui,
};
use iced::widget::{
    button, column, container, mouse_area, opaque, responsive, row, scrollable, space, stack, text,
    text_editor, text_input, tooltip, Stack,
};
use iced::{Element, Fill, Length, Padding, Size};
use nixos_flakes_core::config::Role;
use std::time::{SystemTime, UNIX_EPOCH};

/// Largeur à partir de laquelle la barre latérale reste visible.
pub const WIDE_BREAKPOINT: f32 = 820.0;
const SIDEBAR_WIDTH: f32 = 264.0;
/// Largeurs du panneau principal sous lesquelles les actions passent sous le titre
/// (le dépôt système a un bouton de plus).
const COMPACT_SYSTEM: f32 = 800.0;
const COMPACT_OTHER: f32 = 600.0;
const ROW_HEIGHT: f32 = 32.0;
const ROW_GAP: f32 = 2.0;

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

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// Séparateur horizontal d'un pixel.
fn hairline<'a>(ui: &Ui<'_>) -> Element<'a, Message> {
    container(space::horizontal())
        .width(Fill)
        .height(1)
        .style(theme::flat(ui.tk.border, 0.0))
        .into()
}

impl App {
    pub fn view(&self, _id: iced::window::Id) -> Element<'_, Message> {
        let ui = self.ui();
        let tk = ui.tk;
        let main: Element<'_, Message> = responsive(move |size| self.main_layout(&ui, size)).into();
        let mut layers: Vec<Element<'_, Message>> = vec![container(main)
            .width(Fill)
            .height(Fill)
            .style(theme::sidebar(tk))
            .into()];
        if let Some(l) = self.modal_layer(&ui) {
            layers.push(l);
        }
        if let Some(l) = self.palette_layer(&ui) {
            layers.push(l);
        }
        if !self.toasts.is_empty() {
            layers.push(toast_layer(&ui, &self.toasts));
        }
        Stack::with_children(layers).width(Fill).height(Fill).into()
    }

    fn main_layout(&self, ui: &Ui<'_>, size: Size) -> Element<'_, Message> {
        match layout_mode(size.width) {
            LayoutMode::Wide => row![
                container(self.sidebar_view(ui)).width(SIDEBAR_WIDTH),
                self.main_panel(ui, false, size.width - SIDEBAR_WIDTH),
            ]
            .height(Fill)
            .into(),
            LayoutMode::Narrow => {
                if self.sidebar_open {
                    container(self.sidebar_view(ui))
                        .width(Fill)
                        .height(Fill)
                        .into()
                } else {
                    self.main_panel(ui, true, size.width)
                }
            }
        }
    }

    // -- barre latérale ---------------------------------------------------------

    fn sidebar_view(&self, ui: &Ui<'_>) -> Element<'_, Message> {
        let tk = ui.tk;
        let brand = row![
            container(text("F").size(12).font(semibold()).color(tk.on_accent))
                .center(20)
                .width(20)
                .height(20)
                .style(theme::flat(tk.accent, 5.0)),
            text("Flakes").size(14).font(semibold()).color(tk.text),
            space::horizontal(),
            if self.scanning {
                spinner(ui, tk.text2)
            } else {
                space::horizontal().width(0).into()
            },
        ]
        .spacing(8)
        .align_y(iced::Center);

        let search = {
            let key = "search";
            let b = button(
                row![
                    text("Rechercher…").size(13).color(tk.text3),
                    space::horizontal(),
                    kbd(ui, "Ctrl K", false),
                ]
                .align_y(iced::Center),
            )
            .width(Fill)
            .padding([6, 10])
            .style(theme::button_style(tk, Btn::Secondary, ui.hover(key)))
            .on_press(Message::OpenPalette);
            hoverable(key.to_string(), b)
        };

        let mut list = column![].spacing(10);
        let groups = group_entries(&self.entries);
        if groups.is_empty() {
            list = list.push(if self.scanning {
                let mut sk = column![].spacing(10).padding([4, 8]);
                for w in [180.0, 140.0, 160.0, 120.0] {
                    sk = sk.push(skeleton(ui, w, 14.0));
                }
                Element::from(sk)
            } else {
                container(
                    dim(
                        tk,
                        "Aucun dépôt flake trouvé. Ajoutez une racine de scan dans les réglages.",
                    )
                    .size(12),
                )
                .padding([8, 10])
                .into()
            });
        }
        for (role, idx) in groups {
            list = list.push(self.group_view(ui, role, &idx));
        }

        let footer = column![
            nav_item(ui, "nav:all", "Tout vérifier", None, Message::CheckAll),
            nav_item(ui, "nav:rescan", "Rescanner", None, Message::Rescan),
            nav_item(
                ui,
                "nav:settings",
                "Réglages",
                None,
                if self.page == Page::Settings {
                    Message::Noop
                } else {
                    Message::OpenSettings
                }
            ),
        ]
        .spacing(1);

        container(
            column![
                column![brand, search].spacing(12).padding(Padding {
                    top: 14.0,
                    right: 12.0,
                    bottom: 12.0,
                    left: 14.0
                }),
                scrollable(container(list).padding([2, 8]))
                    .height(Fill)
                    .style(theme::scroll_style(tk)),
                hairline(ui),
                container(footer).padding(8),
            ]
            .height(Fill),
        )
        .height(Fill)
        .style(theme::sidebar(tk))
        .into()
    }

    fn group_view(&self, ui: &Ui<'_>, role: Role, idx: &[usize]) -> Element<'_, Message> {
        let tk = ui.tk;
        let gkey = format!("grp:{}", role_title(role));
        let open = ui.fades.level_or(&gkey, ui.now, 1.0);
        let hkey = format!("ghead:{}", role_title(role));
        let head = button(
            row![
                chevron(ui, open),
                section_label(tk, role_title(role)),
                text(idx.len().to_string()).size(10.5).color(tk.text3),
            ]
            .spacing(6)
            .align_y(iced::Center),
        )
        .width(Fill)
        .padding([4, 6])
        .style(theme::row_style(tk, ui.hover(&hkey), 0.0))
        .on_press(Message::ToggleGroup(role));
        let head = hoverable(hkey, head);

        let mut rows = column![].spacing(ROW_GAP);
        for &i in idx {
            rows = rows.push(self.repo_row(ui, &self.entries[i]));
        }
        let full = idx.len() as f32 * (ROW_HEIGHT + ROW_GAP);
        let body: Element<'_, Message> = if open >= 0.999 {
            rows.into()
        } else if open <= 0.001 {
            space::horizontal().height(0).into()
        } else {
            container(rows)
                .height(Length::Fixed(full * open))
                .clip(true)
                .into()
        };
        column![head, body].spacing(2).into()
    }

    fn repo_row(&self, ui: &Ui<'_>, e: &RepoEntry) -> Element<'_, Message> {
        let tk = ui.tk;
        let b = badge(e);
        let key = App::fade_key(&e.path);
        let hover = ui.hover(&format!("row:{key}"));
        let sel = ui.selection(&key);
        let trailing: Element<'_, Message> = match b {
            Badge::Updates(n) => container(
                text(n.to_string())
                    .size(11)
                    .font(medium())
                    .color(tk.tone_text(Tone::Info)),
            )
            .padding([0, 6])
            .style(theme::flat(with_alpha(tk.accent, 0.16), 8.0))
            .into(),
            Badge::Error => text("!").size(12).font(semibold()).color(tk.danger).into(),
            Badge::Dirty => dot(with_alpha(tk.warning, 0.9), 4.0).into(),
            _ => space::horizontal().width(0).into(),
        };
        let btn = button(
            row![
                status_dot(ui, b),
                text(e.name())
                    .size(13)
                    .font(medium())
                    .color(anim::lerp_color(tk.text2, tk.text, hover.max(sel).min(1.0)))
                    .width(Fill)
                    .wrapping(text::Wrapping::None),
                trailing,
            ]
            .spacing(8)
            .height(Fill)
            .align_y(iced::Center),
        )
        .width(Fill)
        .height(ROW_HEIGHT)
        .padding([0, 8])
        .style(theme::row_style(tk, hover, sel))
        .on_press(Message::Select(e.path.clone()));
        hoverable(format!("row:{key}"), btn)
    }

    // -- panneau principal --------------------------------------------------------

    fn main_panel(&self, ui: &Ui<'_>, narrow: bool, width: f32) -> Element<'_, Message> {
        let tk = ui.tk;
        let k = self.panel_t.value(ui.now);
        let content: Element<'_, Message> = match self.page {
            Page::Settings => settings::view(
                ui,
                &self.cfg,
                &self.new_root,
                &self.tray_status,
                &self.token_input,
                self.token_saved,
                &self.token_source,
            ),
            Page::Main => match self.selected_entry() {
                Some(e) => self.repo_detail(ui, e, narrow, width),
                None => self.no_selection(ui, narrow),
            },
        };
        container(
            column![
                container(fade_in(content, tk.panel, k)).height(Fill),
                self.console_view(ui),
            ]
            .height(Fill),
        )
        .width(Fill)
        .height(Fill)
        .style(theme::panel(tk))
        .into()
    }

    fn nav_toggle(&self, ui: &Ui<'_>) -> Element<'_, Message> {
        action(
            ui,
            "btn:repos",
            "Dépôts",
            Btn::Ghost,
            None,
            Some(Message::ToggleSidebar),
        )
    }

    fn no_selection(&self, ui: &Ui<'_>, narrow: bool) -> Element<'_, Message> {
        let inner = if self.scanning {
            let mut sk = column![].spacing(14).padding(32).max_width(560);
            sk = sk.push(skeleton(ui, 220.0, 22.0));
            for _ in 0..3 {
                sk = sk.push(skeleton(ui, Fill, 38.0));
            }
            Element::from(sk)
        } else if self.entries.is_empty() {
            empty_state(
                ui,
                "Aucun dépôt flake",
                "Ajoutez une racine de scan pour que les dépôts contenant un flake.nix apparaissent ici.",
                Some(("cta:settings", "Ouvrir les réglages", Message::OpenSettings)),
            )
        } else {
            empty_state(
                ui,
                "Choisissez un dépôt",
                "Sélectionnez un dépôt dans la liste, ou tapez Ctrl K pour le chercher.",
                Some(("cta:palette", "Rechercher", Message::OpenPalette)),
            )
        };
        let mut col = column![].height(Fill);
        if narrow {
            col = col.push(container(self.nav_toggle(ui)).padding([10, 16]));
        }
        col.push(inner).into()
    }

    fn repo_detail<'a>(
        &'a self,
        ui: &Ui<'_>,
        e: &'a RepoEntry,
        narrow: bool,
        width: f32,
    ) -> Element<'a, Message> {
        let compact = width
            < if e.role == Role::System {
                COMPACT_SYSTEM
            } else {
                COMPACT_OTHER
            };
        let tk = ui.tk;
        let a = model::actions(e);
        let b = badge(e);
        let p = e.path.clone();
        let msg = |on: bool, m: Message| on.then_some(m);

        let git_line = if e.git.is_repo {
            let branch = e.git.branch.as_deref().unwrap_or("HEAD détachée");
            if e.git.dirty.is_empty() {
                format!("git · {branch}")
            } else {
                format!("git · {branch} · {} modifié(s)", e.git.dirty.len())
            }
        } else {
            "hors git".to_string()
        };

        let mut buttons = row![].spacing(8).align_y(iced::Center);
        if let Some(kind) = e.busy {
            buttons = buttons.push(
                row![
                    spinner(ui, tk.accent),
                    dim(tk, format!("{}…", kind.label())).size(12)
                ]
                .spacing(8)
                .align_y(iced::Center),
            );
        }
        buttons = buttons.push(action(
            ui,
            "btn:check",
            "Vérifier",
            Btn::Secondary,
            Some("R"),
            msg(a.check, Message::Check(p.clone())),
        ));
        if e.role == Role::System {
            buttons = buttons.push(action(
                ui,
                "btn:apply",
                "Appliquer",
                Btn::Danger,
                None,
                msg(a.apply, Message::Apply(p.clone())),
            ));
        }
        buttons = buttons.push(action(
            ui,
            "btn:update",
            "Mettre à jour",
            Btn::Primary,
            Some("U"),
            msg(a.update, Message::Update(p.clone())),
        ));

        let mut crumbs = row![].spacing(8).align_y(iced::Center);
        if narrow {
            crumbs = crumbs.push(self.nav_toggle(ui));
        }
        crumbs = crumbs
            .push(title(tk, e.name(), 18.0))
            .push(pill(ui, role_singular(e.role), role_tone(e.role)))
            .push(pill(ui, b.label(), b.tone()));

        let header = column![
            if compact {
                Element::from(column![crumbs.wrap(), buttons.wrap()].spacing(10))
            } else {
                row![crumbs, space::horizontal(), buttons]
                    .spacing(12)
                    .align_y(iced::Center)
                    .into()
            },
            row![
                dim(tk, model::display_path(&e.path, self.home.as_deref())).size(12),
                faint(tk, "·"),
                faint(tk, git_line),
            ]
            .spacing(6),
        ]
        .spacing(6)
        .padding(Padding {
            top: 16.0,
            right: 24.0,
            bottom: 10.0,
            left: 24.0,
        });

        let tab =
            |t: Tab, label: &str, key: &str| -> Element<'a, Message> {
                let active = self.tab == t;
                let hk = format!("tab:{key}");
                let h = ui.hover(&hk);
                let btn = button(text(label.to_string()).size(13).font(medium()))
                    .padding([7, 10])
                    .style(theme::tab_style(tk, active, h))
                    .on_press(Message::SetTab(t));
                // Le soulignement épouse la largeur du bouton (première couche de la pile).
                stack![
                    hoverable(hk, btn),
                    container(container(space::horizontal()).width(Fill).height(2).style(
                        theme::flat(
                            if active {
                                tk.accent
                            } else {
                                iced::Color::TRANSPARENT
                            },
                            1.0
                        )
                    ))
                    .width(Fill)
                    .height(Fill)
                    .align_bottom(Fill),
                ]
                .into()
            };
        let pending_n = e.pending.as_ref().map_or(0, Vec::len);
        let upd_label = if pending_n > 0 {
            format!("Mises à jour · {pending_n}")
        } else {
            "Mises à jour".to_string()
        };
        let tabs = row![
            tab(Tab::Updates, &upd_label, "updates"),
            tab(Tab::Git, "Git", "git"),
            tab(Tab::Repo, "Dépôt", "repo"),
        ]
        .spacing(4)
        .padding([0, 14]);

        let page: Element<'a, Message> = match self.tab {
            Tab::Updates => self.updates_tab(ui, e),
            Tab::Git => self.git_tab(ui, e),
            Tab::Repo => self.repo_tab(ui, e),
        };

        column![
            header,
            tabs,
            column![progress_bar(ui, e.busy.is_some()), hairline(ui)],
            scrollable(
                container(page)
                    .padding(Padding {
                        top: 20.0,
                        right: 24.0,
                        bottom: 24.0,
                        left: 24.0
                    })
                    .max_width(920)
            )
            .height(Fill)
            .style(theme::scroll_style(tk)),
        ]
        .into()
    }

    fn updates_tab<'a>(&'a self, ui: &Ui<'_>, e: &'a RepoEntry) -> Element<'a, Message> {
        let tk = ui.tk;
        let a = model::actions(e);
        let p = e.path.clone();
        let mut col = column![].spacing(16);
        if let Some(err) = &e.error {
            col = col.push(
                container(
                    column![
                        text("Erreur").size(13).font(semibold()),
                        text(err.clone()).size(12).font(widgets::MONO),
                    ]
                    .spacing(6),
                )
                .padding(14)
                .width(Fill)
                .style(theme::error_banner(tk)),
            );
        }
        let (heading, changes, tone): (String, &[_], Tone) = match (&e.pending, &e.last_update) {
            (Some(pn), _) if !pn.is_empty() => (
                match pn.len() {
                    1 => "1 mise à jour disponible".to_string(),
                    n => format!("{n} mises à jour disponibles"),
                },
                pn.as_slice(),
                Tone::Info,
            ),
            (Some(_), Some(l)) if !l.is_empty() => (
                format!("Dernière mise à jour · {} input(s)", l.len()),
                l.as_slice(),
                Tone::Success,
            ),
            _ => (String::new(), &[], Tone::Neutral),
        };
        if !changes.is_empty() {
            let now = unix_now();
            col = col.push(
                row![dot(tk.tone(tone), 7.0), title(tk, heading, 14.0),]
                    .spacing(8)
                    .align_y(iced::Center),
            );
            col = col.push(changes_table(ui, changes, now));
        } else if e.busy == Some(model::JobKind::Check) && e.pending.is_none() {
            // Vérification en cours : squelettes à la place du tableau.
            let mut sk = column![skeleton(ui, 200.0, 16.0)].spacing(12);
            for _ in 0..3 {
                sk = sk.push(skeleton(ui, Fill, 34.0));
            }
            col = col.push(sk);
        } else if e.error.is_none() {
            col = col.push(match (&e.pending, e.role) {
                (Some(_), _) => empty_state(
                    ui,
                    "Tout est à jour",
                    "Tous les inputs de ce dépôt sont à leur dernière révision.",
                    a.check.then(|| {
                        (
                            "cta:recheck",
                            "Vérifier à nouveau",
                            Message::Check(p.clone()),
                        )
                    }),
                ),
                (None, Role::Detected) => empty_state(
                    ui,
                    "Dépôt détecté",
                    "Suivez ce dépôt pour le vérifier et le mettre à jour.",
                    Some((
                        "cta:track",
                        "Suivre ce dépôt",
                        Message::Role(p.clone(), RoleAction::Track),
                    )),
                ),
                (None, Role::Ignored) => empty_state(
                    ui,
                    "Dépôt ignoré",
                    "Ce dépôt est ignoré : il n'est ni vérifié ni mis à jour.",
                    Some((
                        "cta:unignore",
                        "Ne plus ignorer",
                        Message::Role(p.clone(), RoleAction::Unignore),
                    )),
                ),
                (None, _) => empty_state(
                    ui,
                    "Pas encore vérifié",
                    "Vérifiez ce dépôt pour savoir si des mises à jour sont disponibles.",
                    a.check.then(|| {
                        (
                            "cta:check",
                            "Vérifier maintenant",
                            Message::Check(p.clone()),
                        )
                    }),
                ),
            });
        }
        if let Some(inputs) = self.inputs_section(ui, e) {
            col = col.push(inputs);
        }
        col.into()
    }

    /// Liste des inputs racine : cases à cocher, mise à jour individuelle ou de la sélection.
    fn inputs_section<'a>(&'a self, ui: &Ui<'_>, e: &'a RepoEntry) -> Option<Element<'a, Message>> {
        let list = self.inputs.get(&e.path).filter(|l| !l.is_empty())?;
        if !matches!(e.role, Role::System | Role::Tracked) {
            return None;
        }
        let tk = ui.tk;
        let a = model::actions(e);
        let p = &e.path;
        let pending = e.pending.as_deref().unwrap_or_default();
        let empty = std::collections::BTreeSet::new();
        let sel = self.input_sel.get(p).unwrap_or(&empty);
        let has_updates = list.iter().any(|i| model::input_has_update(i, pending));

        let mut head = row![title(tk, "Inputs", 14.0), faint(tk, list.len().to_string())]
            .spacing(8)
            .align_y(iced::Center);
        head = head.push(space::horizontal());
        if has_updates {
            head = head.push(action(
                ui,
                "inputs:updatable",
                "Avec mise à jour",
                Btn::Ghost,
                None,
                Some(Message::SelectInputs(p.clone(), InputSelect::Updatable)),
            ));
        }
        head = head
            .push(action(
                ui,
                "inputs:all",
                "Tout",
                Btn::Ghost,
                None,
                Some(Message::SelectInputs(p.clone(), InputSelect::All)),
            ))
            .push(action(
                ui,
                "inputs:none",
                "Aucun",
                Btn::Ghost,
                None,
                (!sel.is_empty())
                    .then(|| Message::SelectInputs(p.clone(), InputSelect::NoneSelected)),
            ));

        let mut rows = column![];
        for (n, input) in list.iter().enumerate() {
            let checked = sel.contains(&input.name);
            let (pp, nm) = (p.clone(), input.name.clone());
            let boxed = iced::widget::checkbox(checked)
                .on_toggle(move |_| Message::ToggleInput(pp.clone(), nm.clone()))
                .size(16);
            let mut info = row![text(input.name.clone())
                .size(13)
                .font(medium())
                .color(tk.text)]
            .spacing(8)
            .align_y(iced::Center);
            if model::input_has_update(input, pending) {
                info = info.push(pill(ui, "mise à jour", Tone::Info));
            }
            let mut revs = row![rev_chip(ui, short_rev(&input.short_rev), false)]
                .spacing(6)
                .align_y(iced::Center);
            if let Some(c) = model::input_change(input, pending) {
                revs = revs.push(text("→").size(12).color(tk.text3)).push(rev_chip(
                    ui,
                    short_rev(&c.new_rev),
                    true,
                ));
            }
            let url = input.url.clone().unwrap_or_else(|| "—".to_string());
            let line = row![
                boxed,
                column![info, faint(tk, url).size(11)]
                    .spacing(2)
                    .width(Fill),
                revs,
                action(
                    ui,
                    &format!("input:upd:{n}"),
                    "Mettre à jour",
                    Btn::Secondary,
                    None,
                    a.update
                        .then(|| Message::UpdateInputs(p.clone(), vec![input.name.clone()])),
                ),
            ]
            .spacing(12)
            .padding([8, 16])
            .align_y(iced::Center);
            rows = rows.push(line).push(hairline(ui));
        }
        let table = container(rows)
            .width(Fill)
            .clip(true)
            .style(theme::outlined(tk));

        let footer = row![
            faint(
                tk,
                if sel.is_empty() {
                    "Cochez des inputs pour les mettre à jour sans toucher aux autres.".to_string()
                } else {
                    format!("{} input(s) sélectionné(s)", sel.len())
                }
            ),
            space::horizontal(),
            action(
                ui,
                "inputs:update-sel",
                &model::selection_label(sel.len()),
                Btn::Primary,
                None,
                (a.update && !sel.is_empty()).then(|| Message::UpdateSelection(p.clone())),
            ),
        ]
        .spacing(12)
        .align_y(iced::Center);

        Some(column![head, table, footer].spacing(10).into())
    }

    fn git_tab<'a>(&'a self, ui: &Ui<'_>, e: &'a RepoEntry) -> Element<'a, Message> {
        let tk = ui.tk;
        if !e.git.is_repo {
            return empty_state(
                ui,
                "Hors git",
                "Ce dépôt n'est pas versionné avec git : commit et push sont indisponibles.",
                None,
            );
        }
        let a = model::actions(e);
        let line = |label: &str, value: Element<'a, Message>| -> Element<'a, Message> {
            row![container(dim(tk, label.to_string())).width(150), value,]
                .align_y(iced::Center)
                .into()
        };
        let branch = e
            .git
            .branch
            .as_deref()
            .unwrap_or("HEAD détachée")
            .to_string();
        let state: Element<'a, Message> = if e.git.dirty.is_empty() {
            pill(ui, "Propre", Tone::Success)
        } else {
            pill(
                ui,
                format!("{} fichier(s) modifié(s)", e.git.dirty.len()),
                Tone::Warning,
            )
        };
        let lock: Element<'a, Message> = if e.git.lock_dirty {
            pill(ui, "flake.lock modifié", Tone::Warning)
        } else {
            body(tk, "Intact").into()
        };
        let mut col = column![container(
            column![
                line("Branche", rev_chip(ui, branch, true)),
                hairline(ui),
                line("Arbre de travail", state),
                hairline(ui),
                line("flake.lock", lock),
            ]
            .spacing(10)
        )
        .padding([12, 16])
        .width(Fill)
        .style(theme::outlined(tk))]
        .spacing(16);
        if !e.git.dirty.is_empty() {
            let mut files = column![].spacing(4);
            for f in e.git.dirty.iter().take(14) {
                files = files.push(
                    row![dot(tk.warning, 5.0), mono(tk, f.clone())]
                        .spacing(10)
                        .align_y(iced::Center),
                );
            }
            if e.git.dirty.len() > 14 {
                files = files.push(faint(
                    tk,
                    format!("… et {} autre(s)", e.git.dirty.len() - 14),
                ));
            }
            col = col.push(
                column![
                    section_label(tk, "Modifications non commitées"),
                    container(files)
                        .padding([12, 16])
                        .width(Fill)
                        .style(theme::outlined(tk)),
                ]
                .spacing(8),
            );
        }
        col = col.push(row![action(
            ui,
            "btn:push",
            "Push",
            Btn::Secondary,
            None,
            a.push.then(|| Message::Push(e.path.clone())),
        )]);
        col.into()
    }

    fn repo_tab<'a>(&'a self, ui: &Ui<'_>, e: &'a RepoEntry) -> Element<'a, Message> {
        let tk = ui.tk;
        let a = model::actions(e);
        let p = e.path.clone();
        let blurb = match e.role {
            Role::System => "Dépôt suivi qui décrit la configuration NixOS de la machine. C'est le seul dont la mise à jour peut être appliquée au système.",
            Role::Tracked => "Dépôt que vous avez choisi de maintenir : il est vérifié et mis à jour.",
            Role::Detected => "Trouvé par le scan des racines, mais pas encore choisi : il n'est pas mis à jour.",
            Role::Ignored => "Ignoré : ce dépôt n'est ni vérifié ni mis à jour.",
        };
        let mut roles = row![].spacing(8);
        for (key, label, on, act) in [
            ("role:track", "Suivre", a.track, RoleAction::Track),
            (
                "role:untrack",
                "Ne plus suivre",
                a.untrack,
                RoleAction::Untrack,
            ),
            (
                "role:system",
                "Définir comme dépôt système",
                a.set_system,
                RoleAction::SetSystem,
            ),
            ("role:ignore", "Ignorer", a.ignore, RoleAction::Ignore),
            (
                "role:unignore",
                "Ne plus ignorer",
                a.unignore,
                RoleAction::Unignore,
            ),
        ] {
            if on {
                roles = roles.push(action(
                    ui,
                    key,
                    label,
                    Btn::Secondary,
                    None,
                    Some(Message::Role(p.clone(), act)),
                ));
            }
        }
        column![
            container(
                column![
                    row![
                        container(dim(tk, "Rôle")).width(150),
                        pill(ui, role_singular(e.role), role_tone(e.role)),
                    ]
                    .align_y(iced::Center),
                    hairline(ui),
                    row![
                        container(dim(tk, "Chemin")).width(150),
                        mono(tk, e.path.display().to_string()),
                    ]
                    .align_y(iced::Center),
                ]
                .spacing(10)
            )
            .padding([12, 16])
            .width(Fill)
            .style(theme::outlined(tk)),
            dim(tk, blurb).size(12),
            roles.wrap(),
        ]
        .spacing(16)
        .into()
    }

    // -- console ---------------------------------------------------------------------

    fn console_view(&self, ui: &Ui<'_>) -> Element<'_, Message> {
        let tk = ui.tk;
        let k = self.console_t.value(ui.now);
        let toggle_label = if self.console_open {
            "Réduire"
        } else {
            "Afficher"
        };
        let head_btn = button(
            row![
                chevron(ui, k),
                text("Console").size(12).font(semibold()).color(tk.text2),
                text(format!("{} ligne(s)", self.log.len()))
                    .size(11)
                    .color(tk.text3),
            ]
            .spacing(8)
            .align_y(iced::Center),
        )
        .padding([4, 8])
        .style(theme::row_style(tk, ui.hover("console:toggle"), 0.0))
        .on_press(Message::ToggleConsole);
        let header = row![
            tip(
                ui,
                hoverable("console:toggle".to_string(), head_btn),
                &format!("{toggle_label} la console"),
                tooltip::Position::Top,
            ),
            space::horizontal(),
            action(
                ui,
                "console:clear",
                "Effacer",
                Btn::Ghost,
                None,
                Some(Message::ClearLog)
            ),
        ]
        .align_y(iced::Center)
        .padding([2, 8]);

        let mut col = column![hairline(ui), header];
        if k > 0.002 {
            let lines = self.log.iter().fold(column![].spacing(2), |c, l| {
                let color = match l.kind {
                    LogKind::Info => tk.text2,
                    LogKind::Command => tk.tone_text(Tone::Info),
                    LogKind::Error => tk.danger,
                    LogKind::Success => tk.success,
                };
                c.push(
                    text(l.text.clone())
                        .size(12)
                        .font(widgets::MONO)
                        .color(color),
                )
            });
            col = col.push(
                container(
                    scrollable(container(lines).padding([4, 16]).width(Fill))
                        .anchor_bottom()
                        .height(CONSOLE_HEIGHT)
                        .width(Fill)
                        .style(theme::scroll_style(tk)),
                )
                .height(Length::Fixed(CONSOLE_HEIGHT * k))
                .clip(true),
            );
        }
        container(col).width(Fill).style(theme::console(tk)).into()
    }

    // -- dialogue ---------------------------------------------------------------------

    fn modal_layer<'a>(&'a self, ui: &Ui<'_>) -> Option<Element<'a, Message>> {
        let k = self.modal_t.value(ui.now);
        let m = self.modal.as_ref().or(self.modal_leaving.as_ref())?;
        if k <= 0.002 {
            return None;
        }
        let tk = ui.tk;
        let card = fade_in(self.modal_card(ui, m), tk.raised, k);
        let offset = (1.0 - k) * 28.0;
        Some(opaque(
            mouse_area(
                container(
                    container(card)
                        .padding(22)
                        .max_width(620)
                        .style(theme::floating(tk)),
                )
                .center(Fill)
                .padding(Padding {
                    top: 16.0 + offset,
                    right: 16.0,
                    bottom: 16.0,
                    left: 16.0,
                })
                .style(theme::scrim(tk, k)),
            )
            .on_press(Message::Noop),
        ))
    }

    fn modal_card<'a>(&'a self, ui: &Ui<'_>, m: &'a Modal) -> Element<'a, Message> {
        let tk = ui.tk;
        let editor = text_editor(&self.editor)
            .on_action(Message::EditorAction)
            .height(Length::Fixed(110.0))
            .padding(10)
            .size(13)
            .style(theme::editor_style(tk));
        let name = model::short_name(m.repo());
        match m {
            Modal::CommitDirty { dirty, .. } => {
                let mut files = column![].spacing(3);
                for f in dirty.iter().take(6) {
                    files = files.push(
                        row![dot(tk.warning, 5.0), mono(tk, f.clone())]
                            .spacing(8)
                            .align_y(iced::Center),
                    );
                }
                if dirty.len() > 6 {
                    files = files.push(faint(tk, format!("… et {} autre(s)", dirty.len() - 6)));
                }
                column![
                    title(tk, "Modifications non commitées", 16.0),
                    dim(
                        tk,
                        format!(
                            "Le dépôt « {name} » contient {} fichier(s) modifié(s). \
                             Commitez-les avant de mettre à jour les verrous, pour garder un historique propre.",
                            dirty.len()
                        )
                    ),
                    container(files)
                        .padding([10, 12])
                        .width(Fill)
                        .style(theme::outlined(tk)),
                    section_label(tk, "Message du commit"),
                    editor,
                    row![
                        action(ui, "m:cancel", "Annuler", Btn::Ghost, Some("Échap"), Some(Message::ModalCancel)),
                        space::horizontal(),
                        action(
                            ui,
                            "m:continue",
                            "Continuer quand même",
                            Btn::Secondary,
                            None,
                            Some(Message::ModalContinue)
                        ),
                        action(
                            ui,
                            "m:commit",
                            "Committer puis mettre à jour",
                            Btn::Primary,
                            None,
                            Some(Message::ModalCommit)
                        ),
                    ]
                    .spacing(8),
                ]
                .spacing(12)
                .into()
            }
            Modal::CommitLock { .. } => column![
                title(tk, "Committer flake.lock ?", 16.0),
                dim(
                    tk,
                    format!(
                        "La mise à jour de « {name} » a modifié flake.lock. \
                         Seul ce fichier sera committé ; aucun push automatique."
                    )
                ),
                section_label(tk, "Message du commit"),
                editor,
                row![
                    space::horizontal(),
                    action(
                        ui,
                        "m:later",
                        "Plus tard",
                        Btn::Ghost,
                        Some("Échap"),
                        Some(Message::ModalCancel)
                    ),
                    action(
                        ui,
                        "m:commitlock",
                        "Committer flake.lock",
                        Btn::Primary,
                        None,
                        Some(Message::ModalCommit)
                    ),
                ]
                .spacing(8),
            ]
            .spacing(12)
            .into(),
        }
    }

    // -- palette ------------------------------------------------------------------------

    fn palette_layer<'a>(&'a self, ui: &Ui<'_>) -> Option<Element<'a, Message>> {
        let k = self.palette_t.value(ui.now);
        if k <= 0.002 && !self.palette_open {
            return None;
        }
        let tk = ui.tk;
        let results = self.palette_results();

        let input = text_input(
            "Aller à un dépôt ou lancer une action…",
            &self.palette.query,
        )
        .id(crate::app::PALETTE_INPUT)
        .on_input(Message::PaletteInput)
        .on_submit(Message::PaletteRun(self.palette.selected))
        .size(15)
        .padding([14, 16])
        .style(theme::bare_input_style(tk));

        let mut list = column![].spacing(1).padding([6, 6]);
        if results.is_empty() {
            list = list.push(
                container(dim(
                    tk,
                    format!("Aucun résultat pour « {} »", self.palette.query.trim()),
                ))
                .padding([14, 12]),
            );
        }
        let mut seen_repo = false;
        for (i, it) in results.iter().enumerate() {
            if self.palette.query.trim().is_empty() {
                if i == 0 {
                    list = list.push(palette_heading(tk, "Actions"));
                }
                if it.is_repo && !seen_repo {
                    seen_repo = true;
                    list = list.push(palette_heading(tk, "Dépôts"));
                }
            }
            list = list.push(self.palette_row(ui, i, it));
        }

        let footer = row![
            kbd(ui, "↑↓", false),
            faint(tk, "naviguer"),
            kbd(ui, "↵", false),
            faint(tk, "ouvrir"),
            kbd(ui, "Échap", false),
            faint(tk, "fermer"),
        ]
        .spacing(6)
        .align_y(iced::Center)
        .padding([8, 14]);

        let card = fade_in(
            column![input, hairline(ui), list, hairline(ui), footer],
            tk.raised,
            k,
        );
        let offset = (1.0 - k) * 14.0;
        Some(opaque(
            mouse_area(
                container(
                    mouse_area(
                        container(card)
                            .max_width(580)
                            .width(Fill)
                            .style(theme::floating(tk)),
                    )
                    .on_press(Message::Noop),
                )
                .align_top(Fill)
                .center_x(Fill)
                .padding(Padding {
                    top: 84.0 + offset,
                    right: 16.0,
                    bottom: 16.0,
                    left: 16.0,
                })
                .style(theme::scrim(tk, k * 0.8)),
            )
            .on_press(Message::ClosePalette),
        ))
    }

    fn palette_row<'a>(&'a self, ui: &Ui<'_>, i: usize, it: &Item) -> Element<'a, Message> {
        let tk = ui.tk;
        let hk = format!("pal:{i}");
        let hover = ui.hover(&hk);
        let sel = ui.selection(&format!("pal:{i}"));
        let mut r = row![
            text(it.label.clone())
                .size(13)
                .font(medium())
                .color(anim::lerp_color(tk.text2, tk.text, hover.max(sel).min(1.0))),
            text(it.detail.clone()).size(12).color(tk.text3),
            space::horizontal(),
        ]
        .spacing(10)
        .height(Fill)
        .align_y(iced::Center);
        if let Some(s) = it.kbd {
            r = r.push(kbd(ui, s, false));
        }
        let b = button(r)
            .width(Fill)
            .height(34)
            .padding([0, 10])
            .style(theme::row_style(tk, hover, sel))
            .on_press(Message::PaletteRun(i));
        hoverable(hk, b)
    }
}

fn palette_heading<'a>(tk: theme::Tokens, label: &str) -> Element<'a, Message> {
    container(section_label(tk, label))
        .padding(Padding {
            top: 6.0,
            right: 10.0,
            bottom: 4.0,
            left: 10.0,
        })
        .into()
}

fn role_tone(role: Role) -> Tone {
    match role {
        Role::System => Tone::Warning,
        Role::Tracked => Tone::Info,
        Role::Detected => Tone::Neutral,
        Role::Ignored => Tone::Neutral,
    }
}

/// Tableau des changements : input, révisions (ancienne → nouvelle), dates relatives.
fn changes_table<'a>(
    ui: &Ui<'_>,
    changes: &[nixos_flakes_core::lock::InputChange],
    now: i64,
) -> Element<'a, Message> {
    let tk = ui.tk;
    let cols = [4u16, 6, 3];
    let cell = |el: Element<'a, Message>, w: u16, right: bool| -> Element<'a, Message> {
        let c = container(el).width(Length::FillPortion(w));
        if right {
            c.align_right(Length::FillPortion(w)).into()
        } else {
            c.into()
        }
    };
    let head = row![
        cell(section_label(tk, "Input").into(), cols[0], false),
        cell(section_label(tk, "Révision").into(), cols[1], false),
        cell(section_label(tk, "Publiée").into(), cols[2], true),
    ]
    .spacing(12)
    .padding([8, 16]);
    let mut col = column![head, hairline(ui)];
    for c in changes {
        let age = c
            .new_date
            .map_or_else(|| "—".to_string(), |d| format_relative(d, now));
        let date_el = tip(
            ui,
            dim(tk, age),
            &format!(
                "{} → {}",
                format_opt_date(c.old_date),
                format_opt_date(c.new_date)
            ),
            tooltip::Position::Left,
        );
        let revs = row![
            tip(
                ui,
                rev_chip(ui, short_rev(&c.old_rev), false),
                c.old_rev.as_deref().unwrap_or("—"),
                tooltip::Position::Top
            ),
            text("→").size(12).color(tk.text3),
            tip(
                ui,
                rev_chip(ui, short_rev(&c.new_rev), true),
                c.new_rev.as_deref().unwrap_or("—"),
                tooltip::Position::Top
            ),
        ]
        .spacing(8)
        .align_y(iced::Center);
        col = col.push(
            row![
                cell(
                    text(c.name.clone())
                        .size(13)
                        .font(medium())
                        .color(tk.text)
                        .into(),
                    cols[0],
                    false
                ),
                cell(revs.into(), cols[1], false),
                cell(date_el, cols[2], true),
            ]
            .spacing(12)
            .padding([9, 16])
            .align_y(iced::Center),
        );
        col = col.push(hairline(ui));
    }
    container(col)
        .width(Fill)
        .clip(true)
        .style(theme::outlined(tk))
        .into()
}

/// Pile de toasts en bas à droite, avec glissement et fondu.
fn toast_layer<'a>(ui: &Ui<'_>, toasts: &Toasts) -> Element<'a, Message> {
    let tk = ui.tk;
    let mut col = column![].spacing(8).align_x(iced::Right);
    for t in toasts.items() {
        let p = t.presence(ui.now);
        let color = match t.kind {
            ToastKind::Success => tk.success,
            ToastKind::Error => tk.danger,
            ToastKind::Info => tk.accent,
        };
        let mut texts = column![text(t.title.clone()).size(13).font(medium()).color(tk.text)]
            .spacing(2)
            .width(Fill);
        if let Some(b) = &t.body {
            texts = texts.push(text(b.clone()).size(12).color(tk.text2));
        }
        let id = t.id;
        let close = button(text("×").size(15).color(tk.text3))
            .padding([0, 6])
            .style(theme::tab_style(
                tk,
                false,
                ui.hover(&format!("toast:{id}")),
            ))
            .on_press(Message::DismissToast(id));
        let card = container(
            row![
                container(dot(color, 7.0)).padding([5, 0]),
                texts,
                hoverable(format!("toast:{id}"), close),
            ]
            .spacing(10)
            .align_y(iced::alignment::Vertical::Top),
        )
        .width(340)
        .padding(Padding {
            top: 10.0,
            right: 10.0,
            bottom: 10.0,
            left: 14.0,
        })
        .style(theme::floating(tk));
        let slide = (1.0 - p) * 56.0;
        let shown = fade_in(card, tk.raised, p.min(1.0));
        col = col.push(
            container(row![space::horizontal().width(slide), shown])
                .width(340)
                .clip(true),
        );
    }
    container(col)
        .align_right(Fill)
        .align_bottom(Fill)
        .padding(16)
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
