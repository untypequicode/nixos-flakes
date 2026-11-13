//! `nixos-flakes daemon` : processus unique de fond de la session.
//!
//! Il héberge l'icône d'état, surveille le fichier d'état écrit par la mise à niveau
//! automatique (root), vérifie périodiquement les mises à jour, et expose son état sur
//! D-Bus (voir `nixos_flakes_core::daemon` et `docs/ARCHITECTURE.md`). L'instance est
//! unique : il réclame le nom de bus de session et quitte si un autre démon le détient.

mod cmdline;
mod tray;

use nixos_flakes_core::config::Config;
use nixos_flakes_core::daemon::{
    self as api, CheckSummary, DaemonStatus, UpgradeRecord, BUS_NAME, OBJECT_PATH,
};
use nixos_flakes_core::lock::InputChange;
use nixos_flakes_core::notify;
use nixos_flakes_core::status;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::Notify;
use zbus::object_server::SignalEmitter;

/// Période de surveillance du fichier d'état et des demandes.
const TICK: Duration = Duration::from_secs(5);

/// Durée maximale d'un cycle de vérification (tous dépôts), en plus des délais propres
/// à chaque commande : une vérification bloquée ne fige jamais la boucle.
const CHECK_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Drapeau « vérification en cours » : libéré à la fin RÉELLE de la tâche bloquante (même après
/// un délai dépassé côté boucle, la tâche continue). Tant qu'il est levé, on saute le cycle
/// au lieu d'empiler des vérifications.
struct BusyGuard(Arc<AtomicBool>);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Délais de relance de l'icône : doublent à chaque échec, de 5 s à 5 min.
const TRAY_BACKOFF_MIN: Duration = Duration::from_secs(5);
const TRAY_BACKOFF_MAX: Duration = Duration::from_secs(300);

/// État partagé entre l'interface D-Bus, l'icône et la boucle de surveillance.
pub struct Shared {
    status: Mutex<DaemonStatus>,
    /// Réveille la boucle pour une vérification immédiate.
    pub check_now: Notify,
}

impl Shared {
    /// Verrou de l'état ; un mutex empoisonné est récupéré (l'état reste cohérent : il est
    /// remplacé en bloc).
    fn lock(&self) -> MutexGuard<'_, DaemonStatus> {
        self.status.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Objet D-Bus du démon.
struct Service {
    shared: Arc<Shared>,
}

#[zbus::interface(name = "io.github.untypequicode.NixosFlakes.Daemon")]
impl Service {
    /// État courant (JSON de `DaemonStatus`).
    async fn get_status(&self) -> String {
        self.shared.lock().to_json()
    }

    /// Demande une vérification immédiate.
    async fn check_now(&self) {
        self.shared.check_now.notify_one();
    }

    /// Émis à chaque changement d'état.
    #[zbus(signal)]
    async fn status_changed(emitter: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;
}

/// Mises à jour en attente par dépôt (seulement les dépôts vérifiés sans erreur).
type Pending = HashMap<PathBuf, Vec<InputChange>>;

/// Configuration relue seulement quand la date de modification du fichier change ; une
/// configuration illisible est signalée une seule fois (pas à chaque tick).
struct ConfigCache {
    path: PathBuf,
    seen: bool,
    mtime: Option<SystemTime>,
    loaded: Result<Config, String>,
}

impl ConfigCache {
    fn new(path: PathBuf) -> Self {
        ConfigCache {
            path,
            seen: false,
            mtime: None,
            loaded: Ok(Config::default()),
        }
    }

    /// Configuration courante (rechargée si le fichier a changé).
    fn get(&mut self) -> Result<Config, String> {
        let now = mtime(&self.path);
        if !self.seen || now != self.mtime {
            self.seen = true;
            self.mtime = now;
            self.loaded = Config::load(&self.path).map_err(|e| e.to_string());
            if let Err(e) = &self.loaded {
                eprintln!("configuration illisible, vérifications suspendues : {e}");
            }
        }
        self.loaded.clone()
    }
}

/// Vérifie les mises à jour du dépôt système et des dépôts suivis (bloquant).
fn run_check(config: Result<Config, String>) -> (CheckSummary, Pending, Option<Vec<PathBuf>>) {
    let checked_at = api::now_unix();
    let config = match config {
        Ok(c) => c,
        Err(e) => {
            let summary = CheckSummary {
                updates: 0,
                error: Some(e),
                checked_at,
            };
            // `None` : tous les dépôts sont en échec (configuration illisible).
            return (summary, Pending::new(), None);
        }
    };
    let mut repos: Vec<_> = config
        .system_repo_resolved()
        .into_iter()
        .map(|r| r.path)
        .collect();
    repos.extend(config.tracked.iter().cloned());
    let mut all = Pending::new();
    let mut failed = Vec::new();
    let mut summary = CheckSummary {
        checked_at,
        ..Default::default()
    };
    for repo in repos {
        let role = config.role(&repo);
        let s = status::status(&repo, role, true);
        let changes = s.pending.unwrap_or_default();
        let pending = changes.len();
        summary.updates = summary
            .updates
            .saturating_add(u32::try_from(pending).unwrap_or(u32::MAX));
        match s.error {
            Some(e) => {
                failed.push(repo.clone());
                if summary.error.is_none() {
                    summary.error = Some(format!("{} : {e}", repo.display()));
                }
            }
            None => {
                all.insert(repo, changes);
            }
        }
    }
    (summary, all, Some(failed))
}

/// Dépôts dont des mises à jour sont nouvelles depuis `known` (nom court, nombre).
fn new_updates(known: &Pending, fresh: &Pending) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = fresh
        .iter()
        .filter(|(repo, changes)| {
            notify::has_new_updates(known.get(*repo).map(Vec::as_slice), changes)
        })
        .map(|(repo, changes)| (repo_name(repo), changes.len()))
        .collect();
    out.sort();
    out
}

fn repo_name(repo: &Path) -> String {
    repo.file_name().map_or_else(
        || repo.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Intervalle de vérification périodique (`None` = désactivé).
fn check_interval(config: &Result<Config, String>) -> Option<Duration> {
    let minutes = config.as_ref().map_or(60, |c| c.check_interval_minutes);
    (minutes > 0).then(|| Duration::from_secs(u64::from(minutes) * 60))
}

/// Point d'entrée : bloque jusqu'à SIGTERM/SIGINT. `config_path` : fichier de configuration
/// (option globale `--config`).
pub fn run(gui_command: Option<String>, config_path: PathBuf) -> Result<ExitCode, String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime : {e}"))?;
    runtime.block_on(serve(
        gui_command.filter(|c| !c.trim().is_empty()),
        config_path,
    ))
}

/// Icône d'état avec relance temporisée (échec du démarrage ou chute de l'hôte).
struct TrayKeeper {
    shared: Arc<Shared>,
    gui_command: Option<String>,
    handle: Option<ksni::Handle<tray::StatusTray>>,
    delay: Duration,
    next_try: Instant,
}

impl TrayKeeper {
    fn new(shared: Arc<Shared>, gui_command: Option<String>) -> Self {
        TrayKeeper {
            shared,
            gui_command,
            handle: None,
            delay: TRAY_BACKOFF_MIN,
            next_try: Instant::now(),
        }
    }

    /// Relance l'icône si elle est absente ou fermée et que le délai est écoulé.
    async fn ensure(&mut self) {
        if self.handle.as_ref().is_some_and(|h| !h.is_closed()) {
            return;
        }
        if self.handle.take().is_some() {
            eprintln!("l'icône d'état s'est arrêtée ; nouvelle tentative.");
        }
        if Instant::now() < self.next_try {
            return;
        }
        use ksni::TrayMethods;
        let tray = tray::StatusTray {
            status: self.shared.lock().clone(),
            shared: self.shared.clone(),
            gui_command: self.gui_command.clone(),
        };
        // Sans hôte StatusNotifier, l'icône apparaîtra dès qu'il sera là.
        match tray.assume_sni_available(true).spawn().await {
            Ok(h) => {
                self.handle = Some(h);
                self.delay = TRAY_BACKOFF_MIN;
            }
            Err(e) => {
                eprintln!(
                    "icône d'état indisponible : {e} (nouvelle tentative dans {} s)",
                    self.delay.as_secs()
                );
                self.next_try = Instant::now() + self.delay;
                self.delay = (self.delay * 2).min(TRAY_BACKOFF_MAX);
            }
        }
    }
}

/// Attend SIGTERM ou SIGINT.
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut term), Ok(mut int)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        eprintln!("signaux d'arrêt indisponibles : arrêt propre désactivé.");
        std::future::pending::<()>().await;
        return;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}

async fn serve(gui_command: Option<String>, config_path: PathBuf) -> Result<ExitCode, String> {
    if let Some(cmd) = &gui_command {
        if cmdline::split(cmd).is_none() {
            eprintln!("attention : commande de l'interface invalide (guillemet non fermé) : {cmd}");
        }
    }
    let shared = Arc::new(Shared {
        status: Mutex::new(DaemonStatus::default()),
        check_now: Notify::new(),
    });

    let builder = zbus::connection::Builder::session()
        .and_then(|b| {
            b.serve_at(
                OBJECT_PATH,
                Service {
                    shared: shared.clone(),
                },
            )
        })
        .map_err(|e| format!("D-Bus : {e}"))?;
    let connection = builder.build().await.map_err(|e| format!("D-Bus : {e}"))?;
    // `Builder::name` met en file d'attente au lieu d'échouer : on réclame le nom sans file
    // d'attente pour garantir l'instance unique.
    let reply = zbus::fdo::DBusProxy::new(&connection)
        .await
        .map_err(|e| format!("D-Bus : {e}"))?
        .request_name(
            BUS_NAME.try_into().map_err(|e| format!("D-Bus : {e}"))?,
            zbus::fdo::RequestNameFlags::DoNotQueue.into(),
        )
        .await
        .map_err(|e| format!("D-Bus : {e}"))?;
    if !matches!(
        reply,
        zbus::fdo::RequestNameReply::PrimaryOwner | zbus::fdo::RequestNameReply::AlreadyOwner
    ) {
        eprintln!("le démon nixos-flakes est déjà lancé.");
        return Ok(ExitCode::SUCCESS);
    }
    let iface = connection
        .object_server()
        .interface::<_, Service>(OBJECT_PATH)
        .await
        .map_err(|e| format!("D-Bus : {e}"))?;

    let mut tray = TrayKeeper::new(shared.clone(), gui_command);
    tray.ensure().await;

    let status_path = api::status_file_path();
    let mut config = ConfigCache::new(config_path);
    let mut last_file = mtime(&status_path);
    let mut last_check: Option<Instant> = None;
    let mut check: Option<CheckSummary> = None;
    let mut known: Option<Pending> = None;
    let check_busy = Arc::new(AtomicBool::new(false));
    let mut tick = tokio::time::interval(TICK);
    let stop = shutdown_signal();
    tokio::pin!(stop);

    loop {
        let mut forced = false;
        tokio::select! {
            _ = tick.tick() => {}
            () = shared.check_now.notified() => forced = true,
            () = &mut stop => break,
        }
        tray.ensure().await;
        let file = mtime(&status_path);
        let file_changed = file != last_file;
        last_file = file;
        let cfg = config.get();
        let due = match (last_check, check_interval(&cfg)) {
            (None, _) => true,
            (Some(t), Some(every)) => t.elapsed() >= every,
            (Some(_), None) => false,
        };

        // Une vérification précédente encore en cours (bloquée au-delà de CHECK_TIMEOUT) : on saute
        // le cycle plutôt que d'en empiler une autre.
        let busy = check_busy.load(Ordering::Acquire);
        if busy && (forced || file_changed || due) {
            eprintln!("vérification précédente encore en cours : cycle ignoré.");
        }
        if !busy && (forced || file_changed || due) {
            let upgrade = api::read_upgrade_record(&status_path);
            publish(
                &shared,
                &iface,
                tray.handle.as_ref(),
                check.as_ref(),
                upgrade,
                true,
            )
            .await;
            check_busy.store(true, Ordering::Release);
            let guard = BusyGuard(check_busy.clone());
            let outcome = tokio::time::timeout(
                CHECK_TIMEOUT,
                tokio::task::spawn_blocking(move || {
                    let _guard = guard;
                    run_check(cfg)
                }),
            )
            .await;
            match outcome {
                Ok(Ok((summary, fresh, failed))) => {
                    // Notification de bureau pour les mises à jour jamais signalées jusqu'ici
                    // (la première vérification du démon signale tout ce qui est en attente).
                    let new = new_updates(known.as_ref().unwrap_or(&Pending::new()), &fresh);
                    if !new.is_empty() {
                        let (title, body) = notify::updates_text(&new);
                        tokio::task::spawn_blocking(move || notify::desktop(&title, &body, false));
                    }
                    // Les dépôts en échec gardent leur entrée précédente ; les disparus sont retirés.
                    let mut next = fresh;
                    if let Some(old) = known.take() {
                        for (repo, changes) in old {
                            let kept = failed.as_ref().is_none_or(|f| f.contains(&repo));
                            if kept {
                                next.entry(repo).or_insert(changes);
                            }
                        }
                    }
                    known = Some(next);
                    check = Some(summary);
                }
                Ok(Err(e)) => {
                    eprintln!("vérification interrompue : {e}");
                    check = Some(failed_check(format!("vérification interrompue : {e}")));
                }
                Err(_) => {
                    let msg = format!(
                        "vérification interrompue : délai de {} min dépassé",
                        CHECK_TIMEOUT.as_secs() / 60
                    );
                    eprintln!("{msg}");
                    check = Some(failed_check(msg));
                }
            }
            last_check = Some(Instant::now());
        }
        let upgrade = api::read_upgrade_record(&status_path);
        publish(
            &shared,
            &iface,
            tray.handle.as_ref(),
            check.as_ref(),
            upgrade,
            false,
        )
        .await;
    }

    eprintln!("arrêt demandé : fermeture du démon.");
    if let Some(handle) = tray.handle.take() {
        handle.shutdown().await;
    }
    Ok(ExitCode::SUCCESS)
}

/// Résumé d'une vérification qui n'a pas abouti (état d'erreur publié).
fn failed_check(message: String) -> CheckSummary {
    CheckSummary {
        updates: 0,
        error: Some(message),
        checked_at: api::now_unix(),
    }
}

/// Recalcule l'état ; s'il a changé, le mémorise, émet le signal et met à jour l'icône.
async fn publish(
    shared: &Arc<Shared>,
    iface: &zbus::object_server::InterfaceRef<Service>,
    tray: Option<&ksni::Handle<tray::StatusTray>>,
    check: Option<&CheckSummary>,
    upgrade: Option<UpgradeRecord>,
    checking: bool,
) {
    let new = DaemonStatus::combine(check, upgrade, checking);
    {
        let mut current = shared.lock();
        if *current == new {
            return;
        }
        *current = new.clone();
    }
    if let Err(e) = Service::status_changed(iface.signal_emitter(), &new.to_json()).await {
        eprintln!("signal D-Bus : {e}");
    }
    if let Some(handle) = tray {
        handle.update(|t| t.status = new).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(name: &str, rev: &str) -> InputChange {
        InputChange {
            name: name.into(),
            old_rev: None,
            new_rev: Some(rev.into()),
            old_date: None,
            new_date: None,
        }
    }

    #[test]
    fn only_unseen_updates_are_notified() {
        let repo = PathBuf::from("/srv/nixos");
        let mut known = Pending::new();
        let mut fresh = Pending::new();
        fresh.insert(repo.clone(), vec![change("nixpkgs", "1")]);
        assert_eq!(new_updates(&known, &fresh), [("nixos".to_string(), 1)]);
        known.insert(repo.clone(), vec![change("nixpkgs", "1")]);
        assert!(new_updates(&known, &fresh).is_empty());
        fresh.insert(repo, vec![change("nixpkgs", "2")]);
        assert_eq!(new_updates(&known, &fresh).len(), 1);
    }

    #[test]
    fn interface_name_matches_core_constant() {
        // L'attribut `#[zbus::interface]` exige un littéral : on vérifie la cohérence ici.
        assert_eq!(
            <Service as zbus::object_server::Interface>::name(),
            api::INTERFACE
        );
    }

    #[test]
    fn config_cache_reloads_only_when_mtime_changes() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        std::fs::write(&p, "check_interval_minutes = 7\n").unwrap();
        let mut cache = ConfigCache::new(p.clone());
        assert_eq!(cache.get().unwrap().check_interval_minutes, 7);
        // Même mtime : le contenu n'est pas relu.
        let m = mtime(&p).unwrap();
        std::fs::write(&p, "scan_depth = [").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(m)
            .unwrap();
        assert_eq!(cache.get().unwrap().check_interval_minutes, 7);
        // mtime différent : relecture, erreur mémorisée.
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(m + Duration::from_secs(5))
            .unwrap();
        assert!(cache.get().is_err());
        assert!(cache.get().is_err());
    }

    #[test]
    fn poisoned_status_mutex_is_recovered() {
        let shared = Arc::new(Shared {
            status: Mutex::new(DaemonStatus::default()),
            check_now: Notify::new(),
        });
        let s2 = shared.clone();
        let _ = std::thread::spawn(move || {
            let _g = s2.status.lock().unwrap();
            panic!("empoisonne");
        })
        .join();
        assert!(shared.status.is_poisoned());
        assert_eq!(shared.lock().to_json(), DaemonStatus::default().to_json());
    }

    #[test]
    fn check_interval_follows_config() {
        let cfg = Config {
            check_interval_minutes: 0,
            ..Config::default()
        };
        assert_eq!(check_interval(&Ok(cfg)), None);
        assert_eq!(
            check_interval(&Err("x".into())),
            Some(Duration::from_secs(3600))
        );
    }
}
