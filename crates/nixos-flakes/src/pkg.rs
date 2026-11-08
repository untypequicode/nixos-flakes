//! Sous-commandes `pkg` : recherche de paquets et édition des Paquets déclarés.

use crate::cli::{PkgCommand, WriteOpts};
use crate::commands::{format_table, resolve_repo};
use nixos_flakes_config as cfg;
use nixos_flakes_core::config::Config;
use nixos_flakes_core::nix::{self, Privilege};
use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Exécute une sous-commande `pkg`.
pub fn run(command: PkgCommand, config: &Config) -> Result<ExitCode, String> {
    match command {
        PkgCommand::Search { query, limit } => search(&query, limit),
        PkgCommand::List { repo } => list(&resolve_repo(config, repo.as_deref())),
        PkgCommand::Add {
            name,
            file,
            location,
            opts,
        } => {
            let repo = resolve_repo(config, opts.repo.as_deref());
            let changes = plan_add(&repo, &file, &location, &name)?;
            commit(changes, &opts)
        }
        PkgCommand::Remove {
            name,
            file,
            location,
            opts,
        } => {
            let repo = resolve_repo(config, opts.repo.as_deref());
            let changes = plan_remove(&repo, &file, &location, &name)?;
            commit(changes, &opts)
        }
        PkgCommand::Move {
            name,
            from,
            to,
            opts,
        } => {
            let repo = resolve_repo(config, opts.repo.as_deref());
            let changes = plan_move(&repo, &from, &to, &name)?;
            commit(changes, &opts)
        }
    }
}

fn search(query: &str, limit: usize) -> Result<ExitCode, String> {
    let hits = cfg::search(query, limit).map_err(|e| e.to_string())?;
    if hits.is_empty() {
        println!("Aucun paquet trouvé pour « {query} ».");
        return Ok(ExitCode::SUCCESS);
    }
    if hits.iter().any(|h| h.from_cache) {
        eprintln!("attention : sources injoignables, résultats du cache hors ligne.");
    }
    let rows: Vec<Vec<String>> = hits
        .iter()
        .map(|h| {
            vec![
                h.attr.clone(),
                h.version.clone(),
                h.license.clone().unwrap_or_default(),
                h.description.clone(),
            ]
        })
        .collect();
    print!(
        "{}",
        format_table(&["Paquet", "Version", "Licence", "Description"], &rows)
    );
    Ok(ExitCode::SUCCESS)
}

/// Fichiers `.nix` du dépôt (hors dossiers cachés, `target`, `result`), triés.
fn nix_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(ty) = e.file_type() else { continue };
        if ty.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "result" {
                nix_files(&p, out);
            }
        } else if ty.is_file() && p.extension().is_some_and(|x| x == "nix") {
            out.push(p);
        }
    }
    out.sort();
}

/// Texte de `pkg list` pour `repo`.
fn render_list(repo: &Path) -> String {
    let mut files = Vec::new();
    nix_files(repo, &mut files);
    let mut out = String::new();
    for path in files {
        let Ok(src) = fs::read_to_string(&path) else {
            continue;
        };
        let locs = cfg::list_locations(&src);
        if locs.is_empty() {
            continue;
        }
        let rel = path.strip_prefix(repo).unwrap_or(&path);
        out.push_str(&format!("{}\n", rel.display()));
        for l in locs {
            match &l.read_only {
                Some(why) => out.push_str(&format!("  {}  [lecture seule : {why}]\n", l.id)),
                None => out.push_str(&format!("  {}\n", l.id)),
            }
            for e in l.entries.iter().filter(|e| !e.is_comment_only) {
                out.push_str(&format!("    {}\n", e.name));
            }
        }
    }
    out
}

fn list(repo: &Path) -> Result<ExitCode, String> {
    if !repo.is_dir() {
        return Err(format!("{} n'est pas un dossier", repo.display()));
    }
    let text = render_list(repo);
    if text.is_empty() {
        println!(
            "Aucun Emplacement de paquets trouvé dans {}.",
            repo.display()
        );
    } else {
        print!("{text}");
    }
    Ok(ExitCode::SUCCESS)
}

/// Un Fichier de config à réécrire.
#[derive(Debug)]
struct Change {
    path: PathBuf,
    rel: String,
    old: String,
    new: String,
}

