use crate::*;

fn names(l: &Location) -> Vec<&str> {
    l.entries
        .iter()
        .filter(|e| !e.is_comment_only)
        .map(|e| e.name.as_str())
        .collect()
}

const PLAIN: &str = "{ pkgs, ... }: {\n  environment.systemPackages = with pkgs; [\n    git\n    # vim\n    curl # transfert\n  ];\n}\n";

#[test]
fn plain_with_list() {
    let v = list_locations(PLAIN);
    assert_eq!(v.len(), 1);
    let l = &v[0];
    assert_eq!(l.id, "environment.systemPackages#0");
    assert_eq!(l.scope.as_deref(), Some("pkgs"));
    assert_eq!(names(l), ["git", "curl"]);
    assert!(l
        .entries
        .iter()
        .any(|e| e.is_comment_only && e.name == "# vim"));
    assert!(l.entries.iter().all(|e| e.is_comment_only || e.simple));
    assert!(PLAIN[l.range.clone()].starts_with('[') && PLAIN[l.range.clone()].ends_with(']'));
}

#[test]
fn add_multiline_keeps_style_and_comments() {
    let out = add_package(PLAIN, "environment.systemPackages#0", "htop").unwrap();
    assert_eq!(
        out,
        "{ pkgs, ... }: {\n  environment.systemPackages = with pkgs; [\n    git\n    # vim\n    curl # transfert\n    htop\n  ];\n}\n"
    );
    assert_eq!(
        remove_package(&out, "environment.systemPackages#0", "htop").unwrap(),
        PLAIN
    );
}

