//! Opérations git (via le binaire `git`).

use crate::error::{Error, Result};
use crate::lock::InputChange;
use crate::proc;
use std::path::{Path, PathBuf};
use std::process::Output;

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

/// Racine de l'arbre de travail de `repo` : premier ancêtre portant `.git`
/// (le flake peut être dans un sous-dossier), sinon `repo` lui-même.
pub(crate) fn worktree_root(repo: &Path) -> PathBuf {
    let canon = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    canon
        .ancestors()
        .find(|a| a.join(".git").exists())
        .map_or(canon.clone(), Path::to_path_buf)
}

/// Arguments communs : `safe.directory` déclare les chemins exacts du dépôt
/// (jamais `*`) pour éviter les refus « dubious ownership ».
fn base(repo: &Path, rest: &[&str]) -> Vec<String> {
    let mut safe: Vec<String> = vec![
        repo.to_string_lossy().into_owned(),
        repo.canonicalize()
            .unwrap_or_else(|_| repo.to_path_buf())
            .to_string_lossy()
            .into_owned(),
        worktree_root(repo).to_string_lossy().into_owned(),
    ];
    safe.sort();
    safe.dedup();
    let mut v = Vec::new();
    for s in safe {
        v.push("-c".to_string());
        v.push(format!("safe.directory={s}"));
    }
    v.push("-C".to_string());
    v.push(repo.to_string_lossy().into_owned());
    v.extend(rest.iter().map(|s| (*s).to_string()));
    v
}

/// Exécute git et renvoie la sortie brute (messages en anglais pour pouvoir
/// reconnaître « not a git repository »). Erreur seulement si git ne se lance
/// pas ou dépasse son délai.
fn git_output(repo: &Path, rest: &[&str]) -> Result<Output> {
    let args = base(repo, rest);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    proc::output_env("git", &args, None, &[("LC_ALL", "C")])
}

/// Message d'erreur git : première ligne utile de stderr.
fn git_failure(rest: &[&str], o: &Output) -> Error {
    let err = String::from_utf8_lossy(&o.stderr);
    let msg = err.trim();
    Error::Git(format!(
        "« git {} » a échoué ({}){}",
        rest.first().copied().unwrap_or(""),
        o.status
            .code()
            .map_or_else(|| o.status.to_string(), |c| format!("code {c}")),
        if msg.is_empty() {
            String::new()
        } else {
            format!(" : {msg}")
        }
    ))
}

/// Sortie standard de git ; toute erreur (lancement ou code non nul) est remontée.
fn git_ok(repo: &Path, rest: &[&str]) -> Result<String> {
    let o = git_output(repo, rest)?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        Err(git_failure(rest, &o))
    }
}

/// Comme `git_ok`, mais un code non nul donne `None` (cas attendu : pas de
/// branche, pas d'amont…). Les échecs de lancement restent des erreurs.
fn git_opt(repo: &Path, rest: &[&str]) -> Result<Option<String>> {
    let o = git_output(repo, rest)?;
    Ok(o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).into_owned()))
}

/// `Ok(true)` dans un arbre de travail git, `Ok(false)` si `repo` n'est pas un
/// dépôt (dossier absent ou « not a git repository »), `Err` pour tout autre
/// échec de git (binaire absent, délai, propriétaire douteux, dépôt corrompu…).
fn is_work_tree(repo: &Path) -> Result<bool> {
    if !repo.is_dir() {
        return Ok(false);
    }
    let rest = ["rev-parse", "--is-inside-work-tree"];
    let o = git_output(repo, &rest)?;
    if o.status.success() {
        return Ok(String::from_utf8_lossy(&o.stdout).trim() == "true");
    }
    if String::from_utf8_lossy(&o.stderr).contains("not a git repository") {
        return Ok(false);
    }
    Err(git_failure(&rest, &o))
}

