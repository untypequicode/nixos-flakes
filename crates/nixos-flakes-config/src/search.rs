//! Recherche de paquets nixpkgs : search.nixos.org (via `curl`), repli sur
//! `nix search`, puis cache disque quand tout échoue.

use crate::error::{Error, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Un paquet trouvé dans nixpkgs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageHit {
    /// Attribut nixpkgs (`ripgrep`, `python3Packages.requests`).
    pub attr: String,
    /// Nom (pname).
    pub name: String,
    pub version: String,
    pub description: String,
    pub license: Option<String>,
    /// Vrai si le résultat vient du cache hors ligne.
    pub from_cache: bool,
}

/// Recherche `query` ; au plus `limit` résultats.
/// Ordre : search.nixos.org, `nix search nixpkgs`, cache disque.
pub fn search(query: &str, limit: usize) -> Result<Vec<PackageHit>> {
    search_with("curl", "nix", &cache_dir(), query, limit)
}

const SITE: &str = "https://search.nixos.org";
const DEFAULT_SCHEMA: &str = "53";
const DEFAULT_USER: &str = "aWVSALXpZv";
const DEFAULT_PASS: &str = "X8gPHnzL52wFEekuxsfQ9cSh";
const CHANNEL: &str = "nixos-unstable";

/// Point d'accès Elasticsearch de search.nixos.org.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Backend {
    url: String,
    user: String,
    pass: String,
}

/// Seul endroit qui connaît l'index et les identifiants : ils sont publics dans le
/// JavaScript du site et changent à chaque version ; on les relit à l'exécution et
/// on retombe sur les dernières valeurs connues si la lecture échoue.
fn resolve_backend(curl: &str) -> Backend {
    let found = (|| {
        let html = fetch(curl, &format!("{SITE}/"))?;
        let script = script_path(&html)?;
        let js = fetch(curl, &format!("{SITE}{script}"))?;
        parse_bundle(&js)
    })();
    let (schema, user, pass) = found.unwrap_or_else(|| {
        (
            DEFAULT_SCHEMA.into(),
            DEFAULT_USER.into(),
            DEFAULT_PASS.into(),
        )
    });
    Backend {
        url: format!("{SITE}/backend/latest-{schema}-{CHANNEL}/_search"),
        user,
        pass,
    }
}

fn fetch(curl: &str, url: &str) -> Option<String> {
    run(curl, &["-fsSL", "-m", "15", url])
}

fn script_path(html: &str) -> Option<String> {
    let mut rest = html;
    while let Some(i) = rest.find("src=\"") {
        rest = &rest[i + 5..];
        let end = rest.find('"')?;
        let src = &rest[..end];
        if src.starts_with("/static/js/index.") && src.ends_with(".js") {
            return Some(src.to_string());
        }
    }
    None
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let j = s[i..].find(end)? + i;
    Some(&s[i..j])
}

