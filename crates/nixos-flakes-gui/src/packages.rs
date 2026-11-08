//! Page « Paquets » : recherche nixpkgs, ajout, suppression et déplacement des
//! Paquets déclarés du Dépôt système. Logique pure (état, plans de modification)
//! puis entrées/sorties (lecture du dépôt, écriture) puis vue.

use crate::app::Message;
use crate::model::Tone;
use crate::theme::{self, Btn};
use crate::widgets::{self, action, dim, faint, hoverable, mono, pill, section_label, title, Ui};
use iced::widget::{button, column, container, row, scrollable, space, text, text_input};
use iced::{Element, Fill, Length};
use nixos_flakes_config as cfg;
use nixos_flakes_config::{Location, PackageHit};
use nixos_flakes_core::nix::{self, Privilege};
use std::fs;
use std::path::{Path, PathBuf};

/// Nombre maximal de résultats de recherche.
pub const SEARCH_LIMIT: usize = 30;

/// Texte affiché pour un Emplacement qu'on ne sait pas modifier automatiquement.
pub const READ_ONLY_NOTE: &str = "modifiable en code seulement";

// -- messages ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Msg {
    Open,
    Back,
    Query(String),
    Search,
    SearchDone(u64, Result<Vec<PackageHit>, String>),
    Reload,
    Loaded(Result<Vec<ConfigFile>, String>),
    /// Ajouter le résultat de recherche n° i.
    Add(usize),
    /// Supprimer l'entrée (fichier, Emplacement, nom).
    Remove(usize, usize, String),
    /// Déplacer l'entrée (fichier, Emplacement, nom).
    Move(usize, usize, String),
    PickFile(usize),
    PickLoc(usize),
    Cancel,
    Confirm,
    Written(Result<(), String>),
}

// -- données ----------------------------------------------------------------------

/// Un Fichier de config du Dépôt système qui contient des Emplacements.
#[derive(Debug, Clone)]
pub struct ConfigFile {
    /// Chemin relatif au dépôt.
    pub rel: String,
    pub path: PathBuf,
    pub src: String,
    pub locations: Vec<Location>,
}

impl ConfigFile {
    pub fn new(rel: impl Into<String>, path: PathBuf, src: String) -> Self {
        let locations = cfg::list_locations(&src);
        ConfigFile {
            rel: rel.into(),
            path,
            src,
            locations,
        }
    }
}

/// Opération en cours de préparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Add {
        name: String,
    },
    Remove {
        file: usize,
        loc: usize,
        name: String,
    },
    Move {
        file: usize,
        loc: usize,
        name: String,
    },
}

impl Op {
    /// Faut-il choisir un fichier et un Emplacement de destination ?
    pub fn needs_target(&self) -> bool {
        !matches!(self, Op::Remove { .. })
    }

    /// Emplacement d'origine (suppression, déplacement).
    pub fn source(&self) -> Option<(usize, usize)> {
        match self {
            Op::Add { .. } => None,
            Op::Remove { file, loc, .. } | Op::Move { file, loc, .. } => Some((*file, *loc)),
        }
    }

    pub fn heading(&self) -> String {
        match self {
            Op::Add { name } => format!("Ajouter « {name} »"),
            Op::Remove { name, .. } => format!("Supprimer « {name} »"),
            Op::Move { name, .. } => format!("Déplacer « {name} »"),
        }
    }

    pub fn confirm_label(&self) -> &'static str {
        match self {
            Op::Add { .. } => "Ajouter",
            Op::Remove { .. } => "Supprimer",
            Op::Move { .. } => "Déplacer",
        }
    }
}

/// Un fichier à réécrire, avec son aperçu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub rel: String,
    pub path: PathBuf,
    pub old: String,
    pub new: String,
    pub diff: String,
}

fn change(f: &ConfigFile, new: String) -> Change {
    Change {
        rel: f.rel.clone(),
        path: f.path.clone(),
        diff: cfg::diff(&f.src, &new),
        old: f.src.clone(),
        new,
    }
}

