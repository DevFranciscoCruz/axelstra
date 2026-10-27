//! Export and import of the workspace configuration (`workspace.export`, `workspace.import_preview`,
//! `workspace.import`): one JSON file with the projects, the automations, the settings and the kanban cards, to move
//! the setup to another machine or to keep a copy.
//!
//! The file never holds a token, an agent credential, a session, a log or a scrollback: settings go through an
//! allow-list of keys, and the other parts are built field by field. An import is two steps. `preview` parses and
//! validates the whole file and says what would happen; `import` validates it again under the state locks and only
//! then writes. Nothing is written unless every part of the file is valid. A merge never overwrites an item that is
//! already there unless asked to; a replace swaps the state files (the old ones stay as `<name>.1`) and never touches
//! more than the files this module owns. Cards are written only into the `.kanban` folder of a registered project
//! whose folder exists on this machine, and an existing card is rotated into `<card>.md.1` before it is overwritten.

use crate::automations::{self, Automation};
use crate::projects::{self, Project};
use crate::store::{read_json, write_atomic, write_backed_up, write_json, LockRecover};
use anyhow::Result;
use axelstra_core::coded;
use axelstra_core::{card, paths};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const KIND: &str = "axelstra-workspace";
pub const VERSION: u64 = 1;

/// Largest file `import_preview` / `import` read.
pub const MAX_FILE: u64 = 32 << 20;
const MAX_PROJECTS: usize = 200;
const MAX_AUTOMATIONS: usize = 500;
const MAX_CARDS_PER_PROJECT: usize = 2000;
const MAX_CARDS: usize = 5000;
const MAX_CARD_BYTES: usize = 256 * 1024;
const MAX_PARENTS: usize = 500;
const MAX_MAP: usize = 32;
const MAX_PATH: usize = 1024;

/// The keys of settings.json that travel. Anything else in that file is left out.
const SETTINGS_KEYS: [&str; 7] = ["terse", "pm", "auto_install", "shell", "notify_sound", "notify_mode", "ui"];
const TERSE: [&str; 3] = ["off", "short", "tiny"];
const PMS: [&str; 4] = ["bun", "pnpm", "yarn", "npm"];

/// Names that mark a UI preference as a credential: dropped on export and on import.
fn sensitive(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["token", "secret", "password", "passwd", "credential", "apikey", "api_key", "api-key", "private"].iter().any(|w| n.contains(w))
}

fn data_dir(store: &Path) -> &Path {
    store.parent().unwrap_or(Path::new(""))
}

fn settings_path(store: &Path) -> PathBuf {
    store.with_file_name("settings.json")
}

// ---------------------------------------------------------------------------------------------------- export

/// What an export holds, counted.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Summary {
    pub projects: usize,
    pub automations: usize,
    pub cards: usize,
}

fn scrub_settings(raw: &Value) -> Value {
    let mut out = Map::new();
    let Some(raw) = raw.as_object() else { return Value::Object(out) };
    for k in SETTINGS_KEYS {
        let Some(v) = raw.get(k) else { continue };
        if k == "ui" {
            let ui: Map<String, Value> = v
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(n, v)| !sensitive(n) && v.is_string())
                .map(|(n, v)| (n.clone(), v.clone()))
                .collect();
            out.insert(k.into(), Value::Object(ui));
        } else {
            out.insert(k.into(), v.clone());
        }
    }
    Value::Object(out)
}

/// The workspace as the file `export_to` writes.
pub fn build(store: &Path) -> Result<(Value, Summary)> {
    let projects = projects::try_load(store)?;
    let autos = automations::load(&automations::store_path(store));
    let settings = scrub_settings(&read_json::<Value>(&settings_path(store))?.unwrap_or(Value::Null));
    let mut kanban = Map::new();
    let mut cards = 0;
    for p in &projects {
        let repo = Path::new(&p.path);
        if !repo.join(".kanban").is_dir() {
            continue;
        }
        let mut list = vec![];
        for c in crate::kanban::list(repo)?.cards {
            // the raw file, so a card comes back byte for byte; one that cannot be read again is left out
            let Ok(text) = std::fs::read_to_string(&c.path) else { continue };
            let file = c.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            list.push(json!({"file": file, "text": text}));
        }
        cards += list.len();
        if !list.is_empty() {
            kanban.insert(p.id.clone(), Value::Array(list));
        }
    }
    let exported_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let doc = json!({
        "kind": KIND,
        "version": VERSION,
        "exported_at": exported_at,
        "projects": projects,
        "automations": autos,
        "settings": settings,
        "kanban": kanban,
    });
    Ok((doc, Summary { projects: projects.len(), automations: autos.len(), cards }))
}

/// Writes the workspace to `path`. Refuses the daemon's own data folder (the token and the state files live there) and
/// a folder that does not exist.
pub fn export_to(store: &Path, path: &Path) -> Result<Summary> {
    let Some(parent) = path.parent().filter(|d| !d.as_os_str().is_empty()) else {
        return Err(coded!("workspace.no_folder", "{path} has no folder to write into", path = path.display()));
    };
    if !parent.is_dir() {
        return Err(coded!("workspace.not_folder", "{path} is not a folder", path = parent.display()));
    }
    if path.is_dir() {
        return Err(coded!("workspace.is_folder", "{path} is a folder", path = path.display()));
    }
    let parent_s = parent.to_string_lossy();
    if paths::same(&parent_s, &data_dir(store).to_string_lossy()) {
        return Err(coded!("workspace.in_data_dir", "choose a file outside the Axelstra data folder"));
    }
    let (doc, summary) = build(store)?;
    write_atomic(path, &serde_json::to_vec_pretty(&doc)?)
        .map_err(|e| coded!("workspace.write_failed", "could not write {path}: {detail}", path = path.display(), detail = e))?;
    Ok(summary)
}

// ---------------------------------------------------------------------------------------------------- parse

struct InCard {
    id: String,
    text: String,
}

struct Parsed {
    exported_at: String,
    projects: Vec<Project>,
    automations: Vec<Automation>,
    /// valid allow-listed settings; `ui` is an object of strings
    settings: Map<String, Value>,
    /// per project id of the file
    cards: Vec<(String, Vec<InCard>)>,
}

fn text_ok(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.contains('\0')
}

