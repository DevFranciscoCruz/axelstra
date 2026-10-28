//! `scanx.*`: what keeps antivirus and indexers off the `.axelstra-wt` folders (logic in `crate::scanx`). All of them are the
//! user's: an agent connection is refused (`agent_forbidden`), because a Defender exclusion is a security decision.
//!
//! `scanx.defender_apply` / `scanx.defender_revert` wait for a UAC prompt, so they run as a job: the call answers
//! `{started}` and the result arrives as the watch-stream event `scanx.changed {op, ok, changed, unchanged, error,
//! error_code, error_params}`; `scanx.status` carries the running job and the last result.

use super::*;
use crate::scanx::{self, Fs, Op, Os, Runner};
use std::path::Path;

/// The job running now and the result of the last one.
#[derive(Default)]
struct Job {
    running: Option<&'static str>,
    last: Option<Value>,
}

static JOB: Mutex<Job> = Mutex::new(Job { running: None, last: None });
/// Serializes read-modify-write of the `scanx` key of settings.json.
static SAVE: Mutex<()> = Mutex::new(());

fn op_name(op: Op) -> &'static str {
    match op {
        Op::Add => "apply",
        Op::Remove => "revert",
    }
}

fn audit_method(op: Op) -> &'static str {
    match op {
        Op::Add => "scanx.defender_apply",
        Op::Remove => "scanx.defender_revert",
    }
}

fn projects_of(reg: &Registry) -> Vec<(String, String, String)> {
    crate::projects::load(&reg.store).into_iter().map(|p| (p.id, p.name, p.path)).collect()
}

fn saved(reg: &Registry) -> scanx::Saved {
    scanx::Saved::from_settings(&reg.settings())
}

fn update_saved(reg: &Registry, f: impl FnOnce(&mut scanx::Saved)) -> Result<()> {
    let _g = SAVE.lock().unwrap_or_else(PoisonError::into_inner);
    let mut settings = reg.settings();
    let mut sv = scanx::Saved::from_settings(&settings);
    f(&mut sv);
    sv.write_into(&mut settings);
    let written = write_json(&reg.saved.with_file_name("settings.json"), &settings);
    reg.settings_changed();
    Ok(written?)
}

fn changed_event(reg: &Registry, extra: Value) {
    let mut ev = json!({"event": "scanx.changed"});
    if let (Some(o), Some(x)) = (ev.as_object_mut(), extra.as_object()) {
        o.extend(x.clone());
    }
    reg.broadcast_event(ev);
}

/// `scanx.status`.
fn status(reg: &Registry, os: Os, fs: &dyn Fs, runner: &dyn Runner) -> Value {
    let folders = scanx::folders_of(&projects_of(reg));
    let query =
        (os == Os::Windows).then(|| runner.query(&scanx::status_script(&scanx::drives_of(&folders))).map(|raw| scanx::parse_query(&raw)));
    let (q, err) = match query {
        Some(Ok(q)) => (Some(q), None),
        Some(Err(e)) => (None, Some(e)),
        None => (None, None),
    };
    let mut v = scanx::build_status(os, fs, &folders, q.as_ref(), &saved(reg));
    if let Some(e) = err {
        v["error"] = json!(format!("{e:#}"));
        if let Some(c) = axelstra_core::msg::find(&e) {
            v["error_code"] = json!(c.code);
            v["error_params"] = c.params_json();
        }
    }
    let job = JOB.lock().unwrap_or_else(PoisonError::into_inner);
    v["job"] = json!({"running": job.running, "last": job.last});
    v
}