/// Plan de modification : les fichiers réécrits (ceux qui changent vraiment).
pub fn build_plan(
    files: &[ConfigFile],
    op: &Op,
    target: Option<(usize, usize)>,
) -> Result<Vec<Change>, String> {
    let get = |fi: usize, li: usize| -> Result<(&ConfigFile, &Location), String> {
        let f = files.get(fi).ok_or("fichier introuvable")?;
        let l = f.locations.get(li).ok_or("Emplacement introuvable")?;
        Ok((f, l))
    };
    let e = |x: cfg::Error| x.to_string();
    let mut out: Vec<(usize, String)> = Vec::new();
    match op {
        Op::Add { name } => {
            let (tf, tl) = target.ok_or("choisissez un Emplacement")?;
            let (f, l) = get(tf, tl)?;
            out.push((tf, cfg::add_package(&f.src, &l.id, name).map_err(e)?));
        }
        Op::Remove { file, loc, name } => {
            let (f, l) = get(*file, *loc)?;
            out.push((*file, cfg::remove_package(&f.src, &l.id, name).map_err(e)?));
        }
        Op::Move { file, loc, name } => {
            let (tf, tl) = target.ok_or("choisissez un Emplacement")?;
            let (a, la) = get(*file, *loc)?;
            let (b, lb) = get(tf, tl)?;
            if tf == *file {
                let removed = cfg::remove_package(&a.src, &la.id, name).map_err(e)?;
                out.push((tf, cfg::add_package(&removed, &lb.id, name).map_err(e)?));
            } else {
                let (na, nb) =
                    cfg::move_package(&a.src, &la.id, &b.src, &lb.id, name).map_err(e)?;
                out.push((*file, na));
                out.push((tf, nb));
            }
        }
    }
    let mut changes = Vec::new();
    for (fi, new) in out {
        cfg::validate(&new).map_err(|x| format!("{} : {x}", files[fi].rel))?;
        if new != files[fi].src {
            changes.push(change(&files[fi], new));
        }
    }
    if changes.is_empty() {
        Err("Aucun changement : le paquet est déjà à cet endroit.".to_string())
    } else {
        Ok(changes)
    }
}

/// Étape de choix en cours.
#[derive(Debug, Clone)]
pub struct Flow {
    pub op: Op,
    pub file: Option<usize>,
    pub loc: Option<usize>,
    pub plan: Option<Result<Vec<Change>, String>>,
}

/// État de la page.
#[derive(Debug, Default)]
pub struct PackagesState {
    pub query: String,
    pub searching: bool,
    pub searched: bool,
    pub seq: u64,
    pub results: Vec<PackageHit>,
    pub search_error: Option<String>,
    pub files: Vec<ConfigFile>,
    pub loaded: bool,
    pub loading: bool,
    pub load_error: Option<String>,
    pub flow: Option<Flow>,
    pub writing: bool,
    /// Une écriture vient d'avoir lieu : rien n'est encore appliqué au système.
    pub written: bool,
    pub write_error: Option<String>,
}

impl PackagesState {
    /// Démarre une recherche : renvoie (numéro, requête) ou `None` si la saisie est vide.
    pub fn begin_search(&mut self) -> Option<(u64, String)> {
        let q = self.query.trim().to_string();
        if q.is_empty() {
            self.seq += 1;
            self.results.clear();
            self.searching = false;
            self.searched = false;
            self.search_error = None;
            return None;
        }
        self.seq += 1;
        self.searching = true;
        self.search_error = None;
        Some((self.seq, q))
    }

    /// Enregistre un résultat ; ignoré s'il appartient à une recherche périmée.
    pub fn finish_search(&mut self, seq: u64, res: Result<Vec<PackageHit>, String>) -> bool {
        if seq != self.seq {
            return false;
        }
        self.searching = false;
        self.searched = true;
        match res {
            Ok(hits) => {
                self.results = hits;
                self.search_error = None;
            }
            Err(e) => {
                self.results.clear();
                self.search_error = Some(e);
            }
        }
        true
    }

    pub fn set_files(&mut self, files: Vec<ConfigFile>) {
        self.files = files;
        self.loaded = true;
        self.loading = false;
        self.load_error = None;
        self.flow = None;
    }

    pub fn set_load_error(&mut self, e: String) {
        self.loading = false;
        self.loaded = true;
        self.load_error = Some(e);
    }

    /// Ouvre le sélecteur pour `op` ; une suppression a directement son aperçu.
    pub fn begin_op(&mut self, op: Op) {
        self.written = false;
        self.write_error = None;
        let plan = if op.needs_target() {
            None
        } else {
            Some(build_plan(&self.files, &op, None))
        };
        self.flow = Some(Flow {
            op,
            file: None,
            loc: None,
            plan,
        });
    }

    pub fn pick_file(&mut self, i: usize) {
        if i >= self.files.len() {
            return;
        }
        if let Some(f) = &mut self.flow {
            f.file = Some(i);
            f.loc = None;
            f.plan = None;
        }
    }

    /// Choisit l'Emplacement `j` du fichier choisi ; refusé s'il est en lecture seule
    /// ou s'il est l'origine d'un déplacement. Renvoie vrai si accepté.
    pub fn pick_loc(&mut self, j: usize) -> bool {
        let Some(flow) = &mut self.flow else {
            return false;
        };
        let Some(fi) = flow.file else { return false };
        let Some(loc) = self.files.get(fi).and_then(|f| f.locations.get(j)) else {
            return false;
        };
        if location_block(loc, flow.op.source() == Some((fi, j))).is_some() {
            return false;
        }
        flow.loc = Some(j);
        flow.plan = Some(build_plan(&self.files, &flow.op, Some((fi, j))));
        true
    }

