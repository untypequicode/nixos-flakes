//! Mise à jour et application via `nix` et `nixos-rebuild`.

use crate::error::{Error, Result};
use crate::lock::{self, InputChange, LockedInput};
use crate::proc;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Résultat d'une ise à jour (réelle ou simulée).
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
    let token = crate::token::load().ok().flatten();
    check_with("nix", repo, token.as_ref().map(|t| t.expose()))
}

/// Vrai si l'erreur vient du refus de git (libgit2) sur le propriétaire du dépôt.
fn is_ownership_error(e: &Error) -> bool {
    matches!(e, Error::Command { tail, .. } if tail.contains("not owned by current user"))
}

/// Piste de correction citant le dépôt réellement configuré.
fn owner_hint(repo: &Path) -> String {
    let p = repo.display();
    format!("Ajoutez `services.nixos-flakes.systemRepo.owner = \"<utilisateur>\";` à la configuration NixOS (avec `flakePath` ou `systemRepo.path` = {p}) puis reconstruisez : cela déclare {p} dans /etc/gitconfig (safe.directory) et accorde à cet utilisateur une ACL sur le dépôt.")
}

/// Fichiers seuls nécessaires à `nix flake update` (les sorties ne sont pas évaluées).
const FLAKE_FILES: [&str; 2] = ["flake.nix", "flake.lock"];

/// Copie `flake.nix` et `flake.lock` (s'ils existent) de `repo` vers `dst`.
fn copy_flake_files(repo: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).map_err(|e| Error::io(dst, e))?;
    for name in FLAKE_FILES {
        let src = repo.join(name);
        match std::fs::copy(&src, dst.join(name)) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io(src, e)),
        }
    }
    Ok(())
}

/// Erreur finale quand le repli (copie de flake.nix/flake.lock) échoue aussi :
/// garde l'erreur de propriété d'origine et ajoute une piste de correction.
/// Les fichiers illisibles ne sont mentionnés que s'ils sont en cause.
fn ownership_failure(repo: &Path, original: Error, fallback: Error) -> Error {
    let Error::Command {
        program,
        status,
        tail: orig_tail,
    } = &original
    else {
        return fallback;
    };
    let mut msg = format!("{orig_tail}\n");
    match &fallback {
        Error::Io { path, source } if source.kind() == std::io::ErrorKind::PermissionDenied => {
            msg.push_str(&format!(
                "Repli par copie : {} n'est pas lisible par votre utilisateur.\n",
                path.display()
            ));
        }
        Error::Command { tail, .. } if tail.contains("Permission denied") => {
            msg.push_str(
                "Repli par copie : un fichier du dépôt n'est pas lisible par votre utilisateur.\n",
            );
        }
        Error::Command { tail, .. } => {
            msg.push_str(&format!("Repli par copie également en échec :\n{tail}\n"));
        }
        other => msg.push_str(&format!("Repli par copie également en échec : {other}\n")),
    }
    msg.push_str(&owner_hint(repo));
    Error::Command {
        program: program.clone(),
        status: status.clone(),
        tail: msg,
    }
}

pub(crate) fn check_with(nix: &str, repo: &Path, token: Option<&str>) -> Result<UpdateReport> {
    let repo_s = repo.to_string_lossy().into_owned();
    let tmp = tempfile::tempdir().map_err(|e| Error::io(std::env::temp_dir(), e))?;
    let new_lock = tmp.path().join("flake.lock");
    let new_s = new_lock.to_string_lossy().into_owned();
    let env = proc::nix_env(repo, token);
    let run_check = |flake_ref: &str| {
        proc::run_env(
            nix,
            &[
                FEATURES[0],
                FEATURES[1],
                "flake",
                "update",
                "--flake",
                flake_ref,
                "--output-lock-file",
                &new_s,
            ],
            None,
            &env,
            &mut |_| {},
        )
    };
    if let Err(e) = run_check(&repo_s) {
        // Repli : le libgit2 de nix refuse un dépôt d'un autre propriétaire (et
        // un dépôt root peut contenir des fichiers illisibles). Seuls flake.nix
        // et flake.lock comptent : on les copie dans un dossier temporaire
        // hors git, référencé par `path:`.
        if is_ownership_error(&e) {
            let copy = tmp.path().join("copie");
            let res = copy_flake_files(repo, &copy)
                .and_then(|()| run_check(&format!("path:{}", copy.display())));
            if let Err(e2) = res {
                return Err(ownership_failure(repo, e, e2));
            }
        } else {
            return Err(e);
        }
    }
    let old = read_lock(&repo.join("flake.lock"))?;
    let new = read_lock(&new_lock)?;
    Ok(UpdateReport {
        repo: repo.to_path_buf(),
        changes: lock::diff(&old, &new),
    })
}

