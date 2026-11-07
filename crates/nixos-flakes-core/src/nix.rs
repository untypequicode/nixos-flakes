//! Mise à jour et application via `nix` et `nixos-rebuild`.

use crate::error::{Error, Result};
use crate::lock::{self, InputChange, LockedInput};
use crate::proc;
use std::path::{Path, PathBuf};

/// Résultat d'une mise à jour (réelle ou simulée).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReport {
    pub repo: PathBuf,
    pub changes: Vec<InputChange>,
}

/// Méthode d'élévation de privilèges pour `nixos-rebuild`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Privilege {
    Pkexec,
    Sudo,
}

const FEATURES: [&str; 2] = ["--extra-experimental-features", "nix-command flakes"];

/// Lit un `flake.lock` ; absent => aucun input.
fn read_lock(path: &Path) -> Result<Vec<LockedInput>> {
    match std::fs::read_to_string(path) {
        Ok(s) => lock::parse_lock(&s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(Error::io(path, e)),
    }
}

/// Simule `nix flake update` SANS toucher au dépôt : le nouveau verrou est
/// écrit dans un fichier temporaire via `--output-lock-file`.
///
/// Limite : `nix` évalue le dépôt en place ; il n'écrit rien dedans, mais un
/// dépôt git sans `flake.nix` suivi peut être refusé par nix.
pub fn check(repo: &Path) -> Result<UpdateReport> {
    check_with("nix", repo)
}

pub(crate) fn check_with(nix: &str, repo: &Path) -> Result<UpdateReport> {
    let tmp = tempfile::tempdir().map_err(|e| Error::io(std::env::temp_dir(), e))?;
    let new_lock = tmp.path().join("flake.lock");
    let repo_s = repo.to_string_lossy().into_owned();
    let new_s = new_lock.to_string_lossy().into_owned();
    proc::run(
        nix,
        &[
            FEATURES[0],
            FEATURES[1],
            "flake",
            "update",
            "--flake",
            &repo_s,
            "--output-lock-file",
            &new_s,
        ],
        None,
        &mut |_| {},
    )?;
    let old = read_lock(&repo.join("flake.lock"))?;
    let new = read_lock(&new_lock)?;
    Ok(UpdateReport {
        repo: repo.to_path_buf(),
        changes: lock::diff(&old, &new),
    })
}

/// `nix flake update` en utilisateur ; renvoie les changements réels.
pub fn update(repo: &Path, out: &mut dyn FnMut(&str)) -> Result<UpdateReport> {
    update_with("nix", repo, out)
}

pub(crate) fn update_with(
    nix: &str,
    repo: &Path,
    out: &mut dyn FnMut(&str),
) -> Result<UpdateReport> {
    let lock_path = repo.join("flake.lock");
    let old = read_lock(&lock_path)?;
    let repo_s = repo.to_string_lossy().into_owned();
    proc::run(
        nix,
        &[
            FEATURES[0],
            FEATURES[1],
            "flake",
            "update",
            "--flake",
            &repo_s,
        ],
        None,
        out,
    )?;
    let new = read_lock(&lock_path)?;
    Ok(UpdateReport {
        repo: repo.to_path_buf(),
        changes: lock::diff(&old, &new),
    })
}

fn is_root() -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0)
}

/// Commande finale : `args` préfixé de `pkexec`/`sudo` sauf si déjà root.
pub(crate) fn privileged(privilege: Privilege, root: bool, args: &[&str]) -> Vec<String> {
    let mut v = Vec::new();
    if !root {
        v.push(
            match privilege {
                Privilege::Pkexec => "pkexec",
                Privilege::Sudo => "sudo",
            }
            .to_string(),
        );
    }
    v.extend(args.iter().map(|s| s.to_string()));
    v
}

fn run_privileged(privilege: Privilege, args: &[&str], out: &mut dyn FnMut(&str)) -> Result<()> {
    let cmd = privileged(privilege, is_root(), args);
    let rest: Vec<&str> = cmd[1..].iter().map(String::as_str).collect();
    proc::run(&cmd[0], &rest, None, out)
}

/// `nixos-rebuild switch --flake <repo>` avec élévation de privilèges.
pub fn apply(repo: &Path, privilege: Privilege, out: &mut dyn FnMut(&str)) -> Result<()> {
    let repo_s = repo.to_string_lossy().into_owned();
    run_privileged(
        privilege,
        &["nixos-rebuild", "switch", "--flake", &repo_s],
        out,
    )
}