    pub fn cancel(&mut self) {
        self.flow = None;
    }

    /// Prépare l'écriture : les changements du plan, si l'aperçu est valide.
    pub fn begin_write(&mut self) -> Option<Vec<Change>> {
        if self.writing {
            return None;
        }
        let changes = match self.flow.as_ref()?.plan.as_ref()? {
            Ok(c) => c.clone(),
            Err(_) => return None,
        };
        self.writing = true;
        self.write_error = None;
        Some(changes)
    }

    pub fn finish_write(&mut self, res: Result<(), String>) {
        self.writing = false;
        match res {
            Ok(()) => {
                self.written = true;
                self.flow = None;
            }
            Err(e) => self.write_error = Some(e),
        }
    }

    /// Mode démo : applique les changements à la fausse config en mémoire.
    pub fn apply_in_memory(&mut self, changes: &[Change]) -> Result<(), String> {
        for c in changes {
            let f = self
                .files
                .iter_mut()
                .find(|f| f.rel == c.rel)
                .ok_or_else(|| format!("{} introuvable", c.rel))?;
            *f = ConfigFile::new(c.rel.clone(), c.path.clone(), c.new.clone());
        }
        Ok(())
    }
}

// -- présentation pure --------------------------------------------------------------

/// Version et licence d'un résultat.
pub fn hit_meta(h: &PackageHit) -> String {
    match (h.version.is_empty(), h.license.as_deref()) {
        (true, None) | (true, Some("")) => String::new(),
        (false, None) | (false, Some("")) => h.version.clone(),
        (true, Some(l)) => l.to_string(),
        (false, Some(l)) => format!("{} · {l}", h.version),
    }
}

/// Intitulé d'un Emplacement : identifiant et portée `with`.
pub fn location_label(l: &Location) -> String {
    match &l.scope {
        Some(s) => format!("{} · with {s}", l.id),
        None => l.id.clone(),
    }
}

/// Raison pour laquelle un Emplacement ne peut pas être choisi comme destination.
pub fn location_block(l: &Location, is_source: bool) -> Option<String> {
    if let Some(why) = &l.read_only {
        Some(format!("{READ_ONLY_NOTE} ({why})"))
    } else if is_source {
        Some("Emplacement d'origine".to_string())
    } else {
        None
    }
}

/// Entrées affichables d'un Emplacement : tout sauf les commentaires seuls.
pub fn visible_entries(l: &Location) -> Vec<&cfg::Entry> {
    l.entries.iter().filter(|e| !e.is_comment_only).collect()
}

/// Teinte d'une ligne de diff unifié.
pub fn diff_tone(line: &str) -> Tone {
    if line.starts_with("+++") || line.starts_with("---") {
        Tone::Neutral
    } else if line.starts_with('+') {
        Tone::Success
    } else if line.starts_with('-') {
        Tone::Danger
    } else if line.starts_with("@@") {
        Tone::Info
    } else {
        Tone::Neutral
    }
}

// -- entrées/sorties ----------------------------------------------------------------

fn collect_nix_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(ty) = e.file_type() else { continue };
        if ty.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "result" {
                collect_nix_files(&p, out);
            }
        } else if ty.is_file() && p.extension().is_some_and(|x| x == "nix") {
            out.push(p);
        }
    }
}

/// Lit les Fichiers de config du dépôt qui ont au moins un Emplacement (bloquant).
pub fn load_repo(repo: &Path) -> Result<Vec<ConfigFile>, String> {
    if !repo.is_dir() {
        return Err(format!("{} n'est pas un dossier", repo.display()));
    }
    let mut paths = Vec::new();
    collect_nix_files(repo, &mut paths);
    paths.sort();
    let mut files = Vec::new();
    for path in paths {
        let Ok(src) = fs::read_to_string(&path) else {
            continue;
        };
        let rel = path
            .strip_prefix(repo)
            .unwrap_or(&path)
            .display()
            .to_string();
        let f = ConfigFile::new(rel, path, src);
        if !f.locations.is_empty() {
            files.push(f);
        }
    }
    Ok(files)
}