fn parse_settings(v: &Value) -> Result<Map<String, Value>> {
    let Some(raw) = v.as_object() else { return Err(coded!("workspace.settings_not_object", "settings must be an object")) };
    let mut out = Map::new();
    for k in SETTINGS_KEYS {
        let Some(v) = raw.get(k) else { continue };
        let ok = match k {
            "terse" => v.as_str().is_some_and(|s| TERSE.contains(&s)),
            "pm" => v.as_str().is_some_and(|s| PMS.contains(&s)),
            "auto_install" | "notify_sound" => v.is_boolean(),
            "shell" => v.as_str().is_some_and(|s| crate::server::SHELLS.contains(&s)),
            "notify_mode" => v.as_str().is_some_and(|s| crate::server::NOTIFY_MODES.contains(&s)),
            _ => v.is_object(),
        };
        if !ok {
            return Err(coded!("workspace.bad_setting", "setting {name} has an invalid value", name = k));
        }
        if k == "ui" {
            // the daemon's own rules for UI choices (key names, sizes), applied to an empty settings value
            let ui: Map<String, Value> =
                v.as_object().into_iter().flatten().filter(|(n, _)| !sensitive(n)).map(|(n, v)| (n.clone(), v.clone())).collect();
            let mut probe = json!({});
            crate::server::apply_ui_settings(&mut probe, &json!({"ui": ui}))
                .map_err(|e| coded!("workspace.bad_ui", "setting ui is invalid: {detail}", detail = e))?;
            out.insert(k.into(), probe["ui"].clone());
        } else {
            out.insert(k.into(), v.clone());
        }
    }
    Ok(out)
}

fn parse_cards(raw: &Value, project_ids: &HashSet<&str>) -> Result<Vec<(String, Vec<InCard>)>> {
    let Some(raw) = raw.as_object() else { return Err(coded!("workspace.kanban_not_object", "kanban must be an object")) };
    let mut out = vec![];
    let mut total = 0;
    for (pid, list) in raw {
        if !project_ids.contains(pid.as_str()) {
            return Err(coded!(
                "workspace.kanban_unknown_project",
                "kanban has cards for project {project:?}, which is not in the file",
                project = pid
            ));
        }
        let Some(list) = list.as_array() else {
            return Err(coded!("workspace.kanban_not_list", "kanban.{project} must be a list", project = pid));
        };
        if list.len() > MAX_CARDS_PER_PROJECT {
            return Err(coded!(
                "workspace.too_many_cards_project",
                "project {project} has more than {max} cards",
                project = pid,
                max = MAX_CARDS_PER_PROJECT
            ));
        }
        total += list.len();
        if total > MAX_CARDS {
            return Err(coded!("workspace.too_many_cards", "the file has more than {max} cards", max = MAX_CARDS));
        }
        let mut seen = HashSet::new();
        let mut cards = vec![];
        for c in list {
            let file = c["file"].as_str().unwrap_or_default();
            let text = c["text"].as_str().unwrap_or_default();
            // the name is only checked, never used to write: a card goes to `<id>.md`
            let stem = file.strip_suffix(".md").unwrap_or_default();
            if !crate::kanban::valid_id(stem) {
                return Err(coded!("workspace.bad_card_file", "card file name {file:?} is not allowed", file = file));
            }
            if text.len() > MAX_CARD_BYTES {
                return Err(coded!("workspace.card_too_big", "card {file} is larger than {max} bytes", file = file, max = MAX_CARD_BYTES));
            }
            let parsed = card::parse(text, Path::new(file)).map_err(|e| {
                coded!(
                    "workspace.card_invalid",
                    "card {file} of {project} is invalid: {detail}",
                    file = file,
                    project = pid,
                    detail = format!("{e:#}")
                )
            })?;
            if !crate::kanban::valid_id(&parsed.meta.id) {
                return Err(coded!("workspace.bad_card_id", "card id {id:?} in {file} is not allowed", id = parsed.meta.id, file = file));
            }
            if !seen.insert(parsed.meta.id.clone()) {
                return Err(coded!(
                    "workspace.duplicate_card",
                    "project {project} has two cards with id {id}",
                    project = pid,
                    id = parsed.meta.id
                ));
            }
            cards.push(InCard { id: parsed.meta.id, text: text.to_string() });
        }
        out.push((pid.clone(), cards));
    }
    Ok(out)
}

fn parse(bytes: &[u8]) -> Result<Parsed> {
    if bytes.len() as u64 > MAX_FILE {
        return Err(coded!("workspace.file_too_big", "the file is larger than {mib} MiB", mib = MAX_FILE >> 20));
    }
    let doc: Value =
        serde_json::from_slice(bytes).map_err(|e| coded!("workspace.not_valid", "not a valid workspace file: {detail}", detail = e))?;
    if doc["kind"].as_str() != Some(KIND) {
        return Err(coded!("workspace.not_workspace", "not an Axelstra workspace file"));
    }
    match doc["version"].as_u64() {
        Some(VERSION) => {}
        Some(v) if v > VERSION => {
            return Err(coded!(
                "workspace.newer",
                "written by a newer version of Axelstra (file v{file}, this one reads v{current})",
                file = v,
                current = VERSION
            ))
        }
        _ => return Err(coded!("workspace.no_version", "the file has no usable version")),
    }
    let exported_at = doc["exported_at"].as_str().unwrap_or_default().chars().take(40).collect();

    let raw_projects = doc["projects"].as_array().map_or(&[][..], Vec::as_slice);
    if raw_projects.len() > MAX_PROJECTS {
        return Err(coded!("workspace.too_many_projects", "the file has more than {max} projects", max = MAX_PROJECTS));
    }
    let mut projects: Vec<Project> = vec![];
    for v in raw_projects {
        let p: Project =
            serde_json::from_value(v.clone()).map_err(|e| coded!("workspace.invalid_project", "invalid project: {detail}", detail = e))?;
        if !crate::kanban::valid_id(&p.id) {
            return Err(coded!("workspace.bad_project_id", "project id {id:?} is not allowed", id = p.id));
        }
        if projects.iter().any(|x| x.id == p.id) {
            return Err(coded!("workspace.duplicate_project", "two projects have the id {id}", id = p.id));
        }
        if !text_ok(&p.name, 200) || !text_ok(&p.path, MAX_PATH) || p.setup.as_ref().is_some_and(|s| s.len() > 4096 || s.contains('\0')) {
            return Err(coded!("workspace.bad_project", "project {id} has an invalid name, path or setup", id = p.id));
        }
        if p.parents.len() > MAX_PARENTS || p.parents.iter().any(|(k, v)| !text_ok(k, MAX_PATH) || !text_ok(v, MAX_PATH)) {
            return Err(coded!("workspace.bad_parents", "project {id} has invalid worktree parents", id = p.id));
        }
        projects.push(p);
    }
    let ids: HashSet<&str> = projects.iter().map(|p| p.id.as_str()).collect();

    let raw_autos = doc["automations"].as_array().map_or(&[][..], Vec::as_slice);
    if raw_autos.len() > MAX_AUTOMATIONS {
        return Err(coded!("workspace.too_many_automations", "the file has more than {max} automations", max = MAX_AUTOMATIONS));
    }
    let mut autos: Vec<Automation> = vec![];
    for v in raw_autos {
        let a: Automation =
            serde_json::from_value(v.clone()).map_err(|e| coded!("automation.invalid", "invalid automation: {detail}", detail = e))?;
        automations::validate(&a).map_err(|e| {
            coded!("workspace.automation_invalid", "automation {id:?} is invalid: {detail}", id = a.id, detail = format!("{e:#}"))
        })?;
        if !crate::kanban::valid_id(&a.id) {
            return Err(coded!("workspace.bad_automation_id", "automation id {id:?} is not allowed", id = a.id));
        }
        if autos.iter().any(|x| x.id == a.id) {
            return Err(coded!("workspace.duplicate_automation", "two automations have the id {id}", id = a.id));
        }
        if !ids.contains(a.project.as_str()) {
            return Err(coded!(
                "workspace.automation_unknown_project",
                "automation {id} belongs to project {project:?}, which is not in the file",
                id = a.id,
                project = a.project
            ));
        }
        if a.prompt.len() > 64 * 1024 || a.name.len() > 200 {
            return Err(coded!("workspace.automation_too_big", "automation {id} is too large", id = a.id));
        }
        autos.push(a);
    }

    let settings = if doc["settings"].is_null() { Map::new() } else { parse_settings(&doc["settings"])? };
    let cards = if doc["kanban"].is_null() { vec![] } else { parse_cards(&doc["kanban"], &ids)? };
    Ok(Parsed { exported_at, projects, automations: autos, settings, cards })
}

