//! État de l'application et logique de mise à jour (voir `view.rs` pour l'affichage).

use crate::anim::{self, Fades, Tween};
use crate::cli::Cli;
use crate::daemon_link;
use crate::demo;
use crate::keys::{self, KeyContext, Shortcut};
use crate::model::{
    self, Actions, ConfirmAction, JobKind, Modal, RepoEntry, RoleAction, UpdatePlan,
    DIRTY_COMMIT_MESSAGE,
};
use crate::packages::{self, PackagesState};
use crate::palette::{self, Command, PaletteState};
use crate::settings;
use crate::sync::{self, SyncState};
use crate::tasks::{self, Finished, Job, JobEvent, Outcome};
use crate::terminal;
use crate::theme::{self, Mode, Tokens};
use crate::toast::{ToastKind, Toasts};
use crate::widgets::{self, Ui};
use iced::futures::{stream, Stream, StreamExt};
use iced::keyboard::{Key, Modifiers};
use iced::widget::text_editor;
use iced::{window, Subscription, Task, Theme};
use nixos_flakes_core::config::{Config, Role};
use nixos_flakes_core::daemon::DaemonStatus;
use nixos_flakes_core::git;
use nixos_flakes_core::lock::RootInput;
use nixos_flakes_core::status::RepoStatus;
use nixos_flakes_core::token::Source as TokenSource;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Nombre maximal de lignes conservées dans la console.
pub const LOG_CAPACITY: usize = 3000;
/// Hauteur du corps de la console déployée.
pub const CONSOLE_HEIGHT: f32 = 190.0;
/// Identifiant du champ de saisie de la palette.
pub const PALETTE_INPUT: &str = "palette-input";
/// Champ de saisie du dépôt système (page Réglages).
pub const SYSTEM_REPO_INPUT: &str = "system-repo-input";
/// Hauteur visible de la liste de la palette (la liste défile au-delà).
pub const PALETTE_LIST_HEIGHT: f32 = 330.0;
/// Identifiant de la liste défilante de la palette.
pub const PALETTE_LIST: &str = "palette-list";
/// Hauteur d'une ligne de la palette (34 px + 1 px d'espacement).
const PALETTE_ROW_H: f32 = 35.0;
/// Hauteur approximative d'un intitulé de section de la palette.
const PALETTE_HEADING_H: f32 = 26.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Main,
    Settings,
    Packages,
    Terminal,
}

/// Onglet du panneau de détail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Updates,
    Git,
    Repo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    Info,
    Command,
    Error,
    Success,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub kind: LogKind,
    pub text: String,
}

/// Suite à donner à une commande terminée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Followup {
    /// Après le commit des modifications, lancer la mise à jour.
    Update,
}

/// Verdict avant de lancer une commande depuis un dialogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gate {
    /// Le dépôt est libre.
    Go,
    /// Le dépôt est occupé : le dialogue reste ouvert.
    Wait,
    /// Le dépôt a disparu de la liste : le dialogue se ferme.
    Gone,
}

/// Sélection en bloc des inputs d'un dépôt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputSelect {
    All,
    NoneSelected,
    Updatable,
}

#[derive(Debug, Clone)]
pub enum Message {
    // Données
    Rescan,
    /// Résultat d'un scan (numéro de séquence : seul le dernier compte) ou erreur.
    ScanDone(u64, Result<Vec<RepoStatus>, String>),
    Select(PathBuf),
    // Commandes
    Check(PathBuf),
    CheckAll,
    Update(PathBuf),
    /// Mise à jour limitée à ces inputs racine.
    UpdateInputs(PathBuf, Vec<String>),
    /// Mise à jour des inputs cochés.
    UpdateSelection(PathBuf),
    ToggleInput(PathBuf, String),
    SelectInputs(PathBuf, InputSelect),
    InputsLoaded(PathBuf, Result<Vec<RootInput>, String>),
    Apply(PathBuf),
    Push(PathBuf),
    Role(PathBuf, RoleAction),
    /// Demande une action lourde : ouvre un dialogue de confirmation (ou l'exécute si anodine).
    Ask(ConfirmAction),
    /// Valide le dialogue de confirmation.
    ModalConfirm,
    Job {
        repo: PathBuf,
        event: JobEvent,
    },
    // Dialogues git
    EditorAction(text_editor::Action),
    ModalCommit,
    ModalContinue,
    ModalCancel,
    // Interface
    ClearLog,
    ToggleConsole,
    Noop,
    ToggleSidebar,
    OpenSettings,
    /// Ouvre les réglages et met le focus sur le champ du dépôt système.
    OpenSystemRepoSettings,
    FocusSystemRepo,
    /// Retour à la page principale.
    GoMain,
    OpenHelp,
    CloseHelp,
    /// Résultat d'un enregistrement de la config (texte de succès éventuel).
    /// Fin d'un enregistrement : `Ok(false)` = périmé (un plus récent a suivi) ; la config
    /// enregistrée et la génération servent à restaurer l'état en cas d'échec.
    ConfigSaved(Result<bool, String>, Option<String>, Box<Config>, u64),
    /// État du jeton lu au démarrage.
    TokenLoaded(bool, TokenSource),
    TokenSaved(Result<(), String>),
    TokenCleared(Result<(bool, TokenSource), String>),
    /// Préférence de thème du système, détectée hors du fil de l'interface.
    /// Mode détecté hors du fil de l'interface (`None` : détection impossible, ignoré).
    ModeDetected(Option<Mode>),
    Settings(settings::Msg),
    Packages(packages::Msg),
    SystemTheme(iced::theme::Mode),
    SetTab(Tab),
    ToggleGroup(Role),
    // Animations, survol, toasts
    Frame,
    Hover(String, bool),
    DismissToast(u64),
    // Palette de commandes et clavier
    OpenPalette,
    ClosePalette,
    PaletteInput(String),
    FocusPalette,
    PaletteRun(usize),
    Key(Key, Modifiers, bool),
    // Fenêtre
    CloseRequested,
    WindowClosed(window::Id),
    // Démon (absent = `None`)
    Daemon(Option<DaemonStatus>),
    DaemonCheckNow,
    DaemonCheckSent(Result<(), String>),
    // Synchronisation du dépôt système
    SyncFetch,
    SyncPull,
    SyncDone(Result<git::SyncStatus, String>),
    // Terminal intégré
    OpenTerminal,
    RestartTerminal,
    Terminal(iced_term::Event),
}

/// Délai après la fermeture d'un dialogue pendant lequel Entrée ne confirme pas le suivant.
const MODAL_ENTER_GUARD: Duration = Duration::from_millis(400);

pub struct App {
    pub cli: Cli,
    pub cfg: Config,
    pub cfg_path: PathBuf,
    /// Dernière config connue pour être sur le disque (restaurée si un enregistrement échoue).
    pub cfg_persisted: Config,
    /// Génération du dernier enregistrement demandé (les plus anciens ne s'écrivent pas).
    pub save_gen: Arc<AtomicU64>,
    /// Jusqu'à cet instant, Entrée ne confirme pas le dialogue qui vient d'apparaître.
    pub modal_guard_until: Option<Instant>,
    pub home: Option<PathBuf>,
    pub entries: Vec<RepoEntry>,
    pub selected: Option<PathBuf>,
    pub scanning: bool,
    /// Numéro du dernier scan lancé : un résultat d'un scan antérieur est ignoré.
    pub scan_seq: u64,
    pub log: VecDeque<LogLine>,
    pub console_open: bool,
    pub modal: Option<Modal>,
    /// Dialogues en attente : affichés l'un après l'autre, jamais l'un sur l'autre.
    pub modal_queue: VecDeque<Modal>,
    pub editor: text_editor::Content,
    pub page: Page,
    pub tab: Tab,
    pub mode: Mode,
    pub window: Option<window::Id>,
    pub sidebar_open: bool,
    pub queue: VecDeque<PathBuf>,
    pub followups: HashMap<PathBuf, Followup>,
    /// Inputs racine du `flake.lock` de chaque dépôt (chargés à la sélection).
    pub inputs: HashMap<PathBuf, Vec<RootInput>>,
    /// Inputs cochés par dépôt (vidé après une mise à jour).
    pub input_sel: HashMap<PathBuf, BTreeSet<String>>,
    /// Inputs d'une mise à jour en attente du commit des modifications.
    pub update_request: HashMap<PathBuf, Vec<String>>,
    /// État du démon : `None` = absent (le GUI fonctionne seul).
    pub daemon: Option<DaemonStatus>,
    pub sync: SyncState,
    /// Terminal intégré, créé à la première ouverture de l'onglet.
    pub terminal: Option<iced_term::Terminal>,
    pub terminal_error: Option<String>,
    /// Saisie du jeton GitHub : vidée dès l'enregistrement, jamais journalisée ni écrite en config.
    pub token_input: String,
    pub token_saved: bool,
    /// Origine du jeton (fichier Nix = lecture seule). Toujours `Aucun`/`Interface` en démo.
    pub token_source: nixos_flakes_core::token::Source,
    pub new_root: String,
    /// Erreur de la dernière tentative d'ajout d'une racine (affichée sous le champ).
    pub root_error: Option<String>,
    /// Saisies du formulaire « Dépôt système » (chemin et attribut) et son erreur en ligne.
    pub sys_repo_input: String,
    pub sys_attr_input: String,
    pub sys_error: Option<String>,
    /// Un enregistrement ou effacement du jeton est en cours.
    pub token_busy: bool,
    /// L'aide des raccourcis est affichée.
    pub help_open: bool,
    // -- animation et présentation --
    /// Dernier instant connu (mis à jour à chaque message).
    pub now: Instant,
    /// Origine des animations cycliques.
    pub epoch: Instant,
    pub fades: Fades,
    pub collapsed: HashSet<Role>,
    pub palette_open: bool,
    pub palette: PaletteState,
    pub palette_t: Tween,
    /// Dialogue en cours de fermeture (affiché le temps de son animation de sortie).
    pub modal_leaving: Option<Modal>,
    pub modal_t: Tween,
    pub panel_t: Tween,
    pub console_t: Tween,
    pub theme_t: Tween,
    pub mode_from: Mode,
    pub toasts: Toasts,
    /// Vues de démonstration à ouvrir après le premier scan.
    pub demo_views: Vec<String>,
    /// État de la page Paquets.
    pub packages: PackagesState,
}

/// Paramètres de la fenêtre principale.
pub fn window_settings() -> window::Settings {
    window::Settings {
        size: iced::Size::new(1180.0, 760.0),
        min_size: Some(iced::Size::new(420.0, 400.0)),
        exit_on_close_request: false,
        icon: window::icon::from_rgba(crate::icon::app_rgba(256), 256, 256).ok(),
        platform_specific: window::settings::PlatformSpecific {
            application_id: "nixos-flakes-gui".to_string(),
            ..Default::default()
        },
        ..window::Settings::default()
    }
}

/// Démarrage : charge la config, enregistre éventuellement l'icône, ouvre la fenêtre.
pub fn boot(cli: Cli) -> (App, Task<Message>) {
    let cfg_path = cli
        .config
        .clone()
        .or_else(|| Config::default_path().ok())
        .unwrap_or_default(); // chemin vide : `load`/`save` renvoient alors une erreur explicite
    let mut early_log = Vec::new();
    let cfg = match Config::load(&cfg_path) {
        Ok(c) => c,
        Err(e) => {
            early_log.push(LogLine {
                kind: LogKind::Error,
                text: format!("Configuration illisible, valeurs par défaut utilisées : {e}"),
            });
            Config::default()
        }
    };
    let mut app = App::new(cli, cfg, cfg_path);
    app.log.extend(early_log);

    let mut tasks = vec![app.scan_task()];
    // En démo, le thème reste celui demandé (vue « clair ») au lieu de suivre le système.
    if !app.cli.demo {
        tasks.push(iced::system::theme().map(Message::SystemTheme));
        // Détection (gsettings/dconf) et lecture du jeton : sorties du fil de l'interface.
        tasks.push(Task::perform(
            tasks::blocking(theme::detect_system_mode),
            Message::ModeDetected,
        ));
        tasks.push(Task::perform(
            tasks::blocking(|| {
                (
                    nixos_flakes_core::token::exists(),
                    nixos_flakes_core::token::source(),
                )
            }),
            |r| {
                let (saved, source) = r.unwrap_or((false, TokenSource::Aucun));
                Message::TokenLoaded(saved, source)
            },
        ));
    }
    tasks.push(app.open_window());
    // Un seul fetch à l'ouverture ; ensuite uniquement sur demande.
    tasks.push(app.sync_task(false));
    (app, Task::batch(tasks))
}

