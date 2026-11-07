//! Opérations git (via le binaire `git`).

use crate::error::Result;
use crate::lock::InputChange;
use crate::proc;
use std::path::Path;

/// État git d'un dépôt flake.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitState {
    pub is_repo: bool,
    pub branch: Option<String>,
    /// Chemins modifiés ou non suivis (hors `flake.lock`), relatifs au dossier du dépôt flake.
    pub dirty: Vec<String>,
    /// `flake.lock` modifié.
    pub lock_dirty: bool,
}

/// Arguments communs : `safe.directory=*` évite les refus « dubious ownership ».
fn base<'a>(repo: &'a str, rest: &[&'a str]) -> Vec<&'a str> {
    let mut v = vec!["-c", "safe.directory=*", "-C", repo];
    v.extend_from_slice(rest);
    v
}

fn repo_str(repo: &Path) -> String {
    repo.to_string_lossy().into_owned()
}

/// État git de `repo`. Jamais d'erreur : un dossier hors git (ou sans binaire
/// `git`) donne `is_repo = false`.
pub fn state(repo: &Path) -> Result<GitState> {
    Ok(state_inner(repo).unwrap_or_default())
}

fn state_inner(repo: &Path) -> Option<GitState> {
    let r = repo_str(repo);
    let git = |rest: &[&str]| -> Option<String> {
        let o = proc::output("git", &base(&r, rest), None).ok()?;
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    if git(&["rev-parse", "--is-inside-work-tree"])?.trim() != "true" {
        return None;
    }
    let prefix = git(&["rev-parse", "--show-prefix"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let branch = git(&["symbolic-ref", "--short", "-q", "HEAD"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let porcelain = git(&[
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--",
        ".",
    ])?;
    let (dirty, lock_dirty) = parse_porcelain(&porcelain, &prefix);
    Some(GitState {
        is_repo: true,
        branch,
        dirty,
        lock_dirty,
    })
}

/// Analyse la sortie `status --porcelain=v1 -z`. Renvoie (chemins hors
/// `flake.lock`, `flake.lock` modifié). `prefix` est retiré des chemins.
fn parse_porcelain(text: &str, prefix: &str) -> (Vec<String>, bool) {
    let mut dirty = Vec::new();
    let mut lock_dirty = false;
    let mut parts = text.split('\0');
    while let Some(entry) = parts.next() {
        if entry.len() < 4 {
            continue;
        }
        let xy = &entry[..2];
        let path = entry[3..].strip_prefix(prefix).unwrap_or(&entry[3..]);
        if xy.contains('R') || xy.contains('C') {
            // l'ancien nom suit, séparé par NUL
            let _ = parts.next();
        }
        if path == "flake.lock" {
            lock_dirty = true;
        } else {
            dirty.push(path.to_string());
        }
    }
    (dirty, lock_dirty)
}

fn run_git(repo: &Path, rest: &[&str], out: &mut dyn FnMut(&str)) -> Result<()> {
    let r = repo_str(repo);
    proc::run("git", &base(&r, rest), None, out)
}

/// `git add -A` puis `git commit -m <message>`.
pub fn commit_all(repo: &Path, message: &str, out: &mut dyn FnMut(&str)) -> Result<()> {
    run_git(repo, &["add", "-A"], out)?;
    run_git(repo, &["commit", "-m", message], out)
}

/// Commit de `flake.lock` seul, quel que soit le reste de l'index.
pub fn commit_lock(repo: &Path, message: &str, out: &mut dyn FnMut(&str)) -> Result<()> {
    run_git(repo, &["add", "--", "flake.lock"], out)?;
    run_git(repo, &["commit", "-m", message, "--", "flake.lock"], out)
}

/// `git push`.
pub fn push(repo: &Path, out: &mut dyn FnMut(&str)) -> Result<()> {
    run_git(repo, &["push"], out)
}

fn short(rev: &Option<String>) -> String {
    rev.as_deref()
        .map_or_else(|| "?".to_string(), |r| r.chars().take(7).collect())
}

/// Message de commit : `flake.lock: update inputs` + corps listant les inputs.
pub fn lock_commit_message(changes: &[InputChange]) -> String {
    let title = "flake.lock: update inputs";
    if changes.is_empty() {
        return title.to_string();
    }
    let body: Vec<String> = changes
        .iter()
        .map(|c| match (&c.old_rev, &c.new_rev) {
            (None, Some(_)) => format!("- {}: added {}", c.name, short(&c.new_rev)),
            (Some(_), None) => format!("- {}: removed (was {})", c.name, short(&c.old_rev)),
            _ => format!(
                "- {}: {} -> {}",
                c.name,
                short(&c.old_rev),
                short(&c.new_rev)
            ),
        })
        .collect();
    format!("{title}\n\n{}", body.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{git, init_repo};
    use std::fs;

    fn change(name: &str, old: Option<&str>, new: Option<&str>) -> InputChange {
        InputChange {
            name: name.into(),
            old_rev: old.map(Into::into),
            new_rev: new.map(Into::into),
            old_date: None,
            new_date: None,
        }
    }

    #[test]
    fn non_repo_is_not_an_error() {
        let d = tempfile::tempdir().unwrap();
        let s = state(d.path()).unwrap();
        assert!(!s.is_repo);
        assert_eq!(s, GitState::default());
        let s = state(Path::new("/nonexistent-nixos-flakes")).unwrap();
        assert!(!s.is_repo);
    }

    #[test]
    fn clean_dirty_and_lock() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        init_repo(p);
        fs::write(p.join("flake.nix"), "{}").unwrap();
        fs::write(p.join("flake.lock"), "{}").unwrap();
        git(p, &["add", "-A"]);
        git(p, &["commit", "-q", "-m", "init"]);
        let s = state(p).unwrap();
        assert!(s.is_repo);
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert!(s.dirty.is_empty());
        assert!(!s.lock_dirty);

        fs::write(p.join("flake.lock"), "{\"a\":1}").unwrap();
        fs::write(p.join("flake.nix"), "{ }").unwrap();
        fs::create_dir(p.join("sub")).unwrap();
        fs::write(p.join("sub/new file.nix"), "").unwrap();
        let s = state(p).unwrap();
        assert!(s.lock_dirty);
        assert_eq!(s.dirty, vec!["flake.nix", "sub/new file.nix"]);
    }

    #[test]
    fn unborn_branch_and_detached() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        init_repo(p);
        assert_eq!(state(p).unwrap().branch.as_deref(), Some("main"));
        fs::write(p.join("a"), "").unwrap();
        git(p, &["add", "-A"]);
        git(p, &["commit", "-q", "-m", "i"]);
        git(p, &["checkout", "-q", "--detach"]);
        let s = state(p).unwrap();
        assert!(s.is_repo);
        assert_eq!(s.branch, None);
    }

    #[test]
    fn subdirectory_of_repo_uses_relative_paths() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        init_repo(p);
        fs::create_dir(p.join("sub")).unwrap();
        fs::write(p.join("sub/flake.lock"), "1").unwrap();
        fs::write(p.join("sub/x.nix"), "").unwrap();
        fs::write(p.join("outside"), "").unwrap();
        let s = state(&p.join("sub")).unwrap();
        assert!(s.lock_dirty);
        assert_eq!(s.dirty, vec!["x.nix"]);
    }

    #[test]
    fn renames_are_parsed() {
        let (d, l) = parse_porcelain("R  new\0old\0 M flake.lock\0?? z\0", "");
        assert_eq!(d, vec!["new", "z"]);
        assert!(l);
    }

    #[test]
    fn commit_lock_only_commits_lock() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        init_repo(p);
        fs::write(p.join("flake.nix"), "{}").unwrap();
        fs::write(p.join("flake.lock"), "1").unwrap();
        git(p, &["add", "-A"]);
        git(p, &["commit", "-q", "-m", "init"]);
        fs::write(p.join("flake.lock"), "2").unwrap();
        fs::write(p.join("flake.nix"), "{ }").unwrap();
        fs::write(p.join("other"), "").unwrap();
        git(p, &["add", "other"]);
        let mut lines = Vec::new();
        commit_lock(p, "flake.lock: update inputs", &mut |l| {
            lines.push(l.to_string())
        })
        .unwrap();
        assert_eq!(
            git(p, &["show", "--name-only", "--format=%s", "HEAD"]).trim(),
            "flake.lock: update inputs\n\nflake.lock"
        );
        let s = state(p).unwrap();
        assert!(!s.lock_dirty);
        assert_eq!(s.dirty, vec!["flake.nix", "other"]);
    }

    #[test]
    fn commit_all_commits_everything() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        init_repo(p);
        fs::write(p.join("a"), "").unwrap();
        fs::write(p.join("flake.lock"), "").unwrap();
        commit_all(p, "tout", &mut |_| {}).unwrap();
        let s = state(p).unwrap();
        assert!(s.dirty.is_empty() && !s.lock_dirty);
        // rien à valider => erreur
        assert!(commit_all(p, "vide", &mut |_| {}).is_err());
    }

    #[test]
    fn push_to_local_remote() {
        let r = tempfile::tempdir().unwrap();
        git(r.path(), &["init", "-q", "--bare", "-b", "main"]);
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        init_repo(p);
        git(p, &["remote", "add", "origin", r.path().to_str().unwrap()]);
        fs::write(p.join("a"), "").unwrap();
        commit_all(p, "m", &mut |_| {}).unwrap();
        git(p, &["push", "-q", "-u", "origin", "main"]);
        fs::write(p.join("b"), "").unwrap();
        commit_all(p, "m2", &mut |_| {}).unwrap();
        push(p, &mut |_| {}).unwrap();
        assert!(git(r.path(), &["log", "--format=%s", "main"]).contains("m2"));
    }

    #[test]
    fn push_without_remote_fails() {
        let d = tempfile::tempdir().unwrap();
        init_repo(d.path());
        assert!(push(d.path(), &mut |_| {}).is_err());
    }

    #[test]
    fn commit_message_format() {
        assert_eq!(lock_commit_message(&[]), "flake.lock: update inputs");
        let m = lock_commit_message(&[
            change(
                "nixpkgs",
                Some("0123456789abcdef"),
                Some("fedcba9876543210"),
            ),
            change("new", None, Some("abc")),
            change("gone", Some("1234567890"), None),
        ]);
        assert_eq!(
            m,
            "flake.lock: update inputs\n\n- nixpkgs: 0123456 -> fedcba9\n- new: added abc\n- gone: removed (was 1234567)"
        );
    }
}