// ---------------------------------------------------------------------------------------------------- plan

/// How an item of the file compares with this machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum St {
    New,
    Present,
    Conflict,
}

#[derive(Serialize, Default, Debug)]
pub struct Skipped {
    pub label: String,
    pub reason: String,
    /// for a project: its folder as the file names it, which a path map can point somewhere else
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// One kind of item: what is new here, what is already here as it is, what is here but different, what cannot be
/// brought over, and how many local items the file does not mention.
#[derive(Serialize, Default, Debug)]
pub struct Section {
    pub new: Vec<String>,
    pub present: Vec<String>,
    pub conflict: Vec<String>,
    pub skipped: Vec<Skipped>,
    pub local_only: usize,
}

impl Section {
    fn put(&mut self, st: St, label: String) {
        match st {
            St::New => self.new.push(label),
            St::Present => self.present.push(label),
            St::Conflict => self.conflict.push(label),
        }
    }
    fn skip_path(&mut self, label: String, reason: &str, path: &str) {
        self.skipped.push(Skipped { label, reason: reason.into(), path: Some(path.into()) });
    }
    fn skip(&mut self, label: String, reason: &str) {
        self.skipped.push(Skipped { label, reason: reason.into(), path: None });
    }
}

#[derive(Serialize, Debug)]
pub struct Preview {
    pub version: u64,
    pub exported_at: String,
    pub projects: Section,
    pub automations: Section,
    pub settings: Section,
    pub cards: Section,
    /// cards that would be written and run with permission checks off or with an acceptance command
    pub risky_cards: usize,
    /// automations the file marks as enabled and that would be written; they are imported disabled, and the prompt is
    /// shown so the user can read it before turning them on
    pub risky_automations: Vec<RiskyAutomation>,
}

#[derive(Serialize, Debug)]
pub struct RiskyAutomation {
    pub name: String,
    pub agent: String,
    pub prompt: String,
}

struct ProjPlan {
    file_id: String,
    st: St,
    /// index in the local list, when the folder is already a project
    existing: Option<usize>,
    /// as it would be saved: resolved path, final id, remapped parents
    proj: Project,
}

struct AutoPlan {
    st: St,
    existing: Option<usize>,
    auto: Automation,
}

struct SetItem {
    ui: bool,
    key: String,
    value: Value,
    st: St,
}

struct CardPlan {
    /// index in `Plan::projs`
    project: usize,
    id: String,
    text: String,
    st: St,
    target: PathBuf,
}

struct Plan {
    preview: Preview,
    cur_projects: Vec<Project>,
    cur_autos: Vec<Automation>,
    cur_settings: Value,
    projs: Vec<ProjPlan>,
    autos: Vec<AutoPlan>,
    items: Vec<SetItem>,
    cards: Vec<CardPlan>,
}

/// Applies the first `old -> new` pair whose `old` contains `path` (whole components only).
fn remap(path: &str, map: &[(String, String)]) -> String {
    for (old, new) in map {
        if !paths::is_within(path, old) {
            continue;
        }
        let s = if cfg!(windows) { path.replace('\\', "/") } else { path.to_string() };
        let want = paths::norm(old);
        let mut cuts: Vec<usize> = s.match_indices('/').map(|(i, _)| i).collect();
        cuts.push(s.len());
        for cut in cuts {
            if paths::norm(&s[..cut]) == want {
                return format!("{}{}", new.trim_end_matches(['/', '\\']), &s[cut..]);
            }
        }
    }
    path.to_string()
}

fn norm_card(t: &str) -> String {
    t.trim_start_matches('\u{feff}').replace("\r\n", "\n").trim_end().to_string()
}

fn plan(store: &Path, p: Parsed, map: &[(String, String)]) -> Result<Plan> {
    let cur_projects = projects::try_load(store)?;
    let cur_autos = automations::load(&automations::store_path(store));
    let cur_settings = read_json::<Value>(&settings_path(store))?.unwrap_or_else(|| json!({}));
    let mut pv = Preview {
        version: VERSION,
        exported_at: p.exported_at.clone(),
        projects: Section::default(),
        automations: Section::default(),
        settings: Section::default(),
        cards: Section::default(),
        risky_cards: 0,
        risky_automations: vec![],
    };

    // projects: the folder decides who is who, the id of the file is only a name inside it
    let mut projs: Vec<ProjPlan> = vec![];
    let mut taken: HashSet<String> = cur_projects.iter().map(|x| x.id.clone()).collect();
    let mut skipped_projects: HashMap<String, &'static str> = HashMap::new();
    for ip in &p.projects {
        let path = remap(&ip.path, map);
        let label = format!("{} ({})", ip.name, path);
        if !Path::new(&path).is_dir() {
            pv.projects.skip_path(label, "missing", &ip.path);
            skipped_projects.insert(ip.id.clone(), "missing");
            continue;
        }
        let root = match projects::resolve_root(&path) {
            Ok(r) => r.to_string_lossy().into_owned(),
            Err(_) => {
                pv.projects.skip_path(label, "forbidden", &ip.path);
                skipped_projects.insert(ip.id.clone(), "forbidden");
                continue;
            }
        };
        if projs.iter().any(|x| paths::same(&x.proj.path, &root)) {
            pv.projects.skip_path(label, "duplicate", &ip.path);
            skipped_projects.insert(ip.id.clone(), "duplicate");
            continue;
        }
        let parents: BTreeMap<String, String> = ip.parents.iter().map(|(k, v)| (remap(k, map), remap(v, map))).collect();
        let existing = cur_projects.iter().position(|c| paths::same(&c.path, &root));
        let (st, id) = match existing {
            Some(i) => {
                let c = &cur_projects[i];
                let same = c.name == ip.name && c.setup == ip.setup && parents.iter().all(|(k, v)| c.parents.get(k) == Some(v));
                (if same { St::Present } else { St::Conflict }, c.id.clone())
            }
            None => {
                let mut id = ip.id.clone();
                while taken.contains(&id) {
                    id.push_str("-2");
                }
                (St::New, id)
            }
        };
        taken.insert(id.clone());
        pv.projects.put(st, format!("{} ({})", ip.name, root));
        projs.push(ProjPlan {
            file_id: ip.id.clone(),
            st,
            existing,
            proj: Project { id, name: ip.name.clone(), path: root, setup: ip.setup.clone(), parents, ..Default::default() },
        });
    }
    pv.projects.local_only =
        cur_projects.iter().filter(|c| !projs.iter().any(|x| x.existing.is_some_and(|i| cur_projects[i].id == c.id))).count();

    // automations
    let by_file: HashMap<&str, usize> = projs.iter().enumerate().map(|(i, x)| (x.file_id.as_str(), i)).collect();
    let mut autos: Vec<AutoPlan> = vec![];
    for a in &p.automations {
        let Some(&pi) = by_file.get(a.project.as_str()) else {
            pv.automations.skip(a.name.clone(), "project skipped");
            continue;
        };
        let mut auto = a.clone();
        auto.project = projs[pi].proj.id.clone();
        let existing = cur_autos.iter().position(|c| c.id == auto.id);
        let st = match existing {
            None => St::New,
            Some(i) => {
                let strip = |x: &Automation| {
                    let mut v = serde_json::to_value(x).unwrap_or_default();
                    v["last_run"] = json!(0);
                    v
                };
                if strip(&cur_autos[i]) == strip(&auto) {
                    St::Present
                } else {
                    St::Conflict
                }
            }
        };
        pv.automations.put(st, auto.name.clone());
        if auto.enabled && st != St::Present {
            pv.risky_automations.push(RiskyAutomation { name: auto.name.clone(), agent: auto.agent.clone(), prompt: auto.prompt.clone() });
        }
        autos.push(AutoPlan { st, existing, auto });
    }
    pv.automations.local_only = cur_autos.iter().filter(|c| !autos.iter().any(|a| a.auto.id == c.id)).count();

    // settings
    let mut items = vec![];
    let cur_ui = cur_settings["ui"].as_object().cloned().unwrap_or_default();
    for (k, v) in &p.settings {
        if k == "ui" {
            for (n, v) in v.as_object().into_iter().flatten() {
                let st = match cur_ui.get(n) {
                    None => St::New,
                    Some(c) if c == v => St::Present,
                    Some(_) => St::Conflict,
                };
                pv.settings.put(st, format!("ui: {n}"));
                items.push(SetItem { ui: true, key: n.clone(), value: v.clone(), st });
            }
        } else {
            let st = match cur_settings.get(k) {
                None => St::New,
                Some(c) if c == v => St::Present,
                Some(_) => St::Conflict,
            };
            pv.settings.put(st, k.clone());
            items.push(SetItem { ui: false, key: k.clone(), value: v.clone(), st });
        }
    }
    let theirs_ui = p.settings.get("ui").and_then(Value::as_object);
    pv.settings.local_only =
        SETTINGS_KEYS.iter().filter(|k| **k != "ui" && cur_settings.get(**k).is_some() && !p.settings.contains_key(**k)).count()
            + cur_ui.keys().filter(|n| !theirs_ui.is_some_and(|t| t.contains_key(*n))).count();

    // cards
    let mut cards = vec![];
    for (fid, list) in &p.cards {
        let name = p.projects.iter().find(|x| &x.id == fid).map_or(fid.as_str(), |x| x.name.as_str());
        let Some(&pi) = by_file.get(fid.as_str()) else {
            for c in list {
                pv.cards.skip(format!("{name}/{}", c.id), skipped_projects.get(fid).copied().unwrap_or("project skipped"));
            }
            continue;
        };
        let root = Path::new(&projs[pi].proj.path);
        let kdir = root.join(".kanban");
        if std::fs::symlink_metadata(&kdir).is_ok_and(|m| m.file_type().is_symlink()) {
            for c in list {
                pv.cards.skip(format!("{name}/{}", c.id), "unsafe");
            }
            continue;
        }
        // an existing card is the file of that name, or the card of that id under another name
        let local = if kdir.is_dir() { crate::kanban::list(root).map(|b| b.cards).unwrap_or_default() } else { vec![] };
        for c in list {
            let default_target = kdir.join(format!("{}.md", c.id));
            let target = local.iter().find(|l| l.meta.id == c.id).map_or(default_target, |l| l.path.clone());
            let st = match std::fs::read_to_string(&target) {
                Ok(t) if norm_card(&t) == norm_card(&c.text) => St::Present,
                Ok(_) => St::Conflict,
                Err(_) if target.exists() => St::Conflict,
                Err(_) => St::New,
            };
            pv.cards.put(st, format!("{name}/{}", c.id));
            if st != St::Present {
                if let Ok(parsed) = card::parse(&c.text, &target) {
                    if card::is_bypass(&parsed.meta.permission_mode) || parsed.meta.acceptance.is_some() {
                        pv.risky_cards += 1;
                    }
                }
            }
            cards.push(CardPlan { project: pi, id: c.id.clone(), text: c.text.clone(), st, target });
        }
    }
    // local cards are not listed: counting them would read every project's board for a number nobody acts on
    Ok(Plan { preview: pv, cur_projects, cur_autos, cur_settings, projs, autos, items, cards })
}

fn read_file(path: &str) -> Result<Vec<u8>> {
    if path.is_empty() || path.contains('\0') {
        return Err(coded!("workspace.path_required", "path is required"));
    }
    let meta =
        std::fs::metadata(path).map_err(|e| coded!("workspace.read_failed", "could not read {path}: {detail}", path = path, detail = e))?;
    if !meta.is_file() {
        return Err(coded!("workspace.not_file", "{path} is not a file", path = path));
    }
    if meta.len() > MAX_FILE {
        return Err(coded!("workspace.path_too_big", "{path} is larger than {mib} MiB", path = path, mib = MAX_FILE >> 20));
    }
    std::fs::read(path).map_err(|e| coded!("workspace.read_failed", "could not read {path}: {detail}", path = path, detail = e))
}

/// Parses `map` from a request: a list of `[old, new]` pairs.
pub fn parse_map(v: &Value) -> Result<Vec<(String, String)>> {
    if v.is_null() {
        return Ok(vec![]);
    }
    let Some(list) = v.as_array() else { return Err(coded!("workspace.bad_map", "map must be a list of [old, new] pairs")) };
    if list.len() > MAX_MAP {
        return Err(coded!("workspace.map_too_big", "map has more than {max} pairs", max = MAX_MAP));
    }
    list.iter()
        .map(|pair| {
            let (Some(old), Some(new)) = (pair[0].as_str(), pair[1].as_str()) else {
                return Err(coded!("workspace.bad_map", "map must be a list of [old, new] pairs"));
            };
            if !text_ok(old, MAX_PATH) || !text_ok(new, MAX_PATH) {
                return Err(coded!("workspace.bad_map_path", "map paths must be 1 to {max} characters", max = MAX_PATH));
            }
            Ok((old.to_string(), new.to_string()))
        })
        .collect()
}

/// Reads and validates the file at `path` and says what an import would do. Writes nothing.
pub fn preview(store: &Path, path: &str, map: &[(String, String)]) -> Result<Preview> {
    let parsed = parse(&read_file(path)?)?;
    Ok(plan(store, parsed, map)?.preview)
}

// ---------------------------------------------------------------------------------------------------- import

pub struct Options {
    /// swap the state files for the file's content instead of adding to them
    pub replace: bool,
    /// on a difference, take the file's version (a merge keeps the local one otherwise)
    pub overwrite: bool,
    pub map: Vec<(String, String)>,
}

#[derive(Serialize, Default, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub projects_added: usize,
    pub projects_updated: usize,
    pub automations_added: usize,
    pub automations_updated: usize,
    pub settings_set: usize,
    pub cards_written: usize,
    pub cards_kept: usize,
    pub files: Vec<String>,
}

