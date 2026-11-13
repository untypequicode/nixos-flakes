//! Appels à `nixos-flakes-core` hors du fil de l'interface.
//!
//! Le cœur est synchrone : chaque commande tourne dans un `std::thread` dédié ;
//! ses lignes de sortie et son résultat final sont transmis à l'UI par un canal.

use crate::model::{self, JobKind};
use iced::futures::channel::{mpsc, oneshot};
use iced::futures::Stream;
use nixos_flakes_core::config::Config;
use nixos_flakes_core::git::{self, GitState};
use nixos_flakes_core::nix::{self, Privilege, UpdateReport};
use nixos_flakes_core::scan;
use nixos_flakes_core::status::{self, RepoStatus};
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Commande demandée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Check,
    Update,
    /// Mise à jour limitée à ces inputs racine (jamais vide : sinon `Update`).
    UpdateInputs(Vec<String>),
    /// Application ; l'attribut `nixosConfigurations.<attr>` éventuel (sinon nom d'hôte).
    Apply(Option<String>),
    Commit(String),
    CommitLock(String),
    Push,
}

impl Job {
    pub fn kind(&self) -> JobKind {
        match self {
            Job::Check => JobKind::Check,
            Job::Update | Job::UpdateInputs(_) => JobKind::Update,
            Job::Apply(_) => JobKind::Apply,
            Job::Commit(_) => JobKind::Commit,
            Job::CommitLock(_) => JobKind::CommitLock,
            Job::Push => JobKind::Push,
        }
    }
}

/// Résultat d'une commande (erreurs déjà converties en texte).
#[derive(Debug, Clone)]
pub enum Outcome {
    Check(Result<UpdateReport, String>),
    /// Inputs demandés (vide = tous) et résultat.
    Update(Vec<String>, Result<UpdateReport, String>),
    Done(JobKind, Result<(), String>),
    /// Le travail a paniqué ou son thread n'a pas pu démarrer (« tâche interrompue »).
    Aborted(JobKind, String),
}

impl Outcome {
    /// Texte d'erreur d'un travail interrompu (détail technique éventuel après les deux-points).
    pub fn aborted_text(detail: &str) -> String {
        if detail.trim().is_empty() {
            "tâche interrompue".to_string()
        } else {
            format!("tâche interrompue : {}", detail.trim())
        }
    }
}

#[derive(Debug, Clone)]
pub struct Finished {
    pub outcome: Outcome,
    /// État git relevé après la commande.
    pub git: GitState,
}

/// Événement émis par une commande en cours.
#[derive(Debug, Clone)]
pub enum JobEvent {
    Line(String),
    Finished(Box<Finished>),
}

/// Exécute `job` (bloquant) en envoyant les lignes à `out`.
pub fn execute(repo: &Path, job: &Job, out: &mut dyn FnMut(&str)) -> Outcome {
    let s = |e: nixos_flakes_core::Error| e.to_string();
    match job {
        Job::Check => Outcome::Check(nix::check(repo).map_err(s)),
        Job::Update => Outcome::Update(Vec::new(), nix::update(repo, out).map_err(s)),
        Job::UpdateInputs(names) => Outcome::Update(
            names.clone(),
            nix::update_inputs(repo, names, out).map_err(s),
        ),
        Job::Apply(attr) => Outcome::Done(
            JobKind::Apply,
            nix::apply(repo, attr.as_deref(), Privilege::Pkexec, out).map_err(s),
        ),
        Job::Commit(m) => Outcome::Done(JobKind::Commit, git::commit_all(repo, m, out).map_err(s)),
        Job::CommitLock(m) => Outcome::Done(
            JobKind::CommitLock,
            git::commit_lock(repo, m, out).map_err(s),
        ),
        Job::Push => Outcome::Done(JobKind::Push, git::push(repo, out).map_err(s)),
    }
}

/// Message d'une panique (`&str` ou `String`), vide sinon.
fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

