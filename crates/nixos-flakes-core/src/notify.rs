//! Notifications de bureau.

use crate::lock::InputChange;
use crate::proc;
use std::process::Stdio;

/// Vrai si `new` contient des mises à jour qu'on ne connaissait pas dans `old`.
pub fn has_new_updates(old: Option<&[InputChange]>, new: &[InputChange]) -> bool {
    new.iter().any(|n| {
        !old.unwrap_or(&[])
            .iter()
            .any(|o| o.name == n.name && o.new_rev == n.new_rev)
    })
}

/// Titre et corps de la notification « mises à jour disponibles » (`repos` : nom, nombre).
pub fn updates_text(repos: &[(String, usize)]) -> (String, String) {
    let total: usize = repos.iter().map(|(_, n)| n).sum();
    let title = if total == 1 {
        "1 mise à jour disponible".to_string()
    } else {
        format!("{total} mises à jour disponibles")
    };
    let body = repos
        .iter()
        .map(|(name, n)| format!("{name} : {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    (title, body)
}

/// Arguments passés à `notify-send`.
fn args<'a>(title: &'a str, body: &'a str, critical: bool) -> [&'a str; 7] {
    let urgency = if critical { "critical" } else { "normal" };
    ["-a", "nixos-flakes", "-u", urgency, "--", title, body]
}

/// Envoie une notification avec `notify-send` ; les erreurs sont ignorées.
pub fn desktop(title: &str, body: &str, critical: bool) {
    let _ = proc::command("notify-send")
        .args(args(title, body, critical))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::{args, has_new_updates, updates_text};
    use crate::lock::InputChange;

    fn change(name: &str, rev: &str) -> InputChange {
        InputChange {
            name: name.into(),
            old_rev: None,
            new_rev: Some(rev.into()),
            old_date: None,
            new_date: None,
        }
    }

    #[test]
    fn new_updates_detection() {
        let old = [change("n", "1")];
        assert!(!has_new_updates(Some(&old), &[change("n", "1")]));
        assert!(has_new_updates(Some(&old), &[change("n", "2")]));
        assert!(has_new_updates(None, &[change("n", "1")]));
        assert!(!has_new_updates(None, &[]));
    }

    #[test]
    fn updates_wording() {
        let (t, b) = updates_text(&[("nixos".into(), 1)]);
        assert_eq!(t, "1 mise à jour disponible");
        assert_eq!(b, "nixos : 1");
        let (t, _) = updates_text(&[("a".into(), 2), ("b".into(), 1)]);
        assert_eq!(t, "3 mises à jour disponibles");
    }

    #[test]
    fn urgency_follows_flag() {
        assert_eq!(args("t", "b", true)[3], "critical");
        assert_eq!(args("t", "b", false)[3], "normal");
    }

    #[test]
    fn title_and_body_come_after_separator() {
        let a = args("-titre", "corps", false);
        assert_eq!(&a[4..], ["--", "-titre", "corps"]);
    }
}
