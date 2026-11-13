//! Découpage d'une ligne de commande en arguments, façon shell (guillemets, échappements).

/// Découpe `line` : espaces séparateurs, `'…'` littéral, `"…"` avec `\"` et `\\`,
/// `\x` hors guillemets pour échapper un caractère. `None` si un guillemet reste ouvert
/// ou si la ligne se termine par un `\`.
pub fn split(line: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        ch => cur.push(ch),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => match chars.next()? {
                            ch @ ('"' | '\\') => cur.push(ch),
                            other => {
                                cur.push('\\');
                                cur.push(other);
                            }
                        },
                        ch => cur.push(ch),
                    }
                }
            }
            '\\' => {
                in_word = true;
                cur.push(chars.next()?);
            }
            c if c.is_whitespace() => {
                if in_word {
                    args.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        args.push(cur);
    }
    Some(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(line: &str) -> Option<Vec<String>> {
        split(line)
    }

    #[test]
    fn plain_words() {
        assert_eq!(s("a  b\tc"), Some(vec!["a".into(), "b".into(), "c".into()]));
        assert_eq!(s("   "), Some(vec![]));
        assert_eq!(s(""), Some(vec![]));
    }

    #[test]
    fn quotes_and_escapes() {
        assert_eq!(
            s(r#"gui --titre "Mon appli" 'a b'"#),
            Some(vec![
                "gui".into(),
                "--titre".into(),
                "Mon appli".into(),
                "a b".into()
            ])
        );
        assert_eq!(s(r#""a \"b\" \\""#), Some(vec![r#"a "b" \"#.into()]));
        assert_eq!(s(r"a\ b"), Some(vec!["a b".into()]));
        assert_eq!(s(r#"x"y z"w"#), Some(vec!["xy zw".into()]));
        assert_eq!(s("''"), Some(vec!["".into()]));
    }

    #[test]
    fn unterminated_is_rejected() {
        assert_eq!(s("a 'b"), None);
        assert_eq!(s("a \"b"), None);
        assert_eq!(s("a\\"), None);
    }
}
