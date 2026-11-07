//! Utilitaires partagés par les tests.

use serde_json::Value;
use std::path::Path;
use std::process::Command;

/// `flake.lock` de test avec les inputs `(nom, rev, lastModified)`.
pub(crate) fn lock_json(inputs: &[(&str, &str, i64)]) -> String {
    let mut nodes = serde_json::Map::new();
    let root_inputs: serde_json::Map<String, Value> = inputs
        .iter()
        .map(|(n, _, _)| (n.to_string(), Value::String(n.to_string())))
        .collect();
    nodes.insert("root".into(), serde_json::json!({ "inputs": root_inputs }));
    for (n, rev, t) in inputs {
        nodes.insert(
            n.to_string(),
            serde_json::json!({
                "locked": { "type": "github", "rev": rev, "lastModified": t },
                "original": { "type": "github" }
            }),
        );
    }
    serde_json::json!({ "nodes": nodes, "root": "root", "version": 7 }).to_string()
}

/// Initialise un dépôt git de test (identité locale, branche `main`).
pub(crate) fn init_repo(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "user.email", "test@example.org"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

pub(crate) fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?} : {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).into_owned()
}
