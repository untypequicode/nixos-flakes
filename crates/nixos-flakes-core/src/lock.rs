//! Lecture et comparaison de `flake.lock`.

use crate::error::{Error, Result};
use serde_json::Value;

/// Entrée verrouillée d'un `flake.lock`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedInput {
    pub name: String,
    pub rev: Option<String>,
    pub last_modified: Option<i64>,
}

/// Changement d'un input entre deux verrous. Ajouté : `old_*` à `None` ;
/// retiré : `new_*` à `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputChange {
    pub name: String,
    pub old_rev: Option<String>,
    pub new_rev: Option<String>,
    pub old_date: Option<i64>,
    pub new_date: Option<i64>,
}

/// Analyse un `flake.lock` (nœud `root` ignoré), trié par nom.
pub fn parse_lock(json: &str) -> Result<Vec<LockedInput>> {
    let v: Value = serde_json::from_str(json).map_err(|e| Error::Lock(e.to_string()))?;
    let nodes = v
        .get("nodes")
        .and_then(Value::as_object)
        .ok_or_else(|| Error::Lock("champ « nodes » absent".into()))?;
    let root = v.get("root").and_then(Value::as_str).unwrap_or("root");
    let mut out: Vec<LockedInput> = nodes
        .iter()
        .filter(|(name, _)| name.as_str() != root && name.as_str() != "root")
        .map(|(name, node)| {
            let locked = node.get("locked");
            LockedInput {
                name: name.clone(),
                rev: locked
                    .and_then(|l| l.get("rev"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                last_modified: locked
                    .and_then(|l| l.get("lastModified"))
                    .and_then(Value::as_i64),
            }
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Différences entre deux verrous : changés, ajoutés, retirés (triés par nom).
pub fn diff(old: &[LockedInput], new: &[LockedInput]) -> Vec<InputChange> {
    let mut out = Vec::new();
    for n in new {
        match old.iter().find(|o| o.name == n.name) {
            Some(o) if o.rev == n.rev && o.last_modified == n.last_modified => {}
            Some(o) => out.push(InputChange {
                name: n.name.clone(),
                old_rev: o.rev.clone(),
                new_rev: n.rev.clone(),
                old_date: o.last_modified,
                new_date: n.last_modified,
            }),
            None => out.push(InputChange {
                name: n.name.clone(),
                old_rev: None,
                new_rev: n.rev.clone(),
                old_date: None,
                new_date: n.last_modified,
            }),
        }
    }
    for o in old {
        if !new.iter().any(|n| n.name == o.name) {
            out.push(InputChange {
                name: o.name.clone(),
                old_rev: o.rev.clone(),
                new_rev: None,
                old_date: o.last_modified,
                new_date: None,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Input racine d'un `flake.lock` (ceux que `nix flake update <nom>` accepte).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootInput {
    /// Nom déclaré dans `flake.nix` (clé de `nodes.root.inputs`).
    pub name: String,
    /// Nom du nœud verrouillé correspondant (peut différer : `nixpkgs_2`).
    /// `None` pour un input purement `follows`.
    pub node: Option<String>,
    /// Révision verrouillée complète.
    pub rev: Option<String>,
    /// Révision abrégée (7 caractères).
    pub short_rev: Option<String>,
    pub last_modified: Option<i64>,
    /// Type de source (`github`, `git`, `path`, `tarball`…).
    pub kind: Option<String>,
    pub owner: Option<String>,
    pub repo: Option<String>,
    /// Adresse lisible : `github:owner/repo[/ref]`, ou l'URL/le chemin de la source.
    pub url: Option<String>,
}

fn str_of(v: Option<&Value>, key: &str) -> Option<String> {
    v.and_then(|l| l.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Inputs racine d'un `flake.lock`, triés par nom.
pub fn root_inputs(json: &str) -> Result<Vec<RootInput>> {
    let v: Value = serde_json::from_str(json).map_err(|e| Error::Lock(e.to_string()))?;
    let nodes = v
        .get("nodes")
        .and_then(Value::as_object)
        .ok_or_else(|| Error::Lock("champ « nodes » absent".into()))?;
    let root = v.get("root").and_then(Value::as_str).unwrap_or("root");
    let Some(inputs) = nodes
        .get(root)
        .and_then(|r| r.get("inputs"))
        .and_then(Value::as_object)
    else {
        return Ok(Vec::new());
    };
    let mut out: Vec<RootInput> = inputs
        .iter()
        .map(|(name, target)| {
            let node = target.as_str().map(str::to_string);
            let n = node.as_deref().and_then(|k| nodes.get(k));
            let locked = n.and_then(|n| n.get("locked"));
            let original = n.and_then(|n| n.get("original"));
            let kind = str_of(locked, "type").or_else(|| str_of(original, "type"));
            let rev = str_of(locked, "rev");
            let owner = str_of(locked, "owner").or_else(|| str_of(original, "owner"));
            let repo = str_of(locked, "repo").or_else(|| str_of(original, "repo"));
            let reference = str_of(original, "ref");
            let url = match (kind.as_deref(), &owner, &repo) {
                (Some(k @ ("github" | "gitlab" | "sourcehut")), Some(o), Some(r)) => {
                    Some(match &reference {
                        Some(f) => format!("{k}:{o}/{r}/{f}"),
                        None => format!("{k}:{o}/{r}"),
                    })
                }
                _ => str_of(original, "url").or_else(|| str_of(original, "path")),
            };
            RootInput {
                name: name.clone(),
                node,
                short_rev: rev.as_ref().map(|r| r.chars().take(7).collect()),
                rev,
                last_modified: locked
                    .and_then(|l| l.get("lastModified"))
                    .and_then(Value::as_i64),
                kind,
                owner,
                repo,
                url,
            }
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::lock_json;

    #[test]
    fn parses_and_ignores_root() {
        let p = parse_lock(&lock_json(&[("nixpkgs", "abc", 10), ("home", "def", 20)])).unwrap();
        assert_eq!(
            p,
            vec![
                LockedInput {
                    name: "home".into(),
                    rev: Some("def".into()),
                    last_modified: Some(20)
                },
                LockedInput {
                    name: "nixpkgs".into(),
                    rev: Some("abc".into()),
                    last_modified: Some(10)
                },
            ]
        );
    }

    #[test]
    fn node_without_locked_has_none() {
        let p = parse_lock(r#"{"nodes":{"root":{},"x":{"inputs":{}}},"root":"root"}"#).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].rev, None);
        assert_eq!(p[0].last_modified, None);
    }

    #[test]
    fn invalid_json_is_error() {
        assert!(matches!(parse_lock("nope"), Err(Error::Lock(_))));
        assert!(matches!(parse_lock("{}"), Err(Error::Lock(_))));
    }

    #[test]
    fn diff_changed_added_removed() {
        let old = parse_lock(&lock_json(&[
            ("a", "1", 1),
            ("b", "2", 2),
            ("gone", "9", 9),
        ]))
        .unwrap();
        let new = parse_lock(&lock_json(&[("a", "1", 1), ("b", "3", 5), ("new", "7", 7)])).unwrap();
        let d = diff(&old, &new);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].name, "b");
        assert_eq!(d[0].old_rev.as_deref(), Some("2"));
        assert_eq!(d[0].new_rev.as_deref(), Some("3"));
        assert_eq!(d[0].new_date, Some(5));
        assert_eq!(d[1].name, "gone");
        assert_eq!(d[1].new_rev, None);
        assert_eq!(d[1].old_date, Some(9));
        assert_eq!(d[2].name, "new");
        assert_eq!(d[2].old_rev, None);
        assert_eq!(d[2].new_rev.as_deref(), Some("7"));
    }

    #[test]
    fn root_inputs_lists_only_root_children_with_details() {
        let json = r#"{"nodes":{
          "root":{"inputs":{"nixpkgs":"nixpkgs_2","hm":"home-manager","fl":["hm","flake"]}},
          "nixpkgs_2":{"locked":{"type":"github","owner":"NixOS","repo":"nixpkgs","rev":"abcdef0123456","lastModified":5},
                       "original":{"type":"github","owner":"NixOS","repo":"nixpkgs","ref":"nixos-unstable"}},
          "home-manager":{"locked":{"type":"git","rev":"1234567890","lastModified":6},"original":{"type":"git","url":"https://x/y.git"}},
          "transitif":{"locked":{"type":"github","rev":"z"}}},
          "root":"root","version":7}"#;
        let r = root_inputs(json).unwrap();
        let names: Vec<_> = r.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["fl", "hm", "nixpkgs"]);
        assert_eq!(r[0].node, None);
        assert_eq!(r[1].url.as_deref(), Some("https://x/y.git"));
        assert_eq!(r[2].node.as_deref(), Some("nixpkgs_2"));
        assert_eq!(r[2].short_rev.as_deref(), Some("abcdef0"));
        assert_eq!(
            r[2].url.as_deref(),
            Some("github:NixOS/nixpkgs/nixos-unstable")
        );
        assert_eq!(r[2].owner.as_deref(), Some("NixOS"));
    }

    #[test]
    fn root_inputs_without_inputs_is_empty() {
        assert!(root_inputs(r#"{"nodes":{"root":{}},"root":"root"}"#)
            .unwrap()
            .is_empty());
        assert!(root_inputs("nope").is_err());
    }

    #[test]
    fn diff_identical_is_empty() {
        let a = parse_lock(&lock_json(&[("a", "1", 1)])).unwrap();
        assert!(diff(&a, &a).is_empty());
    }

    #[test]
    fn diff_detects_date_only_change() {
        let a = parse_lock(&lock_json(&[("a", "1", 1)])).unwrap();
        let b = parse_lock(&lock_json(&[("a", "1", 2)])).unwrap();
        assert_eq!(diff(&a, &b).len(), 1);
    }
}
