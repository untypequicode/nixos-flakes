//! Modèle pur de l'interface : entrées de dépôts, badges, regroupement,
//! machine d'états des dialogues git. Aucune dépendance à iced.

use nixos_flakes_core::config::{Config, Role};
use nixos_flakes_core::git::GitState;
use nixos_flakes_core::lock::{InputChange, RootInput};
use nixos_flakes_core::status::RepoStatus;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// Commande longue en cours sur un dépôt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobKind {
    Check,
    Update,
    Apply,
    Commit,
    CommitLock,
    Push,
}

impl JobKind {
    /// Libellé à l'infinitif nominal (« Vérification »…).
    pub fn label(self) -> &'static str {
        match self {
            JobKind::Check => "Vérification",
            JobKind::Update => "Mise à jour",
            JobKind::Apply => "Application",
            JobKind::Commit => "Commit",
            JobKind::CommitLock => "Commit de flake.lock",
            JobKind::Push => "Push",
        }
    }
}

/// Un dépôt flake tel que l'affiche l'interface.
#[derive(Debug, Clone, PartialEq)]
pub struct RepoEntry {
    pub path: PathBuf,
    pub role: Role,
    pub git: GitState,
    /// `None` = pas encore vérifié.
    pub pending: Option<Vec<InputChange>>,
    pub error: Option<String>,
    pub busy: Option<JobKind>,
    /// Résultat de la dernière mise à jour faite dans cette session.
    pub last_update: Option<Vec<InputChange>>,
}

impl From<RepoStatus> for RepoEntry {
    fn from(s: RepoStatus) -> Self {
        RepoEntry {
            path: s.path,
            role: s.role,
            git: s.git,
            pending: s.pending,
            error: s.error,
            busy: None,
            last_update: None,
        }
    }
}

impl RepoEntry {
    /// Nom court : dernier composant du chemin.
    pub fn name(&self) -> String {
        short_name(&self.path)
    }
}

pub fn short_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Ton d'un badge (la couleur est choisie par le thème).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Success,
    Info,
    Warning,
    Danger,
}

/// État résumé d'un dépôt pour la barre latérale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    Busy(JobKind),
    Error,
    Updates(usize),
    Dirty,
    UpToDate,
    Unchecked,
}

impl Badge {
    pub fn label(self) -> String {
        match self {
            Badge::Busy(k) => format!("{}…", k.label()),
            Badge::Error => "Erreur".into(),
            Badge::Updates(1) => "1 mise à jour".into(),
            Badge::Updates(n) => format!("{n} mises à jour"),
            Badge::Dirty => "Modifs non commitées".into(),
            Badge::UpToDate => "À jour".into(),
            Badge::Unchecked => "Pas vérifié".into(),
        }
    }

    pub fn tone(self) -> Tone {
        match self {
            Badge::Busy(_) => Tone::Info,
            Badge::Error => Tone::Danger,
            Badge::Updates(_) => Tone::Info,
            Badge::Dirty => Tone::Warning,
            Badge::UpToDate => Tone::Success,
            Badge::Unchecked => Tone::Neutral,
        }
    }
}

/// Priorité : occupé > erreur > mises à jour > modifs > à jour > inconnu.
pub fn badge(e: &RepoEntry) -> Badge {
    if let Some(k) = e.busy {
        return Badge::Busy(k);
    }
    if e.error.is_some() {
        return Badge::Error;
    }
    match &e.pending {
        Some(p) if !p.is_empty() => Badge::Updates(p.len()),
        _ if e.git.is_repo && !e.git.dirty.is_empty() => Badge::Dirty,
        Some(_) => Badge::UpToDate,
        None => Badge::Unchecked,
    }
}

pub const ROLE_ORDER: [Role; 4] = [Role::System, Role::Tracked, Role::Detected, Role::Ignored];

pub fn role_title(role: Role) -> &'static str {
    match role {
        Role::System => "Système",
        Role::Tracked => "Suivis",
        Role::Detected => "Détectés",
        Role::Ignored => "Ignorés",
    }
}

pub fn role_singular(role: Role) -> &'static str {
    match role {
        Role::System => "Dépôt système",
        Role::Tracked => "Dépôt suivi",
        Role::Detected => "Dépôt détecté",
        Role::Ignored => "Dépôt ignoré",
    }
}