#[test]
fn duplicate_and_missing() {
    let id = "environment.systemPackages#0";
    assert!(matches!(
        add_package(PLAIN, id, "git"),
        Err(Error::Duplicate { .. })
    ));
    // un paquet seulement commenté n'est pas un doublon
    assert!(add_package(PLAIN, id, "vim").is_ok());
    assert!(matches!(
        remove_package(PLAIN, id, "vim"),
        Err(Error::NotFound { .. })
    ));
    assert!(matches!(
        add_package(PLAIN, "x#0", "a"),
        Err(Error::LocationNotFound(_))
    ));
    assert!(matches!(
        add_package(PLAIN, id, "a b"),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn remove_keeps_trailing_comment() {
    let out = remove_package(PLAIN, "environment.systemPackages#0", "curl").unwrap();
    assert!(out.contains("    # vim\n    # transfert\n  ];"));
    let out = remove_package(PLAIN, "environment.systemPackages#0", "git").unwrap();
    assert!(!out.contains("git") && out.contains("# vim") && out.contains("curl # transfert"));
}

#[test]
fn inline_list() {
    let s = "{ home.packages = with pkgs; [ a b ]; }";
    let id = "home.packages#0";
    let out = add_package(s, id, "c").unwrap();
    assert_eq!(out, "{ home.packages = with pkgs; [ a b c ]; }");
    assert_eq!(remove_package(&out, id, "c").unwrap(), s);
    assert_eq!(
        remove_package(s, id, "a").unwrap(),
        "{ home.packages = with pkgs; [ b ]; }"
    );
    let e = "{ home.packages = [ ]; }";
    assert_eq!(
        add_package(e, id, "a").unwrap(),
        "{ home.packages = [ a ]; }"
    );
}

#[test]
fn chained_lists_have_scopes() {
    let s = "{ environment.systemPackages = with pkgs; [ a ] ++ (with pkgs-unstable; [ b ]) ++ (with inputs.llm-agents.packages.${pkgs.stdenv.hostPlatform.system}; [ c ]); }";
    let v = list_locations(s);
    assert_eq!(v.len(), 3);
    assert_eq!(v[0].id, "environment.systemPackages#0");
    assert_eq!(v[0].scope.as_deref(), Some("pkgs"));
    assert_eq!(v[1].scope.as_deref(), Some("pkgs-unstable"));
    assert_eq!(
        v[2].scope.as_deref(),
        Some("inputs.llm-agents.packages.${pkgs.stdenv.hostPlatform.system}")
    );
    let out = add_package(s, "environment.systemPackages#1", "z").unwrap();
    assert!(out.contains("[ b z ]") && out.contains("[ a ]"));
}

#[test]
fn comment_only_list() {
    let s = "{\n  home.packages = with pkgs; [\n    # rien pour l'instant\n  ];\n}\n";
    let l = &list_locations(s)[0];
    assert!(l.entries.len() == 1 && l.entries[0].is_comment_only);
    let out = add_package(s, &l.id, "git").unwrap();
    assert_eq!(
        out,
        "{\n  home.packages = with pkgs; [\n    # rien pour l'instant\n    git\n  ];\n}\n"
    );
    assert_eq!(remove_package(&out, &l.id, "git").unwrap(), s);
    let empty = "{\n  home.packages = [\n  ];\n}\n";
    let out = add_package(empty, "home.packages#0", "git").unwrap();
    assert_eq!(out, "{\n  home.packages = [\n    git\n  ];\n}\n");
}

#[test]
fn optionals_and_mkif() {
    let s = "{ environment.systemPackages = lib.optionals cond (with pkgs; [ a ]); }";
    let v = list_locations(s);
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].scope.as_deref(), Some("pkgs"));
    assert!(add_package(s, &v[0].id, "b").unwrap().contains("[ a b ]"));

    let s = "{ lib, ... }: lib.mkIf cfg.enable { environment.systemPackages = [ a ] ++ extra; }";
    let v = list_locations(s);
    assert_eq!(v.len(), 2);
    assert!(!v[0].is_read_only() && v[1].is_read_only());
    assert!(add_package(s, &v[0].id, "b").is_ok());
    assert!(matches!(
        add_package(s, &v[1].id, "b"),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn nested_attrsets_and_users() {
    let s = "{ environment = { systemPackages = [ a ]; }; users.users.bob.packages = [ b ]; home-manager.users.bob.home.packages = [ c ]; }";
    let ids: Vec<_> = list_locations(s).into_iter().map(|l| l.id).collect();
    assert_eq!(
        ids,
        [
            "environment.systemPackages#0",
            "users.users.bob.packages#0",
            "home-manager.users.bob.home.packages#0"
        ]
    );
}

#[test]
fn generated_lists_are_read_only() {
    for s in [
        "{ environment.systemPackages = map f apps; }",
        "{ environment.systemPackages = if c then [ a ] else [ b ]; }",
        "{ environment.systemPackages = let x = [ a ]; in x; }",
        "{ environment.systemPackages = pkgs.lib.concatMap f xs; }",
    ] {
        let v = list_locations(s);
        assert_eq!(v.len(), 1, "{s}");
        assert!(v[0].is_read_only(), "{s}");
        assert!(matches!(
            add_package(s, &v[0].id, "x"),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            remove_package(s, &v[0].id, "x"),
            Err(Error::Unsupported(_))
        ));
    }
}

#[test]
fn complex_entries_are_refused_for_removal() {
    let s = "{ environment.systemPackages = [ (a.override { x = 1; }) b ]; }";
    let l = &list_locations(s)[0];
    assert!(!l.entries[0].simple && l.entries[1].simple);
    assert!(matches!(
        remove_package(s, &l.id, "(a.override { x = 1; })"),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(
        remove_package(s, &l.id, "b").unwrap(),
        "{ environment.systemPackages = [ (a.override { x = 1; }) ]; }"
    );
}

#[test]
fn scalar_select_entry() {
    let s = "{ environment.systemPackages = [ pkgs.obs-cmd ]; }";
    let l = &list_locations(s)[0];
    assert_eq!(names(l), ["pkgs.obs-cmd"]);
    assert!(l.entries[0].simple);
    let out = add_package(s, &l.id, "pkgs.jq").unwrap();
    assert_eq!(
        out,
        "{ environment.systemPackages = [ pkgs.obs-cmd pkgs.jq ]; }"
    );
}

#[test]
fn move_between_files() {
    let a = "{ environment.systemPackages = with pkgs; [\n  git\n  curl\n]; }";
    let b = "{ home.packages = with pkgs; [\n  jq\n]; }";
    let (a2, b2) = move_package(
        a,
        "environment.systemPackages#0",
        b,
        "home.packages#0",
        "git",
    )
    .unwrap();
    assert_eq!(
        a2,
        "{ environment.systemPackages = with pkgs; [\n  curl\n]; }"
    );
    assert_eq!(b2, "{ home.packages = with pkgs; [\n  jq\n  git\n]; }");
    assert!(move_package(
        a,
        "environment.systemPackages#0",
        b,
        "home.packages#0",
        "jq"
    )
    .is_err());
}

#[test]
fn validate_and_diff() {
    assert!(validate(PLAIN).is_ok());
    assert!(matches!(validate("{ a = ; "), Err(Error::Parse(_))));
    assert!(add_package("{ a = ; ", "x#0", "a").is_err());
    assert_eq!(diff("a\n", "a\n"), "");
    let d = diff("a\nb\n", "a\nc\n");
    assert!(d.contains("-b") && d.contains("+c") && d.contains("@@"));
}

#[test]
fn write_atomic_keeps_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("f.nix");
    write_atomic(&p, "a").unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "a");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    write_atomic(&p, "b").unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "b");
    assert_eq!(
        std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let n = std::fs::read_dir(dir.path()).unwrap().count();
    assert_eq!(n, 1);
}

#[test]
fn nix_validation_skips_or_checks() {
    // Ok(false) si nix-instantiate est absent ; sinon la syntaxe est vérifiée.
    if let Ok(true) = validate_with_nix("{ a = 1; }") {
        assert!(validate_with_nix("{ a = ; ").is_err());
    }
}
