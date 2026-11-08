//! Parcourt une vraie config si `NIXOS_FLAKES_FIXTURE_CONFIG` pointe vers un dossier.

use nixos_flakes_config::{add_package, list_locations, remove_package, validate};
use std::path::{Path, PathBuf};

fn nix_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if !matches!(
                p.file_name().and_then(|n| n.to_str()),
                Some(".git" | "target")
            ) {
                nix_files(&p, out);
            }
        } else if p.extension().is_some_and(|x| x == "nix") {
            out.push(p);
        }
    }
}

#[test]
fn real_config_roundtrips() {
    let Ok(dir) = std::env::var("NIXOS_FLAKES_FIXTURE_CONFIG") else {
        eprintln!("NIXOS_FLAKES_FIXTURE_CONFIG absent : test ignoré");
        return;
    };
    let mut files = Vec::new();
    nix_files(Path::new(&dir), &mut files);
    files.sort();
    let (mut n_loc, mut n_ro, mut n_rt) = (0, 0, 0);
    for f in &files {
        let Ok(src) = std::fs::read_to_string(f) else {
            continue;
        };
        let valid = validate(&src).is_ok();
        let locs = list_locations(&src);
        if !valid {
            continue;
        }
        for l in &locs {
            n_loc += 1;
            if l.is_read_only() {
                n_ro += 1;
                continue;
            }
            let probe = "zz-nixos-flakes-probe";
            let added = add_package(&src, &l.id, probe)
                .unwrap_or_else(|e| panic!("{} {} : ajout : {e}", f.display(), l.id));
            validate(&added).unwrap_or_else(|e| panic!("{} {} : {e}", f.display(), l.id));
            let back = remove_package(&added, &l.id, probe)
                .unwrap_or_else(|e| panic!("{} {} : retrait : {e}", f.display(), l.id));
            assert_eq!(
                back,
                src,
                "aller-retour différent : {} {}",
                f.display(),
                l.id
            );
            n_rt += 1;
        }
    }
    eprintln!(
        "{} fichiers, {n_loc} emplacements ({n_ro} en lecture seule), {n_rt} aller-retours",
        files.len()
    );
}