/// Indices des entrées groupés par rôle (ordre fixe, triés par chemin, groupes vides omis).
pub fn group_entries(entries: &[RepoEntry]) -> Vec<(Role, Vec<usize>)> {
    ROLE_ORDER
        .iter()
        .filter_map(|&role| {
            let mut idx: Vec<usize> = entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.role == role)
                .map(|(i, _)| i)
                .collect();
            idx.sort_by(|&a, &b| entries[a].path.cmp(&entries[b].path));
            (!idx.is_empty()).then_some((role, idx))
        })
        .collect()
}

/// Chemins à afficher : résultat du scan + dépôts connus de la config (hors scan),
/// dédoublonnés après canonicalisation. `exists` filtre les dossiers de la config.
pub fn collect_paths(
    scanned: Vec<PathBuf>,
    cfg: &Config,
    exists: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    let norm = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let mut seen = BTreeMap::new();
    for p in scanned {
        seen.insert(norm(&p), ());
    }
    let known = cfg
        .system_repo
        .iter()
        .chain(cfg.tracked.iter())
        .chain(cfg.ignored.iter());
    for p in known {
        if exists(p) {
            seen.insert(norm(p), ());
        }
    }
    seen.into_keys().collect()
}

/// Fusionne un nouveau scan avec l'état courant : conserve vérifications, erreurs
/// et activité des dépôts déjà connus ; les dépôts disparus sont retirés.
pub fn merge_scan(old: &[RepoEntry], fresh: Vec<RepoStatus>) -> Vec<RepoEntry> {
    fresh
        .into_iter()
        .map(|s| match old.iter().find(|e| e.path == s.path) {
            Some(prev) => RepoEntry {
                path: s.path,
                role: s.role,
                git: s.git,
                pending: prev.pending.clone(),
                error: prev.error.clone(),
                busy: prev.busy,
                last_update: prev.last_update.clone(),
            },
            None => s.into(),
        })
        .collect()
}

/// Dépôts à vérifier pour « Tout vérifier » : système + suivis, non occupés.
pub fn check_all_targets(entries: &[RepoEntry]) -> Vec<PathBuf> {
    let mut v: Vec<&RepoEntry> = entries
        .iter()
        .filter(|e| matches!(e.role, Role::System | Role::Tracked) && e.busy.is_none())
        .collect();
    v.sort_by_key(|e| (e.role != Role::System, e.path.clone()));
    v.into_iter().map(|e| e.path.clone()).collect()
}

/// Prochain dépôt de la file qui n'est pas occupé (les occupés sont abandonnés).
pub fn pick_next(queue: &mut VecDeque<PathBuf>, entries: &[RepoEntry]) -> Option<PathBuf> {
    while let Some(p) = queue.pop_front() {
        let busy = entries.iter().any(|e| e.path == p && e.busy.is_some());
        let known = entries.iter().any(|e| e.path == p);
        if known && !busy {
            return Some(p);
        }
    }
    None
}

/// Résumé global (tray) sur les dépôts système + suivis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overall {
    Unknown,
    UpToDate,
    Updates(usize),
    Error,
}

pub fn overall(entries: &[RepoEntry]) -> Overall {
    let watched: Vec<&RepoEntry> = entries
        .iter()
        .filter(|e| matches!(e.role, Role::System | Role::Tracked))
        .collect();
    if watched.iter().any(|e| e.error.is_some()) {
        return Overall::Error;
    }
    let total: usize = watched
        .iter()
        .map(|e| e.pending.as_ref().map_or(0, Vec::len))
        .sum();
    if total > 0 {
        Overall::Updates(total)
    } else if !watched.is_empty() && watched.iter().all(|e| e.pending.is_some()) {
        Overall::UpToDate
    } else {
        Overall::Unknown
    }
}

/// Actions possibles sur un dépôt (désactive tout si une commande tourne).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Actions {
    pub check: bool,
    pub update: bool,
    pub apply: bool,
    pub push: bool,
    pub track: bool,
    pub untrack: bool,
    pub set_system: bool,
    pub ignore: bool,
    pub unignore: bool,
}