/// Adds or removes the exclusions and records what happened (settings, audit log). Blocks until the UAC prompt is answered.
fn run_job(reg: &Registry, os: Os, runner: &dyn Runner, op: Op) -> Result<Value> {
    if os != Os::Windows {
        return Err(axelstra_core::coded!("scanx.unsupported", "Defender exclusions exist only on Windows"));
    }
    let folders = scanx::folders_of(&projects_of(reg));
    let sv = saved(reg);
    let mut paths = scanx::unique_paths(&folders);
    if op == Op::Remove {
        // a folder we added for a project that is gone since is ours to take back as well
        for r in &sv.applied {
            if !paths.iter().any(|p| scanx::canon(p) == scanx::canon(r)) {
                paths.push(r.clone());
            }
        }
    }
    let res = scanx::run_change(runner, op, &paths);
    reg.audit.record_system(audit_method(op), &json!({"paths": paths}), &res.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
    let change = res?;
    update_saved(reg, |s| {
        s.applied = scanx::recorded_after(&s.applied, op, &paths);
        if op == Op::Add {
            s.dismissed = true;
            s.pending = false;
        }
    })?;
    Ok(json!({"changed": change.changed, "unchanged": change.unchanged}))
}

/// Starts `run_job` on its own thread, one at a time.
fn start_job(reg: &Arc<Registry>, op: Op) -> Result<Value> {
    if Os::current() != Os::Windows {
        return Err(axelstra_core::coded!("scanx.unsupported", "Defender exclusions exist only on Windows"));
    }
    {
        let mut job = JOB.lock().unwrap_or_else(PoisonError::into_inner);
        if job.running.is_some() {
            return Err(axelstra_core::coded!("scanx.busy", "another Defender change is waiting for the administrator prompt"));
        }
        job.running = Some(op_name(op));
    }
    let reg = reg.clone();
    std::thread::spawn(move || {
        let res = run_job(&reg, Os::current(), &scanx::PsRunner, op);
        let mut ev = json!({"op": op_name(op), "ok": res.is_ok()});
        match &res {
            Ok(v) => {
                ev["changed"] = v["changed"].clone();
                ev["unchanged"] = v["unchanged"].clone();
            }
            Err(e) => {
                ev["error"] = json!(format!("{e:#}"));
                if let Some(c) = axelstra_core::msg::find(e) {
                    ev["error_code"] = json!(c.code);
                    ev["error_params"] = c.params_json();
                }
            }
        }
        {
            let mut job = JOB.lock().unwrap_or_else(PoisonError::into_inner);
            job.running = None;
            job.last = Some(ev.clone());
        }
        changed_event(&reg, ev);
    });
    Ok(json!({"started": true, "op": op_name(op)}))
}

/// Whether the one-time notice is due for a `.axelstra-wt` folder that was just created.
fn hint_check(reg: &Registry, runner: &dyn Runner, root: &str) -> bool {
    let sv = saved(reg);
    if sv.shown || sv.dismissed {
        return false;
    }
    let drives: Vec<char> = scanx::drives_of(&[scanx::Folder { project: String::new(), name: String::new(), path: root.to_string() }]);
    let Ok(raw) = runner.query(&scanx::status_script(&drives)) else { return false };
    let q = scanx::parse_query(&raw);
    let fs_name = drives.first().and_then(|d| q.vols.get(d)).cloned();
    let due = scanx::hint_due(&scanx::HintIn {
        new_root: true,
        defender_active: q.active(),
        dev_drive: fs_name.as_deref().is_some_and(|f| f.eq_ignore_ascii_case("ReFS")),
        excluded: q.exclusions.as_ref().is_some_and(|l| l.iter().any(|e| scanx::canon(root).starts_with(&scanx::canon(e)))),
        saved_shown: sv.shown,
        saved_dismissed: sv.dismissed,
    });
    if due {
        let _ = update_saved(reg, |s| {
            s.shown = true;
            s.pending = true;
        });
    }
    due
}

/// True when the `.axelstra-wt` folder of the project does not exist yet (about to be created for the first time).
pub(super) fn root_is_new(repo: &str) -> bool {
    scanx::root_of(Path::new(repo)).is_some_and(|r| !r.exists())
}

/// A `.axelstra-wt` folder was just created: the notice may be due. Windows only, off the request, never in tests.
pub(super) fn root_created(reg: &Arc<Registry>, repo: &str) {
    let Some(root) = scanx::root_of(Path::new(repo)).map(|r| scanx::display_path(&r)) else { return };
    if crate::proto::testing() || Os::current() != Os::Windows {
        return;
    }
    let reg = reg.clone();
    let _ = std::thread::Builder::new().name("scanx-hint".into()).spawn(move || {
        if hint_check(&reg, &scanx::PsRunner, &root) {
            changed_event(&reg, json!({"op": "hint", "ok": true}));
        }
    });
}

/// The orchestrator makes its worktrees in this folder: protect it before the first one (a run can be the first thing to create it).
pub(super) fn run_root_ready(reg: &Arc<Registry>, repo: &Path, wt_root: &Path) {
    let new = !wt_root.exists();
    let _ = std::fs::create_dir_all(wt_root);
    scanx::on_root_ready(wt_root);
    if new {
        root_created(reg, &repo.to_string_lossy());
    }
}

/// Daemon start: existing `.axelstra-wt` folders of the registered projects, in a low-priority thread.
pub(super) fn spawn_startup(reg: &Arc<Registry>) {
    let mut roots: Vec<std::path::PathBuf> = vec![];
    for (_, _, path) in projects_of(reg) {
        if let Some(r) = scanx::root_of(Path::new(&path)) {
            if !roots.iter().any(|x| axelstra_core::paths::same(&x.to_string_lossy(), &r.to_string_lossy())) {
                roots.push(r);
            }
        }
    }
    scanx::spawn_sweep(roots);
}

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let (req, reg) = (c.req, c.reg);
    match req.method.as_str() {
        "scanx.status" => Ok(tokio::task::block_in_place(|| status(reg, Os::current(), &scanx::RealFs, &scanx::PsRunner))),
        "scanx.defender_apply" => start_job(reg, Op::Add),
        "scanx.defender_revert" => start_job(reg, Op::Remove),
        "scanx.diag" => {
            if Os::current() != Os::Windows {
                return Ok(json!({"os": Os::current().name(), "state": "n/a", "procs": []}));
            }
            let raw = tokio::task::block_in_place(|| scanx::PsRunner.query(&scanx::procs_script()))?;
            Ok(json!({"os": "windows", "state": "ok", "procs": scanx::parse_procs(&raw)}))
        }
        "scanx.hint_dismiss" => {
            update_saved(reg, |s| {
                s.dismissed = true;
                s.pending = false;
            })?;
            changed_event(reg, json!({"op": "hint", "ok": true}));
            Ok(json!({}))
        }
        m => Err(unknown_method(m)),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const REPO: &str = r"C:\Users\x\Paradise Group\proj";
    const WT: &str = r"C:\Users\x\Paradise Group\.axelstra-wt";

    struct Defender {
        list: RefCell<Vec<String>>,
        status: String,
        calls: RefCell<usize>,
    }
    unsafe impl Sync for Defender {}

    impl Runner for Defender {
        fn query(&self, _: &str) -> Result<String> {
            Ok(self.status.clone())
        }
        fn elevate(&self, op: Op, paths: &[String]) -> Result<String> {
            *self.calls.borrow_mut() += 1;
            let before = self.list.borrow().clone();
            let mut l = self.list.borrow_mut();
            for p in paths {
                let at = l.iter().position(|x| scanx::canon(x) == scanx::canon(p));
                match (op, at) {
                    (Op::Add, None) => l.push(p.clone()),
                    (Op::Remove, Some(i)) => {
                        l.remove(i);
                    }
                    _ => {}
                }
            }
            Ok(json!({"ok": true, "before": before, "after": *l, "error": null}).to_string())
        }
    }

    fn defender(status: &str) -> Defender {
        Defender { list: RefCell::new(vec![]), status: status.into(), calls: RefCell::new(0) }
    }

    const ACTIVE: &str = r#"{"defender":true,"av":true,"rtp":true,"tamper":true,"excl":["N/A: Must be an administrator to view exclusions"],"vols":{"C":"NTFS"}}"#;

    fn registry(name: &str) -> (Arc<Registry>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("axl-scanx-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let reg = Arc::new(Registry::new(dir.join("projects.json"), "p".into(), "t".into()));
        let p: crate::projects::Project = serde_json::from_value(json!({"id": "p1", "name": "proj", "path": REPO})).unwrap();
        crate::projects::save(&reg.store, &[p]).unwrap();
        (reg, dir)
    }

    #[test]
    fn apply_records_and_audits_and_a_second_apply_adds_nothing() {
        let (reg, dir) = registry("apply");
        let d = defender(ACTIVE);
        let first = run_job(&reg, Os::Windows, &d, Op::Add).unwrap();
        assert_eq!(first["changed"], json!([WT]));
        assert_eq!(saved(&reg).applied, vec![WT.to_string()]);
        assert!(saved(&reg).dismissed, "the notice is done once the user applied");
        let second = run_job(&reg, Os::Windows, &d, Op::Add).unwrap();
        assert_eq!((second["changed"].as_array().unwrap().len(), second["unchanged"].as_array().unwrap().len()), (0, 1));
        assert_eq!(d.list.borrow().len(), 1);
        assert_eq!(saved(&reg).applied.len(), 1);
        let entries = reg.audit.list(10, None);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["method"], "scanx.defender_apply");
        assert_eq!(entries[0]["session"], 0);
        assert_eq!(entries[0]["ok"], true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn revert_takes_back_the_recorded_paths_even_for_a_project_that_is_gone() {
        let (reg, dir) = registry("revert");
        let d = defender(ACTIVE);
        run_job(&reg, Os::Windows, &d, Op::Add).unwrap();
        d.list.borrow_mut().push(r"C:\user\own".into());
        // the project leaves the list; its folder was ours
        crate::projects::save(&reg.store, &[]).unwrap();
        let back = run_job(&reg, Os::Windows, &d, Op::Remove).unwrap();
        assert_eq!(back["changed"], json!([WT]));
        assert_eq!(*d.list.borrow(), vec![r"C:\user\own".to_string()]);
        assert!(saved(&reg).applied.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_runs_off_windows_or_without_folders() {
        let (reg, dir) = registry("n-a");
        let d = defender(ACTIVE);
        let e = run_job(&reg, Os::Other, &d, Op::Add).unwrap_err();
        assert_eq!(axelstra_core::msg::find(&e).unwrap().code, "scanx.unsupported");
        crate::projects::save(&reg.store, &[]).unwrap();
        let e = run_job(&reg, Os::Windows, &d, Op::Add).unwrap_err();
        assert_eq!(axelstra_core::msg::find(&e).unwrap().code, "scanx.no_folders");
        assert_eq!(*d.calls.borrow(), 0);
        let s = status(&reg, Os::Other, &scanx::RealFs, &d);
        assert_eq!(s["defender"]["state"], "n/a");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_notice_is_raised_once_and_dismissal_is_final() {
        let (reg, dir) = registry("hint");
        let d = defender(ACTIVE);
        assert!(hint_check(&reg, &d, WT));
        assert!(status(&reg, Os::Windows, &scanx::RealFs, &d)["hint"].as_bool().unwrap());
        assert!(!hint_check(&reg, &d, WT), "never twice");
        update_saved(&reg, |s| {
            s.dismissed = true;
            s.pending = false;
        })
        .unwrap();
        assert!(!status(&reg, Os::Windows, &scanx::RealFs, &d)["hint"].as_bool().unwrap());
        // a Dev Drive never gets it
        let (reg2, dir2) = registry("hint-dev");
        let dev = defender(&ACTIVE.replace("NTFS", "ReFS"));
        assert!(!hint_check(&reg2, &dev, WT));
        // nor does a machine whose Defender is not the active antivirus
        let off = defender(r#"{"defender":true,"av":false,"rtp":false,"vols":{"C":"NTFS"}}"#);
        assert!(!hint_check(&reg2, &off, WT));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }
}