/// État git de `repo`. Un dossier qui n'est pas un dépôt donne `is_repo =
/// false` ; un échec de git (binaire absent, délai dépassé, dépôt illisible…)
/// est une erreur, pas un « pas un dépôt ».
pub fn state(repo: &Path) -> Result<GitState> {
    if !is_work_tree(repo)? {
        return Ok(GitState::default());
    }
    let prefix = git_ok(repo, &["rev-parse", "--show-prefix"])?
        .trim()
        .to_string();
    let branch = git_opt(repo, &["symbolic-ref", "--short", "-q", "HEAD"])?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let porcelain = git_ok(
        repo,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
            ".",
        ],
    )?;
    let (dirty, lock_dirty) = parse_porcelain(&porcelain, &prefix);
    Ok(GitState {
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
    let args = base(repo, rest);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    proc::run("git", &args, None, out)
}

/// `git add -A -- .` puis `git commit -m <message> -- .` : limité au dossier du
/// flake (ni le reste de l'arbre de travail, ni ce qui est déjà indexé ailleurs).
pub fn commit_all(repo: &Path, message: &str, out: &mut dyn FnMut(&str)) -> Result<()> {
    run_git(repo, &["add", "-A", "--", "."], out)?;
    run_git(repo, &["commit", "-m", message, "--", "."], out)
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

/// Retard/avance d'un dépôt par rapport à sa branche amont.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncStatus {
    pub branch: Option<String>,
    /// Branche amont suivie (ex. `origin/main`), si elle existe.
    pub upstream: Option<String>,
    /// Commits locaux absents de l'amont.
    pub ahead: u32,
    /// Commits de l'amont absents en local.
    pub behind: u32,
    /// Modifications de fichiers suivis (y compris `flake.lock`) : bloquent une avance rapide.
    pub dirty: Vec<String>,
}

impl SyncStatus {
    /// Les historiques ont divergé (commits de chaque côté).
    pub fn diverged(&self) -> bool {
        self.ahead > 0 && self.behind > 0
    }

    /// Vérifie qu'une avance rapide est possible ; sinon, message clair.
    pub fn check_pullable(&self) -> Result<()> {
        if self.upstream.is_none() {
            return Err(Error::Git(
                "aucune branche amont n'est configurée pour la branche courante".into(),
            ));
        }
        if !self.dirty.is_empty() {
            return Err(Error::Git(format!(
                "l'arbre de travail est sale ({} fichier(s) modifié(s)) : commitez ou annulez vos modifications avant de synchroniser",
                self.dirty.len()
            )));
        }
        if self.diverged() {
            return Err(Error::Git(format!(
                "les historiques ont divergé ({} commit(s) local(aux), {} distant(s)) : fusionnez ou rebasez à la main",
                self.ahead, self.behind
            )));
        }
        Ok(())
    }

    /// Résumé lisible : « à jour », « 2 en retard »…
    pub fn summary(&self) -> String {
        match (self.ahead, self.behind) {
            (0, 0) => "à jour".to_string(),
            (a, 0) => format!("{a} en avance (à pousser)"),
            (0, b) => format!("{b} en retard (à récupérer)"),
            (a, b) => format!("divergé : {a} en avance, {b} en retard"),
        }
    }
}

/// Variables d'environnement qui empêchent git d'attendre une saisie.
fn non_interactive() -> Vec<(String, String)> {
    vec![("GIT_TERMINAL_PROMPT".to_string(), "0".to_string())]
}

/// `git fetch` (met à jour les références distantes, ne touche pas à l'arbre de travail).
pub fn fetch(repo: &Path, out: &mut dyn FnMut(&str)) -> Result<()> {
    let args = base(repo, &["fetch"]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    proc::run_env("git", &args, None, &non_interactive(), out)
}

/// Avance/retard de `repo` par rapport à son amont (sans accès réseau : voir [`fetch`]).
/// Un échec de git (autre qu'« aucun amont ») est une erreur : on ne le
/// confond pas avec un arbre propre.
pub fn sync_status(repo: &Path) -> Result<SyncStatus> {
    if !is_work_tree(repo)? {
        return Err(Error::Git(format!(
            "{} n'est pas un dépôt git",
            repo.display()
        )));
    }
    let branch = git_opt(repo, &["symbolic-ref", "--short", "-q", "HEAD"])?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let upstream = git_opt(
        repo,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    )?
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty());
    let (ahead, behind) = if upstream.is_some() {
        let counts = git_ok(
            repo,
            &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
        )?;
        let mut it = counts.split_whitespace().map(|n| n.parse::<u32>().ok());
        match (it.next().flatten(), it.next().flatten()) {
            (Some(a), Some(b)) => (a, b),
            _ => {
                return Err(Error::Git(format!(
                    "sortie inattendue de « git rev-list » : {}",
                    counts.trim()
                )))
            }
        }
    } else {
        (0, 0)
    };
    let porcelain = git_ok(repo, &["status", "--porcelain=v1", "--untracked-files=no"])?;
    let dirty = porcelain
        .lines()
        .filter(|l| l.len() > 3)
        .map(|l| l[3..].to_string())
        .collect();
    Ok(SyncStatus {
        branch,
        upstream,
        ahead,
        behind,
        dirty,
    })
}

/// `git pull --ff-only`, refusé (sans rien modifier) si l'arbre est sale, si les
/// historiques ont divergé ou s'il n'y a pas d'amont. Ne fait rien s'il n'y a rien à récupérer.
pub fn pull_ff(repo: &Path, out: &mut dyn FnMut(&str)) -> Result<()> {
    let status = sync_status(repo)?;
    status.check_pullable()?;
    if status.behind == 0 {
        out("Déjà à jour, rien à récupérer.");
        return Ok(());
    }
    let args = base(repo, &["pull", "--ff-only"]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    proc::run_env("git", &args, None, &non_interactive(), out)
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
    fn git_failure_is_an_error_not_a_non_repo() {
        // Dépôt corrompu : git échoue autrement que par « not a git repository ».
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        init_repo(p);
        fs::write(p.join(".git/config"), "[core\nrubbish\n").unwrap();
        let e = state(p).unwrap_err().to_string();
        assert!(e.contains("git"), "{e}");
        assert!(sync_status(p).is_err());
    }

    #[test]
    fn safe_directory_is_exact_never_wildcard() {
        let d = tempfile::tempdir().unwrap();
        let args = base(d.path(), &["status"]);
        assert!(args.iter().all(|a| a != "safe.directory=*"));
        let want = format!("safe.directory={}", d.path().display());
        assert!(args.contains(&want), "{args:?}");
    }

    #[test]
    fn commit_all_is_limited_to_flake_dir() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        init_repo(p);
        fs::create_dir(p.join("sub")).unwrap();
        fs::write(p.join("sub/a"), "").unwrap();
        fs::write(p.join("hors"), "").unwrap();
        commit_all(&p.join("sub"), "m", &mut |_| {}).unwrap();
        let files = git(p, &["show", "--name-only", "--format=", "HEAD"]);
        assert_eq!(files.trim(), "sub/a");
        assert!(state(p).unwrap().dirty.contains(&"hors".to_string()));
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

    /// Dépôt distant nu + deux clones `a` (qui pousse) et `b` (qui synchronise).
    fn sync_fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let remote = d.path().join("remote.git");
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        fs::create_dir_all(&remote).unwrap();
        git(&remote, &["init", "-q", "--bare", "-b", "main"]);
        fs::create_dir_all(&a).unwrap();
        init_repo(&a);
        git(&a, &["remote", "add", "origin", remote.to_str().unwrap()]);
        fs::write(a.join("f"), "1").unwrap();
        commit_all(&a, "c1", &mut |_| {}).unwrap();
        git(&a, &["push", "-q", "-u", "origin", "main"]);
        git(
            d.path(),
            &["clone", "-q", remote.to_str().unwrap(), b.to_str().unwrap()],
        );
        git(&b, &["config", "user.name", "Test"]);
        git(&b, &["config", "user.email", "test@example.org"]);
        git(&b, &["config", "commit.gpgsign", "false"]);
        (d, a, b)
    }

    fn push_new_commit(a: &Path, name: &str) {
        fs::write(a.join(name), name).unwrap();
        commit_all(a, name, &mut |_| {}).unwrap();
        git(a, &["push", "-q"]);
    }

    #[test]
    fn sync_up_to_date() {
        let (_d, _a, b) = sync_fixture();
        fetch(&b, &mut |_| {}).unwrap();
        let s = sync_status(&b).unwrap();
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind), (0, 0));
        assert_eq!(s.summary(), "à jour");
        pull_ff(&b, &mut |_| {}).unwrap();
    }

    #[test]
    fn sync_behind_fetch_then_pull() {
        let (_d, a, b) = sync_fixture();
        push_new_commit(&a, "c2");
        // Sans fetch, le retard est inconnu.
        assert_eq!(sync_status(&b).unwrap().behind, 0);
        fetch(&b, &mut |_| {}).unwrap();
        let s = sync_status(&b).unwrap();
        assert_eq!((s.ahead, s.behind), (0, 1));
        assert!(s.summary().contains("1 en retard"));
        pull_ff(&b, &mut |_| {}).unwrap();
        assert!(b.join("c2").exists());
        assert_eq!(sync_status(&b).unwrap().behind, 0);
    }

    #[test]
    fn sync_ahead_only_is_not_an_error() {
        let (_d, _a, b) = sync_fixture();
        fs::write(b.join("local"), "x").unwrap();
        commit_all(&b, "local", &mut |_| {}).unwrap();
        fetch(&b, &mut |_| {}).unwrap();
        let s = sync_status(&b).unwrap();
        assert_eq!((s.ahead, s.behind), (1, 0));
        pull_ff(&b, &mut |_| {}).unwrap();
    }

    #[test]
    fn pull_refuses_dirty_tree() {
        let (_d, a, b) = sync_fixture();
        push_new_commit(&a, "c2");
        fetch(&b, &mut |_| {}).unwrap();
        fs::write(b.join("f"), "modifié").unwrap();
        let e = pull_ff(&b, &mut |_| {}).unwrap_err().to_string();
        assert!(e.contains("sale"), "{e}");
        assert!(!b.join("c2").exists());
        assert_eq!(fs::read_to_string(b.join("f")).unwrap(), "modifié");
    }

    #[test]
    fn pull_refuses_diverged_history() {
        let (_d, a, b) = sync_fixture();
        push_new_commit(&a, "c2");
        fs::write(b.join("local"), "x").unwrap();
        commit_all(&b, "local", &mut |_| {}).unwrap();
        fetch(&b, &mut |_| {}).unwrap();
        let s = sync_status(&b).unwrap();
        assert!(s.diverged());
        let e = pull_ff(&b, &mut |_| {}).unwrap_err().to_string();
        assert!(e.contains("divergé"), "{e}");
        assert!(!b.join("c2").exists());
    }

    #[test]
    fn sync_without_upstream_or_repo() {
        let d = tempfile::tempdir().unwrap();
        assert!(sync_status(d.path()).is_err());
        init_repo(d.path());
        let s = sync_status(d.path()).unwrap();
        assert_eq!(s.upstream, None);
        let e = pull_ff(d.path(), &mut |_| {}).unwrap_err().to_string();
        assert!(e.contains("amont"), "{e}");
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
