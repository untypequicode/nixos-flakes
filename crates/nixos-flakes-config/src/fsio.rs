//! Écriture sur disque et validation externe (seules parties avec E/S).

use crate::error::{Error, Result};
use std::fs;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::process::{Command, Stdio};

/// Écrit `content` dans `path` de façon atomique (fichier temporaire voisin puis
/// renommage). Les droits d'un fichier existant sont conservés (0644 sinon) ; un lien
/// symbolique est suivi plutôt que remplacé.
pub fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let dest = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mode = fs::metadata(&dest).map_or(0o644, |m| m.permissions().mode() & 0o7777);
    let dir = dest.parent().unwrap_or_else(|| Path::new("."));
    let file = dest
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let tmp = dir.join(format!(".{file}.{}.tmp", std::process::id()));
    let _ = fs::remove_file(&tmp);
    let res = (|| {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.set_permissions(fs::Permissions::from_mode(mode))?;
        f.sync_all()?;
        fs::rename(&tmp, &dest)
    })();
    if let Err(e) = res {
        let _ = fs::remove_file(&tmp);
        return Err(Error::io(dest, e));
    }
    Ok(())
}

/// Valide `src` avec `nix-instantiate --parse`. `Ok(false)` si `nix-instantiate` est
/// absent (rien n'a été vérifié), `Ok(true)` si Nix a accepté le fichier.
pub fn validate_with_nix(src: &str) -> Result<bool> {
    let dir = tempfile::tempdir().map_err(|e| Error::io(std::env::temp_dir(), e))?;
    let file = dir.path().join("check.nix");
    fs::write(&file, src).map_err(|e| Error::io(&file, e))?;
    let mut cmd = Command::new("nix-instantiate");
    cmd.env("PATH", path_with_wrappers(std::env::var("PATH").ok()))
        .arg("--parse")
        .arg(&file)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    match cmd.output() {
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(source) => Err(Error::Spawn {
            program: "nix-instantiate".into(),
            source,
        }),
        Ok(o) if o.status.success() => Ok(true),
        Ok(o) => Err(Error::Parse(
            String::from_utf8_lossy(&o.stderr).trim().to_string(),
        )),
    }
}

/// `PATH` complété par `/run/wrappers/bin` (comme `nixos-flakes-core`).
fn path_with_wrappers(path: Option<String>) -> String {
    const W: &str = "/run/wrappers/bin";
    match path {
        Some(p) if p.split(':').any(|e| e == W) => p,
        Some(p) if !p.is_empty() => format!("{p}:{W}"),
        _ => W.to_string(),
    }
}
