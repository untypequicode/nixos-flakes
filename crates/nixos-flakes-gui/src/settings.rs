//! Page de réglages : logique pure (édition de la config) et vue.

use crate::theme::Btn;
use crate::widgets::{self, bold, muted, SPACE};
use iced::widget::{button, column, container, row, scrollable, space, text, text_input, toggler};
use iced::{Element, Length};
use nixos_flakes_core::config::Config;
use std::path::{Path, PathBuf};

pub const INTERVALS: [u32; 9] = [5, 15, 30, 60, 120, 240, 480, 720, 1440];
pub const MIN_DEPTH: u8 = 1;
pub const MAX_DEPTH: u8 = 10;

#[derive(Debug, Clone)]
pub enum Msg {
    InputChanged(String),
    AddRoot,
    RemoveRoot(usize),
    Depth(bool),
    Interval(bool),
    ToggleTray(bool),
    Rescan,
    Back,
}

/// Profondeur de scan modifiée d'un cran, bornée.
pub fn step_depth(cur: u8, up: bool) -> u8 {
    if up {
        cur.saturating_add(1).min(MAX_DEPTH)
    } else {
        cur.saturating_sub(1).max(MIN_DEPTH)
    }
}

/// Intervalle suivant/précédent parmi les valeurs prédéfinies.
pub fn step_interval(cur: u32, up: bool) -> u32 {
    if up {
        INTERVALS
            .iter()
            .copied()
            .find(|&i| i > cur)
            .unwrap_or(*INTERVALS.last().unwrap())
    } else {
        INTERVALS
            .iter()
            .rev()
            .copied()
            .find(|&i| i < cur)
            .unwrap_or(INTERVALS[0])
    }
}

/// Chemin saisi : espaces retirés, `~` développé ; `None` si vide.
pub fn parse_root(input: &str, home: Option<&Path>) -> Option<PathBuf> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if s == "~" {
        return home.map(Path::to_path_buf);
    }
    if let Some(rest) = s.strip_prefix("~/") {
        return home.map(|h| h.join(rest));
    }
    Some(PathBuf::from(s))
}

/// Ajoute une racine de scan.
pub fn add_root(cfg: &mut Config, input: &str, home: Option<&Path>) -> Result<(), String> {
    let p = parse_root(input, home).ok_or_else(|| "Chemin vide".to_string())?;
    if !p.is_absolute() {
        return Err(format!("Le chemin doit être absolu : {}", p.display()));
    }
    if cfg.scan_roots.contains(&p) {
        return Err("Cette racine de scan existe déjà".to_string());
    }
    cfg.scan_roots.push(p);
    Ok(())
}

/// Retire la racine d'indice `idx` (sans effet si hors bornes).
pub fn remove_root(cfg: &mut Config, idx: usize) -> bool {
    if idx < cfg.scan_roots.len() {
        cfg.scan_roots.remove(idx);
        true
    } else {
        false
    }
}

pub fn interval_label(minutes: u32) -> String {
    if minutes >= 60 && minutes.is_multiple_of(60) {
        format!("{} h", minutes / 60)
    } else {
        format!("{minutes} min")
    }
}

fn stepper<'a>(value: String, on: fn(bool) -> Msg) -> Element<'a, Msg> {
    row![
        widgets::action("−", Btn::Secondary, Some(on(false))),
        container(text(value).font(bold())).padding([0, 10]),
        widgets::action("+", Btn::Secondary, Some(on(true))),
    ]
    .spacing(6)
    .align_y(iced::Center)
    .into()
}

fn section<'a>(
    title: &'a str,
    hint: &'a str,
    body: impl Into<Element<'a, Msg>>,
) -> Element<'a, Msg> {
    container(
        column![
            text(title).size(16).font(bold()),
            muted(hint).size(12),
            body.into()
        ]
        .spacing(8),
    )
    .padding(16)
    .width(Length::Fill)
    .style(crate::theme::card)
    .into()
}

