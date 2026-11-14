//! Watched folder (Milestone 15): while the app is open, new table files
//! dropped into a folder are run through a saved recipe and the result is
//! written to an output folder. The files dropped in are never changed.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use natendo_engine::recipe::Recipe;
use natendo_engine::{Document, OpenOptions, Progress, Quoting, SaveOptions};
use serde::Serialize;

use crate::extras::Recent;

/// File types picked up.
const TYPES: &[&str] = &["csv", "tsv", "txt", "psv", "tab"];
/// Lines kept in the log.
const MAX_LOG: usize = 500;
/// How often the folder is looked at.
const EVERY: Duration = Duration::from_secs(5);

/// What the screen shows.
#[derive(Debug, Clone, Default, Serialize)]
pub struct WatchStatus {
    pub running: bool,
    pub folder: Option<String>,
    pub recipe: Option<String>,
    pub out: Option<String>,
    pub done: u64,
    pub failed: u64,
    /// Newest first.
    pub log: Vec<String>,
}

/// The chosen folders and recipe (chosen in the app's own dialogs only).
#[derive(Default)]
pub struct WatchSetup {
    pub folder: Option<PathBuf>,
    pub recipe: Option<(PathBuf, Recipe)>,
    pub out: Option<PathBuf>,
}

#[derive(Default)]
pub struct Watcher {
    pub setup: Mutex<WatchSetup>,
    stop: Mutex<Option<Arc<AtomicBool>>>,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    running: bool,
    done: u64,
    failed: u64,
    log: VecDeque<String>,
}

impl State {
    fn note(&mut self, line: String) {
        let stamp = clock();
        self.log.push_front(format!("{stamp}  {line}"));
        self.log.truncate(MAX_LOG);
    }
}

fn clock() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // Local time is not known here without a time zone library: UTC.
    let s = secs % 86_400;
    format!("{:02}:{:02}:{:02} UTC", s / 3600, s / 60 % 60, s % 60)
}

impl Watcher {
    pub fn status(&self) -> WatchStatus {
        let st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let setup = self.setup.lock().unwrap_or_else(|p| p.into_inner());
        WatchStatus {
            running: st.running,
            folder: setup.folder.as_ref().map(|p| p.display().to_string()),
            recipe: setup.recipe.as_ref().map(|(p, _)| p.display().to_string()),
            out: setup.out.as_ref().map(|p| p.display().to_string()),
            done: st.done,
            failed: st.failed,
            log: st.log.iter().cloned().collect(),
        }
    }

    /// Start watching with the chosen setup. Files already in the folder
    /// are run too when `existing`.
    pub fn start(&self, force: bool, existing: bool, recent: Arc<Recent>) -> Result<(), String> {
        let (folder, recipe, out) = {
            let s = self.setup.lock().unwrap_or_else(|p| p.into_inner());
            match (&s.folder, &s.recipe, &s.out) {
                (Some(f), Some((_, r)), Some(o)) => (f.clone(), r.clone(), o.clone()),
                _ => {
                    return Err(
                        "choose the folder to watch, the recipe and the output folder".into(),
                    )
                }
            }
        };
        if same_dir(&folder, &out) || out.starts_with(&folder) {
            return Err("the output folder must not be the watched folder or inside it".into());
        }
        self.stop_now();
        let stop = Arc::new(AtomicBool::new(false));
        *self.stop.lock().unwrap_or_else(|p| p.into_inner()) = Some(stop.clone());
        let state = self.state.clone();
        {
            let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
            st.running = true;
            st.note(format!(
                "Watching {} ({} step recipe)",
                folder.display(),
                recipe.steps.len()
            ));
        }
        std::thread::spawn(move || {
            run(
                &folder, &out, &recipe, force, existing, &stop, &state, &recent,
            );
            let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
            st.running = false;
            st.note("Stopped watching".into());
        });
        Ok(())
    }