/// Inputs racine du `flake.lock` de `repo` (vide si le verrou est absent).
pub fn list_inputs(repo: &Path) -> Result<Vec<lock::RootInput>> {
    let path = repo.join("flake.lock");
    match std::fs::read_to_string(&path) {
        Ok(s) => lock::root_inputs(&s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(Error::io(path, e)),
    }
}

/// Vérifie que chaque nom est un input racine du verrou (et ne ressemble pas
/// à une option de la ligne de commande).
pub fn validate_inputs(repo: &Path, inputs: &[String]) -> Result<()> {
    if inputs.is_empty() {
        return Ok(());
    }
    if let Some(bad) = inputs.iter().find(|n| n.is_empty() || n.starts_with('-')) {
        return Err(Error::Config(format!("nom d'input invalide : « {bad} »")));
    }
    let known = list_inputs(repo)?;
    for n in inputs {
        if !known.iter().any(|k| &k.name == n) {
            let names: Vec<&str> = known.iter().map(|k| k.name.as_str()).collect();
            return Err(Error::Config(format!(
                "input inconnu : « {n} » (inputs du verrou : {})",
                if names.is_empty() {
                    "aucun".to_string()
                } else {
                    names.join(", ")
                }
            )));
        }
    }
    Ok(())
}

/// `nix flake update` en utilisateur ; renvoie les changements réels.
pub fn update(repo: &Path, out: &mut dyn FnMut(&str)) -> Result<UpdateReport> {
    update_inputs(repo, &[], out)
}

/// `nix flake update <input>...` ; une liste vide met tout à jour.
/// Les noms doivent être des inputs racine du `flake.lock` (voir `list_inputs`).
pub fn update_inputs(
    repo: &Path,
    inputs: &[String],
    out: &mut dyn FnMut(&str),
) -> Result<UpdateReport> {
    // Une seule opération modifiante à la fois par dépôt.
    let _lock = crate::repo_lock::acquire(repo)?;
    let _owner = crate::fsutil::OwnerGuard::new(repo);
    update_inputs_unlocked(repo, inputs, out)
}

/// Comme `update_inputs`, sans prendre le verrou du dépôt (l'appelant le tient déjà).
pub(crate) fn update_inputs_unlocked(
    repo: &Path,
    inputs: &[String],
    out: &mut dyn FnMut(&str),
) -> Result<UpdateReport> {
    let token = crate::token::load().ok().flatten();
    update_with("nix", repo, inputs, token.as_ref().map(|t| t.expose()), out)
}

pub(crate) fn update_with(
    nix: &str,
    repo: &Path,
    inputs: &[String],
    token: Option<&str>,
    out: &mut dyn FnMut(&str),
) -> Result<UpdateReport> {
    validate_inputs(repo, inputs)?;
    let lock_path = repo.join("flake.lock");
    let old = read_lock(&lock_path)?;
    let repo_s = repo.to_string_lossy().into_owned();
    let mut args: Vec<&str> = vec![FEATURES[0], FEATURES[1], "flake", "update"];
    args.extend(inputs.iter().map(String::as_str));
    args.extend(["--flake", &repo_s]);
    proc::run_env(nix, &args, None, &proc::nix_env(repo, token), out)?;
    let new = read_lock(&lock_path)?;
    Ok(UpdateReport {
        repo: repo.to_path_buf(),
        changes: lock::diff(&old, &new),
    })
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
    let cmd = privileged(privilege, crate::fsutil::is_root(), args);
    let rest: Vec<&str> = cmd[1..].iter().map(String::as_str).collect();
    proc::run(&cmd[0], &rest, None, out)
}

/// Délai de `nixos-rebuild switch` et `--rollback` : 6 h, aligné sur `TimeoutStartSec` du
/// module NixOS. Une reconstruction légitime peut être très longue, et tuer le parent laisserait
/// un `switch` orphelin. `NIXOS_FLAKES_TIMEOUT_SECS` ne s'y applique volontairement PAS.
pub(crate) const SWITCH_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);

