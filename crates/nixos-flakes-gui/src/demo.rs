//! Mode démonstration (`--demo`) : faux dépôts et commandes simulées, pour essayer
//! l'interface (ou la capturer) sans toucher aux vrais dépôts.

use crate::tasks::{Finished, Job, JobEvent, Outcome};
use iced::futures::channel::mpsc;
use iced::futures::Stream;
use nixos_flakes_core::config::Role;
use nixos_flakes_core::git::GitState;
use nixos_flakes_core::lock::InputChange;
use nixos_flakes_core::nix::UpdateReport;
use nixos_flakes_core::status::RepoStatus;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Dossier personnel fictif (pour l'affichage `~/…`).
pub const HOME: &str = "/home/demo";

const DAY: i64 = 86_400;

fn change(name: &str, old: &str, new: &str, old_age: i64, new_age: i64, now: i64) -> InputChange {
    InputChange {
        name: name.into(),
        old_rev: Some(old.into()),
        new_rev: Some(new.into()),
        old_date: Some(now - old_age),
        new_date: Some(now - new_age),
    }
}

/// Mises à jour fictives : `n` entrées parmi un jeu fixe.
pub fn changes(n: usize, now: i64) -> Vec<InputChange> {
    vec![
        change(
            "nixpkgs",
            "3f1c9a7d2b4e5c6a",
            "9be0d1e47a8c2f10",
            18 * DAY,
            DAY / 3,
            now,
        ),
        change(
            "home-manager",
            "c41d7e0b9a2f3d68",
            "2ab85f6e1c9d0473",
            40 * DAY,
            2 * DAY,
            now,
        ),
        change(
            "flake-utils",
            "b1e32c8d7f904a5e",
            "e7d6a03b82c15f94",
            200 * DAY,
            9 * DAY,
            now,
        ),
        change(
            "rust-overlay",
            "07a4be39c5d1e862",
            "5c8f1d2a9e6b3704",
            12 * DAY,
            3600 * 5,
            now,
        ),
    ]
    .into_iter()
    .take(n)
    .collect()
}

fn git(branch: &str, dirty: &[&str], lock: bool) -> GitState {
    GitState {
        is_repo: true,
        branch: Some(branch.into()),
        dirty: dirty.iter().map(ToString::to_string).collect(),
        lock_dirty: lock,
    }
}

fn status(path: &str, role: Role, git: GitState, pending: Option<Vec<InputChange>>) -> RepoStatus {
    RepoStatus {
        path: PathBuf::from(HOME).join(path),
        role,
        git,
        pending,
        error: None,
    }
}

/// Dépôts fictifs couvrant tous les états visuels.
pub fn statuses(now: i64) -> Vec<RepoStatus> {
    let mut broken = status(
        "dev/rust-playground",
        Role::Tracked,
        GitState::default(),
        None,
    );
    broken.error = Some(
        "error: cannot fetch input 'rust-overlay'\n\
         unable to download 'https://api.github.com/repos/oxalica/rust-overlay/commits/HEAD': \
         Couldn't resolve host name (6)"
            .into(),
    );
    vec![
        status(
            "nixos-config",
            Role::System,
            git(
                "main",
                &["modules/desktop.nix", "hosts/laptop/default.nix"],
                false,
            ),
            Some(changes(4, now)),
        ),
        status(
            "dev/dotfiles",
            Role::Tracked,
            git("main", &[], false),
            Some(vec![]),
        ),
        status(
            "dev/website",
            Role::Tracked,
            git(
                "feature/blog",
                &["flake.nix", "src/main.rs", "README.md"],
                false,
            ),
            Some(changes(2, now)),
        ),
        broken,
        status(
            "dev/experiments",
            Role::Detected,
            git("main", &[], false),
            None,
        ),
        status("dev/old-stuff", Role::Detected, GitState::default(), None),
        status(
            "archive/legacy",
            Role::Ignored,
            git("master", &[], false),
            None,
        ),
    ]
}

/// État git simulé après une commande.
fn git_after(repo: &Path, job: &Job) -> GitState {
    let name = repo.file_name().map(|n| n.to_string_lossy().into_owned());
    if name.as_deref() == Some("old-stuff") || name.as_deref() == Some("rust-playground") {
        return GitState::default();
    }
    match job {
        Job::Update => git("main", &[], true),
        _ => git("main", &[], false),
    }
}

/// Commande simulée : quelques lignes de console, une pause, un résultat plausible.
pub fn run_job(repo: PathBuf, job: Job, now: i64) -> impl Stream<Item = JobEvent> {
    let (tx, rx) = mpsc::unbounded();
    std::thread::spawn(move || {
        let pause = |ms| std::thread::sleep(Duration::from_millis(ms));
        let out = |l: &str| {
            let _ = tx.unbounded_send(JobEvent::Line(l.to_string()));
        };
        let report = |n| UpdateReport {
            repo: repo.clone(),
            changes: changes(n, now),
        };
        let outcome = match &job {
            Job::Check => {
                out("$ nix flake update --dry-run");
                pause(500);
                out("• Résolution de nixpkgs…");
                pause(900);
                Outcome::Check(Ok(report(2)))
            }
            Job::Update => {
                out("$ nix flake update");
                pause(600);
                out("• Updated input 'nixpkgs'");
                pause(700);
                out("• Updated input 'home-manager'");
                pause(700);
                Outcome::Update(Ok(report(2)))
            }
            other => {
                out(&format!("$ {} (simulé)", other.kind().label()));
                pause(1_600);
                Outcome::Done(other.kind(), Ok(()))
            }
        };
        let git = git_after(&repo, &job);
        let _ = tx.unbounded_send(JobEvent::Finished(Box::new(Finished { outcome, git })));
    });
    rx
}
#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::{executor::block_on, StreamExt};

    #[test]
    fn demo_covers_every_visual_state() {
        let s = statuses(1_700_000_000);
        for role in [Role::System, Role::Tracked, Role::Detected, Role::Ignored] {
            assert!(s.iter().any(|r| r.role == role), "{role:?}");
        }
        assert!(s.iter().any(|r| r.error.is_some()));
        assert!(s
            .iter()
            .any(|r| r.pending.as_ref().is_some_and(|p| p.is_empty())));
        assert!(s.iter().any(|r| r.pending.is_none()));
        assert!(s.iter().any(|r| !r.git.dirty.is_empty()));
        assert!(s.iter().any(|r| !r.git.is_repo));
    }

    #[test]
    fn simulated_check_finishes_with_a_report() {
        let evs: Vec<JobEvent> =
            block_on(run_job("/home/demo/x".into(), Job::Check, 1_700_000_000).collect());
        assert!(matches!(evs.last(), Some(JobEvent::Finished(_))));
        assert!(evs.len() >= 2);
    }
}
