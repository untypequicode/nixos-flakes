//! Repérage des Emplacements (listes de Paquets déclarés) dans un Fichier de config.

use rnix::{NodeOrToken, SyntaxKind, SyntaxNode, SyntaxToken};
use std::ops::Range;

/// Une entrée d'un Emplacement : un paquet, ou un commentaire (souvent un paquet commenté).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Texte source de l'entrée (pour un commentaire : le commentaire entier).
    pub name: String,
    /// Intervalle d'octets dans le fichier.
    pub range: Range<usize>,
    /// Vrai si l'entrée n'est qu'un commentaire.
    pub is_comment_only: bool,
    /// Vrai pour un identifiant ou un chemin d'attributs littéral (`git`, `pkgs.git`).
    /// Faux pour toute expression (appel, `if`, interpolation…) : non modifiable.
    pub simple: bool,
}

/// Une liste de Paquets déclarés (ou une expression qui en produit une).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// Identifiant `chemin#n` (n : rang parmi les Emplacements de même chemin, ordre du fichier).
    pub id: String,
    /// Chemin d'attributs complet, ex. `environment.systemPackages`.
    pub attr_path: String,
    /// Portée `with`, ex. `pkgs` ou `inputs.x.packages.${..}` ; `None` si aucune.
    pub scope: Option<String>,
    /// Intervalle d'octets de la liste (crochets inclus) ou de l'expression non littérale.
    pub range: Range<usize>,
    /// Entrées, commentaires compris, dans l'ordre du fichier.
    pub entries: Vec<Entry>,
    /// Raison pour laquelle l'Emplacement est en lecture seule ; `None` s'il est modifiable.
    pub read_only: Option<String>,
}

impl Location {
    /// Vrai si l'Emplacement ne peut pas être modifié automatiquement.
    pub fn is_read_only(&self) -> bool {
        self.read_only.is_some()
    }
}

/// Liste tous les Emplacements de `src`, dans l'ordre du fichier.
/// Un fichier invalide donne ce que l'analyse tolérante a pu reconnaître.
pub fn list_locations(src: &str) -> Vec<Location> {
    let root = rnix::Root::parse(src).syntax();
    let mut out: Vec<Location> = Vec::new();
    for node in root.descendants() {
        if node.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
            continue;
        }
        let Some(path) = full_attr_path(&node) else {
            continue;
        };
        if !is_package_path(&path) {
            continue;
        }
        let Some(value) = value_of(&node) else {
            continue;
        };
        let mut found = Vec::new();
        collect(src, &value, None, &mut found);
        for mut l in found {
            let n = out.iter().filter(|o| o.attr_path == path).count();
            l.id = format!("{path}#{n}");
            l.attr_path = path.clone();
            out.push(l);
        }
    }
    out
}

fn is_trivia(k: SyntaxKind) -> bool {
    matches!(k, SyntaxKind::TOKEN_COMMENT | SyntaxKind::TOKEN_WHITESPACE)
}

fn compact(node: &SyntaxNode) -> String {
    node.descendants_with_tokens()
        .filter_map(NodeOrToken::into_token)
        .filter(|t| !is_trivia(t.kind()))
        .map(|t| t.text().to_string())
        .collect()
}

fn full_attr_path(node: &SyntaxNode) -> Option<String> {
    let mut parts = Vec::new();
    for a in node.ancestors() {
        if a.kind() == SyntaxKind::NODE_ATTRPATH_VALUE {
            let ap = a
                .children()
                .find(|c| c.kind() == SyntaxKind::NODE_ATTRPATH)?;
            parts.push(compact(&ap));
        }
    }
    parts.reverse();
    Some(parts.join("."))
}

fn ends_with_path(p: &str, suffix: &str) -> bool {
    p == suffix || p.ends_with(&format!(".{suffix}"))
}

fn is_package_path(p: &str) -> bool {
    ends_with_path(p, "environment.systemPackages")
        || ends_with_path(p, "home.packages")
        || (p.contains("users.users.") && p.ends_with(".packages"))
}

fn value_of(av: &SyntaxNode) -> Option<SyntaxNode> {
    av.children()
        .filter(|c| c.kind() != SyntaxKind::NODE_ATTRPATH)
        .last()
}