/// Écrit un fichier, avec élévation de privilèges si le dossier n'est pas modifiable.
fn write_one(path: &Path, content: &str) -> Result<(), String> {
    match cfg::write_atomic(path, content) {
        Ok(()) => Ok(()),
        Err(cfg::Error::Io { source, .. })
            if source.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
            let staged = tmp.path().join("fichier.nix");
            fs::write(&staged, content).map_err(|e| e.to_string())?;
            nix::install_file(&staged, path, Privilege::Pkexec, &mut |_| {})
                .map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Écrit les changements (bloquant) : vérifie que les fichiers n'ont pas bougé depuis
/// l'aperçu, valide la syntaxe, puis écrit.
pub fn write_changes(changes: &[Change]) -> Result<(), String> {
    for c in changes {
        let now = fs::read_to_string(&c.path).map_err(|e| format!("{} : {e}", c.rel))?;
        if now != c.old {
            return Err(format!(
                "{} a changé depuis l'aperçu : rechargez la liste et recommencez.",
                c.rel
            ));
        }
        cfg::validate(&c.new).map_err(|e| format!("{} : {e}", c.rel))?;
        cfg::validate_with_nix(&c.new).map_err(|e| format!("{} : {e}", c.rel))?;
    }
    write_all_with(changes, &mut |c, content| write_one(&c.path, content))
}

/// Écrit dans l'ordre inverse (destination d'un déplacement avant la source) ; si une
/// écriture échoue, restaure les fichiers déjà écrits.
fn write_all_with(
    changes: &[Change],
    write: &mut dyn FnMut(&Change, &str) -> Result<(), String>,
) -> Result<(), String> {
    let mut done: Vec<&Change> = Vec::new();
    for c in changes.iter().rev() {
        if let Err(e) = write(c, &c.new) {
            for d in done.iter().rev() {
                if let Err(e2) = write(d, &d.old) {
                    return Err(format!(
                        "{} : {e} ; restauration de {} impossible : {e2}",
                        c.rel, d.rel
                    ));
                }
            }
            return Err(format!("{} : {e}", c.rel));
        }
        done.push(c);
    }
    Ok(())
}

// -- vue --------------------------------------------------------------------------

fn msg(m: Msg) -> Message {
    Message::Packages(m)
}

fn hit_row<'a>(ui: &Ui<'_>, i: usize, h: &PackageHit) -> Element<'a, Message> {
    let tk = ui.tk;
    let key = format!("pkg:hit:{i}");
    let mut head = row![
        text(h.attr.clone())
            .size(13)
            .font(widgets::medium())
            .color(tk.text),
        faint(tk, hit_meta(h)),
    ]
    .spacing(8)
    .align_y(iced::Center);
    if h.from_cache {
        head = head.push(pill(ui, "cache", Tone::Warning));
    }
    let b = button(column![head, dim(tk, h.description.clone()).size(12)].spacing(2))
        .width(Fill)
        .padding([8, 10])
        .style(theme::row_style(tk, ui.hover(&key), 0.0))
        .on_press(msg(Msg::Add(i)));
    hoverable(key, b)
}

fn diff_view<'a>(ui: &Ui<'_>, changes: &[Change]) -> Element<'a, Message> {
    let tk = ui.tk;
    let mut col = column![].spacing(10);
    for c in changes {
        let mut lines = column![].spacing(0);
        for l in c.diff.lines() {
            lines = lines.push(
                text(l.to_string())
                    .size(12)
                    .font(widgets::MONO)
                    .color(tk.tone_text(diff_tone(l))),
            );
        }
        col = col.push(
            column![
                mono(tk, c.rel.clone()),
                container(lines)
                    .padding(8)
                    .width(Fill)
                    .style(theme::outlined(tk)),
            ]
            .spacing(4),
        );
    }
    col.into()
}