pub fn actions(e: &RepoEntry) -> Actions {
    let free = e.busy.is_none();
    let tracked_like = matches!(e.role, Role::System | Role::Tracked);
    Actions {
        check: free && e.role != Role::Ignored,
        update: free && tracked_like,
        apply: free && e.role == Role::System,
        push: free && e.git.is_repo,
        track: e.role == Role::Detected || e.role == Role::Ignored,
        untrack: tracked_like,
        set_system: matches!(e.role, Role::Tracked | Role::Detected),
        ignore: e.role != Role::Ignored && e.role != Role::System,
        unignore: e.role == Role::Ignored,
    }
}

/// Changement de rôle demandé par l'utilisateur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleAction {
    Track,
    Untrack,
    SetSystem,
    Ignore,
    Unignore,
}

impl RoleAction {
    pub fn target(self) -> Role {
        match self {
            RoleAction::Track => Role::Tracked,
            RoleAction::SetSystem => Role::System,
            RoleAction::Ignore => Role::Ignored,
            RoleAction::Untrack | RoleAction::Unignore => Role::Detected,
        }
    }
}

// ---------------------------------------------------------------------------
// Flux git (Q6)
// ---------------------------------------------------------------------------

/// Dialogue modal en cours.
#[derive(Debug, Clone, PartialEq)]
pub enum Modal {
    /// Avant la mise à jour : des fichiers non commités existent.
    CommitDirty {
        repo: PathBuf,
        dirty: Vec<String>,
        message: String,
    },
    /// Après la mise à jour : proposition de commit de `flake.lock`.
    CommitLock { repo: PathBuf, message: String },
}

impl Modal {
    pub fn repo(&self) -> &Path {
        match self {
            Modal::CommitDirty { repo, .. } | Modal::CommitLock { repo, .. } => repo,
        }
    }