/// (version du schéma, utilisateur, mot de passe) lus dans le bundle JS.
fn parse_bundle(js: &str) -> Option<(String, String, String)> {
    let schema = between(js, "elasticsearchMappingSchemaVersion:parseInt(\"", "\")")?;
    let user = between(js, "elasticsearchUsername:\"", "\"")?;
    let pass = between(js, "elasticsearchPassword:\"", "\"")?;
    if schema.is_empty() || !schema.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((schema.into(), user.into(), pass.into()))
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = command(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn command(program: &str) -> Command {
    const W: &str = "/run/wrappers/bin";
    let path = match std::env::var("PATH") {
        Ok(p) if p.split(':').any(|e| e == W) => p,
        Ok(p) if !p.is_empty() => format!("{p}:{W}"),
        _ => W.to_string(),
    };
    let mut c = Command::new(program);
    c.env("PATH", path);
    c
}

fn query_body(query: &str, limit: usize) -> String {
    serde_json::json!({
        "size": limit,
        "query": {"bool": {
            "filter": [{"term": {"type": "package"}}],
            "must": [{"multi_match": {
                "query": query,
                "fields": ["package_attr_name^3", "package_pname^2", "package_description"],
            }}],
        }},
    })
    .to_string()
}

fn search_nixos_org(curl: &str, query: &str, limit: usize) -> Option<Vec<PackageHit>> {
    let b = resolve_backend(curl);
    let auth = format!("{}:{}", b.user, b.pass);
    let body = query_body(query, limit);
    let out = run(
        curl,
        &[
            "-fsS",
            "-m",
            "20",
            "-u",
            &auth,
            "-H",
            "Content-Type: application/json",
            "-X",
            "POST",
            "-d",
            &body,
            &b.url,
        ],
    )?;
    parse_elastic(&out)
}

/// Lit la réponse `_search` d'Elasticsearch (`hits.hits[]._source`).
fn parse_elastic(json: &str) -> Option<Vec<PackageHit>> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let hits = v.get("hits")?.get("hits")?.as_array()?;
    let text = |s: &serde_json::Value, k: &str| {
        s.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    Some(
        hits.iter()
            .filter_map(|h| {
                let s = h.get("_source")?;
                let attr = text(s, "package_attr_name");
                if attr.is_empty() {
                    return None;
                }
                let licenses: Vec<&str> = s
                    .get("package_license")
                    .and_then(|l| l.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|l| {
                                l.get("spdxId")
                                    .or_else(|| l.get("fullName"))
                                    .and_then(|x| x.as_str())
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Some(PackageHit {
                    name: text(s, "package_pname"),
                    version: text(s, "package_pversion"),
                    description: text(s, "package_description"),
                    license: (!licenses.is_empty()).then(|| licenses.join(", ")),
                    attr,
                    from_cache: false,
                })
            })
            .collect(),
    )
}

/// Arguments de `nix search` ; `--` empêche un terme commençant par `-` d'être pris pour une option.
fn nix_search_args(query: &str) -> Vec<&str> {
    vec![
        "--extra-experimental-features",
        "nix-command flakes",
        "search",
        "--json",
        "nixpkgs",
        "--",
        query,
    ]
}

fn search_nix(nix: &str, query: &str, limit: usize) -> Option<Vec<PackageHit>> {
    let out = run(nix, &nix_search_args(query))?;
    let mut hits = parse_nix_search(&out)?;
    hits.truncate(limit);
    Some(hits)
}

/// Lit la sortie de `nix search --json` (attribut -> {pname, version, description}).
fn parse_nix_search(json: &str) -> Option<Vec<PackageHit>> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let map = v.as_object()?;
    let mut hits: Vec<PackageHit> = map
        .iter()
        .map(|(key, s)| {
            let text = |k: &str| {
                s.get(k)
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            // `legacyPackages.<système>.<attr>` : on retire les deux premiers segments.
            let attr = match key.splitn(3, '.').collect::<Vec<_>>().as_slice() {
                ["legacyPackages" | "packages", _, rest] => (*rest).to_string(),
                _ => key.clone(),
            };
            PackageHit {
                attr,
                name: text("pname"),
                version: text("version"),
                description: text("description"),
                license: None,
                from_cache: false,
            }
        })
        .collect();
    hits.sort_by(|a, b| a.attr.cmp(&b.attr));
    Some(hits)
}

/// `$XDG_CACHE_HOME/nixos-flakes/search` (repli sur `~/.cache`).
fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".cache"))
        })
        .unwrap_or_else(|| PathBuf::from(".cache"));
    base.join("nixos-flakes").join("search")
}

fn cache_file(dir: &Path, query: &str) -> PathBuf {
    let slug: String = query
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    let hash = query.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    dir.join(format!("{slug}-{hash:016x}.json"))
}

fn write_cache(dir: &Path, query: &str, hits: &[PackageHit]) {
    let arr: Vec<_> = hits
        .iter()
        .map(|h| {
            serde_json::json!({
                "attr": h.attr, "name": h.name, "version": h.version,
                "description": h.description, "license": h.license,
            })
        })
        .collect();
    // Le cache est un confort : une écriture ratée n'est pas une erreur.
    if fs::create_dir_all(dir).is_ok() {
        let _ = fs::write(
            cache_file(dir, query),
            serde_json::Value::Array(arr).to_string(),
        );
    }
}

fn read_cache(dir: &Path, query: &str, limit: usize) -> Option<Vec<PackageHit>> {
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(cache_file(dir, query)).ok()?).ok()?;
    let text = |s: &serde_json::Value, k: &str| {
        s.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    Some(
        v.as_array()?
            .iter()
            .take(limit)
            .map(|s| PackageHit {
                attr: text(s, "attr"),
                name: text(s, "name"),
                version: text(s, "version"),
                description: text(s, "description"),
                license: s.get("license").and_then(|x| x.as_str()).map(String::from),
                from_cache: true,
            })
            .collect(),
    )
}

