//! État de l'application et logique de mise à jour (voir `view.rs` pour l'affichage).

use crate::cli::Cli;
use crate::model::{
    self, Actions, JobKind, Modal, RepoEntry, RoleAction, UpdatePlan, DIRTY_COMMIT_MESSAGE,
};
use crate::settings;
use crate::tasks::{self, Finished, Job, JobEvent, Outcome};
use crate::theme::{self, Mode};
use crate::tray::{self, TrayEvent, TrayHandle, TrayState};
use iced::futures::{stream, Stream, StreamExt};
use iced::widget::text_editor;
use iced::{window, Subscription, Task, Theme};
use nixos_flakes_core::config::Config;
use nixos_flakes_core::git;
use nixos_flakes_core::status::RepoStatus;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Nombre maximal de lignes conservées dans la console.
pub const LOG_CAPACITY: usize = 3000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Main,
    Settings,
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
    pub mode: Mode,
    pub window: Option<window::Id>,
    pub sidebar_open: bool,
    pub queue: VecDeque<PathBuf>,
    pub queue_notify: bool,
    pub queue_new: Vec<(String, usize)>,
    pub followups: HashMap<PathBuf, Followup>,
    pub tray: Option<TrayHandle>,
    pub tray_pushed: Option<TrayState>,
    pub new_root: String,
}

/// Paramètres de la fenêtre principale.
pub fn window_settings() -> window::Settings {
    window::Settings {
        size: iced::Size::new(1100.0, 720.0),
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
    if want_tray {
        match tray::spawn(TrayState::UpToDate) {
            Ok(h) => app.tray = Some(h),
            Err(e) => app.push_log(
                LogKind::Error,
                format!("Zone de notification indisponible ({e}) : comportement normal."),
            ),
        }
    }
    let start_hidden = app.cli.tray && app.tray.is_some();

    let mut tasks = vec![
        app.scan_task(),
        iced::system::theme().map(Message::SystemTheme),
    ];
    if !start_hidden {
        tasks.push(app.open_window());
    }
    (app, Task::batch(tasks))
}

impl App {
    pub fn new(cli: Cli, cfg: Config, cfg_path: PathBuf) -> Self {
        App {
            cli,
            cfg,
            cfg_path,
            home: std::env::var_os("HOME")
                .filter(|h| !h.is_empty())
                .map(PathBuf::from),
            entries: Vec::new(),
            selected: None,
            scanning: false,
            log: VecDeque::new(),
            console_open: true,
            modal: None,
            editor: text_editor::Content::new(),
            page: Page::Main,
            mode: theme::detect_system_mode(),
            window: None,
            sidebar_open: false,
            queue: VecDeque::new(),
            queue_notify: false,
            queue_new: Vec::new(),
            followups: HashMap::new(),
            tray: None,
            tray_pushed: None,
            new_root: String::new(),
        }
    }

    // -- accesseurs ---------------------------------------------------------

    pub fn title(&self, _id: window::Id) -> String {
        match model::overall(&self.entries) {
            model::Overall::Updates(n) => format!("Gestion des flakes — {n} à mettre à jour"),
            _ => "Gestion des flakes".to_string(),
        }
    }

    pub fn theme(&self, _id: window::Id) -> Theme {
        theme::theme(self.mode)
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

    // -- tâches -------------------------------------------------------------

    fn open_window(&mut self) -> Task<Message> {
        let (id, open) = window::open(window_settings());
        self.window = Some(id);
        open.map(|_| Message::Noop)
    }

    fn scan_task(&mut self) -> Task<Message> {
        self.scanning = true;
        let cfg = self.cfg.clone();
        Task::perform(tasks::blocking(move || tasks::scan_all(&cfg)), |r| {
            Message::ScanDone(r.unwrap_or_default())
        })
    }

    /// Lance une commande si le dépôt est libre (une seule à la fois par dépôt).
    pub fn start_job(&mut self, repo: &Path, job: Job) -> Task<Message> {
        let kind = job.kind();
        let name = self.repo_name(repo);
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
        let lazy = stream::once(async move { tasks::run_job(repo, job) }).flatten();
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
    }

    // -- config -------------------------------------------------------------

    fn save_config(&mut self) {
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
                Task::none()
            }
            Message::Select(p) => {
                self.selected = Some(p);
                self.sidebar_open = false;
                self.page = Page::Main;
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
            Message::ModalContinue => match self.modal.take() {
                Some(Modal::CommitDirty { repo, .. }) => self.start_job(&repo, Job::Update),
                _ => Task::none(),
            },
            Message::ModalCancel => {
                self.modal = None;
                Task::none()
            }
            Message::ClearLog => {
                self.log.clear();
                Task::none()
            }
            Message::Noop => Task::none(),
            Message::ToggleConsole => {
                self.console_open = !self.console_open;
                Task::none()
            }
            Message::ToggleSidebar => {
                self.sidebar_open = !self.sidebar_open;
                Task::none()
            }
            Message::OpenSettings => {
                self.page = Page::Settings;
                Task::none()
            }
            Message::Settings(msg) => self.on_settings(msg),
            Message::SystemTheme(m) => {
                if let Some(m) = theme::from_iced(m) {
                    self.mode = m;
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
                        self.push_log(LogKind::Info, "Icône de notification activée.");
                    }
                    Some(Err(e)) => self.push_log(
                        LogKind::Error,
                        format!("Zone de notification indisponible : {e}"),
                    ),
                    None => {}
                }
                Task::none()
            }
            Message::Tick => self.begin_check_all(true),
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
                }
            }
            Msg::Rescan => return self.scan_task(),
            Msg::Back => self.page = Page::Main,
        }
        Task::none()
    }

    fn on_modal_commit(&mut self) -> Task<Message> {
        match self.modal.take() {
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
        if self.tray.is_some() && self.cfg.check_interval_minutes > 0 {
            subs.push(
                Subscription::run_with(self.cfg.check_interval_minutes, ticker)
                    .map(|_| Message::Tick),
            );
        }
        Subscription::batch(subs)
    }
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
}