fn trimmed(node: &SyntaxNode) -> Range<usize> {
    let toks: Vec<SyntaxToken> = node
        .descendants_with_tokens()
        .filter_map(NodeOrToken::into_token)
        .filter(|t| !is_trivia(t.kind()))
        .collect();
    match (toks.first(), toks.last()) {
        (Some(a), Some(b)) => {
            usize::from(a.text_range().start())..usize::from(b.text_range().end())
        }
        _ => {
            let r = node.text_range();
            usize::from(r.start())..usize::from(r.end())
        }
    }
}

fn raw(
    range: Range<usize>,
    scope: Option<String>,
    entries: Vec<Entry>,
    ro: Option<String>,
) -> Location {
    Location {
        id: String::new(),
        attr_path: String::new(),
        scope,
        range,
        entries,
        read_only: ro,
    }
}

const WRAPPERS: &[&str] = &[
    "optionals",
    "mkIf",
    "mkDefault",
    "mkForce",
    "mkBefore",
    "mkAfter",
    "mkOverride",
    "mkOrder",
];

fn collect(src: &str, expr: &SyntaxNode, scope: Option<String>, out: &mut Vec<Location>) {
    let ro = |why: &str| raw(trimmed(expr), None, Vec::new(), Some(why.to_string()));
    match expr.kind() {
        SyntaxKind::NODE_LIST => out.push(raw(trimmed(expr), scope, entries_of(src, expr), None)),
        SyntaxKind::NODE_WITH => {
            let kids: Vec<_> = expr.children().collect();
            if let (Some(ns), Some(body)) = (kids.first(), kids.last()) {
                if kids.len() >= 2 {
                    collect(src, body, Some(compact(ns)), out);
                    return;
                }
            }
            out.push(ro("`with` incomplet"));
        }
        SyntaxKind::NODE_PAREN => match expr.children().next() {
            Some(inner) => collect(src, &inner, scope, out),
            None => out.push(ro("parenthèses vides")),
        },
        SyntaxKind::NODE_BIN_OP => {
            let concat = expr
                .children_with_tokens()
                .any(|e| e.kind() == SyntaxKind::TOKEN_CONCAT);
            if concat {
                for c in expr.children() {
                    collect(src, &c, scope.clone(), out);
                }
            } else {
                out.push(ro("opération autre que `++`"));
            }
        }
        SyntaxKind::NODE_APPLY => {
            let mut head = expr.clone();
            while head.kind() == SyntaxKind::NODE_APPLY {
                match head.children().next() {
                    Some(h) => head = h,
                    None => break,
                }
            }
            let name = compact(&head);
            let last = name.rsplit('.').next().unwrap_or("");
            let arg = expr.children().last();
            match arg {
                Some(arg) if WRAPPERS.contains(&last) => collect(src, &arg, scope, out),
                Some(arg) if last == "mkMerge" && arg.kind() == SyntaxKind::NODE_LIST => {
                    for c in arg.children() {
                        collect(src, &c, scope.clone(), out);
                    }
                }
                _ => out.push(ro(&format!("liste générée par un appel (`{name}`)"))),
            }
        }
        SyntaxKind::NODE_IF_ELSE => out.push(ro("liste conditionnelle (`if`/`else`)")),
        SyntaxKind::NODE_IDENT | SyntaxKind::NODE_SELECT => {
            out.push(ro("liste définie ailleurs (référence)"));
        }
        SyntaxKind::NODE_LET_IN => out.push(ro("expression `let … in`")),
        _ => out.push(ro("expression non littérale")),
    }
}

fn is_simple(node: &SyntaxNode) -> bool {
    match node.kind() {
        SyntaxKind::NODE_IDENT => true,
        SyntaxKind::NODE_SELECT => node.descendants().all(|d| {
            matches!(
                d.kind(),
                SyntaxKind::NODE_SELECT | SyntaxKind::NODE_IDENT | SyntaxKind::NODE_ATTRPATH
            )
        }),
        _ => false,
    }
}

fn entries_of(src: &str, list: &SyntaxNode) -> Vec<Entry> {
    let mut v = Vec::new();
    for el in list.children_with_tokens() {
        match el {
            NodeOrToken::Node(n) => {
                let range = trimmed(&n);
                v.push(Entry {
                    name: src[range.clone()].to_string(),
                    range,
                    is_comment_only: false,
                    simple: is_simple(&n),
                });
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::TOKEN_COMMENT => {
                let r = t.text_range();
                v.push(Entry {
                    name: t.text().trim_end().to_string(),
                    range: usize::from(r.start())..usize::from(r.end()),
                    is_comment_only: true,
                    simple: false,
                });
            }
            NodeOrToken::Token(_) => {}
        }
    }
    v
}
