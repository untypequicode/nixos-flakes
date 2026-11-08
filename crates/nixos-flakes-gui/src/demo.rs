//! Mode démonstration (`--demo`) : faux dépôts et commandes simulées, pour essayer
//! l'interface (ou la capturer) sans toucher aux vrais dépôts.

use crate::tasks::{Finished, Job, JobEvent, Outcome};
use iced::futures::channel::mpsc;
use iced::futures::Stream;
use nixos_flakes_config::PackageHit;
use nixos_flakes_core::config::Role;
use nixos_flakes_core::git::GitState;
use nixos_flakes_core::lock::{InputChange, RootInput};
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

fn root(name: &str, rev: &str, url: &str, age: i64, now: i64) -> RootInput {
    RootInput {
        name: name.into(),
        node: Some(name.into()),
        rev: Some(rev.into()),
        short_rev: Some(rev.chars().take(7).collect()),
        last_modified: Some(now - age),
        kind: Some("github".into()),
        owner: None,
        repo: None,
        url: Some(url.into()),
    }
}

/// Inputs racine fictifs (les quatre premiers ont une mise à jour fictive).
pub fn inputs(now: i64) -> Vec<RootInput> {
    vec![
        root(
            "nixpkgs",
            "3f1c9a7d2b4e5c6a",
            "github:NixOS/nixpkgs/nixos-unstable",
            18 * DAY,
            now,
        ),
        root(
            "home-manager",
            "c41d7e0b9a2f3d68",
            "github:nix-community/home-manager",
            40 * DAY,
            now,
        ),
        root(
            "flake-utils",
            "b1e32c8d7f904a5e",
            "github:numtide/flake-utils",
            200 * DAY,
            now,
        ),
        root(
            "rust-overlay",
            "07a4be39c5d1e862",
            "github:oxalica/rust-overlay",
            12 * DAY,
            now,
        ),
        root(
            "nixos-hardware",
            "88d0a4c1e5f2b739",
            "github:NixOS/nixos-hardware",
            6 * DAY,
            now,
        ),
    ]
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
        Job::Update | Job::UpdateInputs(_) => git("main", &[], true),
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
                Outcome::Update(Vec::new(), Ok(report(2)))
            }
            Job::UpdateInputs(names) => {
                out(&format!("$ nix flake update {}", names.join(" ")));
                pause(600);
                let mut changes = changes(4, now);
                changes.retain(|c| names.contains(&c.name));
                for c in &changes {
                    out(&format!("• Updated input '{}'", c.name));
                    pause(500);
                }
                Outcome::Update(
                    names.clone(),
                    Ok(UpdateReport {
                        repo: repo.clone(),
                        changes,
                    }),
                )
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
// -- Paquets -----------------------------------------------------------------------

/// Fausse config du Dépôt système : (chemin relatif, source Nix). Rien n'est lu ni écrit.
pub fn config_sources() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "modules/common/packages.nix",
            "{ pkgs, ... }:\n{\n  environment.systemPackages = with pkgs; [\n    git\n    vim\n    htop\n    # firefox\n  ];\n}\n",
        ),
        (
            "modules/desktop.nix",
            "{ pkgs, ... }:\n{\n  environment.systemPackages = with pkgs; [\n    alacritty\n    wl-clipboard\n  ];\n\n  fonts.packages = with pkgs; [\n    inter\n  ];\n}\n",
        ),
        (
            "home/demo.nix",
            "{ pkgs, lib, ... }:\n{\n  home.packages = with pkgs; [\n    fd\n    bat\n  ];\n\n  environment.systemPackages = map (n: pkgs.${n}) [ \"nano\" ];\n}\n",
        ),
    ]
}

/// Fichiers de config fictifs, prêts à être affichés.
pub fn config_files() -> Vec<crate::packages::ConfigFile> {
    config_sources()
        .into_iter()
        .map(|(rel, src)| {
            crate::packages::ConfigFile::new(
                rel,
                PathBuf::from("/demo/nixos-config").join(rel),
                src.to_string(),
            )
        })
        .filter(|f| !f.locations.is_empty())
        .collect()
}

fn hit(attr: &str, version: &str, description: &str, license: &str, cache: bool) -> PackageHit {
    PackageHit {
        attr: attr.into(),
        name: attr.into(),
        version: version.into(),
        description: description.into(),
        license: Some(license.into()),
        from_cache: cache,
    }
}

/// Faux résultats de recherche (filtrés sur la saisie, sans réseau).
pub fn package_hits(query: &str) -> Vec<PackageHit> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    vec![
        hit(
            "git",
            "2.47.1",
            "Distributed version control system",
            "GPL-2.0-only",
            false,
        ),
        hit(
            "gitui",
            "0.26.3",
            "Blazing fast terminal UI for git",
            "MIT",
            false,
        ),
        hit(
            "lazygit",
            "0.44.1",
            "Simple terminal UI for git commands",
            "MIT",
            false,
        ),
        hit(
            "ripgrep",
            "14.1.1",
            "Utility that combines the usability of The Silver Searcher with the speed of grep",
            "Unlicense",
            true,
        ),
        hit(
            "firefox",
            "133.0",
            "Web browser from Mozilla",
            "MPL-2.0",
            false,
        ),
        hit(
            "vim",
            "9.1.0",
            "The most popular clone of the VI editor",
            "Vim",
            false,
        ),
    ]
    .into_iter()
    .filter(|h| h.attr.contains(&q) || h.description.to_lowercase().contains(&q))
    .collect()
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

    #[test]
    fn simulated_partial_update_touches_only_requested_inputs() {
        let job = Job::UpdateInputs(vec!["home-manager".into(), "nixos-hardware".into()]);
        let evs: Vec<JobEvent> =
            block_on(run_job("/home/demo/x".into(), job, 1_700_000_000).collect());
        let Some(JobEvent::Finished(f)) = evs.last() else {
            panic!("pas de fin");
        };
        let Outcome::Update(names, Ok(report)) = &f.outcome else {
            panic!("pas une mise à jour");
        };
        assert_eq!(names.len(), 2);
        assert_eq!(report.changes.len(), 1);
        assert_eq!(report.changes[0].name, "home-manager");
    }

    #[test]
    fn demo_inputs_match_demo_changes() {
        let ins = inputs(1_700_000_000);
        for c in changes(4, 1_700_000_000) {
            assert!(ins.iter().any(|i| i.node.as_deref() == Some(&c.name)));
        }
    }
}
