//! Recherche des dépôts flake sous les racines de scan.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const SKIPPED: &[&str] = &[".git", "target", "node_modules", ".direnv"];

fn skipped(name: &str) -> bool {
    SKIPPED.contains(&name) || name.starts_with("result")
}

/// Dossiers contenant un `flake.nix`, triés, dédoublonnés et canonicalisés.
///
/// Les racines sont à la profondeur 0 ; on descend jusqu'à `max_depth`.
/// Les liens symboliques de dossiers ne sont pas suivis (sauf les racines) et
/// `/nix` est ignoré. On continue la descente dans un dépôt flake trouvé.
pub fn scan(roots: &[PathBuf], max_depth: u8) -> Vec<PathBuf> {
    let mut found = BTreeSet::new();
    for root in roots {
        if let Ok(root) = root.canonicalize() {
            walk(&root, 0, max_depth, &mut found);
        }
    }
    found.into_iter().collect()
}

fn walk(dir: &Path, depth: u8, max_depth: u8, found: &mut BTreeSet<PathBuf>) {
    if dir == Path::new("/nix") {
        return;
    }
    if dir.join("flake.nix").is_file() {
        if let Ok(c) = dir.canonicalize() {
            found.insert(c);
        }
    }
    if depth >= max_depth {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() || ft.is_symlink() {
            continue;
        }
        if skipped(&entry.file_name().to_string_lossy()) {
            continue;
        }
        walk(&entry.path(), depth + 1, max_depth, found);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn flake(p: &Path) {
        fs::create_dir_all(p).unwrap();
        fs::write(p.join("flake.nix"), "{}").unwrap();
    }

    #[test]
    fn finds_sorted_deduped_canonical() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        flake(&root.join("b"));
        flake(&root.join("a"));
        flake(&root.join("deep/er/c"));
        let r = scan(&[root.clone(), root.join("a"), root.clone()], 4);
        assert_eq!(
            r,
            vec![root.join("a"), root.join("b"), root.join("deep/er/c")]
        );
    }

    #[test]
    fn respects_depth() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        flake(&root.join("x/y/z"));
        assert!(scan(std::slice::from_ref(&root), 2).is_empty());
        assert_eq!(
            scan(std::slice::from_ref(&root), 3),
            vec![root.join("x/y/z")]
        );
    }

    #[test]
    fn root_itself_and_nested_flakes() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        fs::write(root.join("flake.nix"), "{}").unwrap();
        flake(&root.join("sub"));
        assert_eq!(
            scan(std::slice::from_ref(&root), 1),
            vec![root.clone(), root.join("sub")]
        );
        assert_eq!(scan(std::slice::from_ref(&root), 0), vec![root]);
    }

    #[test]
    fn skips_noise_dirs() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        for n in [
            ".git",
            "target",
            "node_modules",
            ".direnv",
            "result",
            "result-bin",
        ] {
            flake(&root.join(n).join("f"));
        }
        flake(&root.join("ok"));
        assert_eq!(scan(std::slice::from_ref(&root), 4), vec![root.join("ok")]);
    }

    #[test]
    fn does_not_follow_symlinks() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        flake(&root.join("real/proj"));
        fs::create_dir(root.join("scan")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("scan/link")).unwrap();
        // boucle
        std::os::unix::fs::symlink(&root, root.join("scan/loop")).unwrap();
        assert!(scan(&[root.join("scan")], 5).is_empty());
    }

    #[test]
    fn missing_root_is_ignored() {
        assert!(scan(&[PathBuf::from("/nonexistent-nixos-flakes")], 3).is_empty());
    }
}
