//! Écriture sur disque et validation externe (seules parties avec E/S).

use crate::error::{Error, Result};
use crate::run::{command, output_with_timeout, RunError};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::Duration;

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
    let res = (|| -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.set_permissions(fs::Permissions::from_mode(mode))?;
        f.sync_all()?;
        fs::rename(&tmp, &dest)?;
        // Rend le renommage durable (fsync du dossier). Le fichier est déjà en place :
        // certains systèmes de fichiers refusent le fsync de dossier, on ne l'échoue pas.
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if let Err(e) = res {
        let _ = fs::remove_file(&tmp);
        return Err(Error::io(dest, e));
    }
    Ok(())
}

/// Valide `src` avec `nix-instantiate --parse` (délai de 30 s). `Ok(false)` si
/// `nix-instantiate` est absent (rien n'a été vérifié), `Ok(true)` si Nix a accepté le fichier.
pub fn validate_with_nix(src: &str) -> Result<bool> {
    let dir = tempfile::tempdir().map_err(|e| Error::io(std::env::temp_dir(), e))?;
    let file = dir.path().join("check.nix");
    fs::write(&file, src).map_err(|e| Error::io(&file, e))?;
    let mut cmd = command("nix-instantiate");
    cmd.arg("--parse").arg(&file);
    match output_with_timeout(cmd, Duration::from_secs(30)) {
        Err(RunError::NotFound) => Ok(false),
        Err(RunError::Timeout) => Err(Error::Timeout {
            program: "nix-instantiate".into(),
            secs: 30,
        }),
        Err(RunError::Io(source)) => Err(Error::Spawn {
            program: "nix-instantiate".into(),
            source,
        }),
        Ok(o) if o.status.success() => Ok(true),
        Ok(o) => Err(Error::Parse(
            String::from_utf8_lossy(&o.stderr).trim().to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_replaces_content_and_leaves_no_temp_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.nix");
        write_atomic(&f, "1").unwrap();
        write_atomic(&f, "2").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "2");
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
    }
}
