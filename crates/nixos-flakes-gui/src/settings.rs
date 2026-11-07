//! Page de réglages : logique pure (édition de la config) et vue.

use crate::app::Message;
use crate::theme::{self, Btn};
use crate::tray::TrayStatus;
use crate::widgets::{self, action, dim, faint, title, Ui};
use iced::widget::{column, container, row, scrollable, space, text, text_input, toggler};
use iced::{Element, Fill, Length};
use nixos_flakes_core::config::Config;
use nixos_flakes_core::token::Source;
use std::path::{Path, PathBuf};
pub const INTERVALS: [u32; 9] = [5, 15, 30, 60, 120, 240, 480, 720, 1440];
pub const MIN_DEPTH: u8 = 1;
pub const MAX_DEPTH: u8 = 10;

/// Saisie secrète : `Debug` n'en révèle jamais le contenu.
#[derive(Clone)]
pub struct Secret(pub String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<masqué>)")
    }
}

/// Libellé d'état du jeton GitHub.
pub fn token_status_text(saved: bool) -> &'static str {
    if saved {
        "Jeton enregistré"
    } else {
        "Aucun jeton"
    }
}

/// Libellé d'état selon l'origine du jeton (`saved` : le jeton est lisible).
pub fn token_source_text(source: &Source, saved: bool) -> &'static str {
    match source {
        Source::Fichier(_) if saved => "Jeton configuré par la configuration Nix",
        Source::Fichier(_) => "Jeton configuré par la configuration Nix (fichier absent ou vide)",
        _ => token_status_text(saved),
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    InputChanged(String),
    AddRoot,
    RemoveRoot(usize),
    Depth(bool),
    Interval(bool),
    ToggleTray(bool),
    TokenInput(Secret),
    TokenSave,
    TokenClear,
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

fn stepper<'a>(ui: &Ui<'_>, key: &str, value: String, on: fn(bool) -> Msg) -> Element<'a, Message> {
    let tk = ui.tk;
    row![
        action(
            ui,
            &format!("{key}:minus"),
            "−",
            Btn::Secondary,
            None,
            Some(Message::Settings(on(false)))
        ),
        container(
            text(value)
                .size(13)
                .font(widgets::semibold())
                .color(tk.text)
        )
        .padding([0, 10])
        .center_x(Length::Shrink),
        action(
            ui,
            &format!("{key}:plus"),
            "+",
            Btn::Secondary,
            None,
            Some(Message::Settings(on(true)))
        ),
    ]
    .spacing(6)
    .align_y(iced::Center)
    .into()
}

/// Ligne de réglage : intitulé et aide à gauche, contrôle à droite.
fn setting<'a>(
    ui: &Ui<'_>,
    heading: &str,
    hint: &str,
    control: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let tk = ui.tk;
    container(
        row![
            column![
                text(heading.to_string())
                    .size(13)
                    .font(widgets::medium())
                    .color(tk.text),
                faint(tk, hint.to_string()),
            ]
            .spacing(3)
            .width(Fill),
            control.into(),
        ]
        .spacing(16)
        .align_y(iced::Center),
    )
    .padding([14, 16])
    .width(Fill)
    .into()
}

fn rule<'a>(ui: &Ui<'_>) -> Element<'a, Message> {
    container(space::horizontal())
        .width(Fill)
        .height(1)
        .style(theme::flat(ui.tk.border, 0.0))
        .into()
}