fn run_switch(privilege: Privilege, args: &[&str], out: &mut dyn FnMut(&str)) -> Result<()> {
    let cmd = privileged(privilege, crate::fsutil::is_root(), args);
    let rest: Vec<&str> = cmd[1..].iter().map(String::as_str).collect();
    proc::run_env_timeout(&cmd[0], &rest, None, &[], SWITCH_TIMEOUT, out)
}

/// Copie `src` sur `dest` avec élévation de privilèges (`cp` : propriétaire et droits
/// d'un `dest` existant conservés). Sert à écrire sous un dossier appartenant à root.
pub fn install_file(
    src: &Path,
    dest: &Path,
    privilege: Privilege,
    out: &mut dyn FnMut(&str),
) -> Result<()> {
    let (s, d) = (src.to_string_lossy(), dest.to_string_lossy());
    // Atomique : copie sous un nom temporaire imprévisible (`mktemp`, créé exclusivement dans le
    // dossier cible, avec les droits de la commande élevée), propriétaire et droits de `dest`
    // reportés s'il existe, puis `mv` par-dessus `dest`.
    let script = r#"set -e
tmp=$(mktemp -- "$(dirname -- "$2")/.nixos-flakes-XXXXXXXX")
trap 'rm -f -- "$tmp"' EXIT
cat -- "$1" > "$tmp"
if [ -e "$2" ]; then
  chown --reference="$2" -- "$tmp"
  chmod --reference="$2" -- "$tmp"
else
  chmod --reference="$1" -- "$tmp"
fi
mv -f -- "$tmp" "$2"
trap - EXIT"#;
    run_privileged(privilege, &["sh", "-c", script, "sh", &s, &d], out)
}

/// Référence de flake pour `nixos-rebuild` : `<repo>` ou `<repo>#<attr>`.
pub fn flake_ref(repo: &Path, attr: Option<&str>) -> String {
    match attr {
        Some(a) => format!("{}#{a}", repo.display()),
        None => repo.to_string_lossy().into_owned(),
    }
}

/// Arguments de `nixos-rebuild switch` pour `repo` (et `attr` s'il est défini).
pub(crate) fn switch_args(repo: &Path, attr: Option<&str>) -> Result<Vec<String>> {
    crate::config::validate_repo_path(repo)?;
    if let Some(a) = attr {
        crate::config::validate_attr(a)?;
    }
    Ok(vec![
        "nixos-rebuild".into(),
        "switch".into(),
        "--flake".into(),
        flake_ref(repo, attr),
    ])
}

/// `nixos-rebuild switch --flake <repo>[#<attr>]` avec élévation de privilèges.
pub fn apply(
    repo: &Path,
    attr: Option<&str>,
    privilege: Privilege,
    out: &mut dyn FnMut(&str),
) -> Result<()> {
    // Une seule opération modifiante à la fois par dépôt.
    let _lock = crate::repo_lock::acquire(repo)?;
    apply_unlocked(repo, attr, privilege, out)
}

/// Comme `apply`, sans prendre le verrou du dépôt (l'appelant le tient déjà).
pub(crate) fn apply_unlocked(
    repo: &Path,
    attr: Option<&str>,
    privilege: Privilege,
    out: &mut dyn FnMut(&str),
) -> Result<()> {
    let args = switch_args(repo, attr)?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run_switch(privilege, &args, out)
}

/// `nixos-rebuild switch --rollback` avec élévation de privilèges.
pub fn rollback(privilege: Privilege, out: &mut dyn FnMut(&str)) -> Result<()> {
    run_switch(privilege, &["nixos-rebuild", "switch", "--rollback"], out)
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
        let rep = check_with(&nix, &repo, None).unwrap();
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
        let rep = update_with(&nix, &repo, &[], None, &mut |l| lines.push(l.to_string())).unwrap();
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
        assert!(update_with(&nix, &repo, &[], None, &mut |_| {})
            .unwrap()
            .changes
            .is_empty());
    }

    #[test]
    fn failing_nix_gives_command_error() {
        let d = tempfile::tempdir().unwrap();
        let err = check_with("false", d.path(), None).unwrap_err();
        assert!(matches!(err, Error::Command { .. }));
        let err = check_with("nixos-flakes-inexistant", d.path(), None).unwrap_err();
        assert!(matches!(err, Error::Spawn { .. }));
    }

    /// Faux `nix` qui refuse tout dépôt non `path:` comme libgit2 le ferait.
    fn picky_nix(dir: &Path) -> String {
        let script = dir.join("picky-nix");
        fs::write(
            &script,
            r#"#!/bin/sh
out=""; flake=""
while [ $# -gt 0 ]; do
  case "$1" in
    --flake) flake="$2"; shift;;
    --output-lock-file) out="$2"; shift;;
  esac
  shift
done
case "$flake" in
  path:*) echo '{"nodes":{"root":{}},"root":"root","version":7}' > "$out";;
  *) echo "error: repository path '$flake' is not owned by current user" >&2; exit 1;;