fn flow_view<'a>(ui: &Ui<'_>, st: &'a PackagesState, flow: &'a Flow) -> Element<'a, Message> {
    let tk = ui.tk;
    let mut col = column![title(tk, flow.op.heading(), 15.0)].spacing(10);
    if flow.op.needs_target() {
        col = col.push(section_label(tk, "1. Fichier de config"));
        let mut files = column![].spacing(4);
        for (i, f) in st.files.iter().enumerate() {
            let kind = if flow.file == Some(i) {
                Btn::Primary
            } else {
                Btn::Secondary
            };
            files = files.push(action(
                ui,
                &format!("pkg:pf:{i}"),
                &f.rel,
                kind,
                None,
                Some(msg(Msg::PickFile(i))),
            ));
        }
        col = col.push(
            scrollable(files)
                .height(Length::Shrink)
                .style(theme::scroll_style(tk)),
        );
        if let Some(fi) = flow.file {
            col = col.push(section_label(tk, "2. Emplacement"));
            for (j, l) in st.files[fi].locations.iter().enumerate() {
                let block = location_block(l, flow.op.source() == Some((fi, j)));
                let kind = if flow.loc == Some(j) {
                    Btn::Primary
                } else {
                    Btn::Secondary
                };
                let mut line = row![action(
                    ui,
                    &format!("pkg:pl:{j}"),
                    &location_label(l),
                    kind,
                    None,
                    block.is_none().then(|| msg(Msg::PickLoc(j))),
                )]
                .spacing(10)
                .align_y(iced::Center);
                if let Some(why) = block {
                    line = line.push(faint(tk, why));
                }
                col = col.push(line);
            }
        }
    }
    let mut can_confirm = false;
    match &flow.plan {
        Some(Ok(changes)) => {
            can_confirm = !st.writing;
            col = col.push(section_label(tk, "Aperçu des changements"));
            col = col.push(diff_view(ui, changes));
        }
        Some(Err(e)) => {
            col = col.push(text(e.clone()).size(12).color(tk.danger));
        }
        None => {}
    }
    if let Some(e) = &st.write_error {
        col = col.push(text(e.clone()).size(12).color(tk.danger));
    }
    let mut buttons = row![action(
        ui,
        "pkg:cancel",
        "Annuler",
        Btn::Secondary,
        None,
        Some(msg(Msg::Cancel))
    )]
    .spacing(8);
    let confirm_kind = if matches!(flow.op, Op::Remove { .. }) {
        Btn::Danger
    } else {
        Btn::Primary
    };
    buttons = buttons.push(action(
        ui,
        "pkg:confirm",
        flow.op.confirm_label(),
        confirm_kind,
        None,
        can_confirm.then(|| msg(Msg::Confirm)),
    ));
    if st.writing {
        buttons = buttons.push(dim(tk, "Écriture…").size(12));
    }
    col = col.push(buttons);
    container(col)
        .padding([14, 16])
        .width(Fill)
        .style(theme::card(tk))
        .into()
}

fn declared_view<'a>(ui: &Ui<'_>, st: &'a PackagesState) -> Element<'a, Message> {
    let tk = ui.tk;
    let mut col = column![row![
        section_label(tk, "Paquets déclarés"),
        space::horizontal(),
        action(
            ui,
            "pkg:reload",
            "Recharger",
            Btn::Ghost,
            None,
            (!st.loading).then(|| msg(Msg::Reload))
        ),
    ]
    .align_y(iced::Center)]
    .spacing(8);
    if st.loading {
        col = col.push(dim(tk, "Lecture du dépôt…").size(12));
    }
    if let Some(e) = &st.load_error {
        col = col.push(text(e.clone()).size(12).color(tk.danger));
    }
    if st.loaded && st.load_error.is_none() && st.files.is_empty() {
        col = col.push(dim(tk, "Aucun Emplacement de paquets trouvé dans ce dépôt.").size(12));
    }
    let busy = st.flow.is_some();
    for (fi, f) in st.files.iter().enumerate() {
        let mut card = column![mono(tk, f.rel.clone())].spacing(8);
        for (li, l) in f.locations.iter().enumerate() {
            card = card.push(faint(tk, location_label(l)));
            if let Some(why) = &l.read_only {
                card = card.push(
                    text(format!("{READ_ONLY_NOTE} ({why})"))
                        .size(12)
                        .color(tk.warning),
                );
            }
            for e in visible_entries(l) {
                let mut line = row![mono(tk, e.name.clone()).width(Fill)]
                    .spacing(6)
                    .align_y(iced::Center);
                if l.read_only.is_none() && e.simple {
                    line = line
                        .push(action(
                            ui,
                            &format!("pkg:mv:{fi}:{li}:{}", e.name),
                            "Déplacer",
                            Btn::Ghost,
                            None,
                            (!busy).then(|| msg(Msg::Move(fi, li, e.name.clone()))),
                        ))
                        .push(action(
                            ui,
                            &format!("pkg:rm:{fi}:{li}:{}", e.name),
                            "Supprimer",
                            Btn::Ghost,
                            None,
                            (!busy).then(|| msg(Msg::Remove(fi, li, e.name.clone()))),
                        ));
                } else if l.read_only.is_none() {
                    line = line.push(faint(tk, "expression non modifiable"));
                }
                card = card.push(line);
            }
        }
        col = col.push(
            container(card)
                .padding([12, 14])
                .width(Fill)
                .style(theme::outlined(tk)),
        );
    }
    col.into()
}

