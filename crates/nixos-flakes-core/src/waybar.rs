//! Sortie JSON pour Waybar (`{"text","tooltip","class"}`).

use crate::error::Error;
use crate::lock::InputChange;
use crate::nix::UpdateReport;
use serde_json::json;

/// Date `AAAA-MM-JJ` (UTC) d'un horodatage Unix.
fn date(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    // algorithme « civil_from_days » de Howard Hinnant
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

fn line(c: &InputChange) -> String {
    let d = |t: Option<i64>| t.map_or_else(|| "?".to_string(), date);
    match (&c.old_rev, &c.new_rev) {
        (None, Some(_)) => format!("{}  (nouveau)", c.name),
        (Some(_), None) => format!("{}  (retiré)", c.name),
        _ => format!("{}  {} -> {}", c.name, d(c.old_date), d(c.new_date)),
    }
}

/// JSON compatible Waybar. Classes : `uptodate`, `outdated`, `error`.
pub fn waybar_json(report: Result<&UpdateReport, &Error>) -> String {
    let v = match report {
        Err(e) => json!({
            "text": "!",
            "tooltip": format!("Erreur pendant nix flake update\n{e}"),
            "class": "error",
        }),
        Ok(r) if r.changes.is_empty() => json!({
            "text": "",
            "tooltip": "Tout est à jour",
            "class": "uptodate",
        }),
        Ok(r) => json!({
            "text": format!("❆ {}", r.changes.len()),
            "tooltip": r.changes.iter().map(line).collect::<Vec<_>>().join("\n"),
            "class": "outdated",
        }),
    };
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn report(changes: Vec<InputChange>) -> UpdateReport {
        UpdateReport {
            repo: PathBuf::from("/r"),
            changes,
        }
    }

    fn parse(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn dates() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_709_164_800), "2024-02-29");
        assert_eq!(date(1_700_000_000), "2023-11-14");
        assert_eq!(date(-86_400), "1969-12-31");
    }

    #[test]
    fn uptodate() {
        let v = parse(&waybar_json(Ok(&report(vec![]))));
        assert_eq!(v["class"], "uptodate");
        assert_eq!(v["text"], "");
        assert_eq!(v["tooltip"], "Tout est à jour");
    }

    #[test]
    fn outdated() {
        let r = report(vec![
            InputChange {
                name: "nixpkgs".into(),
                old_rev: Some("a".into()),
                new_rev: Some("b".into()),
                old_date: Some(0),
                new_date: Some(1_709_164_800),
            },
            InputChange {
                name: "neuf".into(),
                old_rev: None,
                new_rev: Some("c".into()),
                old_date: None,
                new_date: Some(5),
            },
        ]);
        let s = waybar_json(Ok(&r));
        assert!(!s.contains('\n'));
        let v = parse(&s);
        assert_eq!(v["class"], "outdated");
        assert_eq!(v["text"], "❆ 2");
        assert_eq!(
            v["tooltip"],
            "nixpkgs  1970-01-01 -> 2024-02-29\nneuf  (nouveau)"
        );
    }

    #[test]
    fn error() {
        let e = Error::Lock("x".into());
        let v = parse(&waybar_json(Err(&e)));
        assert_eq!(v["class"], "error");
        assert_eq!(v["text"], "!");
        assert!(v["tooltip"]
            .as_str()
            .unwrap()
            .starts_with("Erreur pendant nix flake update"));
    }
}
