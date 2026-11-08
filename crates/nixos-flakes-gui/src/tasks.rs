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
use std::path::{Path, PathBuf};

/// Commande demandée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Check,
    Update,
    /// Mise à jour limitée à ces inputs racine (jamais vide : sinon `Update`).
    UpdateInputs(Vec<String>),
    Apply,
    Commit(String),
    CommitLock(String),
    Push,
}

impl Job {
    pub fn kind(&self) -> JobKind {
        match self {
            Job::Check => JobKind::Check,
            Job::Update | Job::UpdateInputs(_) => JobKind::Update,
            Job::Apply => JobKind::Apply,
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
        Job::Apply => Outcome::Done(
            JobKind::Apply,
            nix::apply(repo, Privilege::Pkexec, out).map_err(s),
        ),
        Job::Commit(m) => Outcome::Done(JobKind::Commit, git::commit_all(repo, m, out).map_err(s)),
        Job::CommitLock(m) => Outcome::Done(
            JobKind::CommitLock,
            git::commit_lock(repo, m, out).map_err(s),
        ),
        Job::Push => Outcome::Done(JobKind::Push, git::push(repo, out).map_err(s)),
    }
}

/// Lance `work` dans un thread ; le flux produit les lignes puis un `Finished`.
pub fn spawn_stream<W>(repo: PathBuf, work: W) -> impl Stream<Item = JobEvent>
where
    W: FnOnce(&Path, &mut dyn FnMut(&str)) -> Outcome + Send + 'static,
{
    let (tx, rx) = mpsc::unbounded();
    std::thread::spawn(move || {
        let line_tx = tx.clone();
        let outcome = work(&repo, &mut |l| {
            let _ = line_tx.unbounded_send(JobEvent::Line(l.to_string()));
        });
        let git = git::state(&repo).unwrap_or_default();
        let _ = tx.unbounded_send(JobEvent::Finished(Box::new(Finished { outcome, git })));
    });
    rx
}

pub fn run_job(repo: PathBuf, job: Job) -> impl Stream<Item = JobEvent> {
    spawn_stream(repo, move |r, out| execute(r, &job, out))
}

/// Exécute une fermeture bloquante dans un thread et attend son résultat.
pub fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = Option<T>> {
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    async move { rx.await.ok() }
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

/// Sauvegarde la config (retourne un message d'erreur éventuel).
pub fn save_config(cfg: &Config, path: &Path) -> Result<(), String> {
    cfg.save(path).map_err(|e| e.to_string())
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
        let s = spawn_stream(d.path().to_path_buf(), |_, out| {
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
    fn execute_commit_on_non_repo_is_an_error_not_a_panic() {
        let d = tempfile::tempdir().unwrap();
        let mut lines = vec![];
        let o = execute(d.path(), &Job::Commit("m".into()), &mut |l| {
            lines.push(l.to_string())
        });
        assert!(matches!(o, Outcome::Done(JobKind::Commit, Err(_))));
    }
}