/// Vue de la page de réglages.
pub fn view<'a>(cfg: &'a Config, input: &'a str, tray_active: bool) -> Element<'a, Msg> {
    let roots = cfg
        .scan_roots
        .iter()
        .enumerate()
        .fold(column![].spacing(6), |col, (i, r)| {
            col.push(
                row![
                    text(r.display().to_string()).size(13).width(Length::Fill),
                    button(text("Retirer").size(12))
                        .padding([4, 10])
                        .style(crate::theme::button_style(Btn::Danger))
                        .on_press(Msg::RemoveRoot(i)),
                ]
                .align_y(iced::Center),
            )
        });
    let add = row![
        text_input("/chemin/absolu ou ~/dossier", input)
            .on_input(Msg::InputChanged)
            .on_submit(Msg::AddRoot)
            .style(crate::theme::input_style)
            .padding(8),
        widgets::action("Ajouter", Btn::Primary, Some(Msg::AddRoot)),
    ]
    .spacing(8);

    let tray_hint = if cfg.tray_enabled && !tray_active {
        "Zone de notification demandée mais indisponible sur cette session."
    } else {
        "Affiche une icône ; fermer la fenêtre la réduit dans la zone de notification."
    };

    let content = column![
        row![
            widgets::action("← Retour", Btn::Ghost, Some(Msg::Back)),
            text("Réglages").size(24).font(bold()),
        ]
        .spacing(SPACE)
        .align_y(iced::Center),
        section(
            "Racines de scan",
            "Dossiers à partir desquels on cherche des dépôts flake.",
            column![
                roots,
                add,
                widgets::action("Rescanner maintenant", Btn::Secondary, Some(Msg::Rescan))
            ]
            .spacing(10),
        ),
        section(
            "Profondeur de scan",
            "Nombre maximal de niveaux de sous-dossiers explorés.",
            stepper(cfg.scan_depth.to_string(), Msg::Depth),
        ),
        section(
            "Zone de notification",
            tray_hint,
            toggler(cfg.tray_enabled)
                .label("Activer l'icône de notification")
                .on_toggle(Msg::ToggleTray),
        ),
        section(
            "Intervalle de vérification",
            "Fréquence de la vérification automatique (zone de notification active).",
            stepper(interval_label(cfg.check_interval_minutes), |up| {
                Msg::Interval(up)
            }),
        ),
        row![
            space::horizontal(),
            muted("Les réglages sont enregistrés automatiquement.").size(12)
        ],
    ]
    .spacing(SPACE)
    .padding(20)
    .max_width(760);

    scrollable(container(content).center_x(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Config {
        Config {
            scan_roots: vec![],
            system_repo: None,
            ..Config::default()
        }
    }

    #[test]
    fn depth_is_clamped() {
        assert_eq!(step_depth(4, true), 5);
        assert_eq!(step_depth(MAX_DEPTH, true), MAX_DEPTH);
        assert_eq!(step_depth(1, false), 1);
        assert_eq!(step_depth(4, false), 3);
    }

    #[test]
    fn interval_steps() {
        assert_eq!(step_interval(60, true), 120);
        assert_eq!(step_interval(60, false), 30);
        assert_eq!(step_interval(1440, true), 1440);
        assert_eq!(step_interval(5, false), 5);
        assert_eq!(step_interval(45, true), 60);
        assert_eq!(step_interval(45, false), 30);
    }

    #[test]
    fn interval_labels() {
        assert_eq!(interval_label(30), "30 min");
        assert_eq!(interval_label(120), "2 h");
        assert_eq!(interval_label(90), "90 min");
    }

    #[test]
    fn root_parsing() {
        let home = Path::new("/home/u");
        assert_eq!(parse_root("  ", Some(home)), None);
        assert_eq!(parse_root("~", Some(home)), Some(PathBuf::from("/home/u")));
        assert_eq!(
            parse_root("~/dev", Some(home)),
            Some(PathBuf::from("/home/u/dev"))
        );
        assert_eq!(parse_root("~/dev", None), None);
        assert_eq!(parse_root(" /etc ", None), Some(PathBuf::from("/etc")));
    }

    #[test]
    fn add_and_remove_roots() {
        let mut c = empty();
        assert!(add_root(&mut c, "/a", None).is_ok());
        assert!(add_root(&mut c, "/a", None).is_err());
        assert!(add_root(&mut c, "", None).is_err());
        assert!(add_root(&mut c, "relatif", None).is_err());
        assert!(add_root(&mut c, "~/x", Some(Path::new("/h"))).is_ok());
        assert_eq!(c.scan_roots, [PathBuf::from("/a"), PathBuf::from("/h/x")]);
        assert!(remove_root(&mut c, 0));
        assert!(!remove_root(&mut c, 5));
        assert_eq!(c.scan_roots, [PathBuf::from("/h/x")]);
    }
}