/// Chemin absolu de `file` (relatif au dépôt), qui doit rester dans le dépôt.
fn resolve_file(repo: &Path, file: &Path) -> Result<PathBuf, String> {
    let root = fs::canonicalize(repo).map_err(|e| format!("{} : {e}", repo.display()))?;
    let full = if file.is_absolute() {
        file.to_path_buf()
    } else {
        root.join(file)
    };
    let real = fs::canonicalize(&full).map_err(|e| format!("{} : {e}", full.display()))?;
    if real.starts_with(&root) {
        Ok(real)
    } else {
        Err(format!(
            "{} est en dehors du dépôt {}",
            file.display(),
            repo.display()
        ))
    }
}

fn load(repo: &Path, file: &Path) -> Result<Change, String> {
    let path = resolve_file(repo, file)?;
    let old = fs::read_to_string(&path).map_err(|e| format!("{} : {e}", path.display()))?;
    let root = fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    let rel = path
        .strip_prefix(&root)
        .unwrap_or(&path)
        .display()
        .to_string();
    Ok(Change {
        path,
        rel,
        new: old.clone(),
        old,
    })
}

fn plan_add(repo: &Path, file: &Path, location: &str, name: &str) -> Result<Vec<Change>, String> {
    let mut c = load(repo, file)?;
    c.new = cfg::add_package(&c.old, location, name).map_err(|e| e.to_string())?;
    Ok(vec![c])
}

fn plan_remove(
    repo: &Path,
    file: &Path,
    location: &str,
    name: &str,
) -> Result<Vec<Change>, String> {
    let mut c = load(repo, file)?;
    c.new = cfg::remove_package(&c.old, location, name).map_err(|e| e.to_string())?;
    Ok(vec![c])
}

/// Découpe `FICHIER:ID`.
fn split_target(s: &str) -> Result<(PathBuf, String), String> {
    match s.split_once(':') {
        Some((f, id)) if !f.is_empty() && !id.is_empty() => Ok((PathBuf::from(f), id.to_string())),
        _ => Err(format!("« {s} » : format attendu FICHIER:ID")),
    }
}

fn plan_move(repo: &Path, from: &str, to: &str, name: &str) -> Result<Vec<Change>, String> {
    let (ff, fid) = split_target(from)?;
    let (tf, tid) = split_target(to)?;
    let mut a = load(repo, &ff)?;
    if resolve_file(repo, &tf)? == a.path {
        let removed = cfg::remove_package(&a.old, &fid, name).map_err(|e| e.to_string())?;
        a.new = cfg::add_package(&removed, &tid, name).map_err(|e| e.to_string())?;
        return Ok(vec![a]);
    }
    let mut b = load(repo, &tf)?;
    let (na, nb) =
        cfg::move_package(&a.old, &fid, &b.old, &tid, name).map_err(|e| e.to_string())?;
    a.new = na;
    b.new = nb;
    Ok(vec![a, b])
}

/// Vrai si la réponse est affirmative (o, oui, y, yes).
fn is_yes(answer: &str) -> bool {
    matches!(
        answer.trim().to_lowercase().as_str(),
        "o" | "oui" | "y" | "yes"
    )
}

fn confirm(input: &mut dyn BufRead) -> bool {
    eprint!("Écrire ces changements ? [o/N] ");
    let mut line = String::new();
    input.read_line(&mut line).is_ok() && is_yes(&line)
}

