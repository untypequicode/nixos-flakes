//! `nixos-flakes daemon` : processus unique de fond de la session.
//!
//! Il héerge l'icône d'état, surveille le fichier d'état écrit par la mise à niveau
//! automatique (root), vérifie périodiquement les mises à jour, et expose son état sur
//! D-Bus (voir `nixos_flakes_core::daemon` et `docs/ARCHITECTURE.md`). L'instance est
//! unique : il réclame le nom de bus de session et quitte si un autre démon le détient.

mod tray;

use nixos_flakes_core::config::Config;
use nixos_flakes_core::daemon::{
    self as api, CheckSummary, DaemonStatus, UpgradeRecord, BUS_NAME, OBJECT_PATH,
};
use nixos_flakes_core::lock::InputChange;
use nixos_flakes_core::notify;
use nixos_flakes_core::status;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::Notify;
use zbus::object_server::SignalEmitter;

/// Période de surveillance du fichier d'état et des demandes.
const TICK: Duration = Duration::from_secs(5);

/// État partagé entre l'interface D-Bus, l'icône et la boucle de surveillance.
pub struct Shared {
    status: Mutex<DaemonStatus>,
    /// Réveille la boucle pour une vérification immédiate.
    pub check_now: Notify,
}

/// Objet D-Bus du démon.
struct Service {
    shared: Arc<Shared>,
}

#[zbus::interface(name = "io.github.untypequicode.NixosFlakes.Daemon")]
impl Service {
    /// État courant (JSON de `DaemonStatus`).
    async fn get_status(&self) -> String {
        self.shared
            .status
            .lock()
            .map(|s| s.to_json())
            .unwrap_or_default()
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

/// Vérifie les mises à jour du dépôt système et des dépôts suivis (bloquant).
fn run_check() -> (CheckSummary, Pending, Option<Vec<PathBuf>>) {
    let checked_at = api::now_unix();
    let config = match Config::load(&Config::default_path()) {
        Ok(c) => c,
        Err(e) => {
            let summary = CheckSummary {
                updates: 0,
                error: Some(e.to_string()),
                checked_at,
            };
            // `None` : tous les dépôts sont en échec (configuration illisible).
            return (summary, Pending::new(), None);
        }
    };
    let mut repos: Vec<_> = config.system_repo.iter().cloned().collect();
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

fn repo_name(repo: &std::path::Path) -> String {
    repo.file_name().map_or_else(
        || repo.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn mtime(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Intervalle de vérification périodique (`None` = désactivé).
fn check_interval() -> Option<Duration> {
    let minutes = Config::load(&Config::default_path()).map_or(60, |c| c.check_interval_minutes);
    (minutes > 0).then(|| Duration::from_secs(u64::from(minutes) * 60))
}

/// Point d'entrée : bloque jusqu'à l'arrêt du processus.
pub fn run(gui_command: Option<String>) -> Result<ExitCode, String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime : {e}"))?;
    runtime.block_on(serve(gui_command.filter(|c| !c.trim().is_empty())))
}

async fn serve(gui_command: Option<String>) -> Result<ExitCode, String> {
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

    // Sans hôte StatusNotifier pour l'instant, l'icône apparaîtra dès qu'il sera là.
    let handle = {
        use ksni::TrayMethods;
        let tray = tray::StatusTray {
            status: DaemonStatus::default(),
            shared: shared.clone(),
            gui_command,
        };
        match tray.assume_sni_available(true).spawn().await {
            Ok(h) => Some(h),
            Err(e) => {
                eprintln!("icône d'état indisponible : {e}");
                None
            }
        }
    };

    let status_path = api::status_file_path();
    let mut last_file = mtime(&status_path);
    let mut last_check: Option<Instant> = None;
    let mut check: Option<CheckSummary> = None;
    let mut known: Option<Pending> = None;
    let mut tick = tokio::time::interval(TICK);

    loop {
        let mut forced = false;
        tokio::select! {
            _ = tick.tick() => {}
            () = shared.check_now.notified() => forced = true,
        }
        let file = mtime(&status_path);
        let file_changed = file != last_file;
        last_file = file;
        let due = match (last_check, check_interval()) {
            (None, _) => true,
            (Some(t), Some(every)) => t.elapsed() >= every,
            (Some(_), None) => false,
        };

        if forced || file_changed || due {
            let upgrade = api::read_upgrade_record(&status_path);
            publish(
                &shared,
                &iface,
                handle.as_ref(),
                check.as_ref(),
                upgrade,
                true,
            )
            .await;
            if let Ok((summary, fresh, failed)) = tokio::task::spawn_blocking(run_check).await {
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
            } else {
                check = None;
            }
            last_check = Some(Instant::now());
        }
        let upgrade = api::read_upgrade_record(&status_path);
        publish(
            &shared,
            &iface,
            handle.as_ref(),
            check.as_ref(),
            upgrade,
            false,
        )
        .await;
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
        let Ok(mut current) = shared.status.lock() else {
            return;
        };
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
        let repo = PathBuf::from("/etc/nixos");
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
}