/// Commande de mise à jour : tout si `inputs` est vide, sinon ces inputs seulement.
fn update_job(inputs: Vec<String>) -> Job {
    if inputs.is_empty() {
        Job::Update
    } else {
        Job::UpdateInputs(inputs)
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

impl App {
    pub fn new(cli: Cli, cfg: Config, cfg_path: PathBuf) -> Self {
        let now = Instant::now();
        let demo = cli.demo;
        let demo_views = cli.demo_view.clone();
        let mut cfg = cfg;
        if demo {
            // Rôles des faux dépôts, en mémoire seulement (la config n'est jamais écrite en démo).
            for s in demo::statuses(0) {
                cfg.set_role(&s.path, s.role);
            }
        }
        let cfg_sys_repo = settings::system_repo_prefill(&cfg);
        let cfg_sys_attr = cfg.system_attr.clone().unwrap_or_default();
        let cfg_persisted = cfg.clone();
        App {
            cli,
            cfg,
            cfg_path,
            cfg_persisted,
            save_gen: Arc::new(AtomicU64::new(0)),
            modal_guard_until: None,
            home: if demo {
                Some(PathBuf::from(demo::HOME))
            } else {
                std::env::var_os("HOME")
                    .filter(|h| !h.is_empty())
                    .map(PathBuf::from)
            },
            entries: Vec::new(),
            selected: None,
            scanning: false,
            scan_seq: 0,
            log: VecDeque::new(),
            console_open: true,
            modal: None,
            modal_queue: VecDeque::new(),
            editor: text_editor::Content::new(),
            page: Page::Main,
            tab: Tab::Updates,
            // Le vrai mode système est détecté hors du fil de l'interface (`ModeDetected`).
            mode: Mode::Dark,
            window: None,
            sidebar_open: false,
            queue: VecDeque::new(),
            followups: HashMap::new(),
            inputs: HashMap::new(),
            input_sel: HashMap::new(),
            update_request: HashMap::new(),
            daemon: None,
            sync: SyncState::Idle,
            terminal: None,
            terminal_error: None,
            token_input: String::new(),
            // Lus hors du fil de l'interface au démarrage (`TokenLoaded`) ; jamais en démo.
            token_saved: false,
            token_source: TokenSource::Aucun,
            new_root: String::new(),
            root_error: None,
            sys_repo_input: cfg_sys_repo,
            sys_attr_input: cfg_sys_attr,
            sys_error: None,
            token_busy: false,
            help_open: false,
            now,
            epoch: now,
            fades: Fades::default(),
            collapsed: HashSet::new(),
            palette_open: false,
            palette: PaletteState::default(),
            palette_t: Tween::new(0.0, now),
            modal_leaving: None,
            modal_t: Tween::new(0.0, now),
            panel_t: Tween::new(1.0, now),
            console_t: Tween::new(1.0, now),
            theme_t: Tween::new(1.0, now),
            mode_from: Mode::Dark,
            toasts: Toasts::default(),
            demo_views,
            packages: PackagesState::default(),
        }
    }

    // -- accesseurs ---------------------------------------------------------

    pub fn title(&self, _id: window::Id) -> String {
        match model::overall(&self.entries) {
            model::Overall::Updates(n) => format!("Gestion des flakes — {n} à mettre à jour"),
            _ => "Gestion des flakes".to_string(),
        }
    }

    /// Jetons de couleur courants (interpolés pendant un changement de thème).
    pub fn tokens(&self) -> Tokens {
        let t = self.theme_t.value(self.now);
        Tokens::mix(theme::tokens(self.mode_from), theme::tokens(self.mode), t)
    }

    pub fn theme(&self, _id: window::Id) -> Theme {
        self.tokens().iced_theme("Flakes")
    }

    /// Fond de la fenêtre (derrière tous les widgets).
    pub fn style(&self, _theme: &Theme) -> iced::theme::Style {
        let tk = self.tokens();
        iced::theme::Style {
            background_color: tk.bg,
            text_color: tk.text,
        }
    }

    /// Contexte de rendu partagé par tous les composants.
    pub fn ui(&self) -> Ui<'_> {
        Ui {
            tk: self.tokens(),
            now: self.now,
            fades: &self.fades,
            spin: anim::phase(self.now, self.epoch, widgets::SPIN_PERIOD),
            sweep: anim::phase(self.now, self.epoch, widgets::SWEEP_PERIOD),
        }
    }

    /// Vrai tant qu'une transition ou une animation continue est en cours.
    /// Sinon l'application est au repos : aucune image n'est demandée.
    pub fn is_animating(&self) -> bool {
        let now = self.now;
        self.fades.any_animating(now)
            || self.palette_t.is_animating(now)
            || self.modal_t.is_animating(now)
            || self.panel_t.is_animating(now)
            || self.console_t.is_animating(now)
            || self.theme_t.is_animating(now)
            || self.toasts.is_animating(now)
            || self.scanning
            || self.entries.iter().any(|e| e.busy.is_some())
    }

    pub fn entry(&self, path: &Path) -> Option<&RepoEntry> {
        self.entries.iter().find(|e| e.path == path)
    }

    fn entry_mut(&mut self, path: &Path) -> Option<&mut RepoEntry> {
        self.entries.iter_mut().find(|e| e.path == path)
    }

    pub fn selected_entry(&self) -> Option<&RepoEntry> {
        self.selected.as_deref().and_then(|p| self.entry(p))
    }

    pub fn actions_for(&self, path: &Path) -> Actions {
        self.entry(path).map(model::actions).unwrap_or_default()
    }

    pub fn push_log(&mut self, kind: LogKind, text: impl Into<String>) {
        self.log.push_back(LogLine {
            kind,
            text: text.into(),
        });
        while self.log.len() > LOG_CAPACITY {
            self.log.pop_front();
        }
    }

    fn repo_name(&self, path: &Path) -> String {
        model::short_name(path)
    }

    /// Toast à partir d'un message de journal (première ligne en titre, reste en détail).
    fn toast(&mut self, kind: ToastKind, text: &str) {
        let text = text.trim_start_matches(['✓', '✕', ' ']);
        let mut lines = text.lines();
        let head = lines.next().unwrap_or_default().to_string();
        let rest: Vec<&str> = lines.collect();
        let body = (!rest.is_empty()).then(|| rest.join(" "));
        self.toasts.push(kind, head, body, self.now);
    }

    // -- animations -----------------------------------------------------------

    pub fn fade_key(path: &Path) -> String {
        path.display().to_string()
    }

    fn sync_selection_fades(&mut self) {
        let now = self.now;
        let sel = self.selected.clone();
        let keys: Vec<(String, bool)> = self
            .entries
            .iter()
            .map(|e| {
                (
                    Self::fade_key(&e.path),
                    sel.as_deref() == Some(e.path.as_path()),
                )
            })
            .collect();
        for (k, on) in keys {
            self.fades.set(
                &format!("sel:{k}"),
                f32::from(u8::from(on)),
                0.0,
                now,
                anim::BASE,
            );
        }
    }

    /// Évite les survols « collés » quand l'interface change sous le curseur.
    fn reset_hover(&mut self) {
        self.fades.reset_prefix("hov:", self.now, anim::FAST);
    }

    fn restart_panel(&mut self) {
        self.panel_t = Tween::new(0.0, self.now);
        self.panel_t.retarget(1.0, self.now, anim::SLOW);
    }

    fn set_theme_mode(&mut self, m: Mode) {
        if m != self.mode {
            self.mode_from = self.mode;
            self.mode = m;
            if let Some(t) = self.terminal.as_mut() {
                t.handle(iced_term::Command::ChangeTheme(Box::new(
                    terminal::palette(m),
                )));
            }
            self.theme_t = Tween::new(0.0, self.now);
            self.theme_t.retarget(1.0, self.now, anim::SLOW);
        }
    }

    // -- tâches -------------------------------------------------------------

    fn open_window(&mut self) -> Task<Message> {
        let (id, open) = window::open(window_settings());
        self.window = Some(id);
        open.map(|_| Message::Noop)
    }

    fn scan_task(&mut self) -> Task<Message> {
        self.scanning = true;
        self.scan_seq += 1;
        let seq = self.scan_seq;
        let interrupted = || "analyse interrompue".to_string();
        if self.cli.demo {
            let now = unix_now();
            return Task::perform(
                tasks::blocking(move || {
                    std::thread::sleep(Duration::from_millis(900));
                    demo::statuses(now)
                }),
                move |r| Message::ScanDone(seq, r.ok_or_else(interrupted)),
            );
        }
        let cfg = self.cfg.clone();
        Task::perform(tasks::blocking(move || tasks::scan_all(&cfg)), move |r| {
            Message::ScanDone(seq, r.ok_or_else(interrupted))
        })
    }

    /// Lance une commande si le dépôt est libre (une seule à la fois par dépôt).
    pub fn start_job(&mut self, repo: &Path, job: Job) -> Task<Message> {
        let kind = job.kind();
        let name = self.repo_name(repo);
        let demo_mode = self.cli.demo;
        let Some(entry) = self.entry_mut(repo) else {
            self.push_log(
                LogKind::Error,
                format!("{name} : dépôt introuvable, commande ignorée."),
            );
            self.toast(
                ToastKind::Error,
                &format!("Dépôt introuvable\n{name} n'est plus dans la liste."),
            );
            return Task::none();
        };
        if let Some(busy) = entry.busy {
            let what = busy.label();
            self.push_log(LogKind::Error, format!("{name} : {what} déjà en cours."));
            self.toast(
                ToastKind::Error,
                &format!("{name} est occupé\n{what} déjà en cours : réessayez ensuite."),
            );
            return Task::none();
        }
        entry.busy = Some(kind);
        self.push_log(LogKind::Command, format!("▶ {} — {name}", kind.label()));
        let repo = repo.to_path_buf();
        let for_msg = repo.clone();
        // Le thread n'est créé qu'à la première lecture du flux.
        let lazy = stream::once(async move {
            if demo_mode {
                demo::run_job(repo, job, unix_now()).boxed()
            } else {
                tasks::run_job(repo, job).boxed()
            }
        })
        .flatten();
        Task::run(lazy, move |event| Message::Job {
            repo: for_msg.clone(),
            event,
        })
    }

    /// Charge les inputs racine du `flake.lock` (lecture de fichier, hors du fil de l'interface).
    fn load_inputs_task(&self, repo: &Path) -> Task<Message> {
        let p = repo.to_path_buf();
        if self.cli.demo {
            return Task::done(Message::InputsLoaded(p, Ok(demo::inputs(unix_now()))));
        }
        let r = p.clone();
        Task::perform(
            tasks::blocking(move || tasks::load_inputs(&r)),
            move |res| {
                Message::InputsLoaded(
                    p.clone(),
                    res.unwrap_or_else(|| Err("lecture interrompue".into())),
                )
            },
        )
    }

    fn start_next_in_queue(&mut self) -> Task<Message> {
        match model::pick_next(&mut self.queue, &self.entries) {
            Some(p) => self.start_job(&p, Job::Check),
            None => Task::none(),
        }
    }

    fn begin_check_all(&mut self) -> Task<Message> {
        if !self.queue.is_empty() {
            if self.entries.iter().any(|e| e.busy == Some(JobKind::Check)) {
                self.toast(ToastKind::Info, "Vérification déjà en cours");
                return Task::none();
            }
            // File périmée (plus aucune vérification ne tourne) : on repart de zéro.
            self.queue.clear();
        }
        self.queue = model::check_all_targets(&self.entries).into();
        if self.queue.is_empty() {
            self.push_log(LogKind::Info, "Aucun dépôt suivi à vérifier.");
            return Task::none();
        }
        self.start_next_in_queue()
    }

    /// Lance la mise à jour de `inputs` (vide = tous) ; propose d'abord de commiter si l'arbre est sale.
    fn begin_update(&mut self, repo: &Path, inputs: Vec<String>) -> Task<Message> {
        let Some(entry) = self.entry(repo) else {
            return Task::none();
        };
        if !model::actions(entry).update {
            return Task::none();
        }
        match model::plan_update(&entry.git) {
            UpdatePlan::Proceed => self.start_job(repo, update_job(inputs)),
            UpdatePlan::AskCommit => {
                if self.has_dirty_modal(repo) {
                    let name = self.repo_name(repo);
                    self.toast(
                        ToastKind::Info,
                        &format!(
                            "Demande déjà en attente\n{name} attend votre réponse au dialogue de commit."
                        ),
                    );
                    return Task::none();
                }
                let dirty = entry.git.dirty.clone();
                self.update_request.insert(repo.to_path_buf(), inputs);
                self.open_modal(Modal::CommitDirty {
                    repo: repo.to_path_buf(),
                    dirty,
                    message: DIRTY_COMMIT_MESSAGE.to_string(),
                });
                Task::none()
            }
        }
    }

    /// Ouvre un dialogue ; s'il y en a déjà un, le nouveau attend son tour (jamais d'écrasement).
    fn open_modal(&mut self, modal: Modal) {
        if model::enqueue_modal(&mut self.modal, &mut self.modal_queue, modal) {
            self.show_current_modal();
        }
    }

    /// Prépare l'affichage du dialogue courant (éditeur, animation d'entrée).
    fn show_current_modal(&mut self) {
        let text = match &self.modal {
            Some(Modal::CommitDirty { message, .. } | Modal::CommitLock { message, .. }) => {
                message.clone()
            }
            _ => String::new(),
        };
        self.editor = text_editor::Content::with_text(&text);
        self.modal_leaving = None;
        self.modal_t.retarget(1.0, self.now, anim::SLOW);
        self.reset_hover();
    }

    /// Un dialogue de commit pour ce dépôt est-il ouvert ou en attente ?
    fn has_dirty_modal(&self, repo: &Path) -> bool {
        self.modal
            .iter()
            .chain(self.modal_queue.iter())
            .any(|m| matches!(m, Modal::CommitDirty { repo: r, .. } if r == repo))
    }

    /// Ferme le dialogue (animation de sortie), le renvoie, puis affiche le suivant de la file.
    fn take_modal(&mut self) -> Option<Modal> {
        let m = self.modal.take();
        if m.is_some() {
            self.modal_leaving = m.clone();
            self.modal_t.retarget(0.0, self.now, anim::BASE);
            self.reset_hover();
            if model::advance_modal(&mut self.modal, &mut self.modal_queue).is_some() {
                self.show_current_modal();
                // Garde : un double Entrée rapide ne doit pas confirmer le dialogue suivant.
                self.modal_guard_until = Some(self.now + MODAL_ENTER_GUARD);
            }
        }
        m
    }

    // -- config -------------------------------------------------------------

    /// Enregistre la config hors du fil de l'interface ; `ok` : toast de succès éventuel.
    /// En démo rien n'est écrit (le toast de succès est tout de même montré).
    fn save_config(&mut self, ok: Option<&str>) -> Task<Message> {
        if self.cli.demo {
            if let Some(t) = ok {
                self.toast(ToastKind::Success, t);
            }
            return Task::none();
        }
        let (cfg, path) = (self.cfg.clone(), self.cfg_path.clone());
        let ok = ok.map(str::to_string);
        let counter = Arc::clone(&self.save_gen);
        let generation = counter.fetch_add(1, Ordering::SeqCst) + 1;
        let snapshot = Box::new(cfg.clone());
        Task::perform(
            tasks::blocking(move || tasks::save_config(&counter, generation, &cfg, &path)),
            move |r| {
                Message::ConfigSaved(
                    r.unwrap_or_else(|| Err("enregistrement interrompu".to_string())),
                    ok,
                    snapshot,
                    generation,
                )
            },
        )
    }

    /// Dépôt système effectif (configuration utilisateur, environnement, défaut système).
    pub fn system_repo_path(&self) -> Option<PathBuf> {
        self.cfg.system_repo_resolved().map(|r| r.path)
    }

    /// Attribut `nixosConfigurations` à appliquer pour `repo` (seulement le dépôt système).
    fn apply_attr(&self, repo: &Path) -> Option<String> {
        (self.cfg.role(repo) == Role::System)
            .then(|| self.cfg.system_repo_resolved().and_then(|r| r.attr))
            .flatten()
    }

    fn refresh_roles(&mut self) {
        for e in &mut self.entries {
            e.role = self.cfg.role(&e.path);
        }
    }

    // -- mise à jour --------------------------------------------------------

    pub fn update(&mut self, message: Message) -> Task<Message> {
        // Horloge monotone (les tests peuvent l'avancer à la main).
        self.now = self.now.max(Instant::now());
        self.update_inner(message)
    }

    fn update_inner(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Rescan => self.scan_task(),
            Message::ScanDone(seq, res) => {
                if seq != self.scan_seq {
                    // Résultat d'un scan périmé : un scan plus récent est en cours ou terminé.
                    return Task::none();
                }
                self.scanning = false;
                let fresh = match res {
                    Ok(f) => f,
                    Err(e) => {
                        // Échec : on garde la liste précédente au lieu de vider les dépôts.
                        self.push_log(LogKind::Error, format!("Scan impossible : {e}"));
                        self.toast(
                            ToastKind::Error,
                            &format!("Scan impossible\n{e}\nLa liste précédente est conservée."),
                        );
                        return Task::none();
                    }
                };
                let fresh: Vec<RepoStatus> = fresh
                    .into_iter()
                    .map(|mut s| {
                        s.role = self.cfg.role(&s.path);
                        s
                    })
                    .collect();
                let count = fresh.len();
                self.entries = model::merge_scan(&self.entries, fresh);
                self.entries.sort_by(|a, b| a.path.cmp(&b.path));
                let still_there = self
                    .selected
                    .as_deref()
                    .is_some_and(|p| self.entry(p).is_some());
                if !still_there {
                    self.selected = model::group_entries(&self.entries)
                        .first()
                        .and_then(|(_, idx)| idx.first())
                        .map(|&i| self.entries[i].path.clone());
                }
                self.push_log(
                    LogKind::Info,
                    format!("Scan terminé : {count} dépôt(s) flake."),
                );
                self.sync_selection_fades();
                let load = self
                    .selected
                    .clone()
                    .map_or_else(Task::none, |p| self.load_inputs_task(&p));
                if self.cli.demo {
                    return Task::batch([load, self.apply_demo_views()]);
                }
                load
            }
            Message::Select(p) => {
                let p_loaded = p.clone();
                if self.selected.as_ref() != Some(&p) {
                    self.restart_panel();
                }
                self.selected = Some(p);
                self.sidebar_open = false;
                self.page = Page::Main;
                self.sync_selection_fades();
                self.reset_hover();
                self.load_inputs_task(&p_loaded)
            }
            Message::Check(p) => self.start_job(&p, Job::Check),
            Message::CheckAll => self.begin_check_all(),
            Message::Update(p) => self.begin_update(&p, Vec::new()),
            Message::UpdateInputs(p, names) => self.begin_update(&p, names),
            Message::UpdateSelection(p) => {
                let names: Vec<String> = self
                    .input_sel
                    .get(&p)
                    .map(|s| s.iter().cloned().collect())
                    .unwrap_or_default();
                if names.is_empty() {
                    return Task::none();
                }
                self.begin_update(&p, names)
            }
            Message::ToggleInput(p, name) => {
                model::toggle_selection(self.input_sel.entry(p).or_default(), &name);
                Task::none()
            }
            Message::SelectInputs(p, how) => {
                let inputs = self.inputs.get(&p).map(Vec::as_slice).unwrap_or_default();
                let sel = match how {
                    InputSelect::All => model::select_all(inputs),
                    InputSelect::NoneSelected => BTreeSet::new(),
                    InputSelect::Updatable => {
                        let pending = self.entry(&p).and_then(|e| e.pending.as_deref());
                        model::updatable_names(inputs, pending.unwrap_or_default())
                    }
                };
                self.input_sel.insert(p, sel);
                Task::none()
            }
            Message::InputsLoaded(p, res) => {
                match res {
                    Ok(list) => {
                        if let Some(sel) = self.input_sel.get_mut(&p) {
                            model::prune_selection(sel, &list);
                        }
                        self.inputs.insert(p, list);
                    }
                    Err(e) => {
                        self.inputs.remove(&p);
                        self.push_log(LogKind::Error, format!("Inputs illisibles : {e}"));
                        let name = self.repo_name(&p);
                        self.toast(
                            ToastKind::Error,
                            &format!("Inputs illisibles\n{name} : {e}"),
                        );
                    }
                }
                Task::none()
            }
            Message::Apply(p) => {
                if self.actions_for(&p).apply {
                    let attr = self.apply_attr(&p);
                    self.start_job(&p, Job::Apply(attr))
                } else {
                    Task::none()
                }
            }
            Message::Push(p) => {
                if self.actions_for(&p).push {
                    self.start_job(&p, Job::Push)
                } else {
                    Task::none()
                }
            }
            Message::Ask(action) => self.ask(action),
            Message::Role(p, action) => {
                let name = self.repo_name(&p);
                if let Some(busy) = self.entry(&p).and_then(|e| e.busy) {
                    self.toast(
                        ToastKind::Error,
                        &format!(
                            "{name} est occupé\n{} en cours : changement de rôle impossible pour l'instant.",
                            busy.label()
                        ),
                    );
                    return Task::none();
                }
                self.cfg.set_role(&p, action.target());
                self.refresh_roles();
                let ok = format!(
                    "Rôle modifié\n{name} : {}",
                    model::role_singular(action.target())
                );
                self.save_config(Some(&ok))
            }
            Message::ModalConfirm => self.on_modal_confirm(),
            Message::Job { repo, event } => self.on_job_event(repo, event),
            Message::EditorAction(action) => {
                self.editor.perform(action);
                let text = self.editor.text();
                if let Some(msg) = self.modal.as_mut().and_then(Modal::message_mut) {
                    *msg = text;
                }
                Task::none()
            }
            Message::ModalCommit => self.on_modal_commit(),
            Message::ModalContinue => {
                let Some(Modal::CommitDirty { repo, .. }) = self.modal.clone() else {
                    return Task::none();
                };
                match self.gate(Some(&repo)) {
                    Gate::Wait => Task::none(),
                    Gate::Gone => {
                        self.take_modal();
                        self.update_request.remove(&repo);
                        Task::none()
                    }
                    Gate::Go => {
                        self.take_modal();
                        let inputs = self.update_request.remove(&repo).unwrap_or_default();
                        self.start_job(&repo, update_job(inputs))
                    }
                }
            }
            Message::ModalCancel => {
                if let Some(Modal::CommitDirty { repo, .. }) = self.take_modal() {
                    self.update_request.remove(&repo);
                }
                Task::none()
            }
            Message::ClearLog => {
                self.log.clear();
                Task::none()
            }
            Message::Noop => Task::none(),
            Message::ToggleConsole => {
                self.console_open = !self.console_open;
                let target = f32::from(u8::from(self.console_open));
                self.console_t.retarget(target, self.now, anim::SLOW);
                Task::none()
            }
            Message::ToggleSidebar => {
                self.sidebar_open = !self.sidebar_open;
                Task::none()
            }
            Message::OpenSettings => {
                self.page = Page::Settings;
                self.sidebar_open = false;
                self.restart_panel();
                self.reset_hover();
                Task::none()
            }
            Message::OpenSystemRepoSettings => {
                self.page = Page::Settings;
                self.sidebar_open = false;
                self.restart_panel();
                self.reset_hover();
                Task::batch([
                    iced::widget::operation::focus(SYSTEM_REPO_INPUT),
                    Task::perform(
                        tasks::blocking(|| std::thread::sleep(Duration::from_millis(40))),
                        |_| Message::FocusSystemRepo,
                    ),
                ])
            }
            Message::FocusSystemRepo => iced::widget::operation::focus(SYSTEM_REPO_INPUT),
            Message::GoMain => {
                self.page = Page::Main;
                self.restart_panel();
                self.reset_hover();
                Task::none()
            }
            Message::OpenHelp => {
                self.help_open = true;
                self.reset_hover();
                Task::none()
            }
            Message::CloseHelp => {
                self.help_open = false;
                self.reset_hover();
                Task::none()
            }
            Message::ConfigSaved(res, ok, saved, generation) => {
                match res {
                    Ok(written) => {
                        if written {
                            self.cfg_persisted = *saved;
                        }
                        if let Some(t) = ok {
                            self.toast(ToastKind::Success, &t);
                        }
                    }
                    Err(e) => {
                        self.push_log(
                            LogKind::Error,
                            format!("Enregistrement de la configuration impossible : {e}"),
                        );
                        self.toast(ToastKind::Error, &format!("Enregistrement impossible\n{e}"));
                        // L'affichage doit refléter le disque : retour à la dernière config
                        // écrite (sauf si un enregistrement plus récent est encore en vol).
                        if generation == self.save_gen.load(Ordering::SeqCst) {
                            self.cfg = self.cfg_persisted.clone();
                            self.sys_repo_input = settings::system_repo_prefill(&self.cfg);
                            self.sys_attr_input = self.cfg.system_attr.clone().unwrap_or_default();
                            self.sys_error = None;
                            self.refresh_roles();
                            return self.scan_task();
                        }
                    }
                }
                Task::none()
            }
            Message::TokenLoaded(saved, source) => {
                self.token_saved = saved;
                self.token_source = source;
                Task::none()
            }
            Message::TokenSaved(res) => self.on_token_saved(res),
            Message::TokenCleared(res) => self.on_token_cleared(res),
            Message::ModeDetected(m) => {
                if let Some(m) = m {
                    self.set_theme_mode(m);
                }
                Task::none()
            }
            Message::Settings(msg) => self.on_settings(msg),
            Message::Packages(msg) => self.on_packages(msg),
            Message::SystemTheme(m) => {
                if let Some(m) = theme::from_iced(m) {
                    self.set_theme_mode(m);
                }
                Task::none()
            }
            Message::CloseRequested => iced::exit(),
            Message::WindowClosed(id) => {
                if self.window == Some(id) {
                    self.window = None;
                }
                Task::none()
            }
            Message::Daemon(status) => {
                self.daemon = status;
                Task::none()
            }
            Message::DaemonCheckNow => {
                if self.cli.demo {
                    self.toast(ToastKind::Info, "Mode démo : pas de démon");
                    return Task::none();
                }
                if self.daemon.is_none() {
                    self.toast(
                        ToastKind::Error,
                        "Démon absent\nLancez nixos-flakes daemon (ou son service), puis réessayez.",
                    );
                    return Task::none();
                }
                Task::perform(daemon_link::check_now(), Message::DaemonCheckSent)
            }
            Message::DaemonCheckSent(res) => {
                match res {
                    Ok(()) => self.toast(ToastKind::Info, "Vérification demandée au démon"),
                    Err(e) => {
                        self.toast(ToastKind::Error, "Démon injoignable");
                        self.push_log(LogKind::Error, format!("Démon injoignable : {e}"));
                    }
                }
                Task::none()
            }
            Message::SyncFetch => self.sync_task(false),
            Message::SyncPull => self.sync_task(true),
            Message::SyncDone(res) => self.on_sync_done(res),
            Message::OpenTerminal => {
                self.page = Page::Terminal;
                self.sidebar_open = false;
                self.restart_panel();
                self.reset_hover();
                if self.terminal.is_none() && self.terminal_error.is_none() {
                    self.start_terminal();
                }
                Task::none()
            }
            Message::RestartTerminal => {
                self.start_terminal();
                Task::none()
            }
            Message::Terminal(iced_term::Event::BackendCall(_, cmd)) => {
                let Some(term) = self.terminal.as_mut() else {
                    return Task::none();
                };
                if term.handle(iced_term::Command::ProxyToBackend(cmd))
                    == iced_term::actions::Action::Shutdown
                {
                    // Le shell s'est terminé : on libère le terminal, l'onglet propose de le relancer.
                    self.terminal = None;
                }
                Task::none()
            }
            Message::SetTab(t) => {
                if self.tab != t {
                    self.tab = t;
                    self.restart_panel();
                    self.reset_hover();
                }
                Task::none()
            }
            Message::ToggleGroup(role) => {
                let open = self.collapsed.remove(&role);
                if !open {
                    self.collapsed.insert(role);
                }
                let key = format!("grp:{}", model::role_title(role));
                self.fades
                    .set(&key, f32::from(u8::from(open)), 1.0, self.now, anim::BASE);
                Task::none()
            }
            Message::Frame => {
                self.toasts.tick(self.now);
                self.fades.prune(self.now);
                if self.modal.is_none() && !self.modal_t.is_animating(self.now) {
                    self.modal_leaving = None;
                }
                Task::none()
            }
            Message::Hover(key, on) => {
                self.fades.set(
                    &format!("hov:{key}"),
                    f32::from(u8::from(on)),
                    0.0,
                    self.now,
                    anim::FAST,
                );
                Task::none()
            }
            Message::DismissToast(id) => {
                self.toasts.dismiss(id, self.now);
                Task::none()
            }
            Message::OpenPalette => self.open_palette(),
            Message::ClosePalette => {
                self.close_palette();
                Task::none()
            }
            Message::PaletteInput(q) => {
                self.palette.set_query(q);
                self.sync_palette_fades();
                self.scroll_palette_to_selection()
            }
            Message::PaletteRun(i) => self.run_palette(i),
            Message::FocusPalette => {
                if self.palette_open {
                    iced::widget::operation::focus(PALETTE_INPUT)
                } else {
                    Task::none()
                }
            }
            Message::Key(key, mods, captured) => self.on_key(&key, mods, captured),
        }
    }

    /// Lecture asynchrone des Fichiers de config du Dépôt système (fausse config en démo).
    fn packages_load_task(&mut self) -> Task<Message> {
        if self.cli.demo {
            self.packages.set_files(demo::config_files());
            return Task::none();
        }
        let Some(repo) = self.system_repo_path() else {
            return Task::none();
        };
        self.packages.loading = true;
        Task::perform(tasks::blocking(move || packages::load_repo(&repo)), |r| {
            Message::Packages(packages::Msg::Loaded(
                r.unwrap_or_else(|| Err("lecture interrompue".to_string())),
            ))
        })
    }

    fn packages_search_task(&self, seq: u64, query: String) -> Task<Message> {
        let demo = self.cli.demo;
        Task::perform(
            tasks::blocking(move || {
                if demo {
                    std::thread::sleep(Duration::from_millis(250));
                    Ok(demo::package_hits(&query))
                } else {
                    nixos_flakes_config::search(&query, packages::SEARCH_LIMIT)
                        .map_err(|e| e.to_string())
                }
            }),
            move |r| {
                Message::Packages(packages::Msg::SearchDone(
                    seq,
                    r.unwrap_or_else(|| Err("recherche interrompue".to_string())),
                ))
            },
        )
    }

    fn on_packages(&mut self, msg: packages::Msg) -> Task<Message> {
        use packages::{Msg, Op};
        match msg {
            Msg::Open => {
                self.page = Page::Packages;
                self.sidebar_open = false;
                self.restart_panel();
                self.reset_hover();
                if self.packages.loaded || self.packages.loading {
                    Task::none()
                } else {
                    self.packages_load_task()
                }
            }
            Msg::Back => {
                self.page = Page::Main;
                self.restart_panel();
                self.reset_hover();
                Task::none()
            }
            Msg::Query(q) => {
                self.packages.query = q;
                Task::none()
            }
            Msg::ClearQuery => {
                self.packages.query.clear();
                // Une requête vide remet aussi à zéro les résultats et l'erreur.
                let _ = self.packages.begin_search();
                iced::widget::operation::focus(packages::SEARCH_INPUT)
            }
            Msg::Search => match self.packages.begin_search() {
                Some((seq, q)) => self.packages_search_task(seq, q),
                None => Task::none(),
            },
            Msg::SearchDone(seq, res) => {
                self.packages.finish_search(seq, res);
                Task::none()
            }
            Msg::Reload => self.packages_load_task(),
            Msg::Loaded(res) => {
                match res {
                    Ok(files) => self.packages.set_files(files),
                    Err(e) => {
                        self.push_log(LogKind::Error, format!("Paquets : {e}"));
                        self.toast(
                            ToastKind::Error,
                            &format!("Lecture du dépôt impossible\n{e}"),
                        );
                        self.packages.set_load_error(e);
                    }
                }
                Task::none()
            }
            Msg::Add(i) => {
                if let Some(h) = self.packages.results.get(i) {
                    let name = h.attr.clone();
                    self.packages.begin_op(Op::Add { name });
                }
                Task::none()
            }
            Msg::Remove(file, loc, name) => {
                self.packages.begin_op(Op::Remove { file, loc, name });
                Task::none()
            }
            Msg::Move(file, loc, name) => {
                self.packages.begin_op(Op::Move { file, loc, name });
                Task::none()
            }
            Msg::PickFile(i) => {
                self.packages.pick_file(i);
                Task::none()
            }
            Msg::PickLoc(j) => {
                self.packages.pick_loc(j);
                Task::none()
            }
            Msg::Cancel => {
                self.packages.cancel();
                Task::none()
            }
            Msg::Confirm => {
                let Some(changes) = self.packages.begin_write() else {
                    return Task::none();
                };
                if self.cli.demo {
                    // Démo : la fausse config est modifiée en mémoire, rien n'est écrit.
                    let r = self.packages.apply_in_memory(&changes);
                    return self.update_inner(Message::Packages(Msg::Written(r)));
                }
                Task::perform(
                    tasks::blocking(move || packages::write_changes(&changes)),
                    |r| {
                        Message::Packages(Msg::Written(
                            r.unwrap_or_else(|| Err("écriture interrompue".to_string())),
                        ))
                    },
                )
            }
            Msg::Written(res) => {
                let ok = res.is_ok();
                match &res {
                    Ok(()) => {
                        self.push_log(LogKind::Success, "Paquets : configuration modifiée.");
                        self.toast(
                            ToastKind::Success,
                            "Configuration modifiée. Rien n'est appliqué.",
                        );
                    }
                    Err(e) => {
                        self.push_log(LogKind::Error, format!("Paquets : {e}"));
                        self.toast(ToastKind::Error, &format!("Écriture impossible\n{e}"));
                    }
                }
                self.packages.finish_write(res);
                if ok && !self.cli.demo {
                    return self.packages_load_task();
                }
                Task::none()
            }
        }
    }

    fn on_settings(&mut self, msg: settings::Msg) -> Task<Message> {
        use settings::Msg;
        match msg {
            Msg::InputChanged(s) => {
                self.new_root = s;
                self.root_error = None;
            }
            Msg::ClearInput => {
                self.new_root.clear();
                self.root_error = None;
            }
            Msg::AddRoot => {
                let input = self.new_root.clone();
                match settings::add_root(&mut self.cfg, &input, self.home.as_deref()) {
                    Ok(root) => {
                        self.new_root.clear();
                        self.root_error = None;
                        let ok = format!("Racine ajoutée\n{}", root.display());
                        return Task::batch([self.save_config(Some(&ok)), self.scan_task()]);
                    }
                    Err(e) => {
                        // Le texte saisi est conservé ; l'erreur s'affiche sous le champ.
                        self.push_log(LogKind::Error, format!("Racine refusée : {e}"));
                        self.root_error = Some(e);
                    }
                }
            }
            Msg::AskRemoveRoot(i) => {
                if let Some(root) = self.cfg.scan_roots.get(i).cloned() {
                    self.open_modal(Modal::Confirm(ConfirmAction::RemoveRoot(root)));
                }
            }
            Msg::RemoveRoot(i) => {
                let root = self.cfg.scan_roots.get(i).cloned();
                if settings::remove_root(&mut self.cfg, i) {
                    let ok = format!(
                        "Racine retirée\n{}",
                        root.map(|r| r.display().to_string()).unwrap_or_default()
                    );
                    return Task::batch([self.save_config(Some(&ok)), self.scan_task()]);
                }
            }
            Msg::Depth(up) => {
                self.cfg.scan_depth = settings::step_depth(self.cfg.scan_depth, up);
                let ok = format!("Profondeur de scan : {}", self.cfg.scan_depth);
                return self.save_config(Some(&ok));
            }
            Msg::Interval(up) => {
                self.cfg.check_interval_minutes =
                    settings::step_interval(self.cfg.check_interval_minutes, up);
                let ok = format!(
                    "Intervalle de vérification : {}",
                    settings::interval_label(self.cfg.check_interval_minutes)
                );
                return self.save_config(Some(&ok));
            }
            Msg::TokenInput(s) => self.token_input = s.0,
            Msg::TokenSave => return self.save_token(),
            Msg::TokenClear => return self.clear_token(),
            Msg::Rescan => return self.scan_task(),
            Msg::SysRepoInput(s) => {
                self.sys_repo_input = s;
                self.sys_error = settings::system_form_error(
                    &self.sys_repo_input,
                    &self.sys_attr_input,
                    self.home.as_deref(),
                );
            }
            Msg::SysAttrInput(s) => {
                self.sys_attr_input = s;
                self.sys_error = settings::system_form_error(
                    &self.sys_repo_input,
                    &self.sys_attr_input,
                    self.home.as_deref(),
                );
            }
            Msg::SysRepoSave => {
                match settings::apply_system_repo(
                    &mut self.cfg,
                    &self.sys_repo_input,
                    &self.sys_attr_input,
                    self.home.as_deref(),
                ) {
                    Ok(path) => {
                        self.sys_error = None;
                        self.sys_repo_input = path.display().to_string();
                        self.refresh_roles();
                        let ok = format!("Dépôt système enregistré\n{}", path.display());
                        return Task::batch([self.save_config(Some(&ok)), self.scan_task()]);
                    }
                    Err(e) => {
                        self.push_log(LogKind::Error, format!("Dépôt système refusé : {e}"));
                        self.sys_error = Some(e);
                    }
                }
            }
            Msg::SysRepoReset => {
                if settings::reset_system_repo(&mut self.cfg) {
                    self.sys_repo_input = settings::system_repo_prefill(&self.cfg);
                    self.sys_attr_input.clear();
                    self.sys_error = None;
                    self.refresh_roles();
                    let ok = match self.cfg.system_repo_resolved() {
                        Some(r) => format!("Dépôt système réinitialisé\n{}", r.path.display()),
                        None => "Dépôt système réinitialisé\nAucun défaut système".to_string(),
                    };
                    return Task::batch([self.save_config(Some(&ok)), self.scan_task()]);
                }
            }
            Msg::Back => self.page = Page::Main,
        }
        Task::none()
    }

    /// Enregistre le jeton saisi hors du fil de l'interface. La saisie n'est vidée qu'une fois
    /// l'enregistrement réussi (sinon l'utilisateur la perdrait sans rien voir).
    fn save_token(&mut self) -> Task<Message> {
        let token = self.token_input.trim().to_string();
        if token.is_empty()
            || self.token_busy
            || matches!(self.token_source, TokenSource::Fichier(_))
        {
            return Task::none();
        }
        if self.cli.demo {
            self.token_input.clear();
            self.toast(ToastKind::Info, "Mode démo : jeton non enregistré");
            return Task::none();
        }
        self.token_busy = true;
        Task::perform(
            tasks::blocking(move || {
                nixos_flakes_core::token::save(&token).map_err(|e| e.to_string())
            }),
            |r| Message::TokenSaved(r.unwrap_or_else(|| Err("enregistrement interrompu".into()))),
        )
    }

    fn on_token_saved(&mut self, res: Result<(), String>) -> Task<Message> {
        self.token_busy = false;
        match res {
            Ok(()) => {
                self.token_input.clear();
                self.token_saved = true;
                self.token_source = TokenSource::Interface;
                self.push_log(LogKind::Success, "Jeton GitHub enregistré.");
                self.toast(ToastKind::Success, "Jeton GitHub enregistré");
                self.begin_check_all()
            }
            Err(e) => {
                self.push_log(LogKind::Error, format!("Jeton GitHub non enregistré : {e}"));
                self.toast(
                    ToastKind::Error,
                    &format!("Jeton non enregistré\n{e}\nVotre saisie est conservée."),
                );
                Task::none()
            }
        }
    }

    fn clear_token(&mut self) -> Task<Message> {
        self.token_input.clear();
        if self.token_busy
            || matches!(
                self.token_source,
                TokenSource::Fichier(_) | TokenSource::Systeme { .. }
            )
        {
            return Task::none();
        }
        if self.cli.demo {
            self.token_saved = false;
            self.toast(ToastKind::Success, "Jeton GitHub effacé");
            return Task::none();
        }
        self.token_busy = true;
        Task::perform(
            tasks::blocking(|| {
                nixos_flakes_core::token::clear().map_err(|e| e.to_string())?;
                // Le fichier système peut reprendre le relais une fois le jeton effacé.
                Ok((
                    nixos_flakes_core::token::exists(),
                    nixos_flakes_core::token::source(),
                ))
            }),
            |r| Message::TokenCleared(r.unwrap_or_else(|| Err("effacement interrompu".into()))),
        )
    }

    fn on_token_cleared(&mut self, res: Result<(bool, TokenSource), String>) -> Task<Message> {
        self.token_busy = false;
        match res {
            Ok((saved, source)) => {
                self.token_saved = saved;
                self.token_source = source;
                self.push_log(LogKind::Info, "Jeton GitHub effacé.");
                self.toast(ToastKind::Success, "Jeton GitHub effacé");
            }
            Err(e) => {
                self.push_log(LogKind::Error, format!("Jeton GitHub non effacé : {e}"));
                self.toast(ToastKind::Error, &format!("Jeton non effacé\n{e}"));
            }
        }
        Task::none()
    }

    /// Le dépôt visé peut-il recevoir une commande maintenant ? Sinon, refus visible (toast) :
    /// `Wait` = occupé (le dialogue reste ouvert), `Gone` = dépôt disparu (le dialogue se ferme).
    fn gate(&mut self, repo: Option<&Path>) -> Gate {
        let Some(repo) = repo else {
            return Gate::Go;
        };
        let name = self.repo_name(repo);
        match self.entry(repo).map(|e| e.busy) {
            None => {
                self.toast(
                    ToastKind::Error,
                    &format!("Dépôt introuvable\n{name} n'est plus dans la liste."),
                );
                Gate::Gone
            }
            Some(Some(busy)) => {
                self.toast(
                    ToastKind::Error,
                    &format!(
                        "{name} est occupé\n{} en cours : réessayez quand elle sera terminée.",
                        busy.label()
                    ),
                );
                Gate::Wait
            }
            Some(None) => Gate::Go,
        }
    }

    /// Demande une action lourde : confirmation par dialogue, sauf changement de rôle anodin.
    fn ask(&mut self, action: ConfirmAction) -> Task<Message> {
        let allowed = match &action {
            ConfirmAction::Apply(p) => self.actions_for(p).apply,
            ConfirmAction::Push(p) => self.actions_for(p).push,
            ConfirmAction::Role(p, _) => self.entry(p).is_none_or(|e| e.busy.is_none()),
            ConfirmAction::RemoveRoot(_) => true,
        };
        if !allowed {
            // Occupé : le refus est expliqué ; sinon l'action n'est simplement pas disponible.
            if let Some(busy) = action
                .repo()
                .and_then(|p| self.entry(p))
                .and_then(|e| e.busy)
            {
                let name = self.repo_name(action.repo().unwrap_or(Path::new("")));
                self.toast(
                    ToastKind::Error,
                    &format!(
                        "{name} est occupé\n{} en cours : réessayez ensuite.",
                        busy.label()
                    ),
                );
            }
            return Task::none();
        }
        if let ConfirmAction::Role(p, a) = &action {
            if !a.needs_confirm() {
                return self.update_inner(Message::Role(p.clone(), *a));
            }
        }
        self.open_modal(Modal::Confirm(action));
        Task::none()
    }

    /// Valide le dialogue de confirmation et exécute l'action.
    fn on_modal_confirm(&mut self) -> Task<Message> {
        let Some(Modal::Confirm(action)) = self.modal.clone() else {
            return Task::none();
        };
        match self.gate(action.repo()) {
            Gate::Wait => return Task::none(),
            Gate::Gone => {
                self.take_modal();
                return Task::none();
            }
            Gate::Go => {}
        }
        self.take_modal();
        match action {
            ConfirmAction::Apply(p) => self.update_inner(Message::Apply(p)),
            ConfirmAction::Push(p) => self.update_inner(Message::Push(p)),
            ConfirmAction::Role(p, a) => self.update_inner(Message::Role(p, a)),
            ConfirmAction::RemoveRoot(root) => {
                match self.cfg.scan_roots.iter().position(|r| *r == root) {
                    Some(i) => self.on_settings(settings::Msg::RemoveRoot(i)),
                    None => Task::none(),
                }
            }
        }
    }

    fn on_modal_commit(&mut self) -> Task<Message> {
        let Some(modal) = self.modal.clone() else {
            return Task::none();
        };
        let (repo, message, dirty) = match &modal {
            Modal::CommitDirty { repo, message, .. } => (repo.clone(), message.clone(), true),
            Modal::CommitLock { repo, message } => (repo.clone(), message.clone(), false),
            Modal::Confirm(_) => return self.on_modal_confirm(),
        };
        if !model::commit_message_ok(&message) {
            self.toast(
                ToastKind::Error,
                "Message de commit vide\nSaisissez un message pour committer.",
            );
            return Task::none();
        }
        // Occupé ou disparu : refus visible AVANT de fermer le dialogue ou de consommer la requête.
        match self.gate(Some(&repo)) {
            Gate::Wait => return Task::none(),
            Gate::Gone => {
                self.take_modal();
                self.update_request.remove(&repo);
                return Task::none();
            }
            Gate::Go => {}
        }
        self.take_modal();
        let message = message.trim_end().to_string();
        if dirty {
            self.followups.insert(repo.clone(), Followup::Update);
            self.start_job(&repo, Job::Commit(message))
        } else {
            self.start_job(&repo, Job::CommitLock(message))
        }
    }

    fn on_job_event(&mut self, repo: PathBuf, event: JobEvent) -> Task<Message> {
        let name = self.repo_name(&repo);
        match event {
            JobEvent::Line(l) => {
                self.push_log(LogKind::Info, l);
                Task::none()
            }
            JobEvent::Finished(f) => self.on_finished(repo, name, *f),
        }
    }

    fn on_finished(&mut self, repo: PathBuf, name: String, f: Finished) -> Task<Message> {
        let Finished { outcome, git } = f;
        let followup = self.followups.remove(&repo);
        let is_check = matches!(
            outcome,
            Outcome::Check(_) | Outcome::Aborted(JobKind::Check, _)
        );
        let Some(entry) = self.entry_mut(&repo) else {
            // Le dépôt a disparu pendant la commande : on nettoie tout et la file continue.
            self.update_request.remove(&repo);
            self.push_log(
                LogKind::Info,
                format!("{name} : dépôt disparu de la liste, résultat ignoré."),
            );
            return if is_check {
                self.start_next_in_queue()
            } else {
                Task::none()
            };
        };
        entry.busy = None;
        entry.git = git.clone();
        let mut next = Task::none();
        let mut logs: Vec<(LogKind, String)> = Vec::new();
        let mut modal = None;
        let mut reload_inputs = false;
        // Une requête de mise à jour en attente d'un commit n'a plus lieu d'être.
        let mut drop_request = false;

        match outcome {
            Outcome::Check(res) => match res {
                Ok(report) => {
                    let n = report.changes.len();
                    entry.pending = Some(report.changes);
                    entry.error = None;
                    logs.push((
                        LogKind::Success,
                        if n == 0 {
                            format!("✓ {name} : à jour")
                        } else {
                            format!("✓ {name} : {n} mise(s) à jour disponible(s)")
                        },
                    ));
                }
                Err(e) => {
                    entry.error = Some(e.clone());
                    logs.push((
                        LogKind::Error,
                        format!("✕ {name} : vérification échouée\n{e}"),
                    ));
                }
            },
            Outcome::Update(inputs, res) => match res {
                Ok(report) => {
                    entry.error = None;
                    entry.pending =
                        model::remaining_pending(entry.pending.take(), &inputs, &report.changes);
                    entry.last_update = Some(report.changes.clone());
                    logs.push((
                        LogKind::Success,
                        model::update_done_text(&name, &inputs, report.changes.len()),
                    ));
                    reload_inputs = true;
                    if model::should_propose_lock_commit(&git) {
                        modal = Some(Modal::CommitLock {
                            repo: repo.clone(),
                            message: git::lock_commit_message(&report.changes),
                        });
                    }
                }
                Err(e) => {
                    entry.error = Some(e.clone());
                    let which = if inputs.is_empty() {
                        String::new()
                    } else {
                        format!(" de {}", model::inputs_text(&inputs))
                    };
                    logs.push((
                        LogKind::Error,
                        format!("✕ {name} : mise à jour{which} échouée\n{e}"),
                    ));
                }
            },
            Outcome::Done(kind, res) => match res {
                Ok(()) => {
                    entry.error = None;
                    let hint = if kind == JobKind::CommitLock {
                        " — pensez à pousser (bouton Pousser)"
                    } else {
                        ""
                    };
                    logs.push((
                        LogKind::Success,
                        format!("✓ {name} : {} terminé{hint}", kind.label()),
                    ));
                    if kind == JobKind::Commit && followup == Some(Followup::Update) {
                        let inputs = self.update_request.remove(&repo).unwrap_or_default();
                        next = Task::done(Message::UpdateInputs(repo.clone(), inputs));
                    }
                }
                Err(e) => {
                    entry.error = Some(e.clone());
                    drop_request = kind == JobKind::Commit;
                    logs.push((
                        LogKind::Error,
                        format!("✕ {name} : {} échoué\n{e}", kind.label()),
                    ));
                }
            },
            Outcome::Aborted(kind, detail) => {
                let why = Outcome::aborted_text(&detail);
                entry.error = Some(why.clone());
                drop_request = kind == JobKind::Commit;
                logs.push((
                    LogKind::Error,
                    format!("✕ {name} : {} — {why}", kind.label()),
                ));
            }
        }
        if drop_request {
            self.update_request.remove(&repo);
        }

        for (k, t) in logs {
            let kind = match k {
                LogKind::Success => ToastKind::Success,
                LogKind::Error => ToastKind::Error,
                _ => ToastKind::Info,
            };
            self.toast(kind, &t);
            self.push_log(k, t);
        }
        if let Some(m) = modal {
            self.open_modal(m);
        }
        if reload_inputs {
            self.input_sel.remove(&repo);
            next = Task::batch([next, self.load_inputs_task(&repo)]);
        }
        if is_check {
            let more = self.start_next_in_queue();
            return Task::batch([next, more]);
        }
        next
    }

    // -- palette de commandes et clavier ----------------------------------------

    /// Éléments de la palette (actions disponibles puis dépôts), avant filtrage.
    pub fn palette_items(&self) -> Vec<palette::Item> {
        let mut items = Vec::new();
        let action = |command, label: &str, detail: String, kbd| palette::Item {
            command,
            label: label.to_string(),
            detail,
            kbd,
            is_repo: false,
        };
        if let Some(e) = self.selected_entry() {
            let a = model::actions(e);
            let name = e.name();
            if a.check {
                items.push(action(Command::Check, "Vérifier", name.clone(), Some("R")));
            }
            if a.update {
                items.push(action(
                    Command::Update,
                    "Mettre à jour",
                    name.clone(),
                    Some("U"),
                ));
            }
            if a.apply {
                items.push(action(
                    Command::Apply,
                    "Appliquer au système",
                    name.clone(),
                    Some("A"),
                ));
            }
            if a.push {
                items.push(action(Command::Push, "Pousser", name.clone(), None));
            }
            for (on, act, label) in [
                (a.track, RoleAction::Track, "Suivre ce dépôt"),
                (a.untrack, RoleAction::Untrack, "Ne plus suivre ce dépôt"),
                (
                    a.set_system,
                    RoleAction::SetSystem,
                    "Définir comme dépôt système",
                ),
                (a.ignore, RoleAction::Ignore, "Ignorer ce dépôt"),
                (a.unignore, RoleAction::Unignore, "Ne plus ignorer ce dépôt"),
            ] {
                if on {
                    items.push(action(Command::Role(act), label, name.clone(), None));
                }
            }
        }
        items.push(action(
            Command::CheckAll,
            "Tout vérifier",
            String::new(),
            None,
        ));
        items.push(action(Command::Rescan, "Rescanner", String::new(), None));
        items.push(action(Command::Packages, "Paquets", String::new(), None));
        items.push(action(Command::Terminal, "Terminal", String::new(), None));
        items.push(action(
            Command::Settings,
            "Réglages",
            String::new(),
            Some(","),
        ));
        items.push(action(
            Command::SetSystemRepo,
            "Définir le dépôt système…",
            String::new(),
            None,
        ));
        if self.system_repo_path().is_some() && !self.cli.demo {
            if !self.sync.busy() {
                items.push(action(
                    Command::SyncFetch,
                    "Actualiser le dépôt système (fetch)",
                    String::new(),
                    None,
                ));
            }
            let failed = matches!(self.sync, SyncState::Failed(_));
            let behind = matches!(&self.sync, SyncState::Ready(s) if s.behind > 0);
            if (behind || failed) && !self.sync.busy() {
                items.push(action(
                    Command::SyncPull,
                    "Synchroniser le dépôt système (pull)",
                    String::new(),
                    None,
                ));
            }
        }
        if self.daemon.as_ref().is_some_and(|d| !d.checking) {
            items.push(action(
                Command::DaemonCheck,
                "Vérifier via le démon",
                String::new(),
                None,
            ));
        }
        items.push(action(
            Command::Help,
            "Raccourcis clavier",
            String::new(),
            Some("?"),
        ));
        items.push(action(
            Command::ToggleConsole,
            if self.console_open {
                "Masquer la console"
            } else {
                "Afficher la console"
            },
            String::new(),
            None,
        ));
        for (role, idx) in model::group_entries(&self.entries) {
            for i in idx {
                let e = &self.entries[i];
                items.push(palette::Item {
                    command: Command::Goto(e.path.clone()),
                    label: e.name(),
                    detail: model::role_singular(role).to_string(),
                    kbd: None,
                    is_repo: true,
                });
            }
        }
        items
    }

    /// Lignes de la palette pour la saisie courante (toutes : la liste défile).
    pub fn palette_results(&self) -> Vec<palette::Item> {
        palette::filter(&self.palette_items(), &self.palette.query)
    }

    fn open_palette(&mut self) -> Task<Message> {
        if self.modal.is_some() || self.help_open {
            return Task::none();
        }
        self.palette.reset();
        self.sync_palette_fades();
        self.palette_open = true;
        self.palette_t.retarget(1.0, self.now, anim::BASE);
        self.reset_hover();
        // Le champ n'existe dans l'arbre qu'après ce message : on donne le focus tout de suite
        // puis une seconde fois après le premier rendu.
        Task::batch([
            iced::widget::operation::focus(PALETTE_INPUT),
            Task::perform(
                tasks::blocking(|| std::thread::sleep(Duration::from_millis(40))),
                |_| Message::FocusPalette,
            ),
        ])
    }

    /// Met en surbrillance (en fondu) la ligne sélectionnée de la palette.
    fn sync_palette_fades(&mut self) {
        // Toutes les lignes possibles (la liste n'est plus tronquée).
        for i in 0..self.palette_items().len().max(1) {
            let on = f32::from(u8::from(i == self.palette.selected));
            self.fades
                .set(&format!("sel:pal:{i}"), on, 0.0, self.now, anim::FAST);
        }
    }

    /// Fait défiler la liste de la palette pour garder la ligne sélectionnée visible.
    fn scroll_palette_to_selection(&self) -> Task<Message> {
        let results = self.palette_results();
        let sel = self.palette.selected.min(results.len().saturating_sub(1));
        let headings = self.palette.query.trim().is_empty();
        let mut top = sel as f32 * PALETTE_ROW_H;
        if headings {
            // « Actions » en tête, puis « Dépôts » avant le premier dépôt.
            top += PALETTE_HEADING_H;
            if results.iter().take(sel + 1).any(|i| i.is_repo) {
                top += PALETTE_HEADING_H;
            }
        }
        let y = (top - PALETTE_LIST_HEIGHT / 2.0 + PALETTE_ROW_H / 2.0).max(0.0);
        iced::widget::operation::scroll_to(
            PALETTE_LIST,
            iced::widget::operation::AbsoluteOffset { x: 0.0, y },
        )
    }

    fn close_palette(&mut self) {
        if self.palette_open {
            self.palette_open = false;
            self.palette_t.retarget(0.0, self.now, anim::FAST);
            self.reset_hover();
        }
    }

    fn run_palette(&mut self, index: usize) -> Task<Message> {
        let results = self.palette_results();
        let Some(item) = results.get(index).or_else(|| results.last()) else {
            return Task::none();
        };
        let command = item.command.clone();
        self.close_palette();
        let selected = self.selected.clone();
        let msg = match command {
            Command::Goto(p) => Some(Message::Select(p)),
            Command::Check => selected.map(Message::Check),
            Command::Update => selected.map(Message::Update),
            Command::Apply => selected.map(|p| Message::Ask(ConfirmAction::Apply(p))),
            Command::Push => selected.map(|p| Message::Ask(ConfirmAction::Push(p))),
            Command::Role(a) => selected.map(|p| Message::Ask(ConfirmAction::Role(p, a))),
            Command::CheckAll => Some(Message::CheckAll),
            Command::Rescan => Some(Message::Rescan),
            Command::Settings => Some(Message::OpenSettings),
            Command::SetSystemRepo => Some(Message::OpenSystemRepoSettings),
            Command::Packages => Some(Message::Packages(packages::Msg::Open)),
            Command::Terminal => Some(Message::OpenTerminal),
            Command::ToggleConsole => Some(Message::ToggleConsole),
            Command::SyncPull => Some(Message::SyncPull),
            Command::SyncFetch => Some(Message::SyncFetch),
            Command::DaemonCheck => Some(Message::DaemonCheckNow),
            Command::Help => Some(Message::OpenHelp),
        };
        msg.map_or_else(Task::none, |m| self.update_inner(m))
    }

    fn on_key(&mut self, key: &Key, mods: Modifiers, captured: bool) -> Task<Message> {
        let ctx = KeyContext {
            modal: self.modal.is_some(),
            confirm: self.modal.as_ref().is_some_and(Modal::is_confirm),
            palette: self.palette_open,
            help: self.help_open,
            settings: self.page != Page::Main,
            packages: self.page == Page::Packages,
        };
        let Some(shortcut) = keys::resolve(key, mods, captured, ctx) else {
            return Task::none();
        };
        // Dans le terminal, toutes les touches vont au shell, sauf Ctrl K (palette), que le
        // terminal laisse passer (voir `terminal::spawn`) : les autres couches restent actives.
        if self.page == Page::Terminal
            && !self.palette_open
            && self.modal.is_none()
            && !self.help_open
            && shortcut != Shortcut::OpenPalette
        {
            return Task::none();
        }
        // L'aide ne laisse passer que sa propre fermeture.
        if self.help_open && !matches!(shortcut, Shortcut::Escape | Shortcut::Help) {
            return Task::none();
        }
        match shortcut {
            Shortcut::OpenPalette => {
                if self.palette_open {
                    self.close_palette();
                    Task::none()
                } else {
                    self.open_palette()
                }
            }
            Shortcut::Escape => {
                if self.palette_open {
                    self.close_palette();
                    Task::none()
                } else if self.help_open {
                    self.update_inner(Message::CloseHelp)
                } else if self.modal.is_some() {
                    self.update_inner(Message::ModalCancel)
                } else if self.page == Page::Packages && self.packages.flow.is_some() {
                    // Sur Paquets, Échap annule d'abord le flux en cours.
                    self.packages.cancel();
                    Task::none()
                } else if self.page != Page::Main {
                    self.update_inner(Message::GoMain)
                } else {
                    Task::none()
                }
            }
            Shortcut::Help => {
                if self.help_open {
                    self.update_inner(Message::CloseHelp)
                } else {
                    self.update_inner(Message::OpenHelp)
                }
            }
            Shortcut::OpenSettings => {
                if self.page == Page::Settings {
                    Task::none()
                } else {
                    self.update_inner(Message::OpenSettings)
                }
            }
            Shortcut::FocusSearch => iced::widget::operation::focus(packages::SEARCH_INPUT),
            Shortcut::Tab(i) => {
                let tab = [Tab::Updates, Tab::Git, Tab::Repo].get(i).copied();
                match tab {
                    Some(t) if self.selected.is_some() => self.update_inner(Message::SetTab(t)),
                    _ => Task::none(),
                }
            }
            Shortcut::Move(d) => {
                if self.palette_open {
                    let n = self.palette_results().len();
                    self.palette.move_by(d, n);
                    self.sync_palette_fades();
                    return self.scroll_palette_to_selection();
                }
                let order = model::nav_order(&self.entries, &self.collapsed);
                if let Some(p) = keys::step_selection(&order, self.selected.as_deref(), d) {
                    if self.selected.as_ref() != Some(&p) {
                        return self.update_inner(Message::Select(p));
                    }
                }
                Task::none()
            }
            Shortcut::Enter => {
                if self.palette_open {
                    let i = self.palette.selected;
                    self.run_palette(i)
                } else if self.modal.as_ref().is_some_and(Modal::is_confirm) {
                    if self.modal_guard_until.is_some_and(|t| self.now < t) {
                        return Task::none();
                    }
                    self.update_inner(Message::ModalConfirm)
                } else if let Some(p) = self.selected.clone() {
                    self.update_inner(Message::Select(p))
                } else {
                    Task::none()
                }
            }
            Shortcut::Check => match self.selected.clone() {
                Some(p) => self.update_inner(Message::Check(p)),
                None => Task::none(),
            },
            Shortcut::Update => match self.selected.clone() {
                Some(p) => self.update_inner(Message::Update(p)),
                None => Task::none(),
            },
            Shortcut::Apply => match self.selected.clone() {
                Some(p) => self.update_inner(Message::Ask(ConfirmAction::Apply(p))),
                None => Task::none(),
            },
        }
    }

    /// Ouvre les vues demandées par `--demo-view` (une seule fois, après le premier scan).
    fn apply_demo_views(&mut self) -> Task<Message> {
        let views = std::mem::take(&mut self.demo_views);
        let mut tasks = Vec::new();
        let by_name = |app: &App, n: &str| {
            app.entries
                .iter()
                .find(|e| e.name() == n)
                .map(|e| e.path.clone())
        };
        for v in views {
            match v.as_str() {
                "clair" | "light" => self.set_theme_mode(Mode::Light),
                "palette" => tasks.push(self.open_palette()),
                "recherche" => {
                    tasks.push(self.open_palette());
                    self.palette.set_query("verif".into());
                }
                "modal" => {
                    if let Some(p) = by_name(self, "nixos-config") {
                        tasks.push(self.update_inner(Message::Select(p.clone())));
                        tasks.push(self.update_inner(Message::Update(p)));
                    }
                }
                "replie" => tasks.push(self.update_inner(Message::ToggleGroup(Role::Tracked))),
                "reglages" | "settings" => tasks.push(self.update_inner(Message::OpenSettings)),
                "verif" => tasks.push(self.update_inner(Message::CheckAll)),
                "paquets" | "paquets-ajout" => {
                    use packages::{Msg, Op};
                    tasks.push(self.update_inner(Message::Packages(Msg::Open)));
                    self.packages.query = "git".into();
                    tasks.push(self.update_inner(Message::Packages(Msg::Search)));
                    if v == "paquets-ajout" {
                        self.packages.begin_op(Op::Add {
                            name: "ripgrep".into(),
                        });
                        self.packages.pick_file(0);
                        self.packages.pick_loc(0);
                    }
                }
                "git" => {
                    if let Some(p) = by_name(self, "website") {
                        tasks.push(self.update_inner(Message::Select(p)));
                        tasks.push(self.update_inner(Message::SetTab(Tab::Git)));
                    }
                }
                "erreur" => {
                    if let Some(p) = by_name(self, "rust-playground") {
                        tasks.push(self.update_inner(Message::Select(p)));
                    }
                }
                "site" => {
                    if let Some(p) = by_name(self, "website") {
                        tasks.push(self.update_inner(Message::Select(p)));
                    }
                }
                "toasts" => {
                    self.toast(ToastKind::Success, "✓ dotfiles : à jour");
                    self.toast(ToastKind::Info, "Scan terminé : 7 dépôts");
                    self.toast(
                        ToastKind::Error,
                        "✕ rust-playground : vérification échouée\nimpossible de joindre api.github.com",
                    );
                }
                _ => {}
            }
        }
        Task::batch(tasks)
    }

    // -- synchronisation git du dépôt système ----------------------------------

    /// Lance `git fetch` (puis, si `pull`, `git pull --ff-only`) sur le dépôt système.
    fn sync_task(&mut self, pull: bool) -> Task<Message> {
        if self.cli.demo || self.sync.busy() {
            return Task::none();
        }
        let Some(repo) = self.system_repo_path() else {
            return Task::none();
        };
        self.sync = if pull {
            SyncState::Pulling
        } else {
            SyncState::Fetching
        };
        Task::perform(
            tasks::blocking(move || {
                if pull {
                    sync::pull_and_status(&repo)
                } else {
                    sync::fetch_and_status(&repo)
                }
            }),
            |r| Message::SyncDone(r.unwrap_or_else(|| Err("synchronisation interrompue".into()))),
        )
    }

    fn on_sync_done(&mut self, res: Result<git::SyncStatus, String>) -> Task<Message> {
        let pulled = self.sync == SyncState::Pulling;
        match res {
            Ok(status) => {
                if pulled {
                    let msg = "Dépôt système synchronisé.";
                    self.push_log(LogKind::Success, msg);
                    self.toast(ToastKind::Success, msg);
                }
                self.sync = SyncState::Ready(status);
                // Un pull peut changer les fichiers, le flake.lock, l'état git : on relit tout.
                if pulled {
                    return self.scan_task();
                }
            }
            Err(e) => {
                self.push_log(LogKind::Error, format!("Synchronisation impossible : {e}"));
                // Un fetch raté à l'ouverture (hors ligne…) ne mérite pas de toast.
                if pulled {
                    self.toast(ToastKind::Error, &format!("Synchronisation refusée\n{e}"));
                }
                self.sync = SyncState::Failed(e);
            }
        }
        Task::none()
    }

    // -- terminal -------------------------------------------------------------

    fn start_terminal(&mut self) {
        if self.cli.demo {
            self.terminal_error = Some("Terminal indisponible en mode démo.".to_string());
            return;
        }
        match terminal::spawn(
            self.system_repo_path().as_deref(),
            self.home.as_deref(),
            self.mode,
        ) {
            Ok(t) => {
                self.terminal = Some(t);
                self.terminal_error = None;
            }
            Err(e) => {
                self.push_log(LogKind::Error, format!("Terminal impossible : {e}"));
                self.terminal_error = Some(e);
            }
        }
    }

    // -- abonnements --------------------------------------------------------

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            window::close_requests().map(|_| Message::CloseRequested),
            window::close_events().map(Message::WindowClosed),
            iced::system::theme_changes().map(Message::SystemTheme),
        ];
        subs.push(iced::event::listen_with(key_events));
        if !self.cli.demo {
            subs.push(Subscription::run(daemon_link::events).map(Message::Daemon));
        }
        // Images à ~60 Hz uniquement pendant une animation ; sinon un pouls lent
        // pour l'expiration des toasts, et rien du tout au repos.
        if self.is_animating() {
            subs.push(window::frames().map(|_| Message::Frame));
        } else if !self.toasts.is_empty() {
            subs.push(Subscription::run_with(250_u64, slow_ticker).map(|()| Message::Frame));
        }
        if let Some(t) = &self.terminal {
            subs.push(t.subscription().map(Message::Terminal));
        }
        Subscription::batch(subs)
    }
}