/// `nixos-rebuild switch --rollback` avec élévation de privilèges.
pub fn rollback(privilege: Privilege, out: &mut dyn FnMut(&str)) -> Result<()> {
    run_privileged(privilege, &["nixos-rebuild", "switch", "--rollback"], out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::lock_json;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    /// Faux `nix` : copie `$NEW_LOCK` vers `--output-lock-file`, ou vers `<flake>/flake.lock`.
    fn fake_nix(dir: &Path, new_lock: &str) -> String {
        let src = dir.join("new.lock");
        fs::write(&src, new_lock).unwrap();
        let script = dir.join("fake-nix");
        fs::write(
            &script,
            format!(
                r#"#!/bin/sh
flake=""; outf=""
while [ $# -gt 0 ]; do
  case "$1" in
    --flake) flake="$2"; shift;;
    --output-lock-file) outf="$2"; shift;;
  esac
  shift
done
echo "mise à jour de $flake"
if [ -n "$outf" ]; then cp "{src}" "$outf"; else cp "{src}" "$flake/flake.lock"; fi
"#,
                src = src.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script.to_string_lossy().into_owned()
    }

    #[test]
    fn check_does_not_touch_repo() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        fs::create_dir(&repo).unwrap();
        fs::write(repo.join("flake.nix"), "{}").unwrap();
        let old = lock_json(&[("nixpkgs", "aaaaaaaa", 1)]);
        fs::write(repo.join("flake.lock"), &old).unwrap();
        let nix = fake_nix(d.path(), &lock_json(&[("nixpkgs", "bbbbbbbb", 2)]));
        let rep = check_with(&nix, &repo).unwrap();
        assert_eq!(rep.repo, repo);
        assert_eq!(rep.changes.len(), 1);
        assert_eq!(rep.changes[0].new_rev.as_deref(), Some("bbbbbbbb"));
        assert_eq!(fs::read_to_string(repo.join("flake.lock")).unwrap(), old);
    }

    #[test]
    fn update_reports_real_changes() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        fs::create_dir(&repo).unwrap();
        fs::write(repo.join("flake.lock"), lock_json(&[("a", "1111111", 1)])).unwrap();
        let nix = fake_nix(d.path(), &lock_json(&[("a", "2222222", 2)]));
        let mut lines = Vec::new();
        let rep = update_with(&nix, &repo, &mut |l| lines.push(l.to_string())).unwrap();
        assert_eq!(rep.changes.len(), 1);
        assert!(lines[0].starts_with("mise à jour de"));
        assert!(fs::read_to_string(repo.join("flake.lock"))
            .unwrap()
            .contains("2222222"));
    }

    #[test]
    fn update_without_changes_and_without_initial_lock() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let nix = fake_nix(d.path(), &lock_json(&[]));
        assert!(update_with(&nix, &repo, &mut |_| {})
            .unwrap()
            .changes
            .is_empty());
    }

    #[test]
    fn failing_nix_gives_command_error() {
        let d = tempfile::tempdir().unwrap();
        let err = check_with("false", d.path()).unwrap_err();
        assert!(matches!(err, Error::Command { .. }));
        let err = check_with("nixos-flakes-inexistant", d.path()).unwrap_err();
        assert!(matches!(err, Error::Spawn { .. }));
    }

    #[test]
    fn privilege_prefix() {
        assert_eq!(
            privileged(Privilege::Pkexec, false, &["nixos-rebuild", "switch"]),
            ["pkexec", "nixos-rebuild", "switch"]
        );
        assert_eq!(privileged(Privilege::Sudo, false, &["x"]), ["sudo", "x"]);
        assert_eq!(privileged(Privilege::Sudo, true, &["x"]), ["x"]);
    }

    /// Réseau + vrai nix : `cargo test -- --ignored`.
    #[test]
    #[ignore = "nécessite nix et un accès réseau"]
    fn real_nix_check_leaves_repo_untouched() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join("flake.nix"),
            r#"{ inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable"; outputs = { self, nixpkgs }: {}; }"#,
        )
        .unwrap();
        let rep = check(d.path()).unwrap();
        assert!(!rep.changes.is_empty());
        assert!(!d.path().join("flake.lock").exists());
    }
}
