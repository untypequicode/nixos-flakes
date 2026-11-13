//! Page de réglages : logique pure (édition de la config) et vue.

use crate::app::Message;
use crate::theme::{self, Btn};
use crate::widgets::{self, action, clear_button, dim, faint, title, Ui};
use iced::widget::{column, container, row, scrollable, space, text, text_input};
use iced::{Element, Fill, Length};
use nixos_flakes_core::config::{self, Config, Role};
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
pub fn token_source_text(source: &Source, saved: bool) -> String {
    match source {
        Source::Fichier(_) if saved => "Jeton configuré par la configuration Nix".into(),
        Source::Fichier(_) => {
            "Jeton configuré par la configuration Nix (fichier absent ou vide)".into()
        }
        Source::Systeme {
            path,
            lisible: true,
        } => format!("Jeton lu depuis {}", path.display()),
        Source::Systeme {
            path,
            lisible: false,
        } => format!("Fichier {} illisible : jeton ignoré", path.display()),
        Source::Interface | Source::Aucun => token_status_text(saved).into(),
    }
}

/// Avertissement affiché quand le fichier système existe mais n'est pas lisible.
pub fn system_unreadable_warning(path: &Path) -> String {
    format!(
        "Le fichier {} existe mais votre utilisateur ne peut pas le lire, donc nix l'ignore. \
         Rendez-le lisible (`install -m 640 -g users`) ou enregistrez un jeton ici.",
        path.display()
    )
}

/// Remarque quand le jeton vient du fichier système lisible.
pub fn system_priority_note() -> &'static str {
    "Un jeton enregistré ici est prioritaire sur celui du fichier système."
}