/// Touches pressées (avec indication « déjà consommée par un champ de texte »).
fn key_events(
    event: iced::Event,
    status: iced::event::Status,
    _window: window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key,
            modifiers,
            repeat,
            ..
        }) => {
            // Entrée maintenue ne doit jamais valider un dialogue à répétition.
            if repeat && key == Key::Named(iced::keyboard::key::Named::Enter) {
                return None;
            }
            Some(Message::Key(
                key,
                modifiers,
                status == iced::event::Status::Captured,
            ))
        }
        _ => None,
    }
}

/// Un tick toutes les `ms` millisecondes (pouls lent des toasts).
fn slow_ticker(ms: &u64) -> impl Stream<Item = ()> {
    let period = Duration::from_millis(*ms);
    let (tx, rx) = iced::futures::channel::mpsc::unbounded();
    std::thread::spawn(move || loop {
        std::thread::sleep(period);
        if tx.unbounded_send(()).is_err() {
            break;
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::Badge;
    use nixos_flakes_core::config::Role;
    use nixos_flakes_core::git::GitState;
    use nixos_flakes_core::lock::InputChange;
    use nixos_flakes_core::nix::UpdateReport;

    fn app() -> (App, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        let cfg = Config {
            scan_roots: vec![],
            system_repo: None,
            ..Config::default()
        };
        let a = App::new(Cli::default(), cfg, d.path().join("config.toml"));
        (a, d)
    }

    /// Résultat de scan « courant » (même numéro de séquence que le dernier scan lancé).
    fn scanned(a: &App, v: Vec<RepoStatus>) -> Message {
        Message::ScanDone(a.scan_seq, Ok(v))
    }

    fn status(path: &str, git: GitState) -> RepoStatus {
        RepoStatus {
            path: path.into(),
            role: Role::Detected,
            git,
            pending: None,
            error: None,
        }
    }

    fn change() -> InputChange {
        InputChange {
            name: "nixpkgs".into(),
            old_rev: Some("aaaaaaaa".into()),
            new_rev: Some("bbbbbbbb".into()),
            old_date: Some(0),
            new_date: Some(1),
        }
    }

    fn dirty() -> GitState {
        GitState {
            is_repo: true,
            branch: Some("main".into()),
            dirty: vec!["x.nix".into()],
            lock_dirty: false,
        }
    }

    fn finished(outcome: Outcome, git: GitState) -> Message {
        Message::Job {
            repo: "/r".into(),
            event: JobEvent::Finished(Box::new(Finished { outcome, git })),
        }
    }

    fn with_repo(role: Role, git: GitState) -> (App, tempfile::TempDir) {
        let (mut a, d) = app();
        let mut s = status("/r", git);
        s.role = role;
        a.cfg.set_role(Path::new("/r"), role);
        let _ = a.update(scanned(&a, vec![s]));
        (a, d)
    }

    #[test]
    fn scan_populates_and_selects_first() {
        let (mut a, _d) = app();
        a.scanning = true;
        let _ = a.update(scanned(
            &a,
            vec![
                status("/b", GitState::default()),
                status("/a", GitState::default()),
            ],
        ));
        assert!(!a.scanning);
        assert_eq!(a.entries.len(), 2);
        assert_eq!(a.selected.as_deref(), Some(Path::new("/a")));
    }

    #[test]
    fn role_change_is_persisted_and_applied() {
        let (mut a, _d) = app();
        let _ = a.update(scanned(&a, vec![status("/r", GitState::default())]));
        let _ = a.update(Message::Role("/r".into(), RoleAction::Track));
        assert_eq!(a.entries[0].role, Role::Tracked);
        // L'enregistrement part en arrière-plan : la config en mémoire est déjà à jour.
        assert!(a.cfg.tracked.contains(Path::new("/r")));
        let _ = a.update(Message::Role("/r".into(), RoleAction::SetSystem));
        assert_eq!(a.entries[0].role, Role::System);
        let _ = a.update(Message::Role("/r".into(), RoleAction::Ignore));
        assert_eq!(a.entries[0].role, Role::Ignored);
    }

    #[test]
    fn update_with_dirty_git_asks_to_commit() {
        let (mut a, _d) = with_repo(Role::Tracked, dirty());
        let _ = a.update(Message::Update("/r".into()));
        assert!(matches!(a.modal, Some(Modal::CommitDirty { .. })));
        assert_eq!(a.entries[0].busy, None);
        assert_eq!(a.editor.text().trim_end(), DIRTY_COMMIT_MESSAGE);
        let _ = a.update(Message::ModalCancel);
        assert!(a.modal.is_none());
        assert_eq!(a.entries[0].busy, None);
    }

    #[test]
    fn continue_anyway_starts_update_without_commit() {
        let (mut a, _d) = with_repo(Role::Tracked, dirty());
        let _ = a.update(Message::Update("/r".into()));
        let _ = a.update(Message::ModalContinue);
        assert!(a.modal.is_none());
        assert_eq!(a.entries[0].busy, Some(JobKind::Update));
        assert!(a.followups.is_empty());
    }

    #[test]
    fn commit_then_continue_registers_followup() {
        let (mut a, _d) = with_repo(Role::Tracked, dirty());
        let _ = a.update(Message::Update("/r".into()));
        let _ = a.update(Message::ModalCommit);
        assert_eq!(a.entries[0].busy, Some(JobKind::Commit));
        assert_eq!(a.followups.get(Path::new("/r")), Some(&Followup::Update));
    }

    fn root(name: &str) -> RootInput {
        RootInput {
            name: name.into(),
            node: Some(name.into()),
            rev: None,
            short_rev: Some("abc1234".into()),
            last_modified: None,
            kind: None,
            owner: None,
            repo: None,
            url: None,
        }
    }

    #[test]
    fn input_selection_messages() {
        let (mut a, _d) = with_repo(Role::Tracked, GitState::default());
        let p = PathBuf::from("/r");
        let _ = a.update(Message::InputsLoaded(
            p.clone(),
            Ok(vec![root("nixpkgs"), root("hm")]),
        ));
        a.entries[0].pending = Some(vec![change()]);
        let _ = a.update(Message::SelectInputs(p.clone(), InputSelect::All));
        assert_eq!(a.input_sel[&p].len(), 2);
        let _ = a.update(Message::SelectInputs(p.clone(), InputSelect::NoneSelected));
        assert!(a.input_sel[&p].is_empty());
        let _ = a.update(Message::SelectInputs(p.clone(), InputSelect::Updatable));
        assert_eq!(
            a.input_sel[&p].iter().collect::<Vec<_>>(),
            vec![&"nixpkgs".to_string()]
        );
        let _ = a.update(Message::ToggleInput(p.clone(), "hm".into()));
        assert_eq!(a.input_sel[&p].len(), 2);
        // Un rechargement retire les inputs disparus.
        let _ = a.update(Message::InputsLoaded(p.clone(), Ok(vec![root("hm")])));
        assert_eq!(a.input_sel[&p].len(), 1);
    }

    #[test]
    fn empty_selection_starts_nothing() {
        let (mut a, _d) = with_repo(Role::Tracked, GitState::default());
        let _ = a.update(Message::UpdateSelection("/r".into()));
        assert_eq!(a.entries[0].busy, None);
    }

    #[test]
    fn selection_update_starts_job_and_partial_result_keeps_other_pending() {
        let (mut a, _d) = with_repo(Role::Tracked, GitState::default());
        let p = PathBuf::from("/r");
        let other = InputChange {
            name: "hm".into(),
            ..change()
        };
        a.entries[0].pending = Some(vec![change(), other]);
        let _ = a.update(Message::ToggleInput(p.clone(), "nixpkgs".into()));
        let _ = a.update(Message::UpdateSelection(p.clone()));
        assert_eq!(a.entries[0].busy, Some(JobKind::Update));
        let report = UpdateReport {
            repo: p.clone(),
            changes: vec![change()],
        };
        let _ = a.update(finished(
            Outcome::Update(vec!["nixpkgs".into()], Ok(report)),
            GitState::default(),
        ));
        let left = a.entries[0].pending.as_ref().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].name, "hm");
        assert!(!a.input_sel.contains_key(&p), "sélection réinitialisée");
        assert!(a
            .log
            .iter()
            .any(|l| l.text.contains("mise à jour de nixpkgs terminée")));
    }

    #[test]
    fn dirty_git_keeps_requested_inputs_through_the_modal() {
        let (mut a, _d) = with_repo(Role::Tracked, dirty());
        let _ = a.update(Message::UpdateInputs("/r".into(), vec!["hm".into()]));
        assert_eq!(a.update_request[Path::new("/r")], vec!["hm".to_string()]);
        let _ = a.update(Message::ModalContinue);
        assert!(a.update_request.is_empty());
        assert_eq!(a.entries[0].busy, Some(JobKind::Update));
        let _ = a.update(Message::ModalCancel);
        let (mut b, _d) = with_repo(Role::Tracked, dirty());
        let _ = b.update(Message::UpdateInputs("/r".into(), vec!["hm".into()]));
        let _ = b.update(Message::ModalCancel);
        assert!(b.update_request.is_empty());
    }
    #[test]
    fn update_without_git_proceeds_directly() {
        let (mut a, _d) = with_repo(Role::Tracked, GitState::default());
        let _ = a.update(Message::Update("/r".into()));
        assert!(a.modal.is_none());
        assert_eq!(a.entries[0].busy, Some(JobKind::Update));
    }

    #[test]
    fn detected_repos_cannot_be_updated() {
        let (mut a, _d) = with_repo(Role::Detected, GitState::default());
        let _ = a.update(Message::Update("/r".into()));
        assert_eq!(a.entries[0].busy, None);
        let _ = a.update(Message::Apply("/r".into()));
        assert_eq!(a.entries[0].busy, None);
    }

    #[test]
    fn apply_only_for_system_repo() {
        let (mut a, _d) = with_repo(Role::Tracked, GitState::default());
        let _ = a.update(Message::Apply("/r".into()));
        assert_eq!(a.entries[0].busy, None);
        let _ = a.update(Message::Role("/r".into(), RoleAction::SetSystem));
        let _ = a.update(Message::Apply("/r".into()));
        assert_eq!(a.entries[0].busy, Some(JobKind::Apply));
    }

    #[test]
    fn one_job_at_a_time_per_repo() {
        let (mut a, _d) = with_repo(Role::Tracked, GitState::default());
        let _ = a.update(Message::Check("/r".into()));
        assert_eq!(a.entries[0].busy, Some(JobKind::Check));
        let _ = a.update(Message::Update("/r".into()));
        assert_eq!(a.entries[0].busy, Some(JobKind::Check));
    }

    #[test]
    fn lock_commit_proposed_after_update_when_git_and_lock_changed() {
        let (mut a, _d) = with_repo(Role::Tracked, GitState::default());
        a.entries[0].busy = Some(JobKind::Update);
        let mut g = dirty();
        g.dirty.clear();
        g.lock_dirty = true;
        let report = UpdateReport {
            repo: "/r".into(),
            changes: vec![change()],
        };
        let _ = a.update(finished(Outcome::Update(Vec::new(), Ok(report)), g));
        match &a.modal {
            Some(Modal::CommitLock { message, .. }) => {
                assert!(message.starts_with("flake.lock: update inputs"));
                assert!(message.contains("nixpkgs"));
            }
            m => panic!("inattendu : {m:?}"),
        }
        assert_eq!(a.entries[0].busy, None);
        assert_eq!(a.entries[0].pending, Some(vec![]));
        assert_eq!(a.entries[0].last_update.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn no_lock_commit_proposal_outside_git() {
        let (mut a, _d) = with_repo(Role::Tracked, GitState::default());
        a.entries[0].busy = Some(JobKind::Update);
        let report = UpdateReport {
            repo: "/r".into(),
            changes: vec![change()],
        };
        let _ = a.update(finished(
            Outcome::Update(Vec::new(), Ok(report)),
            GitState::default(),
        ));
        assert!(a.modal.is_none());
    }

    #[test]
    fn check_result_sets_badge_and_error() {
        let (mut a, _d) = with_repo(Role::Tracked, GitState::default());
        a.entries[0].busy = Some(JobKind::Check);
        let report = UpdateReport {
            repo: "/r".into(),
            changes: vec![change()],
        };
        let _ = a.update(finished(Outcome::Check(Ok(report)), GitState::default()));
        assert_eq!(model::badge(&a.entries[0]), Badge::Updates(1));
        a.entries[0].busy = Some(JobKind::Check);
        let _ = a.update(finished(
            Outcome::Check(Err("réseau".into())),
            GitState::default(),
        ));
        assert_eq!(model::badge(&a.entries[0]), Badge::Error);
        assert!(a.log.iter().any(|l| l.kind == LogKind::Error));
    }

    #[test]
    fn failed_commit_cancels_followup() {
        let (mut a, _d) = with_repo(Role::Tracked, dirty());
        a.entries[0].busy = Some(JobKind::Commit);
        a.followups.insert("/r".into(), Followup::Update);
        let _ = a.update(finished(
            Outcome::Done(JobKind::Commit, Err("refus".into())),
            dirty(),
        ));
        assert!(a.followups.is_empty());
        assert!(a.entries[0].error.is_some());
    }

    #[test]
    fn check_all_queues_system_and_tracked() {
        let (mut a, _d) = app();
        let _ = a.update(scanned(
            &a,
            vec![
                status("/a", GitState::default()),
                status("/b", GitState::default()),
                status("/c", GitState::default()),
            ],
        ));
        let _ = a.update(Message::Role("/a".into(), RoleAction::Track));
        let _ = a.update(Message::Role("/b".into(), RoleAction::SetSystem));
        let _ = a.update(Message::CheckAll);
        assert_eq!(a.entries[1].busy, Some(JobKind::Check)); // /b : système d'abord
        assert_eq!(a.queue.len(), 1);
        assert_eq!(a.entries[2].busy, None);
    }

    #[test]
    fn log_is_capped_and_clearable() {
        let (mut a, _d) = app();
        for i in 0..(LOG_CAPACITY + 10) {
            a.push_log(LogKind::Info, format!("l{i}"));
        }
        assert_eq!(a.log.len(), LOG_CAPACITY);
        assert_eq!(a.log.front().unwrap().text, "l10");
        let _ = a.update(Message::ClearLog);
        assert!(a.log.is_empty());
    }

    #[test]
    fn settings_changes_are_saved() {
        let (mut a, d) = app();
        let _ = a.update(Message::Settings(settings::Msg::InputChanged(
            d.path().display().to_string(),
        )));
        let _ = a.update(Message::Settings(settings::Msg::AddRoot));
        let _ = a.update(Message::Settings(settings::Msg::Depth(true)));
        let _ = a.update(Message::Settings(settings::Msg::Interval(true)));
        // L'enregistrement part en arrière-plan : on vérifie la config en mémoire (celle
        // qui est envoyée au thread d'enregistrement).
        assert_eq!(a.cfg.scan_roots, [d.path().to_path_buf()]);
        assert_eq!(a.cfg.scan_depth, 5);
        assert_eq!(a.cfg.check_interval_minutes, 120);
        assert!(a.new_root.is_empty());
        assert!(a.scanning, "l'ajout d'une racine relance un scan");
    }

    #[test]
    fn close_request_exits_the_gui() {
        // Fermer la fenêtre quitte le GUI (le démon, lui, continue).
        let (mut a, _d) = app();
        let id = window::Id::unique();
        a.window = Some(id);
        let _ = a.update(Message::CloseRequested);
        let _ = a.update(Message::WindowClosed(id));
        assert_eq!(a.window, None);
    }

    #[test]
    fn system_theme_updates_mode() {
        let (mut a, _d) = app();
        let _ = a.update(Message::SystemTheme(iced::theme::Mode::Light));
        assert_eq!(a.mode, Mode::Light);
        let _ = a.update(Message::SystemTheme(iced::theme::Mode::Dark));
        assert_eq!(a.mode, Mode::Dark);
        let _ = a.update(Message::SystemTheme(iced::theme::Mode::None));
        assert_eq!(a.mode, Mode::Dark);
    }

    // -- animations, palette, clavier, toasts ---------------------------------

    use iced::keyboard::key::Named;

    fn key(c: &str) -> Message {
        Message::Key(Key::Character(c.into()), Modifiers::empty(), false)
    }

    fn named(n: Named) -> Message {
        Message::Key(Key::Named(n), Modifiers::empty(), false)
    }

    fn ctrl_k() -> Message {
        Message::Key(Key::Character("k".into()), Modifiers::CTRL, false)
    }

    fn later(a: &mut App, ms: u64) {
        a.now += Duration::from_millis(ms);
    }

    fn three_repos() -> (App, tempfile::TempDir) {
        let (mut a, d) = app();
        let _ = a.update(scanned(
            &a,
            vec![
                status("/a", GitState::default()),
                status("/b", GitState::default()),
                status("/c", GitState::default()),
            ],
        ));
        for p in ["/a", "/b", "/c"] {
            let _ = a.update(Message::Role(p.into(), RoleAction::Track));
        }
        (a, d)
    }

    #[test]
    fn app_is_idle_when_nothing_moves() {
        let (mut a, _d) = three_repos();
        later(&mut a, 3_000);
        assert!(!a.is_animating(), "aucune image à demander au repos");
    }

    #[test]
    fn selection_animates_then_settles() {
        let (mut a, _d) = three_repos();
        later(&mut a, 3_000);
        let _ = a.update(Message::Select("/b".into()));
        assert!(a.is_animating(), "fondu de sélection et de panneau");
        later(&mut a, 400);
        assert!(!a.is_animating());
        assert_eq!(a.fades.level("sel:/b", a.now), 1.0);
        assert_eq!(a.fades.level("sel:/a", a.now), 0.0);
    }

    #[test]
    fn running_job_and_scan_keep_frames_coming() {
        let (mut a, _d) = three_repos();
        later(&mut a, 3_000);
        a.entries[0].busy = Some(JobKind::Check);
        later(&mut a, 60_000);
        assert!(a.is_animating(), "pulsation et barre indéterminée");
        a.entries[0].busy = None;
        assert!(!a.is_animating());
        a.scanning = true;
        assert!(a.is_animating(), "squelettes pendant le scan");
    }

    #[test]
    fn hover_fades_in_and_out_then_rests() {
        let (mut a, _d) = app();
        let _ = a.update(Message::Hover("btn:check".into(), true));
        assert!(a.is_animating());
        later(&mut a, 200);
        assert!(!a.is_animating());
        assert_eq!(a.fades.level("hov:btn:check", a.now), 1.0);
        let _ = a.update(Message::Hover("btn:check".into(), false));
        later(&mut a, 200);
        assert_eq!(a.fades.level("hov:btn:check", a.now), 0.0);
        let _ = a.update(Message::Frame);
        assert!(!a.is_animating());
    }

    #[test]
    fn selection_change_clears_stuck_hover() {
        let (mut a, _d) = three_repos();
        let _ = a.update(Message::Hover("row:/a".into(), true));
        later(&mut a, 200);
        let _ = a.update(Message::Select("/c".into()));
        later(&mut a, 300);
        assert_eq!(a.fades.level("hov:row:/a", a.now), 0.0);
    }

    #[test]
    fn group_toggle_collapses_and_navigation_skips_it() {
        let (mut a, _d) = three_repos();
        let _ = a.update(Message::ToggleGroup(Role::Tracked));
        assert!(a.collapsed.contains(&Role::Tracked));
        assert!(model::nav_order(&a.entries, &a.collapsed).is_empty());
        later(&mut a, 300);
        assert_eq!(a.fades.level_or("grp:Suivis", a.now, 1.0), 0.0);
        let _ = a.update(Message::ToggleGroup(Role::Tracked));
        later(&mut a, 300);
        assert_eq!(a.fades.level_or("grp:Suivis", a.now, 1.0), 1.0);
        assert_eq!(model::nav_order(&a.entries, &a.collapsed).len(), 3);
    }

    #[test]
    fn keyboard_moves_selection_and_runs_actions() {
        let (mut a, _d) = three_repos();
        assert_eq!(a.selected.as_deref(), Some(Path::new("/a")));
        let _ = a.update(key("j"));
        assert_eq!(a.selected.as_deref(), Some(Path::new("/b")));
        let _ = a.update(named(Named::ArrowDown));
        assert_eq!(a.selected.as_deref(), Some(Path::new("/c")));
        let _ = a.update(key("j"));
        assert_eq!(a.selected.as_deref(), Some(Path::new("/c")), "borné en bas");
        let _ = a.update(key("k"));
        assert_eq!(a.selected.as_deref(), Some(Path::new("/b")));
        let _ = a.update(key("r"));
        assert_eq!(a.entries[1].busy, Some(JobKind::Check));
    }

    #[test]
    fn u_shortcut_updates_and_asks_to_commit_when_dirty() {
        let (mut a, _d) = with_repo(Role::Tracked, dirty());
        let _ = a.update(key("u"));
        assert!(matches!(a.modal, Some(Modal::CommitDirty { .. })));
        // Dans un dialogue, les lettres ne déclenchent plus rien.
        let _ = a.update(key("r"));
        assert_eq!(a.entries[0].busy, None);
    }

    #[test]
    fn shortcuts_are_ignored_while_typing() {
        let (mut a, _d) = three_repos();
        let _ = a.update(Message::Key(
            Key::Character("j".into()),
            Modifiers::empty(),
            true,
        ));
        assert_eq!(a.selected.as_deref(), Some(Path::new("/a")));
        let _ = a.update(Message::OpenSettings);
        let _ = a.update(key("j"));
        assert_eq!(
            a.selected.as_deref(),
            Some(Path::new("/a")),
            "réglages : champ de saisie"
        );
    }

    #[test]
    fn palette_opens_filters_runs_and_closes() {
        let (mut a, _d) = three_repos();
        let _ = a.update(ctrl_k());
        assert!(a.palette_open);
        assert!(a.is_animating(), "fondu d'ouverture");
        assert!(a.palette_results().len() >= 4);
        // Filtrage : « reg » -> Réglages en tête.
        let _ = a.update(Message::PaletteInput("reg".into()));
        assert_eq!(a.palette_results()[0].command, Command::Settings);
        let _ = a.update(Message::PaletteRun(0));
        assert!(!a.palette_open);
        assert_eq!(a.page, Page::Settings);
        // Échap ferme la palette sans rien lancer.
        let _ = a.update(ctrl_k());
        // Ctrl+K est actif partout, y compris dans les réglages.
        assert!(a.palette_open);
        let _ = a.update(named(Named::Escape));
        assert!(!a.palette_open);
        assert_eq!(a.page, Page::Settings, "Échap ferme d'abord la palette");
        let _ = a.update(named(Named::Escape));
        assert_eq!(a.page, Page::Main, "puis la page de réglages");
        later(&mut a, 400);
        assert!(!a.is_animating());
    }

    #[test]
    fn palette_arrows_wrap_and_enter_goes_to_repo() {
        let (mut a, _d) = three_repos();
        let _ = a.update(ctrl_k());
        let _ = a.update(Message::PaletteInput("c".into()));
        let results = a.palette_results();
        assert!(results
            .iter()
            .any(|i| i.command == Command::Goto("/c".into())));
        let n = results.len();
        let _ = a.update(named(Named::ArrowUp));
        assert_eq!(a.palette.selected, n - 1, "retour à la fin de la liste");
        let _ = a.update(named(Named::ArrowDown));
        assert_eq!(a.palette.selected, 0);
        // « nixos » inconnu : liste vide, Entrée sans effet.
        let _ = a.update(Message::PaletteInput("zzzz".into()));
        assert!(a.palette_results().is_empty());
        let _ = a.update(named(Named::Enter));
        assert!(a.palette_open, "rien à lancer");
        // Aller à un dépôt.
        let _ = a.update(Message::PaletteInput("c".into()));
        let idx = a
            .palette_results()
            .iter()
            .position(|i| i.command == Command::Goto("/c".into()))
            .unwrap();
        let _ = a.update(Message::PaletteRun(idx));
        assert_eq!(a.selected.as_deref(), Some(Path::new("/c")));
        assert!(!a.palette_open);
    }

    #[test]
    fn palette_does_not_open_over_a_modal() {
        let (mut a, _d) = with_repo(Role::Tracked, dirty());
        let _ = a.update(Message::Update("/r".into()));
        assert!(a.modal.is_some());
        let _ = a.update(ctrl_k());
        assert!(!a.palette_open);
        let _ = a.update(named(Named::Escape));
        assert!(a.modal.is_none(), "Échap annule le dialogue");
    }

    #[test]
    fn palette_offers_only_available_actions() {
        let (mut a, _d) = with_repo(Role::Detected, GitState::default());
        let _ = a.update(ctrl_k());
        let labels: Vec<String> = a.palette_items().into_iter().map(|i| i.label).collect();
        assert!(!labels.iter().any(|l| l == "Mettre à jour"));
        assert!(!labels.iter().any(|l| l == "Appliquer au système"));
        assert!(labels.iter().any(|l| l == "Vérifier"));
        assert!(labels.iter().any(|l| l == "Rescanner"));
    }

    #[test]
    fn modal_fades_out_before_being_dropped() {
        let (mut a, _d) = with_repo(Role::Tracked, dirty());
        let _ = a.update(Message::Update("/r".into()));
        assert!(a.modal_t.is_animating(a.now), "entrée animée");
        later(&mut a, 400);
        assert_eq!(a.modal_t.value(a.now), 1.0);
        let _ = a.update(Message::ModalCancel);
        assert!(a.modal.is_none());
        assert!(a.modal_leaving.is_some(), "affiché le temps de la sortie");
        assert!(a.is_animating());
        later(&mut a, 400);
        let _ = a.update(Message::Frame);
        assert!(a.modal_leaving.is_none());
    }

    #[test]
    fn results_produce_toasts_that_expire() {
        let (mut a, _d) = with_repo(Role::Tracked, GitState::default());
        a.entries[0].busy = Some(JobKind::Check);
        let report = UpdateReport {
            repo: "/r".into(),
            changes: vec![change()],
        };
        let _ = a.update(finished(Outcome::Check(Ok(report)), GitState::default()));
        assert_eq!(a.toasts.items().len(), 1);
        assert_eq!(a.toasts.items()[0].kind, ToastKind::Success);
        assert!(
            !a.toasts.items()[0].title.starts_with('✓'),
            "pas de glyphe en double"
        );
        assert!(a.is_animating(), "glissement d'entrée");
        later(&mut a, 1_000);
        assert!(
            !a.is_animating(),
            "toast stable : pas d'images, seulement un pouls lent"
        );
        later(&mut a, 4_500);
        let _ = a.update(Message::Frame);
        later(&mut a, 10_000);
        let _ = a.update(Message::Frame);
        assert!(a.toasts.is_empty());

        a.entries[0].busy = Some(JobKind::Check);
        let _ = a.update(finished(
            Outcome::Check(Err("réseau\ndétail".into())),
            GitState::default(),
        ));
        let t = &a.toasts.items()[0];
        assert_eq!(t.kind, ToastKind::Error);
        assert_eq!(t.body.as_deref().map(|b| b.contains("réseau")), Some(true));
        let id = t.id;
        let _ = a.update(Message::DismissToast(id));
        later(&mut a, 400);
        a.toasts.tick(a.now);
        assert!(a.toasts.is_empty());
    }

    #[test]
    fn theme_change_is_interpolated() {
        let (mut a, _d) = app();
        a.mode = Mode::Dark;
        let _ = a.update(Message::SystemTheme(iced::theme::Mode::Light));
        assert!(a.theme_t.is_animating(a.now));
        let start = a.tokens();
        assert_eq!(start.bg, theme::tokens(Mode::Dark).bg);
        later(&mut a, 100);
        let mid = a.tokens();
        assert!(mid.bg.r > start.bg.r && mid.bg.r < theme::tokens(Mode::Light).bg.r);
        later(&mut a, 500);
        assert_eq!(a.tokens().bg, theme::tokens(Mode::Light).bg);
        assert!(!a.is_animating());
    }

    #[test]
    fn console_toggle_animates_the_drawer() {
        let (mut a, _d) = app();
        assert_eq!(a.console_t.value(a.now), 1.0);
        let _ = a.update(Message::ToggleConsole);
        assert!(a.console_t.is_animating(a.now));
        later(&mut a, 400);
        assert_eq!(a.console_t.value(a.now), 0.0);
        assert!(!a.console_open);
    }

    #[test]
    fn tab_switch_restarts_panel_fade() {
        let (mut a, _d) = three_repos();
        later(&mut a, 3_000);
        let _ = a.update(Message::SetTab(Tab::Git));
        assert_eq!(a.tab, Tab::Git);
        assert!(a.panel_t.is_animating(a.now));
    }

    #[test]
    fn demo_mode_seeds_roles_and_never_writes_config() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("config.toml");
        let cli = Cli {
            demo: true,
            ..Cli::default()
        };
        let mut a = App::new(cli, Config::default(), path.clone());
        let _ = a.update(scanned(&a, demo::statuses(1_700_000_000)));
        let roles: HashSet<Role> = a.entries.iter().map(|e| e.role).collect();
        assert_eq!(roles.len(), 4, "tous les rôles présents");
        let _ = a.update(Message::Role(a.entries[0].path.clone(), RoleAction::Ignore));
        assert!(!path.exists(), "la config n'est jamais écrite en démo");
        assert_eq!(a.home.as_deref(), Some(Path::new(demo::HOME)));
    }

    #[test]
    fn demo_views_open_at_first_scan() {
        let cli = Cli {
            demo: true,
            demo_view: vec!["modal".into(), "clair".into()],
            ..Cli::default()
        };
        let d = tempfile::tempdir().unwrap();
        let mut a = App::new(cli, Config::default(), d.path().join("c.toml"));
        let _ = a.update(scanned(&a, demo::statuses(1_700_000_000)));
        assert!(matches!(a.modal, Some(Modal::CommitDirty { .. })));
        assert_eq!(a.mode, Mode::Light);
        assert!(a.demo_views.is_empty(), "appliquées une seule fois");
    }

    // -- robustesse, confirmations, file de dialogues ---------------------------

    fn toast_titles(a: &App) -> Vec<String> {
        a.toasts.items().iter().map(|t| t.title.clone()).collect()
    }

    fn has_toast(a: &App, kind: ToastKind, needle: &str) -> bool {
        a.toasts.items().iter().any(|t| {
            t.kind == kind
                && (t.title.contains(needle)
                    || t.body.as_deref().is_some_and(|b| b.contains(needle)))
        })
    }

    #[test]
    fn role_change_shows_a_success_toast_and_busy_repos_refuse() {
        let (mut a, _d) = with_repo(Role::Detected, GitState::default());
        let _ = a.update(Message::Role("/r".into(), RoleAction::Track));
        assert_eq!(a.entries[0].role, Role::Tracked);
        // Occupé : le rôle ne change pas et le refus est visible.
        a.entries[0].busy = Some(JobKind::Check);
        let _ = a.update(Message::Role("/r".into(), RoleAction::Ignore));
        assert_eq!(a.entries[0].role, Role::Tracked);
        assert!(has_toast(&a, ToastKind::Error, "occupé"));
        let _ = a.update(Message::Ask(ConfirmAction::Role(
            "/r".into(),
            RoleAction::Ignore,
        )));
        assert!(a.modal.is_none(), "pas de dialogue pour un dépôt occupé");
    }

    #[test]
    fn heavy_role_changes_ask_for_confirmation_light_ones_do_not() {
        let (mut a, _d) = with_repo(Role::Detected, GitState::default());
        let _ = a.update(Message::Ask(ConfirmAction::Role(
            "/r".into(),
            RoleAction::Track,
        )));
        assert!(a.modal.is_none());
        assert_eq!(a.entries[0].role, Role::Tracked, "suivre : sans dialogue");
        let _ = a.update(Message::Ask(ConfirmAction::Role(
            "/r".into(),
            RoleAction::Ignore,
        )));
        assert_eq!(a.entries[0].role, Role::Tracked, "pas encore appliqué");
        assert!(matches!(a.modal, Some(Modal::Confirm(_))));
        let _ = a.update(Message::ModalCancel);
        assert_eq!(a.entries[0].role, Role::Tracked);
        let _ = a.update(Message::Ask(ConfirmAction::Role(
            "/r".into(),
            RoleAction::SetSystem,
        )));
        let _ = a.update(named(Named::Enter));
        assert!(a.modal.is_none());
        assert_eq!(a.entries[0].role, Role::System, "Entrée valide");
    }

    #[test]
    fn apply_and_push_are_confirmed_before_running() {
        let (mut a, _d) = with_repo(Role::System, dirty());
        let _ = a.update(Message::Ask(ConfirmAction::Apply("/r".into())));
        assert!(matches!(
            a.modal,
            Some(Modal::Confirm(ConfirmAction::Apply(_)))
        ));
        assert_eq!(a.entries[0].busy, None, "rien ne part sans confirmation");
        let _ = a.update(named(Named::Escape));
        assert!(a.modal.is_none());
        assert_eq!(a.entries[0].busy, None, "Échap annule");
        let _ = a.update(key("a"));
        assert!(matches!(a.modal, Some(Modal::Confirm(_))), "touche a");
        let _ = a.update(Message::ModalConfirm);
        assert_eq!(a.entries[0].busy, Some(JobKind::Apply));
        // Push sur un dépôt libre.
        a.entries[0].busy = None;
        let _ = a.update(Message::Ask(ConfirmAction::Push("/r".into())));
        assert!(matches!(
            a.modal,
            Some(Modal::Confirm(ConfirmAction::Push(_)))
        ));
        let _ = a.update(Message::ModalConfirm);
        assert_eq!(a.entries[0].busy, Some(JobKind::Push));
    }

    #[test]
    fn confirmation_stays_open_with_a_toast_when_the_repo_got_busy() {
        let (mut a, _d) = with_repo(Role::System, GitState::default());
        let _ = a.update(Message::Ask(ConfirmAction::Apply("/r".into())));
        a.entries[0].busy = Some(JobKind::Check);
        let _ = a.update(Message::ModalConfirm);
        assert!(a.modal.is_some(), "le dialogue n'est pas consommé");
        assert!(has_toast(&a, ToastKind::Error, "occupé"));
        assert_eq!(a.entries[0].busy, Some(JobKind::Check));
    }

    #[test]
    fn remove_root_needs_confirmation() {
        let (mut a, d) = app();
        let root = d.path().to_path_buf();
        a.cfg.scan_roots = vec![root.clone()];
        let _ = a.update(Message::Settings(settings::Msg::AskRemoveRoot(0)));
        assert!(matches!(
            a.modal,
            Some(Modal::Confirm(ConfirmAction::RemoveRoot(_)))
        ));
        assert_eq!(a.cfg.scan_roots.len(), 1);
        let _ = a.update(Message::ModalConfirm);
        assert!(a.cfg.scan_roots.is_empty());
        assert!(a.scanning, "rescan après retrait");
    }

    #[test]
    fn modals_never_overwrite_each_other() {
        let (mut a, _d) = with_repo(Role::Tracked, dirty());
        let _ = a.update(Message::Update("/r".into()));
        let first = a.modal.clone();
        assert!(matches!(first, Some(Modal::CommitDirty { .. })));
        // Un commit de flake.lock arrive pendant que le premier dialogue est ouvert.
        a.entries[0].busy = Some(JobKind::Update);
        let mut g = dirty();
        g.dirty.clear();
        g.lock_dirty = true;
        let report = UpdateReport {
            repo: "/r".into(),
            changes: vec![change()],
        };
        let _ = a.update(finished(Outcome::Update(Vec::new(), Ok(report)), g));
        assert_eq!(a.modal, first, "le dialogue ouvert est conservé");
        assert_eq!(a.modal_queue.len(), 1);
        // Après la fermeture du premier, le second apparaît.
        let _ = a.update(Message::ModalCancel);
        assert!(matches!(a.modal, Some(Modal::CommitLock { .. })));
        assert!(a.modal_queue.is_empty());
        assert!(a.editor.text().starts_with("flake.lock: update inputs"));
        let _ = a.update(Message::ModalCancel);
        assert!(a.modal.is_none());
    }

    #[test]
    fn a_second_update_request_does_not_duplicate_the_commit_dialog() {
        let (mut a, _d) = with_repo(Role::Tracked, dirty());
        let _ = a.update(Message::UpdateInputs("/r".into(), vec!["hm".into()]));
        let _ = a.update(Message::Update("/r".into()));
        assert!(a.modal_queue.is_empty());
        assert_eq!(
            a.update_request[Path::new("/r")],
            vec!["hm".to_string()],
            "la requête d'origine n'est pas écrasée"
        );
    }

    #[test]
    fn commit_refused_when_busy_keeps_the_dialog_and_the_request() {
        let (mut a, _d) = with_repo(Role::Tracked, dirty());
        let _ = a.update(Message::UpdateInputs("/r".into(), vec!["hm".into()]));
        a.entries[0].busy = Some(JobKind::Check);
        let _ = a.update(Message::ModalCommit);
        assert!(a.modal.is_some(), "dialogue conservé");
        assert!(a.followups.is_empty());
        assert_eq!(a.update_request.len(), 1);
        assert!(has_toast(&a, ToastKind::Error, "occupé"));
        let _ = a.update(Message::ModalContinue);
        assert!(a.modal.is_some());
        // Une fois libre, « continuer » fonctionne et consomme la requête.
        a.entries[0].busy = None;
        let _ = a.update(Message::ModalContinue);
        assert!(a.modal.is_none());
        assert!(a.update_request.is_empty());
        assert_eq!(a.entries[0].busy, Some(JobKind::Update));
    }

    #[test]
    fn commit_dialog_for_a_vanished_repo_closes_and_cleans_up() {
        let (mut a, _d) = with_repo(Role::Tracked, dirty());
        let _ = a.update(Message::Update("/r".into()));
        let _ = a.update(scanned(&a, vec![]));
        let _ = a.update(Message::ModalCommit);
        assert!(a.modal.is_none());
        assert!(a.update_request.is_empty() && a.followups.is_empty());
        assert!(has_toast(&a, ToastKind::Error, "introuvable"));
    }

    #[test]
    fn empty_commit_message_is_refused() {
        let (mut a, _d) = with_repo(Role::Tracked, dirty());
        let _ = a.update(Message::Update("/r".into()));
        if let Some(m) = a.modal.as_mut().and_then(Modal::message_mut) {
            *m = "  \n ".to_string();
        }
        let _ = a.update(Message::ModalCommit);
        assert!(a.modal.is_some());
        assert_eq!(a.entries[0].busy, None);
        assert!(has_toast(&a, ToastKind::Error, "vide"));
    }

    #[test]
    fn stale_scan_results_are_ignored() {
        let (mut a, _d) = app();
        let _ = a.update(Message::Rescan);
        let _ = a.update(Message::Rescan);
        assert_eq!(a.scan_seq, 2);
        assert!(a.scanning);
        // Le premier scan finit après le second : ignoré.
        let _ = a.update(Message::ScanDone(
            1,
            Ok(vec![status("/vieux", GitState::default())]),
        ));
        assert!(a.entries.is_empty());
        assert!(a.scanning, "le scan courant n'est pas terminé");
        let _ = a.update(Message::ScanDone(
            2,
            Ok(vec![status("/neuf", GitState::default())]),
        ));
        assert!(!a.scanning);
        assert_eq!(a.entries.len(), 1);
        assert_eq!(a.entries[0].path, Path::new("/neuf"));
    }

    #[test]
    fn failed_scan_keeps_the_previous_list_and_toasts() {
        let (mut a, _d) = three_repos();
        let _ = a.update(Message::Rescan);
        let seq = a.scan_seq;
        let _ = a.update(Message::ScanDone(seq, Err("analyse interrompue".into())));
        assert_eq!(a.entries.len(), 3, "la liste n'est pas vidée");
        assert!(!a.scanning);
        assert!(has_toast(&a, ToastKind::Error, "Scan impossible"));
        // Un résultat vide réel, lui, vide bien la liste.
        let _ = a.update(Message::Rescan);
        let seq = a.scan_seq;
        let _ = a.update(Message::ScanDone(seq, Ok(vec![])));
        assert!(a.entries.is_empty());
    }

    #[test]
    fn finished_for_a_vanished_repo_still_resumes_the_check_all_queue() {
        let (mut a, _d) = three_repos();
        // « /gone » a disparu pendant sa vérification ; /b attend dans la file.
        a.queue.push_back("/b".into());
        a.followups.insert("/gone".into(), Followup::Update);
        a.update_request.insert("/gone".into(), vec![]);
        let report = UpdateReport {
            repo: "/gone".into(),
            changes: vec![],
        };
        let _ = a.update(Message::Job {
            repo: "/gone".into(),
            event: JobEvent::Finished(Box::new(Finished {
                outcome: Outcome::Check(Ok(report)),
                git: GitState::default(),
            })),
        });
        assert_eq!(a.entries[1].busy, Some(JobKind::Check), "la file continue");
        assert!(a.queue.is_empty());
        assert!(a.followups.is_empty() && a.update_request.is_empty());
    }

    #[test]
    fn a_panicked_job_reports_an_error_and_frees_the_repo() {
        let (mut a, _d) = three_repos();
        a.queue.push_back("/b".into());
        a.entries[0].busy = Some(JobKind::Check);
        let _ = a.update(Message::Job {
            repo: "/a".into(),
            event: JobEvent::Finished(Box::new(Finished {
                outcome: Outcome::Aborted(JobKind::Check, "boum".into()),
                git: GitState::default(),
            })),
        });
        assert_eq!(a.entries[0].busy, None);
        assert_eq!(
            a.entries[0].error.as_deref(),
            Some("tâche interrompue : boum")
        );
        assert_eq!(model::badge(&a.entries[0]), Badge::Error);
        assert!(has_toast(&a, ToastKind::Error, "tâche interrompue"));
        assert_eq!(a.entries[1].busy, Some(JobKind::Check), "file reprise");
    }

    #[test]
    fn aborted_commit_clears_followup_and_request() {
        let (mut a, _d) = with_repo(Role::Tracked, dirty());
        a.entries[0].busy = Some(JobKind::Commit);
        a.followups.insert("/r".into(), Followup::Update);
        a.update_request.insert("/r".into(), vec![]);
        let _ = a.update(finished(
            Outcome::Aborted(JobKind::Commit, String::new()),
            dirty(),
        ));
        assert!(a.followups.is_empty() && a.update_request.is_empty());
        assert_eq!(a.entries[0].busy, None);
    }

    #[test]
    fn failed_commit_clears_the_pending_update_request() {
        let (mut a, _d) = with_repo(Role::Tracked, dirty());
        a.entries[0].busy = Some(JobKind::Commit);
        a.followups.insert("/r".into(), Followup::Update);
        a.update_request.insert("/r".into(), vec!["hm".into()]);
        let _ = a.update(finished(
            Outcome::Done(JobKind::Commit, Err("refus".into())),
            dirty(),
        ));
        assert!(a.update_request.is_empty());
    }

    #[test]
    fn refused_jobs_are_visible_toasts() {
        let (mut a, _d) = with_repo(Role::Tracked, GitState::default());
        let _ = a.update(Message::Check("/r".into()));
        let _ = a.update(Message::Check("/r".into()));
        assert!(has_toast(&a, ToastKind::Error, "occupé"));
        let _ = a.update(Message::Check("/absent".into()));
        assert!(has_toast(&a, ToastKind::Error, "introuvable"));
    }

    #[test]
    fn check_all_while_running_is_explained_and_stale_queue_is_reset() {
        let (mut a, _d) = three_repos();
        let _ = a.update(Message::CheckAll);
        let _ = a.update(Message::CheckAll);
        assert!(has_toast(&a, ToastKind::Info, "déjà en cours"));
        // File périmée : plus rien ne tourne.
        for e in &mut a.entries {
            e.busy = None;
        }
        a.queue.push_back("/zzz".into());
        let _ = a.update(Message::CheckAll);
        assert_eq!(a.entries[0].busy, Some(JobKind::Check));
        assert!(!a.queue.contains(&PathBuf::from("/zzz")));
    }

    #[test]
    fn config_save_errors_are_toasted_and_successes_confirmed() {
        let (mut a, _d) = app();
        let _ = a.update(Message::ConfigSaved(
            Err("disque plein".into()),
            None,
            Box::default(),
            0,
        ));
        assert!(has_toast(&a, ToastKind::Error, "disque plein"));
        assert!(a.log.iter().any(|l| l.kind == LogKind::Error));
        let _ = a.update(Message::ConfigSaved(
            Ok(true),
            Some("Réglage enregistré".into()),
            Box::default(),
            0,
        ));
        assert!(has_toast(&a, ToastKind::Success, "Réglage enregistré"));
        let _ = a.update(Message::ConfigSaved(Ok(true), None, Box::default(), 0));
        assert_eq!(
            a.toasts.items().len(),
            2,
            "pas de toast sans texte de succès"
        );
    }

    #[test]
    fn successful_changes_toast_a_confirmation() {
        // En démo, aucune écriture : le toast de succès est montré tout de suite.
        let d = tempfile::tempdir().unwrap();
        let cli = Cli {
            demo: true,
            ..Cli::default()
        };
        let mut a = App::new(cli, Config::default(), d.path().join("c.toml"));
        let _ = a.update(scanned(&a, demo::statuses(1_700_000_000)));
        let p = a.entries[0].path.clone();
        let _ = a.update(Message::Role(p, RoleAction::Track));
        assert!(has_toast(&a, ToastKind::Success, "Rôle modifié"));
        let _ = a.update(Message::Settings(settings::Msg::Depth(true)));
        assert!(has_toast(&a, ToastKind::Success, "Profondeur de scan"));
        let _ = a.update(Message::Settings(settings::Msg::Interval(true)));
        assert!(has_toast(&a, ToastKind::Success, "Intervalle"));
        let _ = a.update(Message::Settings(settings::Msg::TokenClear));
        assert!(has_toast(&a, ToastKind::Success, "effacé"));
    }

    #[test]
    fn adding_a_root_reports_errors_under_the_field() {
        let (mut a, d) = app();
        let missing = d.path().join("absent");
        let _ = a.update(Message::Settings(settings::Msg::InputChanged(
            missing.display().to_string(),
        )));
        let _ = a.update(Message::Settings(settings::Msg::AddRoot));
        assert!(a.root_error.as_deref().unwrap().contains("n'existe pas"));
        assert!(a.cfg.scan_roots.is_empty());
        assert!(!a.new_root.is_empty(), "la saisie est conservée");
        // Modifier la saisie efface l'erreur.
        let _ = a.update(Message::Settings(settings::Msg::InputChanged(
            d.path().display().to_string(),
        )));
        assert!(a.root_error.is_none());
        let _ = a.update(Message::Settings(settings::Msg::AddRoot));
        assert_eq!(a.cfg.scan_roots, [d.path().to_path_buf()]);
        assert!(a.new_root.is_empty() && a.root_error.is_none());
        assert!(a.scanning);
        // Doublon.
        let _ = a.update(Message::Settings(settings::Msg::InputChanged(
            d.path().display().to_string(),
        )));
        let _ = a.update(Message::Settings(settings::Msg::AddRoot));
        assert!(a.root_error.as_deref().unwrap().contains("existe déjà"));
        let _ = a.update(Message::Settings(settings::Msg::ClearInput));
        assert!(a.new_root.is_empty() && a.root_error.is_none());
    }

    #[test]
    fn token_save_keeps_the_input_until_it_succeeds() {
        let (mut a, _d) = app();
        let _ = a.update(Message::Settings(settings::Msg::TokenInput(
            settings::Secret("ghp_x".into()),
        )));
        let _ = a.update(Message::Settings(settings::Msg::TokenSave));
        assert!(a.token_busy);
        assert_eq!(a.token_input, "ghp_x", "pas vidée avant le résultat");
        // Un second clic pendant l'enregistrement est ignoré.
        let _ = a.update(Message::Settings(settings::Msg::TokenSave));
        let _ = a.update(Message::TokenSaved(Err("permission refusée".into())));
        assert!(!a.token_busy);
        assert_eq!(a.token_input, "ghp_x", "la saisie survit à l'échec");
        assert!(has_toast(&a, ToastKind::Error, "permission refusée"));
        assert!(!a.token_saved);
        let _ = a.update(Message::TokenSaved(Ok(())));
        assert!(a.token_input.is_empty());
        assert!(a.token_saved && a.token_source == TokenSource::Interface);
        assert!(has_toast(&a, ToastKind::Success, "enregistré"));
        // Le secret ne fuit ni dans le journal ni dans les toasts.
        assert!(!a.log.iter().any(|l| l.text.contains("ghp_x")));
        assert!(!toast_titles(&a).iter().any(|t| t.contains("ghp_x")));
    }

    #[test]
    fn token_clear_result_updates_state_and_toasts() {
        let (mut a, _d) = app();
        a.token_saved = true;
        a.token_source = TokenSource::Interface;
        let _ = a.update(Message::Settings(settings::Msg::TokenClear));
        assert!(a.token_busy);
        let _ = a.update(Message::TokenCleared(Ok((false, TokenSource::Aucun))));
        assert!(!a.token_busy && !a.token_saved);
        assert!(has_toast(&a, ToastKind::Success, "effacé"));
        let _ = a.update(Message::TokenCleared(Err("verrouillé".into())));
        assert!(has_toast(&a, ToastKind::Error, "verrouillé"));
    }

    #[test]
    fn startup_state_comes_from_background_tasks() {
        let (mut a, _d) = app();
        // `App::new` ne lit ni le jeton ni les préférences système.
        assert!(!a.token_saved && a.token_source == TokenSource::Aucun);
        assert_eq!(a.mode, Mode::Dark);
        let _ = a.update(Message::TokenLoaded(true, TokenSource::Interface));
        assert!(a.token_saved);
        let _ = a.update(Message::ModeDetected(None));
        assert_eq!(a.mode, Mode::Dark, "détection impossible : mode inchangé");
        let _ = a.update(Message::ModeDetected(Some(Mode::Light)));
        assert_eq!(a.mode, Mode::Light);
        let _ = a.update(Message::ModeDetected(None));
        assert_eq!(a.mode, Mode::Light, "None n'écrase pas un thème déjà reçu");
    }

    #[test]
    fn daemon_check_without_daemon_explains_itself() {
        let (mut a, _d) = app();
        let _ = a.update(Message::DaemonCheckNow);
        assert!(has_toast(&a, ToastKind::Error, "Démon absent"));
    }

    #[test]
    fn inputs_read_errors_are_toasted() {
        let (mut a, _d) = with_repo(Role::Tracked, GitState::default());
        let _ = a.update(Message::InputsLoaded(
            "/r".into(),
            Err("lock illisible".into()),
        ));
        assert!(has_toast(&a, ToastKind::Error, "lock illisible"));
    }

    // -- aide, palette étendue, terminal ------------------------------------------

    #[test]
    fn question_mark_opens_the_help_and_escape_closes_it() {
        let (mut a, _d) = three_repos();
        let _ = a.update(Message::Key(
            Key::Character("?".into()),
            Modifiers::SHIFT,
            false,
        ));
        assert!(a.help_open);
        // L'aide capte le clavier : j ne déplace plus la sélection, Ctrl K n'ouvre rien.
        let _ = a.update(key("j"));
        assert_eq!(a.selected.as_deref(), Some(Path::new("/a")));
        let _ = a.update(ctrl_k());
        assert!(!a.palette_open);
        let _ = a.update(named(Named::Escape));
        assert!(!a.help_open);
        // Aussi depuis la palette.
        let _ = a.update(ctrl_k());
        let _ = a.update(Message::PaletteInput("raccourcis".into()));
        assert_eq!(a.palette_results()[0].command, Command::Help);
        let _ = a.update(Message::PaletteRun(0));
        assert!(a.help_open && !a.palette_open);
        let _ = a.update(Message::Key(
            Key::Character("?".into()),
            Modifiers::SHIFT,
            false,
        ));
        assert!(!a.help_open, "? ferme aussi l'aide");
    }

    #[test]
    fn palette_lists_every_command_without_truncation() {
        let (mut a, _d) = three_repos();
        let _ = a.update(Message::Role("/a".into(), RoleAction::SetSystem));
        a.daemon = Some(DaemonStatus::default());
        let _ = a.update(ctrl_k());
        let labels: Vec<String> = a.palette_results().into_iter().map(|i| i.label).collect();
        for want in [
            "Vérifier",
            "Mettre à jour",
            "Appliquer au système",
            "Paquets",
            "Terminal",
            "Réglages",
            "Tout vérifier",
            "Rescanner",
            "Vérifier via le démon",
            "Raccourcis clavier",
            "Afficher la console",
        ] {
            assert!(
                labels.iter().any(|l| l.contains(want)
                    || (want == "Afficher la console" && l.contains("console"))),
                "manque : {want} ({labels:?})"
            );
        }
        assert!(labels
            .iter()
            .any(|l| l == "Ignorer ce dépôt" || l == "Ne plus suivre ce dépôt"));
        assert!(labels.len() > 9, "plus de neuf lignes : la liste défile");
        assert!(
            labels.iter().any(|l| l == "c"),
            "les dépôts suivent les actions"
        );
    }

    #[test]
    fn palette_role_and_navigation_commands_run() {
        let (mut a, _d) = three_repos();
        let _ = a.update(ctrl_k());
        let _ = a.update(Message::PaletteInput("ignorer".into()));
        let _ = a.update(Message::PaletteRun(0));
        assert!(matches!(
            a.modal,
            Some(Modal::Confirm(ConfirmAction::Role(_, RoleAction::Ignore)))
        ));
        let _ = a.update(Message::ModalCancel);
        let _ = a.update(ctrl_k());
        let _ = a.update(Message::PaletteInput("paquets".into()));
        let _ = a.update(Message::PaletteRun(0));
        assert_eq!(a.page, Page::Packages);
        let _ = a.update(ctrl_k());
        let _ = a.update(Message::PaletteInput("terminal".into()));
        let _ = a.update(Message::PaletteRun(0));
        assert_eq!(a.page, Page::Terminal);
    }

    #[test]
    fn ctrl_k_opens_the_palette_on_the_terminal_page_other_keys_go_to_the_shell() {
        let (mut a, _d) = three_repos();
        a.page = Page::Terminal;
        let _ = a.update(key("j"));
        let _ = a.update(key("?"));
        let _ = a.update(key("a"));
        let _ = a.update(named(Named::Escape));
        assert_eq!(a.selected.as_deref(), Some(Path::new("/a")));
        assert!(!a.help_open && a.modal.is_none());
        assert_eq!(a.page, Page::Terminal, "Échap appartient au shell");
        let _ = a.update(ctrl_k());
        assert!(a.palette_open);
        let _ = a.update(named(Named::Escape));
        assert!(!a.palette_open);
        assert_eq!(a.page, Page::Terminal);
    }

    #[test]
    fn new_shortcuts_drive_tabs_and_settings() {
        let (mut a, _d) = three_repos();
        let _ = a.update(key("2"));
        assert_eq!(a.tab, Tab::Git);
        let _ = a.update(key("3"));
        assert_eq!(a.tab, Tab::Repo);
        let _ = a.update(key("1"));
        assert_eq!(a.tab, Tab::Updates);
        let _ = a.update(key(","));
        assert_eq!(a.page, Page::Settings);
        let _ = a.update(named(Named::Escape));
        assert_eq!(a.page, Page::Main);
    }

    #[test]
    fn escape_on_packages_cancels_the_flow_before_leaving() {
        let (mut a, _d) = app();
        a.page = Page::Packages;
        a.packages.begin_op(packages::Op::Add {
            name: "ripgrep".into(),
        });
        assert!(a.packages.flow.is_some());
        let _ = a.update(named(Named::Escape));
        assert!(a.packages.flow.is_none());
        assert_eq!(a.page, Page::Packages);
        let _ = a.update(named(Named::Escape));
        assert_eq!(a.page, Page::Main);
    }

    #[test]
    fn clearing_the_package_search_resets_results() {
        let (mut a, _d) = app();
        a.packages.query = "git".into();
        let (seq, _) = a.packages.begin_search().unwrap();
        a.packages.finish_search(seq, Err("hors ligne".into()));
        assert!(a.packages.search_error.is_some());
        let _ = a.update(Message::Packages(packages::Msg::ClearQuery));
        assert!(a.packages.query.is_empty());
        assert!(a.packages.search_error.is_none() && a.packages.results.is_empty());
    }

    #[test]
    fn pages_from_the_sidebar_leave_the_narrow_sidebar() {
        let (mut a, _d) = app();
        a.sidebar_open = true;
        let _ = a.update(Message::OpenSettings);
        assert!(!a.sidebar_open);
        a.sidebar_open = true;
        let _ = a.update(Message::OpenTerminal);
        assert!(!a.sidebar_open);
        let _ = a.update(Message::GoMain);
        assert_eq!(a.page, Page::Main);
    }

    // -- dépôt système configurable ------------------------------------------------

    fn flake_dir(d: &Path, name: &str) -> PathBuf {
        let p = d.join(name);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("flake.nix"), "{}").unwrap();
        p
    }

    #[test]
    fn saving_the_system_repo_persists_it_with_toast_and_attr() {
        let (mut a, d) = app();
        let repo = flake_dir(d.path(), "ma config");
        let _ = a.update(Message::Settings(settings::Msg::SysRepoInput(
            repo.display().to_string(),
        )));
        let _ = a.update(Message::Settings(settings::Msg::SysAttrInput(
            "laptop".into(),
        )));
        assert!(a.sys_error.is_none());
        let _ = a.update(Message::Settings(settings::Msg::SysRepoSave));
        let canon = repo.canonicalize().unwrap();
        assert_eq!(a.cfg.system_repo.as_deref(), Some(canon.as_path()));
        assert_eq!(a.cfg.system_attr.as_deref(), Some("laptop"));
        assert_eq!(a.system_repo_path(), Some(canon));
        // L'écriture est asynchrone : le toast de succès arrive avec `ConfigSaved`.
        let _ = a.update(Message::ConfigSaved(
            Ok(true),
            Some("Dépôt système enregistré".into()),
            Box::default(),
            0,
        ));
        assert!(has_toast(
            &a,
            ToastKind::Success,
            "Dépôt système enregistré"
        ));
    }

    #[test]
    fn invalid_system_repo_input_shows_inline_error_and_saves_nothing() {
        let (mut a, d) = app();
        let _ = a.update(Message::Settings(settings::Msg::SysRepoInput(
            d.path().join("absent").display().to_string(),
        )));
        assert!(a.sys_error.as_deref().unwrap().contains("n'existe pas"));
        let _ = a.update(Message::Settings(settings::Msg::SysRepoSave));
        assert!(a.cfg.system_repo.is_none());
        let _ = a.update(Message::Settings(settings::Msg::SysRepoInput(
            "relatif".into(),
        )));
        assert!(a.sys_error.as_deref().unwrap().contains("absolu"));
        let _ = a.update(Message::Settings(settings::Msg::SysRepoInput(
            d.path().display().to_string(),
        )));
        assert!(a.sys_error.as_deref().unwrap().contains("flake.nix"));
    }

    #[test]
    fn reset_returns_to_the_system_default() {
        let (mut a, d) = app();
        let repo = flake_dir(d.path(), "depot");
        a.cfg.system_repo = Some(repo);
        a.cfg.system_attr = Some("x".into());
        let _ = a.update(Message::Settings(settings::Msg::SysRepoReset));
        assert!(a.cfg.system_repo.is_none() && a.cfg.system_attr.is_none());
        assert!(a.sys_repo_input.is_empty() && a.sys_attr_input.is_empty());
    }

    #[test]
    fn empty_state_and_palette_entry_lead_to_the_system_repo_setting() {
        let (mut a, _d) = app();
        // Palette : « Définir le dépôt système… » ouvre les réglages.
        assert!(a
            .palette_items()
            .iter()
            .any(|i| i.command == Command::SetSystemRepo));
        let _ = a.update(Message::OpenSystemRepoSettings);
        assert_eq!(a.page, Page::Settings);
        // Sans dépôt système (hors machine ayant un défaut), l'écran principal propose le choix.
        if a.system_repo_path().is_none() {
            assert!(a.cfg.system_repo.is_none());
        }
    }

    #[test]
    fn apply_uses_the_system_attr_only_for_the_system_repo() {
        let (mut a, _d) = app();
        a.cfg.system_repo = Some("/sys".into());
        a.cfg.system_attr = Some("laptop".into());
        assert_eq!(a.apply_attr(Path::new("/sys")).as_deref(), Some("laptop"));
        assert_eq!(a.apply_attr(Path::new("/autre")), None);
    }

    #[test]
    fn failed_config_save_restores_the_persisted_state() {
        let (mut a, _d) = app();
        a.cfg.scan_depth = 3;
        a.cfg_persisted = a.cfg.clone();
        // Un réglage modifié en mémoire, puis l'écriture échoue : retour à l'état du disque.
        a.cfg.scan_depth = 9;
        let generation = a.save_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = a.update(Message::ConfigSaved(
            Err("disque plein".into()),
            None,
            Box::new(a.cfg.clone()),
            generation,
        ));
        assert_eq!(a.cfg.scan_depth, 3, "l'affichage reflète le disque");
        assert!(has_toast(&a, ToastKind::Error, "disque plein"));
        // Un échec périmé (un enregistrement plus récent est en vol) ne restaure rien.
        a.cfg.scan_depth = 5;
        let old = a.save_gen.fetch_add(2, Ordering::SeqCst) + 1;
        let _ = a.update(Message::ConfigSaved(
            Err("tard".into()),
            None,
            Box::new(a.cfg.clone()),
            old,
        ));
        assert_eq!(a.cfg.scan_depth, 5);
        // Un succès fait avancer l'état persisté.
        let latest = a.save_gen.load(Ordering::SeqCst);
        let _ = a.update(Message::ConfigSaved(
            Ok(true),
            None,
            Box::new(a.cfg.clone()),
            latest,
        ));
        assert_eq!(a.cfg_persisted.scan_depth, 5);
    }

    #[test]
    fn enter_does_not_confirm_the_next_queued_modal_right_away() {
        let (mut a, _d) = with_repo(Role::System, dirty());
        a.open_modal(Modal::Confirm(ConfirmAction::Push("/r".into())));
        a.open_modal(Modal::Confirm(ConfirmAction::Apply("/r".into())));
        assert_eq!(a.modal_queue.len(), 1);
        let _ = a.update(named(Named::Enter));
        assert_eq!(a.entries[0].busy, Some(JobKind::Push));
        assert!(
            matches!(a.modal, Some(Modal::Confirm(ConfirmAction::Apply(_)))),
            "le dialogue suivant est affiché"
        );
        // Deuxième Entrée dans le délai de garde : ignorée.
        a.entries[0].busy = None;
        let _ = a.update(named(Named::Enter));
        assert!(a.modal.is_some(), "Entrée trop rapide ignorée");
        assert_eq!(a.entries[0].busy, None);
        // Les clics restent actifs pendant la garde.
        let guard = a.modal_guard_until.unwrap();
        assert!(a.now < guard);
        // Après le délai, Entrée confirme.
        a.now = guard + Duration::from_millis(1);
        let _ = a.update(named(Named::Enter));
        assert!(a.modal.is_none());
        assert_eq!(a.entries[0].busy, Some(JobKind::Apply));
    }
}
