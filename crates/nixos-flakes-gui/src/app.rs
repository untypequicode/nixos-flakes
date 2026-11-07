//! État de l'application et logique de mise à jour (voir `view.rs` pour l'affichage).

use crate::anim::{self, Fades, Tween};
use crate::cli::Cli;
use crate::demo;
use crate::keys::{self, KeyContext, Shortcut};
use crate::model::{
    self, Actions, JobKind, Modal, RepoEntry, RoleAction, UpdatePlan, DIRTY_COMMIT_MESSAGE,
};
use crate::palette::{self, Command, PaletteState};
use crate::settings;
use crate::tasks::{self, Finished, Job, JobEvent, Outcome};
use crate::theme::{self, Mode, Tokens};
use crate::toast::{ToastKind, Toasts};
use crate::tray::{self, TrayEvent, TrayHandle, TrayState, TrayStatus};
use crate::widgets::{self, Ui};
use iced::futures::{stream, Stream, StreamExt};
use iced::keyboard::{Key, Modifiers};
use iced::widget::text_editor;
use iced::{window, Subscription, Task, Theme};
use nixos_flakes_core::config::{Config, Role};
use nixos_flakes_core::git;
use nixos_flakes_core::status::RepoStatus;
use nixos_flakes_core::token::Source as TokenSource;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Nombre maximal de lignes conservées dans la console.
pub const LOG_CAPACITY: usize = 3000;
/// Hauteur du corps de la console déployée.
pub const CONSOLE_HEIGHT: f32 = 190.0;
/// Identifiant du champ de saisie de la palette.
pub const PALETTE_INPUT: &str = "palette-input";
/// Nombre maximal de lignes affichées dans la palette.
pub const PALETTE_ROWS: usize = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Main,
    Settings,
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

#[derive(Debug, Clone)]
pub enum Message {
    // Données
    Rescan,
    ScanDone(Vec<RepoStatus>),
    Select(PathBuf),
    // Commandes
    Check(PathBuf),
    CheckAll,
    Update(PathBuf),
    Apply(PathBuf),
    Push(PathBuf),
    Role(PathBuf, RoleAction),
    Job { repo: PathBuf, event: JobEvent },
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
    Settings(settings::Msg),
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
    // Fenêtre et tray
    CloseRequested(window::Id),
    WindowClosed(window::Id),
    Tray(TrayEvent),
    TrayStarted(Option<Result<TrayHandle, String>>),
    Tick,
}

pub struct App {
    pub cli: Cli,
    pub cfg: Config,
    pub cfg_path: PathBuf,
    pub home: Option<PathBuf>,
    pub entries: Vec<RepoEntry>,
    pub selected: Option<PathBuf>,
    pub scanning: bool,
    pub log: VecDeque<LogLine>,
    pub console_open: bool,
    pub modal: Option<Modal>,
    pub editor: text_editor::Content,
    pub page: Page,
    pub tab: Tab,
    pub mode: Mode,
    pub window: Option<window::Id>,
    pub sidebar_open: bool,
    pub queue: VecDeque<PathBuf>,
    pub queue_notify: bool,
    pub queue_new: Vec<(String, usize)>,
    pub followups: HashMap<PathBuf, Followup>,
    pub tray: Option<TrayHandle>,
    pub tray_pushed: Option<TrayState>,
    pub tray_status: TrayStatus,
    /// Saisie du jeton GitHub : vidée dès l'enregistrement, jamais journalisée ni écrite en config.
    pub token_input: String,
    pub token_saved: bool,
    /// Origine du jeton (fichier Nix = lecture seule). Toujours `Aucun`/`Interface` en démo.
    pub token_source: nixos_flakes_core::token::Source,
    pub new_root: String,
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
}