esac
"#,
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script.to_string_lossy().into_owned()
    }

    #[test]
    fn check_falls_back_to_path_ref_on_ownership_error() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let nix = picky_nix(d.path());
        assert!(check_with(&nix, &repo, None).is_ok());
    }

    /// Faux `nix` : refuse le dépôt (propriétaire) ; `path:` échoue avec `fallback_msg`.
    fn failing_both(dir: &Path, fallback_msg: &str) -> String {
        let script = dir.join("both-fail-nix");
        fs::write(
            &script,
            format!(
                r#"#!/bin/sh
flake=""
while [ $# -gt 0 ]; do
  case "$1" in --flake) flake="$2"; shift;; esac
  shift
done
case "$flake" in
  path:*) echo "{fallback_msg}" >&2; exit 1;;
  *) echo "error: repository path '$flake' is not owned by current user" >&2; exit 1;;
esac
"#
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script.to_string_lossy().into_owned()
    }

    #[test]
    fn failed_fallback_keeps_original_error_and_hints_owner_option() {
        let d = tempfile::tempdir().unwrap();
        let nix = failing_both(d.path(), "error: boom");
        let err = check_with(&nix, d.path(), None).unwrap_err().to_string();
        assert!(err.contains("not owned by current user"), "{err}");
        assert!(err.contains("error: boom"), "{err}");
        assert!(
            err.contains("services.nixos-flakes.systemRepo.owner"),
            "{err}"
        );
        assert!(
            err.contains("safe.directory") && err.contains("ACL"),
            "{err}"
        );
    }

    #[test]
    fn permission_denied_fallback_explains_unreadable_file() {
        let d = tempfile::tempdir().unwrap();
        let nix = failing_both(d.path(), "error: opening file: Permission denied");
        let err = check_with(&nix, d.path(), None).unwrap_err().to_string();
        assert!(err.contains("n'est pas lisible"), "{err}");
        assert!(err.contains("not owned by current user"), "{err}");
        assert!(
            err.contains("services.nixos-flakes.systemRepo.owner"),
            "{err}"
        );
    }

    #[test]
    fn update_does_not_fall_back() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let nix = picky_nix(d.path());
        assert!(update_with(&nix, &repo, &[], None, &mut |_| {}).is_err());
    }

    #[test]
    fn nix_receives_safe_directory_and_token_env() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let script = d.path().join("env-nix");
        fs::write(
            &script,
            "#!/bin/sh\necho \"$GIT_CONFIG_VALUE_0\"\necho \"$NIX_CONFIG\"\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let mut lines = Vec::new();
        let _ = update_with(
            script.to_str().unwrap(),
            &repo,
            &[],
            Some("ghp_secret"),
            &mut |l| lines.push(l.to_string()),
        );
        assert!(lines.contains(&repo.display().to_string()));
        assert!(lines.contains(&"access-tokens = github.com=ghp_secret".to_string()));
    }

    /// Faux `nix` qui n'accepte `path:` que si le dossier ne contient que flake.nix/flake.lock.
    fn copy_checking_nix(dir: &Path) -> String {
        let script = dir.join("copy-nix");
        fs::write(
            &script,
            r#"#!/bin/sh
out=""; flake=""
while [ $# -gt 0 ]; do
  case "$1" in
    --flake) flake="$2"; shift;;
    --output-lock-file) out="$2"; shift;;
  esac
  shift
done
case "$flake" in
  path:*)
    d="${flake#path:}"
    [ "$(ls "$d" | tr "\n" " ")" = "flake.lock flake.nix " ] || { echo "fichiers en trop" >&2; exit 1; }
    cp "$d/flake.lock" "$out";;
  *) echo "error: repository path '$flake' is not owned by current user" >&2; exit 1;;