    pub fn message_mut(&mut self) -> &mut String {
        match self {
            Modal::CommitDirty { message, .. } | Modal::CommitLock { message, .. } => message,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatePlan {
    Proceed,
    AskCommit,
}

/// Avant une mise à jour : demander un commit si git et modifs présentes.
pub fn plan_update(git: &GitState) -> UpdatePlan {
    if git.is_repo && !git.dirty.is_empty() {
        UpdatePlan::AskCommit
    } else {
        UpdatePlan::Proceed
    }
}

/// Après une mise à jour : proposer de committer `flake.lock` ?
pub fn should_propose_lock_commit(git: &GitState) -> bool {
    git.is_repo && git.lock_dirty
}

pub const DIRTY_COMMIT_MESSAGE: &str = "wip: sauvegarde avant mise à jour de flake.lock";

/// Chemin avec `$HOME` remplacé par `~`.
pub fn display_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

// ---------------------------------------------------------------------------
// Dates
// ---------------------------------------------------------------------------

/// `AAAA-MM-JJ` (UTC) d'un horodatage Unix.
pub fn format_date(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    // Algorithme « civil from days » de Howard Hinnant.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn format_opt_date(ts: Option<i64>) -> String {
    ts.map_or_else(|| "—".to_string(), format_date)
}

/// Date relative en français (« hier », « il y a 3 j »…) par rapport à `now` (secondes Unix).
pub fn format_relative(ts: i64, now: i64) -> String {
    let d = now - ts;
    if d < 0 {
        return "à venir".to_string();
    }
    let (min, hour, day) = (60, 3_600, 86_400);
    match d {
        _ if d < min => "à l'instant".to_string(),
        _ if d < hour => format!("il y a {} min", d / min),
        _ if d < day => format!("il y a {} h", d / hour),
        _ if d < 2 * day => "hier".to_string(),
        _ if d < 30 * day => format!("il y a {} j", d / day),
        _ if d < 365 * day => format!("il y a {} mois", d / (30 * day)),
        _ => {
            let y = d / (365 * day);
            format!("il y a {y} an{}", if y > 1 { "s" } else { "" })
        }
    }
}

/// Ordre de parcours des dépôts dans la barre latérale (groupes repliés exclus).
pub fn nav_order(entries: &[RepoEntry], collapsed: &HashSet<Role>) -> Vec<PathBuf> {
    group_entries(entries)
        .into_iter()
        .filter(|(role, _)| !collapsed.contains(role))
        .flat_map(|(_, idx)| idx.into_iter().map(|i| entries[i].path.clone()))
        .collect()
}

/// Révision abrégée (7 caractères) ou « — ».
pub fn short_rev(rev: &Option<String>) -> String {
    rev.as_deref()
        .map_or_else(|| "—".to_string(), |r| r.chars().take(7).collect())
}

// ---------------------------------------------------------------------------
// Mises à jour sélectives (par input racine)
// ---------------------------------------------------------------------------

/// Une mise à jour est-elle disponible pour cet input racine ? Les changements
/// portent sur les nœuds du verrou (`nixpkgs_2`…) : on compare au nœud de l'input.
pub fn input_has_update(input: &RootInput, changes: &[InputChange]) -> bool {
    input
        .node
        .as_deref()
        .is_some_and(|node| changes.iter().any(|c| c.name == node))
}

/// Changement correspondant à un input racine, s'il y en a un.
pub fn input_change<'a>(input: &RootInput, changes: &'a [InputChange]) -> Option<&'a InputChange> {
    let node = input.node.as_deref()?;
    changes.iter().find(|c| c.name == node)
}

/// Noms des inputs racine ayant une mise à jour disponible.
pub fn updatable_names(inputs: &[RootInput], changes: &[InputChange]) -> BTreeSet<String> {
    inputs
        .iter()
        .filter(|i| input_has_update(i, changes))
        .map(|i| i.name.clone())
        .collect()
}

/// Bascule un input dans la sélection.
pub fn toggle_selection(sel: &mut BTreeSet<String>, name: &str) {
    if !sel.remove(name) {
        sel.insert(name.to_string());
    }
}

/// Tous les inputs.
pub fn select_all(inputs: &[RootInput]) -> BTreeSet<String> {
    inputs.iter().map(|i| i.name.clone()).collect()
}

/// Retire de la sélection les inputs qui n'existent plus (après rechargement).
pub fn prune_selection(sel: &mut BTreeSet<String>, inputs: &[RootInput]) {
    sel.retain(|n| inputs.iter().any(|i| &i.name == n));
}

/// Libellé du bouton de mise à jour de la sélection.
pub fn selection_label(n: usize) -> String {
    format!("Mettre à jour la sélection ({n})")
}

/// Texte d'une mise à jour : « nixpkgs, home-manager » ou « tous les inputs ».
pub fn inputs_text(inputs: &[String]) -> String {
    if inputs.is_empty() {
        "tous les inputs".to_string()
    } else {
        inputs.join(", ")
    }
}

/// Message de fin de mise à jour (journal et toast).
pub fn update_done_text(name: &str, inputs: &[String], changed: usize) -> String {
    if inputs.is_empty() {
        format!("✓ {name} : mise à jour terminée ({changed} input(s) modifié(s))")
    } else {
        format!(
            "✓ {name} : mise à jour de {} terminée ({changed} entrée(s) modifiée(s))",
            inputs_text(inputs)
        )
    }
}

/// Mises à jour encore disponibles après une mise à jour : tout si elle portait
/// sur tous les inputs, sinon ce qui n'a pas été touché. `None` reste `None`.
pub fn remaining_pending(
    pending: Option<Vec<InputChange>>,
    inputs: &[String],
    updated: &[InputChange],
) -> Option<Vec<InputChange>> {
    if inputs.is_empty() {
        return Some(Vec::new());
    }
    pending.map(|p| {
        p.into_iter()
            .filter(|c| !updated.iter().any(|u| u.name == c.name))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(name: &str, new: &str) -> InputChange {
        InputChange {
            name: name.into(),
            old_rev: Some("0000000".into()),
            new_rev: Some(new.into()),
            old_date: Some(0),
            new_date: Some(86_400),
        }
    }

    fn entry(path: &str, role: Role) -> RepoEntry {
        RepoEntry {
            path: PathBuf::from(path),
            role,
            git: GitState::default(),
            pending: None,
            error: None,
            busy: None,
            last_update: None,
        }
    }

    fn dirty_git() -> GitState {
        GitState {
            is_repo: true,
            branch: Some("main".into()),
            dirty: vec!["a.nix".into()],
            lock_dirty: false,
        }
    }

    #[test]
    fn relative_dates() {
        let now = 100 * 86_400;
        assert_eq!(format_relative(now - 5, now), "à l'instant");
        assert_eq!(format_relative(now - 600, now), "il y a 10 min");
        assert_eq!(format_relative(now - 7_300, now), "il y a 2 h");
        assert_eq!(format_relative(now - 86_400 - 10, now), "hier");
        assert_eq!(format_relative(now - 5 * 86_400, now), "il y a 5 j");
        assert_eq!(format_relative(now - 65 * 86_400, now), "il y a 2 mois");
        assert_eq!(format_relative(now - 800 * 86_400, now), "il y a 2 ans");
        assert_eq!(format_relative(now + 10, now), "à venir");
    }

    #[test]
    fn nav_order_follows_groups_and_skips_collapsed() {
        let es = vec![
            entry("/t2", Role::Tracked),
            entry("/s", Role::System),
            entry("/t1", Role::Tracked),
            entry("/d", Role::Detected),
        ];
        let all = nav_order(&es, &HashSet::new());
        let names: Vec<_> = all.iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(names, ["/s", "/t1", "/t2", "/d"]);
        let some = nav_order(&es, &HashSet::from([Role::Tracked]));
        assert_eq!(some, [PathBuf::from("/s"), PathBuf::from("/d")]);
    }

    #[test]
    fn badge_priorities() {
        let mut e = entry("/a", Role::Tracked);
        assert_eq!(badge(&e), Badge::Unchecked);
        e.pending = Some(vec![]);
        assert_eq!(badge(&e), Badge::UpToDate);
        e.git = dirty_git();
        assert_eq!(badge(&e), Badge::Dirty);
        e.pending = Some(vec![change("n", "1"), change("m", "2")]);
        assert_eq!(badge(&e), Badge::Updates(2));
        e.error = Some("boum".into());
        assert_eq!(badge(&e), Badge::Error);
        e.busy = Some(JobKind::Update);
        assert_eq!(badge(&e), Badge::Busy(JobKind::Update));
    }

    #[test]
    fn dirty_ignored_when_not_a_git_repo() {
        let mut e = entry("/a", Role::Tracked);
        e.git.dirty = vec!["x".into()];
        assert_eq!(badge(&e), Badge::Unchecked);
    }

    #[test]
    fn badge_labels() {
        assert_eq!(Badge::Updates(1).label(), "1 mise à jour");
        assert_eq!(Badge::Updates(3).label(), "3 mises à jour");
        assert_eq!(Badge::UpToDate.label(), "À jour");
        assert_eq!(Badge::Busy(JobKind::Check).label(), "Vérification…");
        assert_eq!(Badge::Error.tone(), Tone::Danger);
    }

    #[test]
    fn grouping_order_and_sort() {
        let es = vec![
            entry("/z", Role::Detected),
            entry("/b", Role::Tracked),
            entry("/a", Role::Tracked),
            entry("/s", Role::System),
        ];
        let g = group_entries(&es);
        assert_eq!(
            g.iter().map(|(r, _)| *r).collect::<Vec<_>>(),
            [Role::System, Role::Tracked, Role::Detected]
        );
        assert_eq!(g[1].1, vec![2, 1]);
        assert!(group_entries(&[]).is_empty());
    }

    #[test]
    fn collect_paths_merges_and_dedups() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let b = d.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let mut cfg = Config {
            scan_roots: vec![],
            system_repo: None,
            ..Config::default()
        };
        cfg.tracked.insert(a.clone());
        cfg.ignored.insert(b.clone());
        cfg.tracked.insert(d.path().join("absent"));
        let out = collect_paths(vec![a.canonicalize().unwrap()], &cfg, |p| p.is_dir());
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn merge_keeps_known_state() {
        let mut old = entry("/a", Role::Tracked);
        old.pending = Some(vec![change("n", "1")]);
        old.busy = Some(JobKind::Check);
        let fresh = vec![
            RepoStatus {
                path: "/a".into(),
                role: Role::System,
                git: dirty_git(),
                pending: None,
                error: None,
            },
            RepoStatus {
                path: "/new".into(),
                role: Role::Detected,
                git: GitState::default(),
                pending: None,
                error: None,
            },
        ];
        let m = merge_scan(&[old, entry("/gone", Role::Tracked)], fresh);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].role, Role::System);
        assert!(m[0].git.is_repo);
        assert_eq!(m[0].pending.as_ref().unwrap().len(), 1);
        assert_eq!(m[0].busy, Some(JobKind::Check));
        assert_eq!(m[1].pending, None);
    }

    #[test]
    fn check_all_and_queue() {
        let mut busy = entry("/busy", Role::Tracked);
        busy.busy = Some(JobKind::Update);
        let es = vec![
            entry("/t", Role::Tracked),
            entry("/s", Role::System),
            entry("/d", Role::Detected),
            busy,
        ];
        let targets = check_all_targets(&es);
        assert_eq!(targets, [PathBuf::from("/s"), PathBuf::from("/t")]);
        let mut q: VecDeque<PathBuf> = ["/busy", "/gone", "/t"].iter().map(PathBuf::from).collect();
        assert_eq!(pick_next(&mut q, &es), Some(PathBuf::from("/t")));
        assert_eq!(pick_next(&mut q, &es), None);
    }

    #[test]
    fn overall_summary() {
        let mut a = entry("/a", Role::System);
        let mut b = entry("/b", Role::Tracked);
        let d = entry("/d", Role::Detected);
        assert_eq!(overall(&[]), Overall::Unknown);
        assert_eq!(
            overall(&[a.clone(), b.clone(), d.clone()]),
            Overall::Unknown
        );
        a.pending = Some(vec![]);
        assert_eq!(
            overall(&[a.clone(), b.clone(), d.clone()]),
            Overall::Unknown
        );
        b.pending = Some(vec![]);
        assert_eq!(
            overall(&[a.clone(), b.clone(), d.clone()]),
            Overall::UpToDate
        );
        b.pending = Some(vec![change("x", "1"), change("y", "2")]);
        assert_eq!(overall(&[a.clone(), b.clone()]), Overall::Updates(2));
        a.error = Some("e".into());
        assert_eq!(overall(&[a, b]), Overall::Error);
    }

    #[test]
    fn action_availability() {
        let sys = entry("/s", Role::System);
        let a = actions(&sys);
        assert!(a.apply && a.update && a.check && a.untrack && !a.set_system && !a.ignore);
        let tr = actions(&entry("/t", Role::Tracked));
        assert!(!tr.apply && tr.update && tr.set_system && tr.ignore);
        let de = actions(&entry("/d", Role::Detected));
        assert!(!de.update && de.check && de.track && de.set_system && de.ignore);
        let ig = actions(&entry("/i", Role::Ignored));
        assert!(!ig.check && ig.unignore && ig.track && !ig.ignore);
        let mut busy = entry("/b", Role::System);
        busy.busy = Some(JobKind::Apply);
        let b = actions(&busy);
        assert!(!b.check && !b.update && !b.apply);
        assert!(!actions(&entry("/t", Role::Tracked)).push);
        let mut g = entry("/g", Role::Tracked);
        g.git.is_repo = true;
        assert!(actions(&g).push);
    }

    #[test]
    fn role_targets() {
        assert_eq!(RoleAction::Track.target(), Role::Tracked);
        assert_eq!(RoleAction::Untrack.target(), Role::Detected);
        assert_eq!(RoleAction::SetSystem.target(), Role::System);
        assert_eq!(RoleAction::Ignore.target(), Role::Ignored);
        assert_eq!(RoleAction::Unignore.target(), Role::Detected);
    }

    #[test]
    fn git_flow_decisions() {
        assert_eq!(plan_update(&GitState::default()), UpdatePlan::Proceed);
        assert_eq!(plan_update(&dirty_git()), UpdatePlan::AskCommit);
        let mut clean = dirty_git();
        clean.dirty.clear();
        assert_eq!(plan_update(&clean), UpdatePlan::Proceed);
        // Hors git : jamais de proposition, même avec des champs incohérents.
        let non_git = GitState {
            is_repo: false,
            dirty: vec!["x".into()],
            lock_dirty: true,
            branch: None,
        };
        assert_eq!(plan_update(&non_git), UpdatePlan::Proceed);
        assert!(!should_propose_lock_commit(&non_git));
        let mut g = dirty_git();
        assert!(!should_propose_lock_commit(&g));
        g.lock_dirty = true;
        assert!(should_propose_lock_commit(&g));
    }

    #[test]
    fn modal_accessors() {
        let mut m = Modal::CommitLock {
            repo: "/r".into(),
            message: "a".into(),
        };
        m.message_mut().push('b');
        assert_eq!(m.repo(), Path::new("/r"));
        assert_eq!(
            m,
            Modal::CommitLock {
                repo: "/r".into(),
                message: "ab".into()
            }
        );
    }

    #[test]
    fn dates() {
        assert_eq!(format_date(0), "1970-01-01");
        assert_eq!(format_date(1_700_000_000), "2023-11-14");
        assert_eq!(format_date(951_782_400), "2000-02-29");
        assert_eq!(format_opt_date(None), "—");
    }

    #[test]
    fn home_is_abbreviated() {
        let h = Path::new("/home/u");
        assert_eq!(display_path(Path::new("/home/u/a/b"), Some(h)), "~/a/b");
        assert_eq!(display_path(h, Some(h)), "~");
        assert_eq!(display_path(Path::new("/etc/nixos"), Some(h)), "/etc/nixos");
        assert_eq!(display_path(Path::new("/home/u/x"), None), "/home/u/x");
    }

    #[test]
    fn revs() {
        assert_eq!(short_rev(&Some("abcdef0123".into())), "abcdef0");
        assert_eq!(short_rev(&None), "—");
        assert_eq!(short_name(Path::new("/home/u/nixos")), "nixos");
    }

    fn root(name: &str, node: Option<&str>) -> RootInput {
        RootInput {
            name: name.into(),
            node: node.map(Into::into),
            rev: None,
            short_rev: Some("abc1234".into()),
            last_modified: None,
            kind: Some("github".into()),
            owner: None,
            repo: None,
            url: Some(format!("github:o/{name}")),
        }
    }

    #[test]
    fn changes_match_root_nodes_not_transitive_ones() {
        let inputs = [
            root("nixpkgs", Some("nixpkgs")),
            root("hm", Some("home-manager")),
            root("follower", None),
        ];
        let changes = [change("home-manager", "a"), change("nixpkgs_2", "b")];
        assert!(!input_has_update(&inputs[0], &changes));
        assert!(input_has_update(&inputs[1], &changes));
        assert!(!input_has_update(&inputs[2], &changes));
        assert_eq!(
            updatable_names(&inputs, &changes),
            BTreeSet::from(["hm".to_string()])
        );
        assert_eq!(
            input_change(&inputs[1], &changes).map(|c| c.name.as_str()),
            Some("home-manager")
        );
    }

    #[test]
    fn selection_toggle_all_prune() {
        let inputs = [root("a", Some("a")), root("b", Some("b"))];
        let mut sel = BTreeSet::new();
        toggle_selection(&mut sel, "a");
        assert_eq!(sel.len(), 1);
        toggle_selection(&mut sel, "a");
        assert!(sel.is_empty());
        sel = select_all(&inputs);
        assert_eq!(sel.len(), 2);
        sel.insert("gone".into());
        prune_selection(&mut sel, &inputs);
        assert_eq!(sel, select_all(&inputs));
    }

    #[test]
    fn selection_and_update_texts() {
        assert_eq!(selection_label(3), "Mettre à jour la sélection (3)");
        assert_eq!(inputs_text(&[]), "tous les inputs");
        assert_eq!(inputs_text(&["a".into(), "b".into()]), "a, b");
        assert!(update_done_text("r", &[], 2).contains("2 input(s)"));
        let t = update_done_text("r", &["nixpkgs".into()], 2);
        assert!(t.contains("nixpkgs") && t.contains("2 entrée(s)"));
    }

    #[test]
    fn partial_update_keeps_untouched_pending() {
        let pending = Some(vec![change("a", "1"), change("b", "2")]);
        let updated = [change("a", "1")];
        let left = remaining_pending(pending.clone(), &["a".into()], &updated).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].name, "b");
        assert_eq!(remaining_pending(pending, &[], &updated), Some(vec![]));
        assert_eq!(remaining_pending(None, &["a".into()], &updated), None);
    }
}