#[derive(Debug, Clone)]
pub enum Msg {
    InputChanged(String),
    AddRoot,
    /// Demande de retrait (confirmation) de la racine d'indice i.
    AskRemoveRoot(usize),
    /// Retrait effectif (après confirmation).
    RemoveRoot(usize),
    /// Vide le champ d'ajout et son erreur.
    ClearInput,
    Depth(bool),
    Interval(bool),
    TokenInput(Secret),
    TokenSave,
    TokenClear,
    Rescan,
    /// Saisie du chemin du dépôt système.
    SysRepoInput(String),
    /// Saisie de l'attribut `nixosConfigurations.<attr>`.
    SysAttrInput(String),
    /// Enregistre le dépôt système saisi.
    SysRepoSave,
    /// Revient au défaut système (efface le choix de l'utilisateur).
    SysRepoReset,
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

/// Ajoute une racine de scan (chemin absolu, existant, dossier, pas déjà présent).
/// Renvoie la racine ajoutée, ou l'erreur à afficher sous le champ.
pub fn add_root(cfg: &mut Config, input: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let p = parse_root(input, home).ok_or_else(|| {
        if input.trim().starts_with('~') {
            "Impossible de développer « ~ » : dossier personnel inconnu".to_string()
        } else {
            "Saisissez un chemin".to_string()
        }
    })?;
    if !p.is_absolute() {
        return Err(format!(
            "Le chemin doit être absolu (ou commencer par ~/) : {}",
            p.display()
        ));
    }
    if cfg.scan_roots.contains(&p) {
        return Err("Cette racine de scan existe déjà".to_string());
    }
    if !p.exists() {
        return Err(format!("Ce dossier n'existe pas : {}", p.display()));
    }
    if !p.is_dir() {
        return Err(format!("Ce chemin n'est pas un dossier : {}", p.display()));
    }
    cfg.scan_roots.push(p.clone());
    Ok(p)
}

/// Dépôt système saisi : `~` développé, absolu, existant, dossier contenant `flake.nix`.
/// Renvoie le chemin canonique (liens symboliques résolus) ou l'erreur à afficher.
pub fn check_system_repo_input(input: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let p = parse_root(input, home).ok_or_else(|| {
        if input.trim().starts_with('~') {
            "Impossible de développer « ~ » : dossier personnel inconnu".to_string()
        } else {
            "Saisissez le chemin du dossier qui contient flake.nix".to_string()
        }
    })?;
    if !p.is_absolute() {
        return Err(format!(
            "Le chemin doit être absolu (ou commencer par ~/) : {}",
            p.display()
        ));
    }
    // `#` et `?` changeraient le sens de `<chemin>#<attr>` pour nix/nixos-rebuild.
    config::validate_repo_path(&p).map_err(|e| e.to_string())?;
    if !p.exists() {
        return Err(format!("Ce dossier n'existe pas : {}", p.display()));
    }
    if !p.is_dir() {
        return Err(format!("Ce chemin n'est pas un dossier : {}", p.display()));
    }
    if !p.join("flake.nix").is_file() {
        return Err(format!(
            "Pas de flake.nix dans {} (indiquez le dossier du flake)",
            p.display()
        ));
    }
    p.canonicalize()
        .map_err(|e| format!("{} : {e}", p.display()))
}

/// Attribut saisi : vide = nom d'hôte (`None`), sinon caractères sûrs seulement.
pub fn parse_attr_input(input: &str) -> Result<Option<String>, String> {
    let a = input.trim();
    if a.is_empty() {
        return Ok(None);
    }
    config::validate_attr(a).map_err(|_| {
        "Nom de configuration invalide (lettres, chiffres, « - », « _ » et « . » seulement)"
            .to_string()
    })?;
    Ok(Some(a.to_string()))
}

/// Première erreur de validation du formulaire du dépôt système (affichée en ligne).
/// Un champ de chemin vide n'est pas une erreur : le bouton est simplement inactif.
pub fn system_form_error(repo: &str, attr: &str, home: Option<&Path>) -> Option<String> {
    if !repo.trim().is_empty() {
        if let Err(e) = check_system_repo_input(repo, home) {
            return Some(e);
        }
    }
    parse_attr_input(attr).err()
}

/// Champ « chemin » prérempli : le dépôt système effectif (choisi, d'environnement ou défaut
/// système), pour qu'on puisse ne modifier que l'attribut.
pub fn system_repo_prefill(cfg: &Config) -> String {
    cfg.system_repo_resolved()
        .map(|r| r.path.display().to_string())
        .unwrap_or_default()
}

/// Enregistre le dépôt système (et l'attribut) dans la configuration utilisateur.
/// L'ancien dépôt système devient suivi. Renvoie le chemin enregistré.
/// Si le chemin saisi est celui du défaut système (rien n'est choisi), il n'est pas figé
/// dans la configuration : seul l'attribut est enregistré.
pub fn apply_system_repo(
    cfg: &mut Config,
    repo: &str,
    attr: &str,
    home: Option<&Path>,
) -> Result<PathBuf, String> {
    let default = if cfg.system_repo.is_none() {
        cfg.system_repo_resolved().map(|r| r.path)
    } else {
        None
    };
    apply_system_repo_with(cfg, repo, attr, home, default.as_deref())
}

/// Comme [`apply_system_repo`], avec le défaut système effectif en paramètre
/// (`None` si un dépôt est déjà choisi ou s'il n'y a pas de défaut).
pub fn apply_system_repo_with(
    cfg: &mut Config,
    repo: &str,
    attr: &str,
    home: Option<&Path>,
    default: Option<&Path>,
) -> Result<PathBuf, String> {
    let path = check_system_repo_input(repo, home)?;
    let attr = parse_attr_input(attr)?;
    let is_default =
        default.is_some_and(|d| d.canonicalize().unwrap_or_else(|_| d.to_path_buf()) == path);
    if !is_default {
        cfg.set_role(&path, Role::System);
    }
    cfg.system_attr = attr;
    Ok(path)
}

/// Efface le choix de l'utilisateur : retour au défaut système. Faux si rien n'était choisi.
pub fn reset_system_repo(cfg: &mut Config) -> bool {
    let had = cfg.system_repo.is_some() || cfg.system_attr.is_some();
    cfg.system_repo = None;
    cfg.system_attr = None;
    had
}

/// Texte de l'état courant : chemin, provenance et configuration.
pub fn system_repo_summary(cfg: &Config) -> String {
    match cfg.system_repo_resolved() {
        Some(r) => format!(
            "{} ({}{})",
            r.path.display(),
            r.source.label(),
            r.attr
                .as_deref()
                .map_or_else(String::new, |a| format!(", configuration {a}"))
        ),
        None => "Aucun dépôt système défini".to_string(),
    }
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

/// Formulaire du dépôt système (saisies et erreur de validation en ligne).
pub struct SysForm<'a> {
    pub repo: &'a str,
    pub attr: &'a str,
    pub error: Option<&'a str>,
}

/// Vue de la page de réglages.
#[allow(clippy::too_many_arguments)]
pub fn view<'a>(
    ui: &Ui<'_>,
    cfg: &'a Config,
    sys: SysForm<'a>,
    input: &'a str,
    root_error: Option<&'a str>,
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
                        Some(Message::Settings(Msg::AskRemoveRoot(i)))
                    ),
                ]
                .align_y(iced::Center),
            )
        });
    let mut add_row = row![text_input("/chemin/absolu ou ~/dossier", input)
        .on_input(|s| Message::Settings(Msg::InputChanged(s)))
        .on_submit(Message::Settings(Msg::AddRoot))
        .style(theme::input_style(tk))
        .size(13)
        .padding([7, 10])]
    .spacing(8)
    .align_y(iced::Center);
    if !input.is_empty() {
        add_row = add_row.push(clear_button(
            ui,
            "root:clear",
            Message::Settings(Msg::ClearInput),
        ));
    }
    add_row = add_row.push(action(
        ui,
        "root:add",
        "Ajouter",
        Btn::Primary,
        Some("↵"),
        Some(Message::Settings(Msg::AddRoot)),
    ));
    let mut add = column![add_row].spacing(6);
    if let Some(e) = root_error {
        add = add.push(text(e.to_string()).size(12).color(tk.danger));
    }

    let token_form = || -> Element<'a, Message> {
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
                (token_saved && !matches!(token_source, Source::Systeme { .. }))
                    .then_some(Message::Settings(Msg::TokenClear))
            ),
        ]
        .spacing(8)
        .align_y(iced::Center)
        .into()
    };
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
    } else if let Source::Systeme { path, lisible } = token_source {
        let info: Element<'a, Message> = if *lisible {
            column![
                text(token_source_text(token_source, true))
                    .size(13)
                    .color(tk.text),
                faint(tk, system_priority_note()),
            ]
            .spacing(3)
            .into()
        } else {
            text(system_unreadable_warning(path))
                .size(12)
                .color(tk.warning)
                .into()
        };
        column![info, token_form()].spacing(8).into()
    } else {
        token_form()
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

    let can_save = !sys.repo.trim().is_empty() && sys.error.is_none();
    let can_reset = cfg.system_repo.is_some() || cfg.system_attr.is_some();
    let mut sys_col = column![
        column![
            text("Dépôt système")
                .size(13)
                .font(widgets::medium())
                .color(tk.text),
            faint(
                tk,
                "Dossier de la configuration NixOS (celui qui contient flake.nix), où qu'il soit : \
                 ~/nixos-config, /srv/nixos, /etc/nixos…"
            ),
        ]
        .spacing(3),
        text(system_repo_summary(cfg))
            .size(12)
            .font(widgets::MONO)
            .color(tk.text),
        text_input("/chemin/absolu ou ~/nixos-config", sys.repo)
            .id(crate::app::SYSTEM_REPO_INPUT)
            .on_input(|s| Message::Settings(Msg::SysRepoInput(s)))
            .on_submit(Message::Settings(Msg::SysRepoSave))
            .style(theme::input_style(tk))
            .size(13)
            .padding([7, 10]),
        text_input("Configuration (attribut) — vide : nom d'hôte", sys.attr)
            .on_input(|s| Message::Settings(Msg::SysAttrInput(s)))
            .on_submit(Message::Settings(Msg::SysRepoSave))
            .style(theme::input_style(tk))
            .size(13)
            .padding([7, 10]),
    ]
    .spacing(8);
    if let Some(e) = sys.error {
        sys_col = sys_col.push(text(e.to_string()).size(12).color(tk.danger));
    }
    sys_col = sys_col.push(
        row![
            action(
                ui,
                "sysrepo:save",
                "Enregistrer",
                Btn::Primary,
                None,
                can_save.then_some(Message::Settings(Msg::SysRepoSave))
            ),
            action(
                ui,
                "sysrepo:reset",
                "Réinitialiser",
                Btn::Secondary,
                None,
                can_reset.then_some(Message::Settings(Msg::SysRepoReset))
            ),
            faint(tk, "Réinitialiser : retour au défaut du système."),
        ]
        .spacing(8)
        .align_y(iced::Center),
    );
    let system_block = container(sys_col.spacing(10)).padding([14, 16]).width(Fill);

    let card = container(column![
        system_block,
        rule(ui),
        roots_block,
        rule(ui),
        setting(
            ui,
            "Profondeur de scan",
            "Nombre maximal de niveaux de sous-dossiers explorés.",
            stepper(ui, "depth", cfg.scan_depth.to_string(), Msg::Depth),
        ),
        rule(ui),
        token_block,
        rule(ui),
        setting(
            ui,
            "Intervalle de vérification",
            "Fréquence de la vérification automatique, effectuée par le démon.",
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
    fn system_token_texts() {
        let path = PathBuf::from("/etc/nix/access-tokens.conf");
        let ok = Source::Systeme {
            path: path.clone(),
            lisible: true,
        };
        assert_eq!(
            token_source_text(&ok, true),
            "Jeton lu depuis /etc/nix/access-tokens.conf"
        );
        let ko = Source::Systeme {
            path: path.clone(),
            lisible: false,
        };
        assert!(token_source_text(&ko, false).contains("illisible"));
        let w = system_unreadable_warning(&path);
        assert!(w.contains("/etc/nix/access-tokens.conf"));
        assert!(w.contains("ne peut pas le lire"));
        assert!(w.contains("install -m 640 -g users"));
        assert!(system_priority_note().contains("prioritaire"));
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
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let mut c = empty();
        let a_in = a.display().to_string();
        assert_eq!(add_root(&mut c, &a_in, None), Ok(a.clone()));
        assert!(add_root(&mut c, &a_in, None).is_err());
        assert!(add_root(&mut c, "", None).is_err());
        assert!(add_root(&mut c, "relatif", None).is_err());
        assert!(add_root(&mut c, "~/b", Some(d.path())).is_ok());
        assert_eq!(c.scan_roots, [a, b.clone()]);
        assert!(remove_root(&mut c, 0));
        assert!(!remove_root(&mut c, 5));
        assert_eq!(c.scan_roots, [b]);
    }

    #[test]
    fn root_errors_are_explicit() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("f");
        std::fs::write(&file, "x").unwrap();
        let mut c = empty();
        let err = |c: &mut Config, s: &str, h: Option<&Path>| add_root(c, s, h).unwrap_err();
        assert!(err(&mut c, "  ", None).contains("Saisissez"));
        assert!(err(&mut c, "dossier", None).contains("absolu"));
        assert!(err(&mut c, "~/x", None).contains("dossier personnel"));
        let missing = d.path().join("absent");
        assert!(err(&mut c, &missing.display().to_string(), None).contains("n'existe pas"));
        assert!(err(&mut c, &file.display().to_string(), None).contains("pas un dossier"));
        assert!(c.scan_roots.is_empty(), "rien n'est ajouté en cas d'erreur");
        let ok = d.path().display().to_string();
        assert!(add_root(&mut c, &ok, None).is_ok());
        assert!(err(&mut c, &ok, None).contains("existe déjà"));
    }

    #[test]
    fn system_repo_input_validation() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("nixos config");
        std::fs::create_dir(&repo).unwrap();
        let err = |s: &str, h: Option<&Path>| check_system_repo_input(s, h).unwrap_err();
        assert!(err("  ", None).contains("Saisissez"));
        assert!(err("relatif", None).contains("absolu"));
        assert!(err("/srv/a#b", None).contains('#'));
        assert!(err("/srv/a?b", None).contains('?'));
        assert!(system_form_error("/srv/a#b", "", None).is_some());
        assert!(err("~/x", None).contains("dossier personnel"));
        assert!(err(&d.path().join("absent").display().to_string(), None).contains("n'existe pas"));
        assert!(err(&repo.display().to_string(), None).contains("flake.nix"));
        std::fs::write(repo.join("flake.nix"), "{}").unwrap();
        let canon = repo.canonicalize().unwrap();
        assert_eq!(
            check_system_repo_input(&repo.display().to_string(), None).unwrap(),
            canon
        );
        // `~` développé depuis le dossier personnel.
        assert_eq!(
            check_system_repo_input("~/nixos config", Some(d.path())).unwrap(),
            canon
        );
        let file = d.path().join("f");
        std::fs::write(&file, "x").unwrap();
        assert!(err(&file.display().to_string(), None).contains("pas un dossier"));
    }

    #[test]
    fn attr_input_and_form_error() {
        assert_eq!(parse_attr_input("  "), Ok(None));
        assert_eq!(parse_attr_input(" laptop "), Ok(Some("laptop".to_string())));
        assert!(parse_attr_input("a b").is_err());
        assert!(parse_attr_input("a;b").is_err());
        // Champ de chemin vide : pas d'erreur (bouton inactif), mais l'attribut est vérifié.
        assert_eq!(system_form_error("", "", None), None);
        assert!(system_form_error("", "x y", None).is_some());
    }

    #[test]
    fn apply_and_reset_system_repo() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        for p in [&a, &b] {
            std::fs::create_dir(p).unwrap();
            std::fs::write(p.join("flake.nix"), "{}").unwrap();
        }
        let mut c = empty();
        assert!(!reset_system_repo(&mut c));
        assert_eq!(
            apply_system_repo(&mut c, &a.display().to_string(), "h", None).unwrap(),
            a.canonicalize().unwrap()
        );
        assert_eq!(c.system_attr.as_deref(), Some("h"));
        // Changer de dépôt : l'ancien devient suivi, l'attribut est remplacé.
        apply_system_repo(&mut c, &b.display().to_string(), "", None).unwrap();
        assert_eq!(c.role(&a), Role::Tracked);
        assert_eq!(c.system_attr, None);
        assert!(apply_system_repo(&mut c, "/nope", "", None).is_err());
        assert_eq!(c.role(&b), Role::System, "inchangé en cas d'erreur");
        assert!(reset_system_repo(&mut c));
        assert_eq!(c.system_repo, None);
        assert!(system_repo_summary(&Config::default()).len() > 5);
    }

    #[test]
    fn attr_alone_can_change_when_the_repo_is_the_system_default() {
        let d = tempfile::tempdir().unwrap();
        let sys = d.path().join("sys");
        std::fs::create_dir(&sys).unwrap();
        std::fs::write(sys.join("flake.nix"), "{}").unwrap();
        let mut c = empty();
        // Chemin du défaut système prérempli : seul l'attribut est enregistré.
        let path = apply_system_repo_with(
            &mut c,
            &sys.display().to_string(),
            "laptop",
            None,
            Some(&sys),
        )
        .unwrap();
        assert_eq!(path, sys.canonicalize().unwrap());
        assert_eq!(c.system_repo, None, "le défaut système n'est pas figé");
        assert_eq!(c.system_attr.as_deref(), Some("laptop"));
        // Un autre chemin que le défaut est bien choisi.
        let other = d.path().join("other");
        std::fs::create_dir(&other).unwrap();
        std::fs::write(other.join("flake.nix"), "{}").unwrap();
        apply_system_repo_with(&mut c, &other.display().to_string(), "", None, Some(&sys)).unwrap();
        assert_eq!(c.system_repo, Some(other.canonicalize().unwrap()));
        // Champ prérempli avec le chemin choisi.
        assert_eq!(
            system_repo_prefill(&c),
            other.canonicalize().unwrap().display().to_string()
        );
    }
}
