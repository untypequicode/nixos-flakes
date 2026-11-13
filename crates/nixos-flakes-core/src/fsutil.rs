//! Utilitaires de fichiers (privé à la bibliothèque).

use crate::error::{Error, Result};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{
    chownat, fstat, openat, statat, AtFlags, Dir, FileType, Gid, Mode, OFlags, Uid, CWD,
};
use std::ffi::{CStr, CString};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Écrit `bytes` dans `path` de façon atomique : fichier temporaire unique dans
/// le même dossier (création exclusive), `fsync`, puis `rename` par-dessus
/// `path`. Un lecteur voit donc l'ancien ou le nouveau contenu, jamais un
/// fichier tronqué. Droits : `mode` s'il est fourni, sinon ceux du fichier
/// existant, sinon 0644. Le dossier parent doit exister.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8], mode: Option<u32>) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| Error::io(path, std::io::Error::other("nom de fichier absent")))?
        .to_string_lossy();
    let mode = mode
        .or_else(|| fs::metadata(path).ok().map(|m| m.mode() & 0o7777))
        .unwrap_or(0o644);
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let res = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&tmp)?;
        // Indépendant du umask.
        f.set_permissions(fs::Permissions::from_mode(mode))?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if let Err(e) = res {
        let _ = fs::remove_file(&tmp);
        return Err(Error::io(path, e));
    }
    // Rend le renommage durable ; best effort.
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Vrai si le processus tourne en root.
pub(crate) fn is_root() -> bool {
    fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0)
}

/// Rend à leur propriétaire d'origine les fichiers qu'un processus root crée dans
/// un dépôt appartenant à un autre utilisateur (`flake.lock`, `.git/*`) : sans
/// cela l'utilisateur ne pourrait plus commiter. Inactif hors root. Le
/// rétablissement a lieu à la destruction du garde (même en cas d'erreur).
pub(crate) struct OwnerGuard {
    /// (dossier parent, nom, propriétaire d'origine).
    lock: Option<(PathBuf, &'static str, (u32, u32))>,
    git: Option<(PathBuf, &'static str, (u32, u32))>,
}

fn owner_of(p: &Path) -> Option<(u32, u32)> {
    fs::symlink_metadata(p).ok().map(|m| (m.uid(), m.gid()))
}

impl OwnerGuard {
    pub(crate) fn new(repo: &Path) -> Self {
        if !is_root() {
            return OwnerGuard {
                lock: None,
                git: None,
            };
        }
        let canon = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
        let lock_owner = owner_of(&canon.join("flake.lock")).or_else(|| owner_of(&canon));
        let git = canon
            .ancestors()
            .find(|a| a.join(".git").is_dir())
            .and_then(|a| owner_of(&a.join(".git")).map(|o| (a.to_path_buf(), ".git", o)));
        OwnerGuard {
            lock: lock_owner.map(|o| (canon, "flake.lock", o)),
            git,
        }
    }
}

/// Chown injectable : reçoit le descripteur du dossier parent, le nom de l'entrée et la
/// propriété voulue. L'implémentation réelle ne suit jamais de lien.
type ChownFn<'a> = &'a mut dyn FnMut(BorrowedFd<'_>, &CStr, (u32, u32));

fn chown_nofollow(dir: BorrowedFd<'_>, name: &CStr, owner: (u32, u32)) {
    let _ = chownat(
        dir,
        name,
        Some(Uid::from_raw(owner.0)),
        Some(Gid::from_raw(owner.1)),
        AtFlags::SYMLINK_NOFOLLOW,
    );
}

/// Ouvre un dossier sans suivre de lien (relatif à `parent`, sinon chemin absolu).
fn open_dir_nofollow(parent: Option<BorrowedFd<'_>>, name: &CStr) -> Option<OwnedFd> {
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::DIRECTORY | OFlags::CLOEXEC;
    openat(parent.unwrap_or(CWD), name, flags, Mode::empty()).ok()
}

/// Rend `owner` à l'entrée `name` de `parent` et, si `recurse`, à tout son contenu. Parcours
/// sans jamais suivre de lien : chaque dossier est ouvert avec `O_NOFOLLOW|O_DIRECTORY` relatif
/// au descripteur de son parent, les entrées sont lues avec `fstatat(AT_SYMLINK_NOFOLLOW)`.
/// Seuls les fichiers réguliers (à un seul lien dur) et les dossiers dont le propriétaire
/// diffère sont modifiés ; liens, sockets, etc. sont ignorés. Les points de montage
/// (`st_dev` différent de `dev`) ne sont pas traversés.
fn restore_entry(
    parent: BorrowedFd<'_>,
    name: &CStr,
    owner: (u32, u32),
    dev: Option<u64>,
    recurse: bool,
    chown: ChownFn<'_>,
) {
    let Ok(st) = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) else {
        return;
    };
    // `st_dev` (alias selon l'architecture) est comparé à celui du parent.
    let st_dev = st.st_dev;
    if dev.is_some_and(|d| d != st_dev) {
        return;
    }
    let differs = (st.st_uid, st.st_gid) != owner;
    match FileType::from_raw_mode(st.st_mode) {
        FileType::RegularFile => {
            if differs && st.st_nlink <= 1 {
                chown(parent, name, owner);
            }
        }
        FileType::Directory => {
            if recurse {
                if let Some(fd) = open_dir_nofollow(Some(parent), name) {
                    // Le dossier ouvert doit être celui qu'on vient d'examiner.
                    let same =
                        fstat(&fd).is_ok_and(|s2| s2.st_ino == st.st_ino && s2.st_dev == st_dev);
                    if same {
                        if let Ok(dir) = Dir::read_from(&fd) {
                            for e in dir.flatten() {
                                let n = e.file_name();
                                if n.to_bytes() != b"." && n.to_bytes() != b".." {
                                    restore_entry(fd.as_fd(), n, owner, Some(st_dev), true, chown);
                                }
                            }
                        }
                    }
                }
            }
            if differs {
                chown(parent, name, owner);
            }
        }
        _ => {}
    }
}

/// Comme `restore_entry` pour `parent_path/name` ; `parent_path` est ouvert avec `O_NOFOLLOW`.
fn restore_at(
    parent_path: &Path,
    name: &str,
    owner: (u32, u32),
    recurse: bool,
    chown: ChownFn<'_>,
) {
    let (Ok(p), Ok(n)) = (
        CString::new(parent_path.as_os_str().as_bytes()),
        CString::new(name),
    ) else {
        return;
    };
    let Some(pfd) = open_dir_nofollow(None, &p) else {
        return;
    };
    let dev = fstat(&pfd).ok().map(|s| s.st_dev);
    restore_entry(pfd.as_fd(), &n, owner, dev, recurse, chown);
}

impl Drop for OwnerGuard {
    fn drop(&mut self) {
        let mut real = |d: BorrowedFd<'_>, n: &CStr, o: (u32, u32)| chown_nofollow(d, n, o);
        if let Some((p, n, o)) = &self.lock {
            restore_at(p, n, *o, false, &mut real);
        }
        if let Some((p, n, o)) = &self.git {
            restore_at(p, n, *o, true, &mut real);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_content_without_leftovers() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f.toml");
        write_atomic(&p, b"un", None).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"un");
        assert_eq!(fs::metadata(&p).unwrap().mode() & 0o777, 0o644);
        write_atomic(&p, b"deux", Some(0o600)).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"deux");
        assert_eq!(fs::metadata(&p).unwrap().mode() & 0o777, 0o600);
        // Sans mode explicite : celui du fichier existant est conservé.
        write_atomic(&p, b"trois", None).unwrap();
        assert_eq!(fs::metadata(&p).unwrap().mode() & 0o777, 0o600);
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn owner_guard_is_harmless_on_own_files() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join(".git")).unwrap();
        fs::write(d.path().join("flake.lock"), "{}").unwrap();
        let me = owner_of(d.path()).unwrap();
        {
            let _g = OwnerGuard::new(d.path());
        }
        // Chown vers soi-même : sans effet, rien ne casse.
        restore_at(d.path(), ".git", me, true, &mut |d, n, o| {
            chown_nofollow(d, n, o)
        });
        assert_eq!(owner_of(&d.path().join("flake.lock")), Some(me));
    }