/// Écrit `c`, avec élévation de privilèges si le dossier n'est pas modifiable.
fn write_change(path: &Path, content: &str, privilege: Privilege) -> Result<(), String> {
    match cfg::write_atomic(path, content) {
        Ok(()) => Ok(()),
        Err(cfg::Error::Io { source, .. })
            if source.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            eprintln!(
                "{} n'est pas modifiable : écriture avec élévation de privilèges.",
                path.display()
            );
            let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
            let staged = tmp.path().join("fichier.nix");
            fs::write(&staged, content).map_err(|e| e.to_string())?;
            nix::install_file(&staged, path, privilege, &mut |l| eprintln!("{l}"))
                .map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Écrit dans l'ordre inverse (destination d'un déplacement avant la source) ; si une
/// écriture échoue, restaure les fichiers déjà écrits.
fn write_all_with(
    changes: &[Change],
    write: &mut dyn FnMut(&Change, &str) -> Result<(), String>,
) -> Result<(), String> {
    let mut done: Vec<&Change> = Vec::new();
    for c in changes.iter().rev() {
        if let Err(e) = write(c, &c.new) {
            for d in done.iter().rev() {
                if let Err(e2) = write(d, &d.old) {
                    return Err(format!(
                        "{} : {e} ; restauration de {} impossible : {e2}",
                        c.rel, d.rel
                    ));
                }
            }
            return Err(format!("{} : {e}", c.rel));
        }
        println!("{} écrit.", c.rel);
        done.push(c);
    }
    Ok(())
}

/// Affiche les diffs, valide, confirme puis écrit.
fn commit(changes: Vec<Change>, opts: &WriteOpts) -> Result<ExitCode, String> {
    commit_with(changes, opts, &mut std::io::stdin().lock())
}

fn commit_with(
    changes: Vec<Change>,
    opts: &WriteOpts,
    input: &mut dyn BufRead,
) -> Result<ExitCode, String> {
    let changes: Vec<Change> = changes.into_iter().filter(|c| c.old != c.new).collect();
    if changes.is_empty() {
        println!("Aucun changement.");
        return Ok(ExitCode::SUCCESS);
    }
    for c in &changes {
        cfg::validate(&c.new).map_err(|e| format!("{} : {e}", c.rel))?;
        cfg::validate_with_nix(&c.new).map_err(|e| format!("{} : {e}", c.rel))?;
        println!("=== {} ===", c.rel);
        print!("{}", cfg::diff(&c.old, &c.new));
    }
    if opts.dry_run {
        println!("Simulation (--dry-run) : rien n'a été écrit.");
        return Ok(ExitCode::SUCCESS);
    }
    if !opts.yes && !confirm(input) {
        println!("Annulé : rien n'a été écrit.");
        return Ok(ExitCode::FAILURE);
    }
    let privilege = if opts.sudo {
        Privilege::Sudo
    } else {
        Privilege::Pkexec
    };
    for c in &changes {
        let now = fs::read_to_string(&c.path).map_err(|e| format!("{} : {e}", c.rel))?;
        if now != c.old {
            return Err(format!(
                "{} a changé depuis l'aperçu : rien n'a été écrit, recommencez.",
                c.rel
            ));
        }
    }
    write_all_with(&changes, &mut |c, content| {
        write_change(&c.path, content, privilege)
    })?;
    println!("Rien n'est appliqué : lancez « nixos-flakes apply » pour activer.");
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "{ pkgs, ... }:\n{\n  environment.systemPackages = with pkgs; [\n    git\n    vim\n  ];\n}\n";
    const B: &str = "{ pkgs, ... }:\n{\n  home.packages = with pkgs; [\n    htop\n  ];\n}\n";

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("modules")).unwrap();
        fs::write(d.path().join("a.nix"), A).unwrap();
        fs::write(d.path().join("modules/b.nix"), B).unwrap();
        fs::write(d.path().join("notes.txt"), "x").unwrap();
        d
    }

    fn yes() -> WriteOpts {
        WriteOpts {
            yes: true,
            ..WriteOpts::default()
        }
    }

    #[test]
    fn list_shows_files_locations_and_packages() {
        let d = repo();
        let t = render_list(d.path());
        assert!(t.contains("a.nix\n  environment.systemPackages#0\n    git\n    vim\n"));
        assert!(t.contains("modules/b.nix\n  home.packages#0\n    htop\n"));
    }

    #[test]
    fn add_writes_file_with_yes() {
        let d = repo();
        let ch = plan_add(
            d.path(),
            Path::new("a.nix"),
            "environment.systemPackages#0",
            "ripgrep",
        )
        .unwrap();
        assert!(commit_with(ch, &yes(), &mut "".as_bytes()).is_ok());
        let s = fs::read_to_string(d.path().join("a.nix")).unwrap();
        assert!(s.contains("    vim\n    ripgrep\n"));
    }

    #[test]
    fn dry_run_and_refusal_do_not_write() {
        let d = repo();
        let mk = || {
            plan_add(
                d.path(),
                Path::new("a.nix"),
                "environment.systemPackages#0",
                "fd",
            )
            .unwrap()
        };
        let dry = WriteOpts {
            dry_run: true,
            yes: true,
            ..WriteOpts::default()
        };
        commit_with(mk(), &dry, &mut "".as_bytes()).unwrap();
        let code = commit_with(mk(), &WriteOpts::default(), &mut "n\n".as_bytes()).unwrap();
        assert_eq!(code, ExitCode::FAILURE);
        assert_eq!(fs::read_to_string(d.path().join("a.nix")).unwrap(), A);
        commit_with(mk(), &WriteOpts::default(), &mut "oui\n".as_bytes()).unwrap();
        assert!(fs::read_to_string(d.path().join("a.nix"))
            .unwrap()
            .contains("fd"));
    }

    #[test]
    fn remove_and_move_across_files() {
        let d = repo();
        let ch = plan_remove(
            d.path(),
            Path::new("a.nix"),
            "environment.systemPackages#0",
            "vim",
        )
        .unwrap();
        commit_with(ch, &yes(), &mut "".as_bytes()).unwrap();
        assert!(!fs::read_to_string(d.path().join("a.nix"))
            .unwrap()
            .contains("vim"));

        let ch = plan_move(
            d.path(),
            "a.nix:environment.systemPackages#0",
            "modules/b.nix:home.packages#0",
            "git",
        )
        .unwrap();
        assert_eq!(ch.len(), 2);
        commit_with(ch, &yes(), &mut "".as_bytes()).unwrap();
        assert!(!fs::read_to_string(d.path().join("a.nix"))
            .unwrap()
            .contains("git"));
        assert!(fs::read_to_string(d.path().join("modules/b.nix"))
            .unwrap()
            .contains("git"));
    }

    #[test]
    fn move_write_failure_restores_first_file() {
        let d = repo();
        let ch = plan_move(
            d.path(),
            "a.nix:environment.systemPackages#0",
            "modules/b.nix:home.packages#0",
            "git",
        )
        .unwrap();
        let mut order = Vec::new();
        let r = write_all_with(&ch, &mut |c, content| {
            order.push((c.rel.clone(), content == c.old));
            if c.rel == "a.nix" && content == c.new {
                Err("boom".into())
            } else {
                Ok(())
            }
        });
        assert!(r.is_err());
        assert_eq!(order.len(), 3);
        assert_eq!(order[0], ("modules/b.nix".to_string(), false));
        assert_eq!(order[2], ("modules/b.nix".to_string(), true));
    }

    #[test]
    fn commit_refuses_when_file_changed_since_preview() {
        let d = repo();
        let ch = plan_add(
            d.path(),
            Path::new("a.nix"),
            "environment.systemPackages#0",
            "fd",
        )
        .unwrap();
        let edited = format!("{A}# edit\n");
        fs::write(d.path().join("a.nix"), &edited).unwrap();
        assert!(commit_with(ch, &yes(), &mut "".as_bytes()).is_err());
        assert_eq!(fs::read_to_string(d.path().join("a.nix")).unwrap(), edited);
    }

    #[test]
    fn move_within_one_file() {
        let d = repo();
        let src = "{ pkgs, ... }:\n{\n  environment.systemPackages = with pkgs; [ git ];\n  users.users.u.packages = with pkgs; [ vim ];\n}\n";
        fs::write(d.path().join("c.nix"), src).unwrap();
        let ch = plan_move(
            d.path(),
            "c.nix:environment.systemPackages#0",
            "c.nix:users.users.u.packages#0",
            "git",
        )
        .unwrap();
        assert_eq!(ch.len(), 1);
        assert!(ch[0].new.contains("[ vim git ]"));
    }

    #[test]
    fn errors_are_reported() {
        let d = repo();
        let r = d.path();
        assert!(plan_add(r, Path::new("a.nix"), "nope#0", "x").is_err());
        assert!(plan_add(r, Path::new("a.nix"), "environment.systemPackages#0", "git").is_err());
        assert!(plan_add(r, Path::new("../x.nix"), "x#0", "x").is_err());
        assert!(plan_move(r, "a.nix", "b.nix:x#0", "x").is_err());
    }

    #[test]
    fn rejects_files_outside_repo() {
        let d = repo();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("o.nix"), A).unwrap();
        assert!(resolve_file(d.path(), &outside.path().join("o.nix")).is_err());
    }

    #[test]
    fn yes_answers() {
        assert!(is_yes("O\n") && is_yes("yes") && is_yes(" oui "));
        assert!(!is_yes("") && !is_yes("n"));
    }
}