/// Lance `work` dans un thread ; le flux produit les lignes puis toujours un `Finished` :
/// si le travail panique (ou si le thread ne démarre pas), c'est `Outcome::Aborted`, pour
/// que le dépôt ne reste jamais « occupé ».
pub fn spawn_stream<W>(repo: PathBuf, kind: JobKind, work: W) -> impl Stream<Item = JobEvent>
where
    W: FnOnce(&Path, &mut dyn FnMut(&str)) -> Outcome + Send + 'static,
{
    let (tx, rx) = mpsc::unbounded();
    let fail_tx = tx.clone();
    let fail_repo = repo.clone();
    let spawned = std::thread::Builder::new()
        .name("flakes-job".into())
        .spawn(move || {
            let line_tx = tx.clone();
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                work(&repo, &mut |l| {
                    let _ = line_tx.unbounded_send(JobEvent::Line(l.to_string()));
                })
            }))
            .unwrap_or_else(|p| Outcome::Aborted(kind, panic_text(p.as_ref())));
            let git = catch_unwind(AssertUnwindSafe(|| git::state(&repo).unwrap_or_default()))
                .unwrap_or_default();
            let _ = tx.unbounded_send(JobEvent::Finished(Box::new(Finished { outcome, git })));
        });
    if let Err(e) = spawned {
        let git = git::state(&fail_repo).unwrap_or_default();
        let _ = fail_tx.unbounded_send(JobEvent::Finished(Box::new(Finished {
            outcome: Outcome::Aborted(kind, e.to_string()),
            git,
        })));
    }
    rx
}

pub fn run_job(repo: PathBuf, job: Job) -> impl Stream<Item = JobEvent> {
    let kind = job.kind();
    spawn_stream(repo, kind, move |r, out| execute(r, &job, out))
}

/// Exécute une fermeture bloquante dans un thread et attend son résultat.
/// `None` = le thread a paniqué ou n'a pas pu démarrer : à ne jamais confondre avec un
/// résultat vide (l'appelant affiche une erreur et conserve l'état précédent).
pub fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = Option<T>> {
    let (tx, rx) = oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("flakes-blocking".into())
        .spawn(move || {
            let _ = tx.send(f());
        });
    let ok = spawned.is_ok();
    async move {
        if ok {
            rx.await.ok()
        } else {
            None
        }
    }
}

/// Scan complet : dépôts trouvés + dépôts de la config, avec leur état git (sans `nix`).
pub fn scan_all(cfg: &Config) -> Vec<RepoStatus> {
    let found = scan::scan(&cfg.scan_roots, cfg.scan_depth);
    model::collect_paths(found, cfg, |p| p.join("flake.nix").is_file())
        .into_iter()
        .map(|p| {
            let role = cfg.role(&p);
            status::status(&p, role, false)
        })
        .collect()
}

/// Inputs racine du `flake.lock` (lecture de fichier, erreur déjà en texte).
pub fn load_inputs(repo: &Path) -> Result<Vec<nixos_flakes_core::lock::RootInput>, String> {
    nix::list_inputs(repo).map_err(|e| e.to_string())
}

