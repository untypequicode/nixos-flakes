//! Ajout, retrait et déplacement de Paquets déclarés, en préservant la mise en forme.

use crate::error::{Error, Result};
use crate::locate::{list_locations, Entry, Location};
use crate::validate;

fn find(src: &str, id: &str) -> Result<Location> {
    list_locations(src)
        .into_iter()
        .find(|l| l.id == id)
        .ok_or_else(|| Error::LocationNotFound(id.to_string()))
}

fn writable(loc: &Location) -> Result<()> {
    match &loc.read_only {
        Some(why) => Err(Error::Unsupported(format!(
            "{} est en lecture seule : {why}",
            loc.id
        ))),
        None => Ok(()),
    }
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '\'' | '.'))
        && !name.ends_with('.')
        && !name.contains("..")
}

fn line_start(src: &str, pos: usize) -> usize {
    src[..pos].rfind('\n').map_or(0, |i| i + 1)
}

fn leading_ws(src: &str, pos: usize) -> &str {
    let s = line_start(src, pos);
    let rest = &src[s..];
    let n = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    &rest[..n]
}

/// Ajoute `name` à l'Emplacement `location_id`, à la fin de la liste, dans le style
/// de la liste (une entrée par ligne ou en ligne). Erreur si déjà présent.
pub fn add_package(src: &str, location_id: &str, name: &str) -> Result<String> {
    let loc = find(src, location_id)?;
    writable(&loc)?;
    if !valid_name(name) {
        return Err(Error::Unsupported(format!(
            "« {name} » n'est pas un nom de paquet littéral"
        )));
    }
    if loc
        .entries
        .iter()
        .any(|e| !e.is_comment_only && e.name == name)
    {
        return Err(Error::Duplicate {
            name: name.to_string(),
            location: loc.id,
        });
    }
    let (start, end) = (loc.range.start, loc.range.end);
    let real: Option<&Entry> = loc.entries.iter().rev().find(|e| !e.is_comment_only);
    let any: Option<&Entry> = loc.entries.last();
    let mut out = src.to_string();
    if src[start..end].contains('\n') {
        let own_line = |e: &Entry| line_start(src, e.range.start) > start;
        let indent = match real.filter(|e| own_line(e)).or(any.filter(|e| own_line(e))) {
            Some(e) => leading_ws(src, e.range.start).to_string(),
            None => format!("{}  ", leading_ws(src, start)),
        };
        let pos = match real.or(any) {
            Some(a) => match src[a.range.end..].find('\n').map(|i| i + a.range.end) {
                Some(nl) if nl < end => nl,
                _ => a.range.end,
            },
            None => start + 1,
        };
        out.insert_str(pos, &format!("\n{indent}{name}"));
    } else if let Some(a) = real.or(any) {
        out.insert_str(a.range.end, &format!(" {name}"));
    } else {
        out.replace_range(start + 1..end - 1, &format!(" {name} "));
    }
    validate(&out)?;
    Ok(out)
}

/// Retire l'entrée `name` de l'Emplacement `location_id`, sans toucher aux commentaires
/// voisins. Erreur si absente, ou si l'entrée n'est pas un paquet littéral.
pub fn remove_package(src: &str, location_id: &str, name: &str) -> Result<String> {
    let loc = find(src, location_id)?;
    writable(&loc)?;
    let entry = loc
        .entries
        .iter()
        .find(|e| !e.is_comment_only && e.name == name)
        .ok_or_else(|| Error::NotFound {
            name: name.to_string(),
            location: loc.id.clone(),
        })?;
    if !entry.simple {
        return Err(Error::Unsupported(format!(
            "l'entrée « {name} » n'est pas un nom de paquet littéral"
        )));
    }
    let (s, e) = (entry.range.start, entry.range.end);
    let ls = line_start(src, s);
    let hws = |c: char| c == ' ' || c == '\t';
    let after_ws = src[e..].len() - src[e..].trim_start_matches(hws).len();
    let next = src[e + after_ws..].chars().next();
    let alone_before = src[ls..s].chars().all(hws) && ls > loc.range.start;
    let (from, to) = match next {
        Some('\n') | Some('\r') if alone_before => {
            let nl = src[e..].find('\n').map_or(src.len(), |i| e + i + 1);
            (ls, nl)
        }
        Some(']') if alone_before => (ls - 1, e),
        None | Some('\n') | Some('\r') | Some(']') => {
            let before = src[..s].len() - src[..s].trim_end_matches(hws).len();
            (s - before, e)
        }
        _ => (s, e + after_ws),
    };
    let mut out = src.to_string();
    out.replace_range(from..to, "");
    validate(&out)?;
    Ok(out)
}

/// Déplace `name` de l'Emplacement `loc_a` du fichier `src_a` vers `loc_b` du fichier
/// `src_b`. Renvoie les deux nouveaux contenus ; rien n'est écrit si l'un des deux échoue.
pub fn move_package(
    src_a: &str,
    loc_a: &str,
    src_b: &str,
    loc_b: &str,
    name: &str,
) -> Result<(String, String)> {
    let a = remove_package(src_a, loc_a, name)?;
    let b = add_package(src_b, loc_b, name)?;
    Ok((a, b))
}