/// Vue de la page. `apply` : message de l'Application si elle est disponible.
pub fn view<'a>(
    ui: &Ui<'_>,
    st: &'a PackagesState,
    system_repo: Option<&Path>,
    apply: Option<Message>,
) -> Element<'a, Message> {
    let tk = ui.tk;
    let header = row![
        action(
            ui,
            "pkg:back",
            "← Retour",
            Btn::Ghost,
            Some("Échap"),
            Some(msg(Msg::Back))
        ),
        title(tk, "Paquets", 18.0),
    ]
    .spacing(10)
    .align_y(iced::Center);

    let mut content = column![header].spacing(16).padding([20, 24]).max_width(820);

    let Some(repo) = system_repo else {
        content = content.push(
            container(
                column![
                    title(tk, "Aucun Dépôt système", 15.0),
                    dim(
                        tk,
                        "Les Paquets déclarés se modifient dans le Dépôt système. Ouvrez un dépôt \
                         dans la liste et choisissez « Définir comme système », ou vérifiez les \
                         racines de scan dans les réglages."
                    )
                    .size(13),
                    row![action(
                        ui,
                        "pkg:settings",
                        "Ouvrir les réglages",
                        Btn::Primary,
                        None,
                        Some(Message::OpenSettings)
                    )],
                ]
                .spacing(10),
            )
            .padding([14, 16])
            .width(Fill)
            .style(theme::card(tk)),
        );
        return scrollable(container(content).center_x(Fill))
            .width(Fill)
            .height(Fill)
            .style(theme::scroll_style(tk))
            .into();
    };

    content = content.push(faint(tk, format!("Dépôt système : {}", repo.display())));

    // Recherche.
    let can_search = !st.query.trim().is_empty() && !st.searching;
    content = content.push(
        row![
            text_input("Rechercher un paquet nixpkgs…", &st.query)
                .on_input(|s| msg(Msg::Query(s)))
                .on_submit(msg(Msg::Search))
                .style(theme::input_style(tk))
                .size(13)
                .padding([7, 10]),
            action(
                ui,
                "pkg:search",
                "Rechercher",
                Btn::Primary,
                Some("↵"),
                can_search.then(|| msg(Msg::Search))
            ),
        ]
        .spacing(8)
        .align_y(iced::Center),
    );
    if st.searching {
        content = content.push(dim(tk, "Recherche en cours…").size(12));
    } else if let Some(e) = &st.search_error {
        content = content.push(text(e.clone()).size(12).color(tk.danger));
    } else if st.searched && st.results.is_empty() {
        content = content.push(dim(tk, "Aucun résultat.").size(12));
    } else if !st.results.is_empty() {
        let mut list = column![].spacing(2);
        for (i, h) in st.results.iter().enumerate() {
            list = list.push(hit_row(ui, i, h));
        }
        content = content.push(
            container(list)
                .padding(4)
                .width(Fill)
                .style(theme::outlined(tk)),
        );
    }

    if let Some(flow) = &st.flow {
        content = content.push(flow_view(ui, st, flow));
    }

    if st.written {
        let mut notice = row![column![
            text("Modification écrite")
                .size(13)
                .font(widgets::medium())
                .color(tk.text),
            dim(
                tk,
                "Rien n'est appliqué : activez la configuration pour la prendre en compte."
            )
            .size(12),
        ]
        .spacing(2)
        .width(Fill),]
        .spacing(10)
        .align_y(iced::Center);
        if let Some(m) = apply {
            notice = notice.push(action(
                ui,
                "pkg:apply",
                "Appliquer au système",
                Btn::Primary,
                None,
                Some(m),
            ));
        }
        content = content.push(
            container(notice)
                .padding([12, 16])
                .width(Fill)
                .style(theme::card(tk)),
        );
    }

    content = content.push(declared_view(ui, st));

    scrollable(container(content).center_x(Fill))
        .width(Fill)
        .height(Fill)
        .style(theme::scroll_style(tk))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    #[test]
    fn move_write_failure_restores_first_file() {
        let mk = |rel: &str| Change {
            rel: rel.into(),
            path: PathBuf::from(rel),
            old: format!("old-{rel}"),
            new: format!("new-{rel}"),
            diff: String::new(),
        };
        let ch = vec![mk("src"), mk("dst")];
        let mut log: Vec<(String, String)> = Vec::new();
        let r = write_all_with(&ch, &mut |c, content| {
            log.push((c.rel.clone(), content.to_string()));
            if c.rel == "src" {
                Err("boom".into())
            } else {
                Ok(())
            }
        });
        assert!(r.is_err());
        assert_eq!(
            log,
            vec![
                ("dst".to_string(), "new-dst".to_string()),
                ("src".to_string(), "new-src".to_string()),
                ("dst".to_string(), "old-dst".to_string()),
            ]
        );
    }

    fn demo_state() -> PackagesState {
        let mut st = PackagesState::default();
        st.set_files(demo::config_files());
        st
    }

    fn file_idx(st: &PackagesState, rel: &str) -> usize {
        st.files.iter().position(|f| f.rel == rel).unwrap()
    }

    fn writable(st: &PackagesState, fi: usize) -> usize {
        st.files[fi]
            .locations
            .iter()
            .position(|l| l.read_only.is_none())
            .unwrap()
    }

    #[test]
    fn demo_data_has_writable_and_read_only_locations() {
        let st = demo_state();
        assert!(st.files.len() >= 3);
        assert!(st.files.iter().all(|f| !f.locations.is_empty()));
        let all: Vec<&Location> = st.files.iter().flat_map(|f| &f.locations).collect();
        assert!(all.iter().any(|l| l.read_only.is_some()));
        assert!(all.iter().any(|l| l.read_only.is_none()));
        assert!(st.files.iter().all(|f| cfg::validate(&f.src).is_ok()));
    }

    #[test]
    fn demo_search_filters_and_flags_cache() {
        assert!(demo::package_hits("").is_empty());
        let hits = demo::package_hits("GIT");
        assert!(hits.iter().any(|h| h.attr == "git"));
        assert!(hits.iter().all(|h| h.attr.contains("git")
            || h.name.contains("git")
            || h.description.to_lowercase().contains("git")));
        assert!(demo::package_hits("ripgrep").iter().any(|h| h.from_cache));
        assert!(demo::package_hits("zzzz-inexistant").is_empty());
    }

    #[test]
    fn search_ignores_stale_results() {
        let mut st = PackagesState {
            query: "git".into(),
            ..PackagesState::default()
        };
        let (s1, q) = st.begin_search().unwrap();
        assert_eq!(q, "git");
        assert!(st.searching);
        st.query = "vim".into();
        let (s2, _) = st.begin_search().unwrap();
        assert!(s2 > s1);
        assert!(!st.finish_search(s1, Ok(demo::package_hits("git"))));
        assert!(st.searching && st.results.is_empty());
        assert!(st.finish_search(s2, Ok(demo::package_hits("vim"))));
        assert!(!st.searching && st.searched && !st.results.is_empty());
        assert!(st.finish_search(s2, Err("hors ligne".into())));
        assert!(st.results.is_empty());
        assert_eq!(st.search_error.as_deref(), Some("hors ligne"));
    }

    #[test]
    fn empty_query_clears_results_without_searching() {
        let mut st = PackagesState {
            results: demo::package_hits("git"),
            query: "   ".into(),
            ..PackagesState::default()
        };
        assert!(st.begin_search().is_none());
        assert!(st.results.is_empty() && !st.searching);
    }

    #[test]
    fn add_flow_previews_then_writes_in_memory() {
        let mut st = demo_state();
        st.begin_op(Op::Add {
            name: "ripgrep".into(),
        });
        assert!(st.flow.as_ref().unwrap().plan.is_none());
        assert!(st.begin_write().is_none(), "pas d'aperçu, pas d'écriture");
        let fi = file_idx(&st, "modules/common/packages.nix");
        st.pick_file(fi);
        let lj = writable(&st, fi);
        assert!(st.pick_loc(lj));
        let plan = st.flow.as_ref().unwrap().plan.clone().unwrap().unwrap();
        assert_eq!(plan.len(), 1);
        assert!(plan[0].diff.contains("+    ripgrep"), "{}", plan[0].diff);
        let changes = st.begin_write().unwrap();
        assert!(st.writing);
        assert!(st.begin_write().is_none(), "une seule écriture à la fois");
        st.apply_in_memory(&changes).unwrap();
        st.finish_write(Ok(()));
        assert!(st.written && st.flow.is_none() && !st.writing);
        assert!(st.files[fi].src.contains("ripgrep"));
        assert!(st.files[fi].locations[lj]
            .entries
            .iter()
            .any(|e| e.name == "ripgrep"));
    }

    #[test]
    fn read_only_locations_cannot_be_picked() {
        let mut st = demo_state();
        let (fi, lj) = st
            .files
            .iter()
            .enumerate()
            .find_map(|(i, f)| {
                f.locations
                    .iter()
                    .position(|l| l.read_only.is_some())
                    .map(|j| (i, j))
            })
            .unwrap();
        st.begin_op(Op::Add { name: "x".into() });
        st.pick_file(fi);
        assert!(!st.pick_loc(lj));
        assert!(st.flow.as_ref().unwrap().plan.is_none());
        let why = location_block(&st.files[fi].locations[lj], false).unwrap();
        assert!(why.starts_with(READ_ONLY_NOTE));
    }

    #[test]
    fn remove_has_immediate_preview_and_cancel_keeps_files() {
        let mut st = demo_state();
        let fi = file_idx(&st, "modules/common/packages.nix");
        let lj = writable(&st, fi);
        let before = st.files[fi].src.clone();
        st.begin_op(Op::Remove {
            file: fi,
            loc: lj,
            name: "htop".into(),
        });
        let plan = st.flow.as_ref().unwrap().plan.clone().unwrap().unwrap();
        assert!(plan[0].diff.contains("-    htop"), "{}", plan[0].diff);
        st.cancel();
        assert!(st.flow.is_none());
        assert_eq!(
            st.files[fi].src, before,
            "rien n'est modifié avant confirmation"
        );
    }

    #[test]
    fn move_between_files_changes_both_and_same_location_is_refused() {
        let mut st = demo_state();
        let a = file_idx(&st, "modules/common/packages.nix");
        let b = file_idx(&st, "modules/desktop.nix");
        let la = writable(&st, a);
        let lb = writable(&st, b);
        st.begin_op(Op::Move {
            file: a,
            loc: la,
            name: "htop".into(),
        });
        st.pick_file(a);
        assert!(!st.pick_loc(la), "l'origine n'est pas une destination");
        st.pick_file(b);
        assert!(st.pick_loc(lb));
        let changes = st.begin_write().unwrap();
        assert_eq!(changes.len(), 2);
        st.apply_in_memory(&changes).unwrap();
        assert!(!st.files[a].locations[la]
            .entries
            .iter()
            .any(|e| e.name == "htop"));
        assert!(st.files[b].locations[lb]
            .entries
            .iter()
            .any(|e| e.name == "htop"));
    }

    #[test]
    fn failed_write_keeps_the_flow() {
        let mut st = demo_state();
        let fi = file_idx(&st, "modules/common/packages.nix");
        let lj = writable(&st, fi);
        st.begin_op(Op::Remove {
            file: fi,
            loc: lj,
            name: "htop".into(),
        });
        assert!(st.begin_write().is_some());
        st.finish_write(Err("refusé".into()));
        assert!(!st.writing && !st.written && st.flow.is_some());
        assert_eq!(st.write_error.as_deref(), Some("refusé"));
    }

    #[test]
    fn adding_an_existing_package_is_a_noop_error() {
        let mut st = demo_state();
        let fi = file_idx(&st, "modules/common/packages.nix");
        let lj = writable(&st, fi);
        st.begin_op(Op::Add { name: "git".into() });
        st.pick_file(fi);
        st.pick_loc(lj);
        let plan = st.flow.as_ref().unwrap().plan.clone().unwrap();
        assert!(plan.is_err());
        assert!(st.begin_write().is_none());
    }

    #[test]
    fn view_model_helpers() {
        let mut h = PackageHit {
            attr: "git".into(),
            name: "git".into(),
            version: "2.47".into(),
            description: "d".into(),
            license: Some("GPL-2.0".into()),
            from_cache: false,
        };
        assert_eq!(hit_meta(&h), "2.47 · GPL-2.0");
        h.license = None;
        assert_eq!(hit_meta(&h), "2.47");
        h.version.clear();
        assert_eq!(hit_meta(&h), "");
        assert_eq!(diff_tone("+a"), Tone::Success);
        assert_eq!(diff_tone("-a"), Tone::Danger);
        assert_eq!(diff_tone("--- avant"), Tone::Neutral);
        assert_eq!(diff_tone("@@ -1 +1 @@"), Tone::Info);
        let st = demo_state();
        let l = &st.files[0].locations[0];
        assert!(location_label(l).starts_with(&l.id));
        assert!(visible_entries(l).iter().all(|e| !e.is_comment_only));
    }

    #[test]
    fn load_repo_reads_only_files_with_locations() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("m")).unwrap();
        fs::create_dir(d.path().join(".git")).unwrap();
        fs::write(
            d.path().join("m/a.nix"),
            "{ pkgs, ... }: { environment.systemPackages = with pkgs; [ git ]; }",
        )
        .unwrap();
        fs::write(d.path().join("b.nix"), "{ }").unwrap();
        fs::write(d.path().join(".git/c.nix"), "{ }").unwrap();
        let files = load_repo(d.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].rel, "m/a.nix");
        assert!(load_repo(&d.path().join("absent")).is_err());
    }

    #[test]
    fn write_changes_refuses_a_file_changed_since_preview() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.nix");
        fs::write(&p, "{ }").unwrap();
        let c = Change {
            rel: "a.nix".into(),
            path: p.clone(),
            old: "{ x = 1; }".into(),
            new: "{ x = 2; }".into(),
            diff: String::new(),
        };
        let e = write_changes(&[c]).unwrap_err();
        assert!(e.contains("a changé"));
        assert_eq!(fs::read_to_string(&p).unwrap(), "{ }");
    }
}