fn search_with(
    curl: &str,
    nix: &str,
    cache: &Path,
    query: &str,
    limit: usize,
) -> Result<Vec<PackageHit>> {
    let live = search_nixos_org(curl, query, limit).or_else(|| search_nix(nix, query, limit));
    if let Some(hits) = live {
        write_cache(cache, query, &hits);
        return Ok(hits);
    }
    read_cache(cache, query, limit).ok_or_else(|| {
        Error::Search(
            "ni search.nixos.org ni « nix search » n'ont répondu, et aucun résultat en cache"
                .into(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn nix_search_query_follows_double_dash() {
        let a = nix_search_args("--option");
        let n = a.len();
        assert_eq!(&a[n - 2..], &["--", "--option"]);
    }

    const ELASTIC: &str = r#"{"hits":{"total":{"value":2},"hits":[
      {"_source":{"type":"package","package_attr_name":"ripgrep","package_pname":"ripgrep",
       "package_pversion":"15.2.0","package_description":"Fast grep",
       "package_license":[{"spdxId":"Unlicense","fullName":"The Unlicense"},{"fullName":"MIT License"}]}},
      {"_source":{"type":"package","package_attr_name":"ripgrep-all","package_pname":"ripgrep-all",
       "package_pversion":"0.10","package_description":null}}]}}"#;

    const NIX_JSON: &str = r#"{
      "legacyPackages.x86_64-linux.hello":{"pname":"hello","version":"2.12","description":"Salut"},
      "legacyPackages.x86_64-linux.python3Packages.requests":{"pname":"requests","version":"2","description":"HTTP"}}"#;

    const BUNDLE: &str = r#"x{flags:{elasticsearchMappingSchemaVersion:parseInt("77"),elasticsearchUrl:"/backend",elasticsearchUsername:"usr",elasticsearchPassword:"pw"}}"#;

    #[test]
    fn parses_elastic_fixture() {
        let h = parse_elastic(ELASTIC).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].attr, "ripgrep");
        assert_eq!(h[0].version, "15.2.0");
        assert_eq!(h[0].license.as_deref(), Some("Unlicense, MIT License"));
        assert_eq!(h[1].description, "");
        assert_eq!(h[1].license, None);
        assert!(!h[0].from_cache);
        assert!(parse_elastic("pas du json").is_none());
    }

    #[test]
    fn parses_nix_search_and_strips_prefix() {
        let h = parse_nix_search(NIX_JSON).unwrap();
        assert_eq!(h[0].attr, "hello");
        assert_eq!(h[1].attr, "python3Packages.requests");
        assert_eq!(h[1].name, "requests");
    }

    #[test]
    fn bundle_and_html_parsing() {
        assert_eq!(
            parse_bundle(BUNDLE),
            Some(("77".into(), "usr".into(), "pw".into()))
        );
        assert_eq!(parse_bundle("rien"), None);
        let html =
            r#"<script src="/static/js/779.a.js"></script><script src="/static/js/index.b.js">"#;
        assert_eq!(script_path(html).as_deref(), Some("/static/js/index.b.js"));
    }

    #[test]
    fn query_body_filters_packages() {
        let v: serde_json::Value = serde_json::from_str(&query_body("git", 5)).unwrap();
        assert_eq!(v["size"], 5);
        assert_eq!(v["query"]["bool"]["filter"][0]["term"]["type"], "package");
        assert_eq!(v["query"]["bool"]["must"][0]["multi_match"]["query"], "git");
    }

    fn script(dir: &Path, name: &str, body: &str) -> String {
        let p = dir.join(name);
        fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        p.to_string_lossy().into_owned()
    }

    fn fake_curl(dir: &Path) -> String {
        fs::write(dir.join("elastic.json"), ELASTIC).unwrap();
        fs::write(dir.join("bundle.js"), BUNDLE).unwrap();
        script(
            dir,
            "fake-curl",
            &format!(
                r#"for a; do url="$a"; done
case "$url" in
  */_search) echo "$url" > "{d}/seen-url"; cat "{d}/elastic.json";;
  *.js) cat "{d}/bundle.js";;
  *) echo '<script src="/static/js/index.x.js"></script>';;
esac"#,
                d = dir.display()
            ),
        )
    }

    fn fake_nix(dir: &Path) -> String {
        fs::write(dir.join("nix.json"), NIX_JSON).unwrap();
        script(
            dir,
            "fake-nix",
            &format!("cat \"{}/nix.json\"", dir.display()),
        )
    }

    #[test]
    fn primary_source_discovers_index_and_writes_cache() {
        let d = tempfile::tempdir().unwrap();
        let cache = d.path().join("cache");
        let curl = fake_curl(d.path());
        let h = search_with(&curl, "/nonexistent/nix", &cache, "ripgrep", 10).unwrap();
        assert_eq!(h[0].attr, "ripgrep");
        let url = fs::read_to_string(d.path().join("seen-url")).unwrap();
        assert!(url.contains("/backend/latest-77-nixos-unstable/_search"));
        let cached = read_cache(&cache, "ripgrep", 10).unwrap();
        assert_eq!(cached.len(), 2);
        assert!(cached.iter().all(|c| c.from_cache));
    }

    #[test]
    fn falls_back_to_nix_search() {
        let d = tempfile::tempdir().unwrap();
        let curl = script(d.path(), "bad-curl", "exit 22");
        let nix = fake_nix(d.path());
        let h = search_with(&curl, &nix, &d.path().join("c"), "hello", 1).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].attr, "hello");
        assert!(!h[0].from_cache);
    }

    #[test]
    fn uses_cache_when_everything_fails_then_errors_without_it() {
        let d = tempfile::tempdir().unwrap();
        let cache = d.path().join("c");
        let curl = script(d.path(), "bad-curl", "exit 6");
        let nix = script(d.path(), "bad-nix", "exit 1");
        assert!(matches!(
            search_with(&curl, &nix, &cache, "hello", 5),
            Err(Error::Search(_))
        ));
        write_cache(&cache, "hello", &parse_nix_search(NIX_JSON).unwrap());
        let h = search_with(&curl, &nix, &cache, "hello", 5).unwrap();
        assert_eq!(h.len(), 2);
        assert!(h.iter().all(|x| x.from_cache));
        assert!(search_with(&curl, &nix, &cache, "autre", 5).is_err());
    }
}