esac
"#,
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script.to_string_lossy().into_owned()
    }

    #[test]
    fn fallback_copies_only_flake_files() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        fs::create_dir(&repo).unwrap();
        fs::write(repo.join("flake.nix"), "{}").unwrap();
        fs::write(repo.join("flake.lock"), lock_json(&[("a", "1", 1)])).unwrap();
        fs::write(repo.join("secret.txt"), "x").unwrap();
        let nix = copy_checking_nix(d.path());
        let rep = check_with(&nix, &repo, None).unwrap();
        assert!(rep.changes.is_empty());
        assert!(fs::read_to_string(repo.join("flake.lock"))
            .unwrap()
            .contains("\"a\""));
    }

    #[test]
    fn unreadable_flake_file_is_reported_with_owner_hint() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let f = repo.join("flake.nix");
        fs::write(&f, "{}").unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&f).is_ok() {
            return; // root : les permissions ne s'appliquent pas
        }
        let nix = picky_nix(d.path());
        let err = check_with(&nix, &repo, None).unwrap_err().to_string();
        assert!(err.contains("not owned by current user"), "{err}");
        assert!(err.contains("flake.nix n'est pas lisible"), "{err}");
        assert!(err.contains("systemRepo.owner"), "{err}");
    }

    #[test]
    fn fallback_failure_without_permission_does_not_mention_unreadable() {
        let d = tempfile::tempdir().unwrap();
        let nix = failing_both(d.path(), "error: boom");
        let err = check_with(&nix, d.path(), None).unwrap_err().to_string();
        assert!(!err.contains("pas lisible"), "{err}");
    }

    /// Faux `nix` qui écrit ses arguments dans `<repo>/args`.
    fn args_nix(dir: &Path) -> String {
        let script = dir.join("args-nix");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\necho \"$@\" > \"{}\"\n",
                dir.join("args").display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script.to_string_lossy().into_owned()
    }

    #[test]
    fn update_inputs_passes_names_and_validates() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        fs::create_dir(&repo).unwrap();
        fs::write(
            repo.join("flake.lock"),
            lock_json(&[("nixpkgs", "1", 1), ("home", "2", 2)]),
        )
        .unwrap();
        let nix = args_nix(d.path());
        let mut lines = Vec::new();
        let names = vec!["home".to_string(), "nixpkgs".to_string()];
        update_with(&nix, &repo, &names, None, &mut |l| {
            lines.push(l.to_string())
        })
        .unwrap();
        let args = fs::read_to_string(d.path().join("args")).unwrap();
        assert!(args.contains("flake update home nixpkgs --flake"), "{args}");
        // Vide = tout.
        update_with(&nix, &repo, &[], None, &mut |_| {}).unwrap();
        let args = fs::read_to_string(d.path().join("args")).unwrap();
        assert!(args.contains("flake update --flake"), "{args}");
        // Invalides : rien n'est lancé.
        let _ = fs::remove_file(d.path().join("args"));
        for bad in ["--commit-lock-file", "", "inconnu"] {
            let r = update_with(&nix, &repo, &[bad.to_string()], None, &mut |_| {});
            assert!(matches!(r, Err(Error::Config(_))), "{bad}");
        }
        assert!(!d.path().join("args").exists());
    }

    #[test]
    fn list_inputs_reads_root_inputs() {
        let d = tempfile::tempdir().unwrap();
        assert!(list_inputs(d.path()).unwrap().is_empty());
        fs::write(
            d.path().join("flake.lock"),
            lock_json(&[("a", "1234567890", 1)]),
        )
        .unwrap();
        let l = list_inputs(d.path()).unwrap();
        assert_eq!(l[0].name, "a");
        assert_eq!(l[0].short_rev.as_deref(), Some("1234567"));
    }

    #[test]
    fn switch_args_rejects_ref_metacharacters() {
        assert!(switch_args(Path::new("/srv/a#b"), None).is_err());
        assert!(switch_args(Path::new("/srv/a?x=1"), Some("h")).is_err());
    }

    #[test]
    fn switch_args_with_and_without_attr() {
        let repo = Path::new("/home/u/ma config");
        assert_eq!(
            switch_args(repo, None).unwrap(),
            ["nixos-rebuild", "switch", "--flake", "/home/u/ma config"]
        );
        assert_eq!(
            switch_args(repo, Some("laptop")).unwrap()[3],
            "/home/u/ma config#laptop"
        );
        assert!(switch_args(repo, Some("a b;rm")).is_err());
        assert_eq!(
            flake_ref(Path::new("/srv/nixos"), Some("h")),
            "/srv/nixos#h"
        );
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