/// Paramètres de la fenêtre principale.
pub fn window_settings() -> window::Settings {
    window::Settings {
        size: iced::Size::new(1180.0, 760.0),
        min_size: Some(iced::Size::new(420.0, 400.0)),
        exit_on_close_request: false,
        platform_specific: window::settings::PlatformSpecific {
            application_id: "nixos-flakes-gui".to_string(),
            ..Default::default()
        },
        ..window::Settings::default()
    }
}

/// Démarrage : charge la config, enregistre éventuellement l'icône, ouvre la fenêtre.
pub fn boot(cli: Cli) -> (App, Task<Message>) {
    let cfg_path = cli.config.clone().unwrap_or_else(Config::default_path);
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

    let want_tray = app.cli.tray || app.cfg.tray_enabled;
    if want_tray && !app.cli.demo {
        match tray::spawn(TrayState::UpToDate) {
            Ok(h) => {
                app.tray = Some(h);
                app.tray_status = TrayStatus::Registered;
            }
            Err(e) => {
                app.push_log(
                    LogKind::Error,
                    format!("Zone de notification indisponible : {e}. L'application continue sans icône."),
                );
                app.tray_status = TrayStatus::Failed(e);
            }
        }
    }
    let start_hidden = app.cli.tray && app.tray.is_some();

    let mut tasks = vec![app.scan_task()];
    // En démo, le thème reste celui demandé (vue « clair ») au lieu de suivre le système.
    if !app.cli.demo {
        tasks.push(iced::system::theme().map(Message::SystemTheme));
    }
    if !start_hidden {
        tasks.push(app.open_window());
    }
    (app, Task::batch(tasks))
}

/// Un jeton est-il déjà enregistré ? (Jamais en démo : on ne touche pas au vrai fichier.)
fn demo_token_saved(demo: bool) -> bool {
    !demo && nixos_flakes_core::token::exists()
}