    #[test]
    fn failure_leaves_original_and_no_tmp() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        write_atomic(&p, b"ok", None).unwrap();
        // Destination = dossier non vide : le rename échoue.
        let sub = d.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("x"), "").unwrap();
        assert!(write_atomic(&sub, b"x", None).is_err());
        assert_eq!(fs::read(&p).unwrap(), b"ok");
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 2);
        assert!(write_atomic(&d.path().join("absent/f"), b"x", None).is_err());
    }
}

#[cfg(test)]
mod walk_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn symlinks_are_neither_followed_nor_chowned() {
        let d = tempfile::tempdir().unwrap();
        let outside = d.path().join("dehors");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("secret"), "x").unwrap();
        let tree = d.path().join("arbre");
        fs::create_dir_all(tree.join("sub")).unwrap();
        fs::write(tree.join("a"), "").unwrap();
        fs::write(tree.join("sub/b"), "").unwrap();
        symlink(&outside, tree.join("lien_dossier")).unwrap();
        symlink(outside.join("secret"), tree.join("lien_fichier")).unwrap();
        symlink("/etc/shadow", tree.join("sub/lien_shadow")).unwrap();

        // Propriétaire voulu différent du réel : tout fichier/dossier régulier est « à rendre ».
        let me = owner_of(d.path()).unwrap();
        let want = (me.0 + 1, me.1);
        let mut seen: Vec<String> = Vec::new();
        restore_at(d.path(), "arbre", want, true, &mut |_, n, o| {
            assert_eq!(o, want);
            seen.push(n.to_string_lossy().into_owned());
        });
        seen.sort();
        assert_eq!(seen, ["a", "arbre", "b", "sub"]);
    }

    #[test]
    fn root_symlink_and_hardlinks_are_skipped() {
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("reel");
        fs::create_dir(&real).unwrap();
        fs::write(real.join("f"), "").unwrap();
        symlink(&real, d.path().join(".git")).unwrap();
        fs::hard_link(real.join("f"), d.path().join("dur")).unwrap();
        let me = owner_of(d.path()).unwrap();
        let want = (me.0 + 1, me.1);
        let mut seen = Vec::new();
        let mut rec = |_: BorrowedFd<'_>, n: &CStr, _: (u32, u32)| {
            seen.push(n.to_string_lossy().into_owned())
        };
        restore_at(d.path(), ".git", want, true, &mut rec);
        restore_at(d.path(), "dur", want, false, &mut rec);
        assert!(seen.is_empty(), "{seen:?}");
    }
}