/// Verrou : deux enregistrements successifs ne s'entrelacent pas sur le disque.
static SAVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Sauvegarde la config, sauf si un enregistrement plus récent a été demandé depuis
/// (`counter` ne vaut plus `generation`) : le verrou ne garantit pas l'ordre des threads,
/// la génération évite qu'une ancienne version écrase la nouvelle.
/// Renvoie `Ok(true)` si le fichier a été écrit, `Ok(false)` si l'appel était périmé.
pub fn save_config(
    counter: &AtomicU64,
    generation: u64,
    cfg: &Config,
    path: &Path,
) -> Result<bool, String> {
    let _guard = SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if counter.load(Ordering::SeqCst) != generation {
        return Ok(false);
    }
    cfg.save(path).map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::{executor::block_on, StreamExt};

    #[test]
    fn job_kinds() {
        assert_eq!(Job::Check.kind(), JobKind::Check);
        assert_eq!(Job::Commit("m".into()).kind(), JobKind::Commit);
        assert_eq!(Job::CommitLock("m".into()).kind(), JobKind::CommitLock);
        assert_eq!(Job::Push.kind(), JobKind::Push);
        assert_eq!(Job::UpdateInputs(vec!["a".into()]).kind(), JobKind::Update);
    }

    #[test]
    fn stream_emits_lines_then_finished() {
        let d = tempfile::tempdir().unwrap();
        let s = spawn_stream(d.path().to_path_buf(), JobKind::Push, |_, out| {
            out("un");
            out("deux");
            Outcome::Done(JobKind::Push, Ok(()))
        });
        let events: Vec<JobEvent> = block_on(s.collect());
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], JobEvent::Line(l) if l == "un"));
        assert!(matches!(&events[1], JobEvent::Line(l) if l == "deux"));
        match &events[2] {
            JobEvent::Finished(f) => {
                assert!(matches!(f.outcome, Outcome::Done(JobKind::Push, Ok(()))));
                assert!(!f.git.is_repo);
            }
            e => panic!("inattendu : {e:?}"),
        }
    }

    #[test]
    fn blocking_returns_value() {
        assert_eq!(block_on(blocking(|| 40 + 2)), Some(42));
    }

    #[test]
    fn blocking_panic_is_none_not_an_empty_result() {
        let r: Option<Vec<u8>> = block_on(blocking(|| -> Vec<u8> { panic!("boum") }));
        assert_eq!(r, None);
        assert_eq!(block_on(blocking(Vec::<u8>::new)), Some(vec![]));
    }

    #[test]
    fn a_panicking_job_still_finishes_with_aborted() {
        let d = tempfile::tempdir().unwrap();
        let s = spawn_stream(d.path().to_path_buf(), JobKind::Check, |_, out| {
            out("début");
            panic!("boum");
        });
        let events: Vec<JobEvent> = block_on(s.collect());
        assert_eq!(events.len(), 2);
        match &events[1] {
            JobEvent::Finished(f) => match &f.outcome {
                Outcome::Aborted(JobKind::Check, why) => assert_eq!(why, "boum"),
                o => panic!("inattendu : {o:?}"),
            },
            e => panic!("inattendu : {e:?}"),
        }
    }

    #[test]
    fn aborted_text_mentions_the_interruption() {
        assert_eq!(Outcome::aborted_text(""), "tâche interrompue");
        assert_eq!(Outcome::aborted_text(" boum "), "tâche interrompue : boum");
    }

    #[test]
    fn config_save_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("c.toml");
        let cfg = Config {
            scan_depth: 7,
            ..Config::default()
        };
        let counter = AtomicU64::new(1);
        assert_eq!(save_config(&counter, 1, &cfg, &p), Ok(true));
        assert_eq!(Config::load(&p).unwrap().scan_depth, 7);
    }

    #[test]
    fn stale_config_save_never_overwrites_a_newer_one() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("c.toml");
        let with_depth = |scan_depth| Config {
            scan_depth,
            ..Config::default()
        };
        let counter = AtomicU64::new(2);
        // Le deuxième enregistrement passe en premier ; le premier arrive en retard.
        assert_eq!(save_config(&counter, 2, &with_depth(8), &p), Ok(true));
        assert_eq!(save_config(&counter, 1, &with_depth(3), &p), Ok(false));
        assert_eq!(Config::load(&p).unwrap().scan_depth, 8);
    }

    #[test]
    fn execute_commit_on_non_repo_is_an_error_not_a_panic() {
        let d = tempfile::tempdir().unwrap();
        let mut lines = vec![];
        let o = execute(d.path(), &Job::Commit("m".into()), &mut |l| {
            lines.push(l.to_string())
        });
        assert!(matches!(o, Outcome::Done(JobKind::Commit, Err(_))));
    }
}