/// Origine du jeton ; en démo on ignore l'environnement et le vrai fichier.
fn demo_token_source(demo: bool) -> nixos_flakes_core::token::Source {
    if demo {
        nixos_flakes_core::token::Source::Aucun
    } else {
        nixos_flakes_core::token::source()
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
        App {
            cli,
            cfg,
            cfg_path,
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
            log: VecDeque::new(),
            console_open: true,
            modal: None,
            editor: text_editor::Content::new(),
            page: Page::Main,
            tab: Tab::Updates,
            mode: if demo {
                Mode::Dark
            } else {
                theme::detect_system_mode()
            },
            window: None,
            sidebar_open: false,
            queue: VecDeque::new(),
            queue_notify: false,
            queue_new: Vec::new(),
            followups: HashMap::new(),
            tray: None,
            tray_pushed: None,
            tray_status: TrayStatus::Disabled,
            token_input: String::new(),
            token_saved: demo_token_saved(demo),
            token_source: demo_token_source(demo),
            new_root: String::new(),
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
        if self.cli.demo {
            let now = unix_now();
            return Task::perform(
                tasks::blocking(move || {
                    std::thread::sleep(Duration::from_millis(900));
                    demo::statuses(now)
                }),
                |r| Message::ScanDone(r.unwrap_or_default()),
            );
        }
        let cfg = self.cfg.clone();
        Task::perform(tasks::blocking(move || tasks::scan_all(&cfg)), |r| {
            Message::ScanDone(r.unwrap_or_default())
        })
    }

    /// Lance une commande si le dépôt est libre (une seule à la fois par dépôt).
    pub fn start_job(&mut self, repo: &Path, job: Job) -> Task<Message> {
        let kind = job.kind();
        let name = self.repo_name(repo);
        let demo_mode = self.cli.demo;
        let Some(entry) = self.entry_mut(repo) else {
            return Task::none();
        };
        if let Some(busy) = entry.busy {
            let msg = format!("{name} : {} déjà en cours.", busy.label());
            self.push_log(LogKind::Error, msg);
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

    fn start_next_in_queue(&mut self) -> Task<Message> {
        match model::pick_next(&mut self.queue, &self.entries) {
            Some(p) => self.start_job(&p, Job::Check),
            None => {
                self.finish_queue();
                Task::none()
            }
        }
    }

    fn finish_queue(&mut self) {
        if self.queue_notify && !self.queue_new.is_empty() {
            let (title, body) = model::notification_text(&self.queue_new);
            tasks::notify(title, body);
        }
        self.queue_notify = false;
        self.queue_new.clear();
    }

    fn begin_check_all(&mut self, notify: bool) -> Task<Message> {
        if !self.queue.is_empty() {
            return Task::none();
        }
        self.queue = model::check_all_targets(&self.entries).into();
        self.queue_notify = notify;
        self.queue_new.clear();
        if self.queue.is_empty() {
            self.push_log(LogKind::Info, "Aucun dépôt suivi à vérifier.");
            return Task::none();
        }
        self.start_next_in_queue()
    }

    fn begin_update(&mut self, repo: &Path) -> Task<Message> {
        let Some(entry) = self.entry(repo) else {
            return Task::none();
        };
        if !model::actions(entry).update {
            return Task::none();
        }
        match model::plan_update(&entry.git) {
            UpdatePlan::Proceed => self.start_job(repo, Job::Update),
            UpdatePlan::AskCommit => {
                self.open_modal(Modal::CommitDirty {
                    repo: repo.to_path_buf(),
                    dirty: entry.git.dirty.clone(),
                    message: DIRTY_COMMIT_MESSAGE.to_string(),
                });
                Task::none()
            }
        }
    }

    fn open_modal(&mut self, modal: Modal) {
        let text = match &modal {
            Modal::CommitDirty { message, .. } | Modal::CommitLock { message, .. } => {
                message.clone()
            }
        };
        self.editor = text_editor::Content::with_text(&text);
        self.modal = Some(modal);
        self.modal_leaving = None;
        self.modal_t.retarget(1.0, self.now, anim::SLOW);
        self.reset_hover();
    }

    /// Ferme le dialogue (animation de sortie) et le renvoie.
    fn take_modal(&mut self) -> Option<Modal> {
        let m = self.modal.take();
        if m.is_some() {
            self.modal_leaving = m.clone();
            self.modal_t.retarget(0.0, self.now, anim::BASE);
            self.reset_hover();
        }
        m
    }

    // -- config -------------------------------------------------------------

    fn save_config(&mut self) {
        if self.cli.demo {
            return;
        }
        if let Err(e) = tasks::save_config(&self.cfg, &self.cfg_path) {
            self.push_log(
                LogKind::Error,
                format!("Enregistrement de la configuration impossible : {e}"),
            );
        }
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
        let task = self.update_inner(message);
        self.sync_tray();
        task
    }

    fn update_inner(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Rescan => self.scan_task(),
            Message::ScanDone(fresh) => {
                self.scanning = false;
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
                if self.cli.demo {
                    return self.apply_demo_views();
                }
                Task::none()
            }
            Message::Select(p) => {
                if self.selected.as_ref() != Some(&p) {
                    self.restart_panel();
                }
                self.selected = Some(p);
                self.sidebar_open = false;
                self.page = Page::Main;
                self.sync_selection_fades();
                self.reset_hover();
                Task::none()
            }
            Message::Check(p) => self.start_job(&p, Job::Check),
            Message::CheckAll => self.begin_check_all(false),
            Message::Update(p) => self.begin_update(&p),
            Message::Apply(p) => {
                if self.actions_for(&p).apply {
                    self.start_job(&p, Job::Apply)
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
            Message::Role(p, action) => {
                self.cfg.set_role(&p, action.target());
                self.save_config();
                self.refresh_roles();
                Task::none()
            }
            Message::Job { repo, event } => self.on_job_event(repo, event),
            Message::EditorAction(action) => {
                self.editor.perform(action);
                let text = self.editor.text();
                if let Some(m) = self.modal.as_mut() {
                    *m.message_mut() = text;
                }
                Task::none()
            }
            Message::ModalCommit => self.on_modal_commit(),
            Message::ModalContinue => match self.take_modal() {
                Some(Modal::CommitDirty { repo, .. }) => self.start_job(&repo, Job::Update),
                _ => Task::none(),
            },
            Message::ModalCancel => {
                self.take_modal();
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
                self.restart_panel();
                self.reset_hover();
                Task::none()
            }
            Message::Settings(msg) => self.on_settings(msg),
            Message::SystemTheme(m) => {
                if let Some(m) = theme::from_iced(m) {
                    self.set_theme_mode(m);
                }
                Task::none()
            }
            Message::CloseRequested(id) => {
                if self.tray.is_some() {
                    window::close(id)
                } else {
                    iced::exit()
                }
            }
            Message::WindowClosed(id) => {
                if self.window == Some(id) {
                    self.window = None;
                }
                Task::none()
            }
            Message::Tray(ev) => self.on_tray_event(ev),
            Message::TrayStarted(res) => {
                match res {
                    Some(Ok(h)) => {
                        self.tray_pushed = None;
                        self.tray = Some(h);
                        self.tray_status = TrayStatus::Registered;
                        self.push_log(LogKind::Info, "Icône de notification activée.");
                    }
                    Some(Err(e)) => {
                        self.push_log(
                            LogKind::Error,
                            format!("Zone de notification indisponible : {e}"),
                        );
                        self.tray_status = TrayStatus::Failed(e);
                    }
                    None => {}
                }
                Task::none()
            }
            Message::Tick => self.begin_check_all(true),
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
                Task::none()
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

    fn on_settings(&mut self, msg: settings::Msg) -> Task<Message> {
        use settings::Msg;
        match msg {
            Msg::InputChanged(s) => self.new_root = s,
            Msg::AddRoot => {
                let input = std::mem::take(&mut self.new_root);
                match settings::add_root(&mut self.cfg, &input, self.home.as_deref()) {
                    Ok(()) => {
                        self.save_config();
                        return self.scan_task();
                    }
                    Err(e) => {
                        self.new_root = input;
                        self.push_log(LogKind::Error, e);
                    }
                }
            }
            Msg::RemoveRoot(i) => {
                if settings::remove_root(&mut self.cfg, i) {
                    self.save_config();
                    return self.scan_task();
                }
            }
            Msg::Depth(up) => {
                self.cfg.scan_depth = settings::step_depth(self.cfg.scan_depth, up);
                self.save_config();
            }
            Msg::Interval(up) => {
                self.cfg.check_interval_minutes =
                    settings::step_interval(self.cfg.check_interval_minutes, up);
                self.save_config();
            }
            Msg::ToggleTray(on) => {
                self.cfg.tray_enabled = on;
                self.save_config();
                if on && self.tray.is_none() {
                    return Task::perform(
                        tasks::blocking(|| tray::spawn(TrayState::UpToDate)),
                        Message::TrayStarted,
                    );
                }
                if !on && !self.cli.tray {
                    if let Some(h) = self.tray.take() {
                        h.shutdown();
                    }
                    self.tray_status = TrayStatus::Disabled;
                }
            }
            Msg::TokenInput(s) => self.token_input = s.0,
            Msg::TokenSave => return self.save_token(),
            Msg::TokenClear => self.clear_token(),
            Msg::Rescan => return self.scan_task(),
            Msg::Back => self.page = Page::Main,
        }
        Task::none()
    }

    /// Enregistre le jeton saisi (la saisie est vidée dans tous les cas), puis relance une vérification.
    fn save_token(&mut self) -> Task<Message> {
        let input = std::mem::take(&mut self.token_input);
        let token = input.trim();
        if token.is_empty() || matches!(self.token_source, TokenSource::Fichier(_)) {
            return Task::none();
        }
        if self.cli.demo {
            self.toast(ToastKind::Info, "Mode démo : jeton non enregistré");
            return Task::none();
        }
        match nixos_flakes_core::token::save(token) {
            Ok(()) => {
                self.token_saved = true;
                self.token_source = TokenSource::Interface;
                self.push_log(LogKind::Success, "Jeton GitHub enregistré.");
                return self.begin_check_all(true);
            }
            Err(e) => self.push_log(LogKind::Error, format!("Jeton GitHub non enregistré : {e}")),
        }
        Task::none()
    }

    fn clear_token(&mut self) {
        self.token_input.clear();
        if matches!(
            self.token_source,
            TokenSource::Fichier(_) | TokenSource::Systeme { .. }
        ) {
            return;
        }
        if self.cli.demo {
            self.token_saved = false;
            return;
        }
        match nixos_flakes_core::token::clear() {
            Ok(()) => {
                // Le fichier système peut reprendre le relais une fois le jeton effacé.
                self.token_saved = nixos_flakes_core::token::exists();
                self.token_source = nixos_flakes_core::token::source();
                self.push_log(LogKind::Info, "Jeton GitHub effacé.");
            }
            Err(e) => self.push_log(LogKind::Error, format!("Jeton GitHub non effacé : {e}")),
        }
    }

    fn on_modal_commit(&mut self) -> Task<Message> {
        match self.take_modal() {
            Some(Modal::CommitDirty { repo, message, .. }) => {
                self.followups.insert(repo.clone(), Followup::Update);
                self.start_job(&repo, Job::Commit(message.trim_end().to_string()))
            }
            Some(Modal::CommitLock { repo, message }) => {
                self.start_job(&repo, Job::CommitLock(message.trim_end().to_string()))
            }
            None => Task::none(),
        }
    }

    fn on_tray_event(&mut self, ev: TrayEvent) -> Task<Message> {
        match ev {
            TrayEvent::Open => match self.window {
                Some(id) => Task::batch([window::minimize(id, false), window::gain_focus(id)]),
                None => self.open_window(),
            },
            TrayEvent::CheckNow => self.begin_check_all(true),
            TrayEvent::Quit => {
                if let Some(h) = self.tray.take() {
                    h.shutdown();
                }
                iced::exit()
            }
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
        let Some(entry) = self.entry_mut(&repo) else {
            return Task::none();
        };
        entry.busy = None;
        entry.git = git.clone();
        let mut next = Task::none();
        let mut logs: Vec<(LogKind, String)> = Vec::new();
        let mut is_check = false;
        let mut modal = None;
        let mut new_update: Option<(String, usize)> = None;

        match outcome {
            Outcome::Check(res) => {
                is_check = true;
                match res {
                    Ok(report) => {
                        let n = report.changes.len();
                        if model::has_new_updates(entry.pending.as_deref(), &report.changes) {
                            new_update = Some((name.clone(), n));
                        }
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
                }
            }
            Outcome::Update(res) => match res {
                Ok(report) => {
                    entry.error = None;
                    entry.pending = Some(Vec::new());
                    entry.last_update = Some(report.changes.clone());
                    logs.push((
                        LogKind::Success,
                        format!(
                            "✓ {name} : mise à jour terminée ({} input(s) modifié(s))",
                            report.changes.len()
                        ),
                    ));
                    if model::should_propose_lock_commit(&git) {
                        modal = Some(Modal::CommitLock {
                            repo: repo.clone(),
                            message: git::lock_commit_message(&report.changes),
                        });
                    }
                }
                Err(e) => {
                    entry.error = Some(e.clone());
                    logs.push((
                        LogKind::Error,
                        format!("✕ {name} : mise à jour échouée\n{e}"),
                    ));
                }
            },
            Outcome::Done(kind, res) => match res {
                Ok(()) => {
                    entry.error = None;
                    let hint = if kind == JobKind::CommitLock {
                        " — pensez à pousser (bouton Push)"
                    } else {
                        ""
                    };
                    logs.push((
                        LogKind::Success,
                        format!("✓ {name} : {} terminé{hint}", kind.label()),
                    ));
                    if kind == JobKind::Commit && followup == Some(Followup::Update) {
                        next = Task::done(Message::Update(repo.clone()));
                    }
                }
                Err(e) => {
                    entry.error = Some(e.clone());
                    logs.push((
                        LogKind::Error,
                        format!("✕ {name} : {} échoué\n{e}", kind.label()),
                    ));
                }
            },
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
        if let Some(n) = new_update {
            if self.queue_notify {
                self.queue_new.push(n);
            }
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
                items.push(action(Command::Apply, "Appliquer au système", name, None));
            }
        }
        items.push(action(
            Command::CheckAll,
            "Tout vérifier",
            String::new(),
            None,
        ));
        items.push(action(Command::Rescan, "Rescanner", String::new(), None));
        items.push(action(Command::Settings, "Réglages", String::new(), None));
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

    /// Lignes visibles de la palette pour la saisie courante.
    pub fn palette_results(&self) -> Vec<palette::Item> {
        let mut r = palette::filter(&self.palette_items(), &self.palette.query);
        r.truncate(PALETTE_ROWS);
        r
    }

    fn open_palette(&mut self) -> Task<Message> {
        if self.modal.is_some() {
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
        for i in 0..PALETTE_ROWS {
            let on = f32::from(u8::from(i == self.palette.selected));
            self.fades
                .set(&format!("sel:pal:{i}"), on, 0.0, self.now, anim::FAST);
        }
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
            Command::Apply => selected.map(Message::Apply),
            Command::CheckAll => Some(Message::CheckAll),
            Command::Rescan => Some(Message::Rescan),
            Command::Settings => Some(Message::OpenSettings),
            Command::ToggleConsole => Some(Message::ToggleConsole),
        };
        msg.map_or_else(Task::none, |m| self.update_inner(m))
    }

    fn on_key(&mut self, key: &Key, mods: Modifiers, captured: bool) -> Task<Message> {
        let ctx = KeyContext {
            modal: self.modal.is_some(),
            palette: self.palette_open,
            settings: self.page == Page::Settings,
        };
        let Some(shortcut) = keys::resolve(key, mods, captured, ctx) else {
            return Task::none();
        };
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
                } else if self.modal.is_some() {
                    self.update_inner(Message::ModalCancel)
                } else if self.page == Page::Settings {
                    self.page = Page::Main;
                    self.restart_panel();
                    self.reset_hover();
                    Task::none()
                } else {
                    Task::none()
                }
            }
            Shortcut::Move(d) => {
                if self.palette_open {
                    let n = self.palette_results().len();
                    self.palette.move_by(d, n);
                    self.sync_palette_fades();
                } else {
                    let order = model::nav_order(&self.entries, &self.collapsed);
                    if let Some(p) = keys::step_selection(&order, self.selected.as_deref(), d) {
                        if self.selected.as_ref() != Some(&p) {
                            return self.update_inner(Message::Select(p));
                        }
                    }
                }
                Task::none()
            }
            Shortcut::Enter => {
                if self.palette_open {
                    let i = self.palette.selected;
                    self.run_palette(i)
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

    // -- tray ---------------------------------------------------------------

    fn sync_tray(&mut self) {
        let Some(h) = &self.tray else { return };
        let state = TrayState::from(model::overall(&self.entries));
        if self.tray_pushed != Some(state) {
            h.set_state(state);
            self.tray_pushed = Some(state);
        }
    }

    // -- abonnements --------------------------------------------------------

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            window::close_requests().map(Message::CloseRequested),
            window::close_events().map(Message::WindowClosed),
            iced::system::theme_changes().map(Message::SystemTheme),
            Subscription::run(tray_events),
        ];
        subs.push(iced::event::listen_with(key_events));
        // Images à ~60 Hz uniquement pendant une animation ; sinon un pouls lent
        // pour l'expiration des toasts, et rien du tout au repos.
        if self.is_animating() {
            subs.push(window::frames().map(|_| Message::Frame));
        } else if !self.toasts.is_empty() {
            subs.push(Subscription::run_with(250_u64, slow_ticker).map(|()| Message::Frame));
        }
        if self.tray.is_some() && self.cfg.check_interval_minutes > 0 {
            subs.push(
                Subscription::run_with(self.cfg.check_interval_minutes, ticker)
                    .map(|_| Message::Tick),
            );
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
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => Some(
            Message::Key(key, modifiers, status == iced::event::Status::Captured),
        ),
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

fn tray_events() -> impl Stream<Item = Message> {
    stream::iter(tray::take_receiver())
        .flatten()
        .map(Message::Tray)
}

/// Un tick toutes les `minutes` minutes (thread dédié, sans exécuteur particulier).
fn ticker(minutes: &u32) -> impl Stream<Item = ()> {
    let period = Duration::from_secs(u64::from(*minutes) * 60);
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
        let _ = a.update(Message::ScanDone(vec![s]));
        (a, d)
    }

    #[test]
    fn scan_populates_and_selects_first() {
        let (mut a, _d) = app();
        a.scanning = true;
        let _ = a.update(Message::ScanDone(vec![
            status("/b", GitState::default()),
            status("/a", GitState::default()),
        ]));
        assert!(!a.scanning);
        assert_eq!(a.entries.len(), 2);
        assert_eq!(a.selected.as_deref(), Some(Path::new("/a")));
    }

    #[test]
    fn role_change_is_persisted_and_applied() {
        let (mut a, d) = app();
        let _ = a.update(Message::ScanDone(vec![status("/r", GitState::default())]));
        let _ = a.update(Message::Role("/r".into(), RoleAction::Track));
        assert_eq!(a.entries[0].role, Role::Tracked);
        let saved = Config::load(&d.path().join("config.toml")).unwrap();
        assert!(saved.tracked.contains(Path::new("/r")));
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
        let _ = a.update(finished(Outcome::Update(Ok(report)), g));
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
        let _ = a.update(finished(Outcome::Update(Ok(report)), GitState::default()));
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
        let _ = a.update(Message::ScanDone(vec![
            status("/a", GitState::default()),
            status("/b", GitState::default()),
            status("/c", GitState::default()),
        ]));
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
            "/srv".into(),
        )));
        let _ = a.update(Message::Settings(settings::Msg::AddRoot));
        let _ = a.update(Message::Settings(settings::Msg::Depth(true)));
        let _ = a.update(Message::Settings(settings::Msg::Interval(true)));
        let saved = Config::load(&d.path().join("config.toml")).unwrap();
        assert_eq!(saved.scan_roots, [PathBuf::from("/srv")]);
        assert_eq!(saved.scan_depth, 5);
        assert_eq!(saved.check_interval_minutes, 120);
        assert!(a.new_root.is_empty());
    }

    #[test]
    fn close_request_without_tray_exits_and_with_tray_keeps_running() {
        // Sans tray : une tâche de sortie est produite ; avec tray : on ferme seulement la fenêtre.
        let (mut a, _d) = app();
        let id = window::Id::unique();
        a.window = Some(id);
        let _ = a.update(Message::CloseRequested(id));
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
        let _ = a.update(Message::ScanDone(vec![
            status("/a", GitState::default()),
            status("/b", GitState::default()),
            status("/c", GitState::default()),
        ]));
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
        let _ = a.update(Message::ScanDone(demo::statuses(1_700_000_000)));
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
        let _ = a.update(Message::ScanDone(demo::statuses(1_700_000_000)));
        assert!(matches!(a.modal, Some(Modal::CommitDirty { .. })));
        assert_eq!(a.mode, Mode::Light);
        assert!(a.demo_views.is_empty(), "appliquées une seule fois");
    }
}