/// Vue de la page de réglages.
pub fn view<'a>(
    ui: &Ui<'_>,
    cfg: &'a Config,
    input: &'a str,
    tray: &'a TrayStatus,
    token_input: &'a str,
    token_saved: bool,
    token_source: &'a Source,
) -> Element<'a, Message> {
    let tk = ui.tk;
    let roots = cfg
        .scan_roots
        .iter()
        .enumerate()
        .fold(column![].spacing(2), |col, (i, r)| {
            col.push(
                row![
                    text(r.display().to_string())
                        .size(12)
                        .font(widgets::MONO)
                        .color(tk.text)
                        .width(Fill),
                    action(
                        ui,
                        &format!("root:rm:{i}"),
                        "Retirer",
                        Btn::Ghost,
                        None,
                        Some(Message::Settings(Msg::RemoveRoot(i)))
                    ),
                ]
                .align_y(iced::Center),
            )
        });
    let add = row![
        text_input("/chemin/absolu ou ~/dossier", input)
            .on_input(|s| Message::Settings(Msg::InputChanged(s)))
            .on_submit(Message::Settings(Msg::AddRoot))
            .style(theme::input_style(tk))
            .size(13)
            .padding([7, 10]),
        action(
            ui,
            "root:add",
            "Ajouter",
            Btn::Primary,
            Some("↵"),
            Some(Message::Settings(Msg::AddRoot))
        ),
    ]
    .spacing(8)
    .align_y(iced::Center);

    let tray_hint = if cfg.tray_enabled && matches!(tray, TrayStatus::Failed(_)) {
        "Zone de notification demandée mais indisponible sur cette session."
    } else {
        "Affiche une icône ; fermer la fenêtre la réduit dans la zone de notification."
    };
    let mut tray_diag = column![faint(tk, format!("État : {}", tray.text()))].spacing(3);
    if let Some(h) = tray.hint() {
        tray_diag = tray_diag.push(faint(tk, h));
    }
    let tray_diag = container(tray_diag).padding([0, 16]).width(Fill);

    let token_controls: Element<'a, Message> = if let Source::Fichier(p) = token_source {
        // Géré par Nix : ni saisie, ni Enregistrer, ni Effacer.
        column![
            text(token_source_text(token_source, true))
                .size(13)
                .color(tk.text),
            text(p.display().to_string())
                .size(12)
                .font(widgets::MONO)
                .color(tk.text),
        ]
        .spacing(3)
        .into()
    } else {
        row![
            text_input("Coller le jeton (jamais affiché)", token_input)
                .secure(true)
                .on_input(|s| Message::Settings(Msg::TokenInput(Secret(s))))
                .on_submit(Message::Settings(Msg::TokenSave))
                .style(theme::input_style(tk))
                .size(13)
                .padding([7, 10]),
            action(
                ui,
                "token:save",
                "Enregistrer",
                Btn::Primary,
                None,
                (!token_input.trim().is_empty()).then_some(Message::Settings(Msg::TokenSave))
            ),
            action(
                ui,
                "token:clear",
                "Effacer",
                Btn::Secondary,
                None,
                token_saved.then_some(Message::Settings(Msg::TokenClear))
            ),
        ]
        .spacing(8)
        .align_y(iced::Center)
        .into()
    };

    let token_block = container(
        column![
            column![
                text("Jeton GitHub")
                    .size(13)
                    .font(widgets::medium())
                    .color(tk.text),
                faint(
                    tk,
                    "Sans jeton, GitHub limite l'API à 60 requêtes par heure. Le jeton est \
                     stocké dans un fichier 0600, hors de la configuration Nix."
                ),
                faint(
                    tk,
                    "Il sert à la vérification et à la mise à jour ; l'application du \
                     système (pkexec) n'utilise pas le jeton."
                ),
            ]
            .spacing(3),
            token_controls,
            dim(tk, token_source_text(token_source, token_saved)).size(12),
        ]
        .spacing(12),
    )
    .padding([14, 16])
    .width(Fill);

    let roots_block = container(
        column![
            column![
                text("Racines de scan")
                    .size(13)
                    .font(widgets::medium())
                    .color(tk.text),
                faint(
                    tk,
                    "Dossiers à partir desquels on cherche des dépôts flake."
                ),
            ]
            .spacing(3),
            roots,
            add,
            row![action(
                ui,
                "root:rescan",
                "Rescanner maintenant",
                Btn::Secondary,
                None,
                Some(Message::Settings(Msg::Rescan))
            )],
        ]
        .spacing(12),
    )
    .padding([14, 16])
    .width(Fill);

    let card = container(column![
        roots_block,
        rule(ui),
        setting(
            ui,
            "Profondeur de scan",
            "Nombre maximal de niveaux de sous-dossiers explorés.",
            stepper(ui, "depth", cfg.scan_depth.to_string(), Msg::Depth),
        ),
        rule(ui),
        setting(
            ui,
            "Zone de notification",
            tray_hint,
            toggler(cfg.tray_enabled)
                .on_toggle(|on| Message::Settings(Msg::ToggleTray(on)))
                .style(theme::toggler_style(tk)),
        ),
        tray_diag,
        rule(ui),
        token_block,
        rule(ui),
        setting(
            ui,
            "Intervalle de vérification",
            "Fréquence de la vérification automatique (zone de notification active).",
            stepper(
                ui,
                "interval",
                interval_label(cfg.check_interval_minutes),
                |up| { Msg::Interval(up) }
            ),
        ),
    ])
    .width(Fill)
    .style(theme::outlined(tk));

    let content = column![
        row![
            action(
                ui,
                "settings:back",
                "← Retour",
                Btn::Ghost,
                Some("Échap"),
                Some(Message::Settings(Msg::Back))
            ),
            title(tk, "Réglages", 18.0),
        ]
        .spacing(10)
        .align_y(iced::Center),
        card,
        row![
            space::horizontal(),
            dim(tk, "Les réglages sont enregistrés automatiquement.").size(12),
            space::horizontal()
        ],
    ]
    .spacing(16)
    .padding([20, 24])
    .max_width(760);

    scrollable(container(content).center_x(Fill))
        .width(Fill)
        .height(Fill)
        .style(theme::scroll_style(tk))
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
    fn token_texts_never_leak() {
        assert_eq!(token_status_text(true), "Jeton enregistré");
        assert_eq!(token_status_text(false), "Aucun jeton");
        let nix = Source::Fichier(PathBuf::from("/run/secrets/gh"));
        assert_eq!(
            token_source_text(&nix, true),
            "Jeton configuré par la configuration Nix"
        );
        assert!(token_source_text(&nix, false).contains("absent"));
        assert_eq!(
            token_source_text(&Source::Interface, true),
            "Jeton enregistré"
        );
        assert_eq!(token_source_text(&Source::Aucun, false), "Aucun jeton");
        let dbg = format!("{:?}", Msg::TokenInput(Secret("ghp_secret".into())));
        assert!(!dbg.contains("ghp_secret"));
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