/// Imports the file at `path`. The caller holds the automations lock; this takes the projects one. The whole file is
/// validated and planned before the first write.
pub fn import(store: &Path, path: &str, o: &Options) -> Result<Outcome> {
    let parsed = parse(&read_file(path)?)?;
    let _g = projects::STORE_LOCK.lock_recover();
    let plan = plan(store, parsed, &o.map)?;
    let mut out = Outcome::default();
    let take = |st: St| st == St::New || (st == St::Conflict && o.overwrite);

    // projects
    let mut projects = if o.replace { vec![] } else { plan.cur_projects.clone() };
    for pp in &plan.projs {
        if o.replace {
            projects.push(pp.proj.clone());
            if pp.existing.is_some() {
                out.projects_updated += usize::from(pp.st == St::Conflict);
            } else {
                out.projects_added += 1;
            }
            continue;
        }
        match pp.existing {
            None => {
                projects.push(pp.proj.clone());
                out.projects_added += 1;
            }
            Some(i) if take(pp.st) => {
                let t = &mut projects[i];
                t.name = pp.proj.name.clone();
                t.setup = pp.proj.setup.clone();
                t.parents.extend(pp.proj.parents.clone());
                out.projects_updated += 1;
            }
            Some(_) => {}
        }
    }
    let projects_changed = o.replace || out.projects_added + out.projects_updated > 0;

    // automations
    let mut autos = if o.replace { vec![] } else { plan.cur_autos.clone() };
    for ap in &plan.autos {
        let mut a = ap.auto.clone();
        // nothing fires retroactively: a new automation starts counting now; an edited one keeps its history
        a.last_run = automations::now_ts();
        // a shared file must not schedule agents by itself: the user turns each automation on after reading its prompt
        a.enabled = false;
        if let Some(i) = ap.existing {
            let old = &plan.cur_autos[i];
            if serde_json::to_value(&old.schedule)? == serde_json::to_value(&a.schedule)? {
                a.last_run = old.last_run;
            }
        }
        if o.replace {
            autos.push(a);
            if ap.existing.is_none() {
                out.automations_added += 1;
            } else {
                out.automations_updated += usize::from(ap.st == St::Conflict);
            }
        } else if ap.existing.is_none() {
            autos.push(a);
            out.automations_added += 1;
        } else if take(ap.st) {
            if let Some(slot) = ap.existing.and_then(|i| autos.iter_mut().find(|x| x.id == plan.cur_autos[i].id)) {
                *slot = a;
                out.automations_updated += 1;
            }
        }
    }
    let autos_changed = o.replace || out.automations_added + out.automations_updated > 0;

    // settings
    let mut settings = plan.cur_settings.clone();
    if !settings.is_object() {
        settings = json!({});
    }
    if o.replace {
        if let Some(m) = settings.as_object_mut() {
            for k in SETTINGS_KEYS {
                m.remove(k);
            }
        }
    }
    for it in &plan.items {
        if !(o.replace || take(it.st)) {
            continue;
        }
        if it.ui {
            if !settings["ui"].is_object() {
                settings["ui"] = json!({});
            }
            settings["ui"][&it.key] = it.value.clone();
        } else {
            settings[&it.key] = it.value.clone();
        }
        out.settings_set += 1;
    }
    let settings_changed = settings != plan.cur_settings;

    // everything is decided: write. Projects first, then what refers to them
    let dir = data_dir(store);
    if projects_changed {
        projects::save(store, &projects)?;
        out.files.push("projects.json".into());
    }
    if autos_changed {
        automations::save_all(&automations::store_path(store), &autos)?;
        out.files.push("automations.json".into());
    }
    if settings_changed {
        write_json(&dir.join("settings.json"), &settings)?;
        out.files.push("settings.json".into());
    }
    for c in &plan.cards {
        if !take(c.st) {
            out.cards_kept += 1;
            continue;
        }
        let root = Path::new(&plan.projs[c.project].proj.path);
        if let Some(d) = c.target.parent() {
            std::fs::create_dir_all(d)?;
        }
        let _ = crate::kanban::exclude_kanban(root);
        // the previous card, when there is one, stays as `<card>.md.1`
        write_backed_up(&c.target, c.text.as_bytes())
            .map_err(|e| coded!("workspace.card_write_failed", "could not write card {id}: {detail}", id = c.id, detail = e))?;
        out.cards_written += 1;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_carry_message_codes() {
        let c = |e: anyhow::Error| axelstra_core::msg::find(&e).map(|c| c.code.clone()).unwrap_or_default();
        assert_eq!(c(parse(b"not json").map(|_| ()).unwrap_err()), "workspace.not_valid");
        assert_eq!(c(parse(b"{}").map(|_| ()).unwrap_err()), "workspace.not_workspace");
        let doc = json!({"kind": KIND, "version": VERSION + 1});
        let e = parse(doc.to_string().as_bytes()).map(|_| ()).unwrap_err();
        assert_eq!(c(e), "workspace.newer");
        let doc = json!({"kind": KIND, "version": VERSION, "settings": []});
        assert_eq!(c(parse(doc.to_string().as_bytes()).map(|_| ()).unwrap_err()), "workspace.settings_not_object");
        assert_eq!(c(read_file("").unwrap_err()), "workspace.path_required");
        assert_eq!(c(parse_map(&json!("x")).unwrap_err()), "workspace.bad_map");
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axl-ws-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        // projects are stored under their canonical path (macOS /var -> /private/var, Windows 8.3 names): so are the maps
        axelstra_core::git::canon(&d).unwrap()
    }

    /// A machine: a data dir with a projects.json at `<root>/data`.
    fn machine(root: &Path, name: &str) -> PathBuf {
        let d = root.join(name).join("data");
        std::fs::create_dir_all(&d).unwrap();
        d.join("projects.json")
    }

    fn folder(root: &Path, p: &str) -> PathBuf {
        let d = root.join(p);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn add_project(store: &Path, repo: &Path) -> Project {
        projects::add(store, &repo.to_string_lossy()).unwrap()
    }

    fn card_text(id: &str, extra: &str) -> String {
        format!("---\nid: {id}\ntitle: T {id}\nstatus: todo\n{extra}---\n\nbody of {id}\n")
    }

    fn put_card(repo: &Path, id: &str, extra: &str) {
        std::fs::create_dir_all(repo.join(".kanban")).unwrap();
        std::fs::write(repo.join(".kanban").join(format!("{id}.md")), card_text(id, extra)).unwrap();
    }

    fn auto(project: &str, id: &str) -> Automation {
        serde_json::from_value(json!({
            "id": id, "name": "daily", "project": project, "agent": "claude", "prompt": "look around",
            "schedule": {"kind": "daily", "time": "09:00"}, "enabled": true, "last_run": 5
        }))
        .unwrap()
    }

    fn opts(replace: bool, overwrite: bool) -> Options {
        Options { replace, overwrite, map: vec![] }
    }

    /// A source machine with one project, a card, an automation and settings.
    fn source(root: &Path) -> (PathBuf, PathBuf, String) {
        let store = machine(root, "src");
        let repo = folder(root, "src/repo");
        let p = add_project(&store, &repo);
        put_card(&repo, "alpha", "");
        automations::save_all(&automations::store_path(&store), &[auto(&p.id, "auto-1")]).unwrap();
        write_json(&settings_path(&store), &json!({"terse": "short", "notify_mode": "waiting", "ui": {"axelstra.pins": "[\"a\"]"}}))
            .unwrap();
        (store.clone(), repo, p.id)
    }

    fn exported(store: &Path, root: &Path) -> String {
        let f = root.join("out.json");
        export_to(store, &f).unwrap();
        f.to_string_lossy().into_owned()
    }

    #[test]
    fn round_trip_into_an_empty_machine() {
        let root = dir("roundtrip");
        let (store, repo, _) = source(&root);
        let file = exported(&store, &root);
        let dst = machine(&root, "dst");
        // the same folder exists on the other machine under another name
        let repo2 = folder(&root, "dst/repo");
        let map = vec![(repo.to_string_lossy().into_owned(), repo2.to_string_lossy().into_owned())];

        let pv = preview(&dst, &file, &map).unwrap();
        assert_eq!((pv.projects.new.len(), pv.automations.new.len(), pv.cards.new.len()), (1, 1, 1));
        assert_eq!(pv.settings.new.len(), 3);
        assert!(!dst.exists(), "preview wrote");
        assert_eq!(pv.risky_automations.len(), 1);
        assert!(!pv.risky_automations[0].prompt.is_empty(), "the prompt must be visible in the preview");

        let out = import(&dst, &file, &Options { replace: false, overwrite: false, map }).unwrap();
        assert_eq!((out.projects_added, out.automations_added, out.cards_written), (1, 1, 1));
        let ps = projects::try_load(&dst).unwrap();
        assert_eq!(ps.len(), 1);
        assert!(paths::same(&ps[0].path, &repo2.canonicalize().unwrap().to_string_lossy().replace(r"\\?\", "")));
        let autos = automations::load(&automations::store_path(&dst));
        assert_eq!((autos.len(), autos[0].project.as_str()), (1, ps[0].id.as_str()));
        assert_ne!(autos[0].last_run, 5, "a new automation must not fire retroactively");
        assert!(!autos[0].enabled, "an imported automation must start disabled");
        let s: Value = read_json(&settings_path(&dst)).unwrap().unwrap();
        assert_eq!(s["terse"], "short");
        assert_eq!(s["ui"]["axelstra.pins"], "[\"a\"]");
        assert_eq!(std::fs::read_to_string(repo2.join(".kanban/alpha.md")).unwrap(), card_text("alpha", ""));
        // importing again changes nothing
        let again = preview(&dst, &file, &[(repo.to_string_lossy().into_owned(), repo2.to_string_lossy().into_owned())]).unwrap();
        assert!(again.projects.new.is_empty() && again.cards.new.is_empty() && again.automations.new.is_empty());
        assert_eq!(again.cards.present.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn export_never_carries_tokens_sessions_or_logs() {
        let root = dir("scrub");
        let (store, repo, _) = source(&root);
        let data = store.parent().unwrap();
        std::fs::write(data.join("token"), "MASTER-TOKEN-1").unwrap();
        std::fs::write(data.join("sessions.json"), r#"{"v":1,"data":[{"agent_token":"AGENT-TOKEN-2","scrollback":"SCROLL-3"}]}"#).unwrap();
        std::fs::write(data.join("crash.log"), "LOG-4").unwrap();
        write_json(
            &settings_path(&store),
            &json!({
                "terse": "tiny", "token": "SETTINGS-TOKEN-5", "agent_token": "AGENT-6", "api_key": "KEY-7",
                "ui": {"axelstra.pins": "ok", "axelstra.apiToken": "UI-TOKEN-8", "axelstra.secretNote": "SECRET-9"}
            }),
        )
        .unwrap();
        let file = exported(&store, &root);
        let text = std::fs::read_to_string(file).unwrap();
        for leak in
            ["MASTER-TOKEN-1", "AGENT-TOKEN-2", "SCROLL-3", "LOG-4", "SETTINGS-TOKEN-5", "AGENT-6", "KEY-7", "UI-TOKEN-8", "SECRET-9"]
        {
            assert!(!text.contains(leak), "{leak} leaked");
        }
        let doc: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["settings"]["terse"], "tiny");
        assert_eq!(doc["settings"]["ui"]["axelstra.pins"], "ok");
        assert!(doc["settings"].get("token").is_none());
        assert_eq!(doc["kind"], KIND);
        assert!(doc.get("sessions").is_none());
        let _ = repo;
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_token_smuggled_into_the_file_is_not_imported() {
        let root = dir("smuggle");
        let (store, _, _) = source(&root);
        let file = exported(&store, &root);
        let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        doc["settings"]["token"] = json!("EVIL");
        doc["settings"]["ui"]["axelstra.agentToken"] = json!("EVIL2");
        std::fs::write(&file, serde_json::to_vec(&doc).unwrap()).unwrap();
        let dst = machine(&root, "dst");
        import(&dst, &file, &opts(false, false)).unwrap();
        let s = std::fs::read_to_string(settings_path(&dst)).unwrap();
        assert!(!s.contains("EVIL"), "{s}");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn assert_rejected_and_nothing_written(root: &Path, file: &Path, needle: &str) {
        let dst = machine(root, "victim");
        let repo = folder(root, "victim/repo");
        add_project(&dst, &repo);
        write_json(&settings_path(&dst), &json!({"terse": "off"})).unwrap();
        let before: Vec<_> = std::fs::read_dir(dst.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        let f = file.to_string_lossy();
        for replace in [false, true] {
            let e = import(&dst, &f, &opts(replace, true)).unwrap_err().to_string();
            assert!(e.contains(needle), "{e}");
        }
        assert!(preview(&dst, &f, &[]).is_err());
        let mut after: Vec<_> = std::fs::read_dir(dst.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        let mut before = before;
        before.sort();
        after.sort();
        assert_eq!(before, after);
        assert!(!repo.join(".kanban").exists());
        assert_eq!(read_json::<Value>(&settings_path(&dst)).unwrap().unwrap()["terse"], "off");
        let _ = std::fs::remove_dir_all(root.join("victim"));
    }

    fn doc_with(root: &Path, edit: impl FnOnce(&mut Value)) -> PathBuf {
        let (store, _, _) = source(root);
        let file = exported(&store, root);
        let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        edit(&mut doc);
        let f = root.join("hostile.json");
        std::fs::write(&f, serde_json::to_vec(&doc).unwrap()).unwrap();
        f
    }

    #[test]
    fn hostile_input_writes_nothing() {
        let root = dir("hostile");
        // a file that is not JSON, not ours, from the future
        std::fs::write(root.join("junk.json"), "{ nope").unwrap();
        assert_rejected_and_nothing_written(&root, &root.join("junk.json"), "not a valid workspace file");
        let f = doc_with(&root, |d| d["kind"] = json!("something-else"));
        assert_rejected_and_nothing_written(&root, &f, "not an Axelstra workspace file");
        let f = doc_with(&root, |d| d["version"] = json!(2));
        assert_rejected_and_nothing_written(&root, &f, "newer version");
        let f = doc_with(&root, |d| d["version"] = json!("1"));
        assert_rejected_and_nothing_written(&root, &f, "version");
        // traversal and absolute card names
        for bad in ["../evil.md", "..\\evil.md", "/etc/x.md", "a/b.md", "a.txt", ".md", "x y.md"] {
            let f = doc_with(&root, |d| {
                let pid = d["projects"][0]["id"].as_str().unwrap().to_string();
                d["kanban"][&pid][0]["file"] = json!(bad);
            });
            assert_rejected_and_nothing_written(&root, &f, "not allowed");
        }
        // a card whose id escapes
        let f = doc_with(&root, |d| {
            let pid = d["projects"][0]["id"].as_str().unwrap().to_string();
            d["kanban"][&pid][0]["text"] = json!(card_text("../../x", ""));
        });
        assert_rejected_and_nothing_written(&root, &f, "invalid");
        // a card of a project that is not in the file
        let f = doc_with(&root, |d| d["kanban"]["ghost"] = json!([]));
        assert_rejected_and_nothing_written(&root, &f, "not in the file");
        // a bad automation, a bad setting, a bad project id, a duplicate project
        let f = doc_with(&root, |d| d["automations"][0]["agent"] = json!("rm -rf"));
        assert_rejected_and_nothing_written(&root, &f, "invalid");
        let f = doc_with(&root, |d| d["settings"]["shell"] = json!("zsh"));
        assert_rejected_and_nothing_written(&root, &f, "setting shell");
        let f = doc_with(&root, |d| d["projects"][0]["id"] = json!("../x"));
        assert_rejected_and_nothing_written(&root, &f, "not allowed");
        let f = doc_with(&root, |d| {
            let p = d["projects"][0].clone();
            d["projects"].as_array_mut().unwrap().push(p);
        });
        assert_rejected_and_nothing_written(&root, &f, "two projects");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn oversized_input_is_refused() {
        let root = dir("big");
        let f = doc_with(&root, |d| {
            let pid = d["projects"][0]["id"].as_str().unwrap().to_string();
            d["kanban"][&pid][0]["text"] = json!(format!("{}{}", card_text("alpha", ""), "x".repeat(MAX_CARD_BYTES)));
        });
        assert_rejected_and_nothing_written(&root, &f, "larger than");
        let f = doc_with(&root, |d| {
            let pid = d["projects"][0]["id"].as_str().unwrap().to_string();
            d["kanban"][&pid] = Value::Array(
                (0..MAX_CARDS_PER_PROJECT + 1)
                    .map(|i| json!({"file": format!("c{i}.md"), "text": card_text(&format!("c{i}"), "")}))
                    .collect(),
            );
        });
        assert_rejected_and_nothing_written(&root, &f, "more than");
        let f = doc_with(&root, |d| {
            d["projects"] = Value::Array((0..MAX_PROJECTS + 1).map(|i| json!({"id": format!("p{i}"), "name": "n", "path": "/x"})).collect())
        });
        assert_rejected_and_nothing_written(&root, &f, "more than");
        // bigger than the whole-file limit is refused before parsing
        let huge = root.join("huge.json");
        let f = std::fs::File::create(&huge).unwrap();
        f.set_len(MAX_FILE + 1).unwrap();
        assert!(read_file(&huge.to_string_lossy()).unwrap_err().to_string().contains("larger"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_folder_is_skipped_with_its_cards_and_automations() {
        let root = dir("skip");
        let (store, _, _) = source(&root);
        let file = exported(&store, &root);
        let dst = machine(&root, "dst");
        let pv = preview(&dst, &file, &[]).unwrap();
        // the folder of the source machine is also on this one (same temp dir), so make the path point nowhere
        assert_eq!(pv.projects.new.len(), 1);
        let gone = vec![(root.join("src/repo").to_string_lossy().into_owned(), root.join("nowhere").to_string_lossy().into_owned())];
        let pv = preview(&dst, &file, &gone).unwrap();
        assert_eq!(pv.projects.skipped.len(), 1);
        assert_eq!(pv.projects.skipped[0].reason, "missing");
        assert_eq!((pv.cards.skipped.len(), pv.automations.skipped.len()), (1, 1));
        let out = import(&dst, &file, &Options { replace: false, overwrite: true, map: gone }).unwrap();
        assert_eq!(out, Outcome { settings_set: 3, files: vec!["settings.json".into()], ..Default::default() });
        assert!(!dst.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_keeps_what_is_there_unless_told_otherwise() {
        let root = dir("merge");
        let (store, repo, pid) = source(&root);
        let file = exported(&store, &root);
        // the destination already has the project (renamed), the card (edited), the automation (edited) and a setting
        let dst = machine(&root, "dst");
        let mut p = add_project(&dst, &repo);
        p.name = "Local name".into();
        projects::save(&dst, &[p]).unwrap();
        put_card(&repo, "alpha", "priority: 9\n");
        let mut a = auto(&pid, "auto-1");
        a.prompt = "local prompt".into();
        a.last_run = 77;
        automations::save_all(&automations::store_path(&dst), &[a]).unwrap();
        write_json(&settings_path(&dst), &json!({"terse": "tiny", "shell": "bash"})).unwrap();

        let pv = preview(&dst, &file, &[]).unwrap();
        assert_eq!((pv.projects.conflict.len(), pv.automations.conflict.len(), pv.cards.conflict.len()), (1, 1, 1));
        assert_eq!(pv.settings.conflict, ["terse"]);
        assert_eq!(pv.settings.local_only, 1);

        let local_card = std::fs::read_to_string(repo.join(".kanban/alpha.md")).unwrap();
        let out = import(&dst, &file, &opts(false, false)).unwrap();
        assert_eq!((out.projects_updated, out.automations_updated, out.cards_written, out.cards_kept), (0, 0, 0, 1));
        assert_eq!(projects::try_load(&dst).unwrap()[0].name, "Local name");
        assert_eq!(automations::load(&automations::store_path(&dst))[0].prompt, "local prompt");
        assert_eq!(std::fs::read_to_string(repo.join(".kanban/alpha.md")).unwrap(), local_card);
        let s: Value = read_json(&settings_path(&dst)).unwrap().unwrap();
        assert_eq!((s["terse"].as_str(), s["shell"].as_str(), s["notify_mode"].as_str()), (Some("tiny"), Some("bash"), Some("waiting")));

        // asked to overwrite: the file wins, the card it replaces stays as .md.1, history of the automation stays
        let out = import(&dst, &file, &opts(false, true)).unwrap();
        assert_eq!((out.projects_updated, out.automations_updated, out.cards_written), (1, 1, 1));
        assert_ne!(projects::try_load(&dst).unwrap()[0].name, "Local name");
        let a = &automations::load(&automations::store_path(&dst))[0];
        assert_eq!((a.prompt.as_str(), a.last_run), ("look around", 77));
        assert_eq!(std::fs::read_to_string(repo.join(".kanban/alpha.md")).unwrap(), card_text("alpha", ""));
        assert_eq!(std::fs::read_to_string(repo.join(".kanban/alpha.md.1")).unwrap(), local_card);
        assert_eq!(read_json::<Value>(&settings_path(&dst)).unwrap().unwrap()["terse"], "short");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn replace_swaps_the_state_files_and_keeps_the_old_ones() {
        let root = dir("replace");
        let (store, repo, _) = source(&root);
        let file = exported(&store, &root);
        let dst = machine(&root, "dst");
        let other = folder(&root, "dst/other");
        add_project(&dst, &other);
        write_json(&settings_path(&dst), &json!({"terse": "tiny", "pm": "npm"})).unwrap();
        let old_projects = std::fs::read_to_string(&dst).unwrap();
        // the cards of the project the file does not mention are not touched
        put_card(&other, "mine", "");

        let pv = preview(&dst, &file, &[]).unwrap();
        assert_eq!(pv.projects.local_only, 1);
        import(&dst, &file, &opts(true, false)).unwrap();
        let ps = projects::try_load(&dst).unwrap();
        assert_eq!(ps.len(), 1);
        assert!(paths::same(&ps[0].path, &repo.canonicalize().unwrap().to_string_lossy().replace(r"\\?\", "")));
        assert_eq!(std::fs::read_to_string(dst.with_file_name("projects.json.1")).unwrap(), old_projects);
        let s: Value = read_json(&settings_path(&dst)).unwrap().unwrap();
        assert_eq!((s["terse"].as_str(), s["pm"].as_str()), (Some("short"), None));
        assert!(dst.with_file_name("settings.json.1").is_file());
        assert!(other.join(".kanban/mine.md").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn replace_still_never_overwrites_a_card_unless_asked() {
        let root = dir("replcard");
        let (store, repo, _) = source(&root);
        let file = exported(&store, &root);
        let dst = machine(&root, "dst");
        add_project(&dst, &repo);
        put_card(&repo, "alpha", "priority: 3\n");
        let before = std::fs::read_to_string(repo.join(".kanban/alpha.md")).unwrap();
        import(&dst, &file, &opts(true, false)).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join(".kanban/alpha.md")).unwrap(), before);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn project_id_collisions_are_renamed_and_automations_follow() {
        let root = dir("idclash");
        let (store, _, pid) = source(&root);
        let file = exported(&store, &root);
        let dst = machine(&root, "dst");
        // another folder already uses the id of the file's project
        let mine = folder(&root, "dst/mine");
        let mut p = add_project(&dst, &mine);
        p.id = pid.clone();
        projects::save(&dst, &[p]).unwrap();
        import(&dst, &file, &opts(false, false)).unwrap();
        let ps = projects::try_load(&dst).unwrap();
        assert_eq!(ps.len(), 2);
        assert_ne!(ps[0].id, ps[1].id);
        let a = &automations::load(&automations::store_path(&dst))[0];
        assert_eq!(a.project, ps[1].id);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn risky_cards_are_counted() {
        let root = dir("risky");
        let (store, repo, _) = source(&root);
        put_card(&repo, "bypass", "permission_mode: bypassPermissions\n");
        put_card(&repo, "accept", "acceptance: \"echo ok\"\n");
        let file = exported(&store, &root);
        let dst = machine(&root, "dst");
        let repo2 = folder(&root, "dst/repo");
        let map = [(repo.to_string_lossy().into_owned(), repo2.to_string_lossy().into_owned())];
        let pv = preview(&dst, &file, &map).unwrap();
        assert_eq!((pv.cards.new.len(), pv.risky_cards), (3, 2));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remap_works_on_whole_components_only() {
        let m = vec![("/a/b".to_string(), "/x".to_string())];
        assert_eq!(remap("/a/b/c", &m), "/x/c");
        assert_eq!(remap("/a/b", &m), "/x");
        assert_eq!(remap("/a/bc", &m), "/a/bc");
        assert_eq!(remap("/q", &m), "/q");
        let m = vec![("/a/b".to_string(), "/x/".to_string()), ("/a".to_string(), "/y".to_string())];
        assert_eq!(remap("/a/b/c", &m), "/x/c");
        assert_eq!(remap("/a/z", &m), "/y/z");
    }

    #[test]
    fn map_parsing_is_strict() {
        assert!(parse_map(&Value::Null).unwrap().is_empty());
        assert_eq!(parse_map(&json!([["/a", "/b"]])).unwrap(), [("/a".to_string(), "/b".to_string())]);
        assert!(parse_map(&json!("x")).is_err());
        assert!(parse_map(&json!([["/a"]])).is_err());
        assert!(parse_map(&json!([["", "/b"]])).is_err());
    }

    #[test]
    fn export_refuses_the_data_folder_and_missing_folders() {
        let root = dir("target");
        let (store, _, _) = source(&root);
        assert!(export_to(&store, &store.parent().unwrap().join("token")).is_err());
        assert!(export_to(&store, &root.join("nope/x.json")).is_err());
        assert!(export_to(&store, &root).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