    pub fn stop_now(&self) {
        if let Some(s) = self.stop.lock().unwrap_or_else(|p| p.into_inner()).take() {
            s.store(true, Ordering::Relaxed);
        }
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Files of the watched folder with their size and change time.
fn listing(dir: &Path) -> HashMap<PathBuf, (u64, Option<SystemTime>)> {
    let mut out = HashMap::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten().take(20_000) {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name.starts_with('~') {
            continue;
        }
        let ok = p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| TYPES.iter().any(|t| t.eq_ignore_ascii_case(x)));
        if !ok {
            continue;
        }
        if let Ok(m) = e.metadata() {
            if m.is_file() {
                out.insert(p, (m.len(), m.modified().ok()));
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn run(
    folder: &Path,
    out: &Path,
    recipe: &Recipe,
    force: bool,
    existing: bool,
    stop: &AtomicBool,
    state: &Mutex<State>,
    recent: &Recent,
) {
    // Files already there count as done unless asked otherwise.
    let mut done: HashMap<PathBuf, (u64, Option<SystemTime>)> = if existing {
        HashMap::new()
    } else {
        listing(folder)
    };
    // A file is run once its size and time stayed the same for one look
    // (it is not being written any more).
    let mut last: HashMap<PathBuf, (u64, Option<SystemTime>)> = HashMap::new();
    while !stop.load(Ordering::Relaxed) {
        let now = listing(folder);
        for (p, sig) in &now {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            if done.get(p) == Some(sig) || last.get(p) != Some(sig) {
                continue;
            }
            done.insert(p.clone(), *sig);
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let line = match run_one(p, out, recipe, force, recent) {
                Ok(msg) => {
                    let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
                    st.done += 1;
                    format!("{name}: {msg}")
                }
                Err(e) => {
                    let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
                    st.failed += 1;
                    format!("{name}: not done: {e}")
                }
            };
            state.lock().unwrap_or_else(|e| e.into_inner()).note(line);
        }
        last = now;
        for _ in 0..(EVERY.as_millis() / 250) {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

fn run_one(
    path: &Path,
    out: &Path,
    recipe: &Recipe,
    force: bool,
    recent: &Recent,
) -> Result<String, String> {
    let mut d =
        Document::open_indexed(path, OpenOptions::default(), None).map_err(|e| e.to_string())?;
    let problems = recipe.column_problems(&d);
    if !problems.is_empty() && !force {
        return Err(format!(
            "its columns differ from the recipe's ({})",
            problems.join("; ")
        ));
    }
    let now = natendo_engine::clean::now_utc();
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(1, |t| t.as_nanos() as u64);
    d.run_recipe(recipe, now, seed, &Progress::new())
        .map_err(|e| e.to_string())?;
    let target = out.join(path.file_name().ok_or("no file name")?);
    let r = d
        .save(SaveOptions {
            path: Some(target.clone()),
            rewrite: true,
            quoting: Quoting::Needed,
            overwrite_changed: true,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
    // A private file's result is private too.
    if recent.is_private(path) && !recent.is_private(&target) {
        let _ = recent.set_private(&target, true);
    }
    Ok(format!("{} rows written to {}", r.rows, target.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use natendo_engine::recipe::{RecipeOp, RECIPE_VERSION};

    #[test]
    fn runs_new_files_through_the_recipe() {
        let base = std::env::temp_dir().join(format!("nd-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (inbox, outbox) = (base.join("in"), base.join("out"));
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::create_dir_all(&outbox).unwrap();
        std::fs::write(inbox.join("old.csv"), b"a,b,c\r\n1,2,3\r\n").unwrap();
        let recipe = Recipe {
            version: RECIPE_VERSION,
            name: String::new(),
            columns: vec!["a".into(), "b".into(), "c".into()],
            header: true,
            steps: vec![RecipeOp::DeleteCols { at: 1, count: 1 }],
            left_out: Vec::new(),
        };
        let recent = Arc::new(Recent::default());
        // One file, then a file with other columns.
        let r = run_one(&inbox.join("old.csv"), &outbox, &recipe, false, &recent).unwrap();
        assert!(r.contains("2 rows"), "{r}");
        assert_eq!(
            std::fs::read_to_string(outbox.join("old.csv")).unwrap(),
            "a,c\r\n1,3\r\n"
        );
        std::fs::write(inbox.join("other.csv"), b"x,y\r\n1,2\r\n").unwrap();
        assert!(run_one(&inbox.join("other.csv"), &outbox, &recipe, false, &recent).is_err());
        // The watcher refuses an output folder inside the watched one.
        let w = Watcher::default();
        {
            let mut s = w.setup.lock().unwrap();
            s.folder = Some(inbox.clone());
            s.recipe = Some((base.join("r.json"), recipe));
            s.out = Some(inbox.join("sub"));
        }
        assert!(w.start(false, false, recent).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }
}
