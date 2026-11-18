//! Run a recipe on many files now (Tools › Run recipe on files…): the files,
//! the recipe and the output folder are chosen in the app's own dialogs;
//! each file is opened, run through the recipe and written to the output
//! folder under its own name. The files chosen are never changed.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use natendo_engine::recipe::{Question, Recipe};
use serde::Serialize;

use crate::extras::Recent;
use crate::watch;

/// Files run at once at most.
pub const MAX_FILES: usize = 10_000;

/// One file's result.
#[derive(Debug, Clone, Serialize)]
pub struct FileResult {
    pub name: String,
    pub ok: bool,
    pub rows_in: u64,
    pub rows_out: u64,
    /// Where it was written, or why not.
    pub message: String,
}

/// What the screen shows.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BatchStatus {
    pub files: Vec<String>,
    pub recipe: Option<String>,
    pub steps: Vec<String>,
    pub questions: Vec<Question>,
    pub out: Option<String>,
    pub running: bool,
    pub done: usize,
    pub failed: usize,
    pub current: Option<String>,
    pub results: Vec<FileResult>,
}

#[derive(Default)]
pub struct BatchSetup {
    pub files: Vec<PathBuf>,
    pub recipe: Option<(PathBuf, Recipe)>,
    pub out: Option<PathBuf>,
}

#[derive(Default)]
struct State {
    running: bool,
    current: Option<String>,
    results: Vec<FileResult>,
}

#[derive(Default)]
pub struct Batch {
    pub setup: Mutex<BatchSetup>,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

impl Batch {
    pub fn status(&self) -> BatchStatus {
        let st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let s = self.setup.lock().unwrap_or_else(|p| p.into_inner());
        BatchStatus {
            files: s.files.iter().map(|p| p.display().to_string()).collect(),
            recipe: s.recipe.as_ref().map(|(p, _)| p.display().to_string()),
            steps: s
                .recipe
                .as_ref()
                .map(|(_, r)| r.steps.iter().map(|x| x.describe()).collect())
                .unwrap_or_default(),
            questions: s
                .recipe
                .as_ref()
                .map(|(_, r)| r.questions.clone())
                .unwrap_or_default(),
            out: s.out.as_ref().map(|p| p.display().to_string()),
            running: st.running,
            done: st.results.iter().filter(|r| r.ok).count(),
            failed: st.results.iter().filter(|r| !r.ok).count(),
            current: st.current.clone(),
            results: st.results.clone(),
        }
    }

    /// Run the recipe on every chosen file (in the background).
    pub fn start(
        &self,
        answers: &[(String, String)],
        force: bool,
        recent: Arc<Recent>,
    ) -> Result<(), String> {
        if self.state.lock().unwrap_or_else(|p| p.into_inner()).running {
            return Err("files are still being run".into());
        }
        let (files, recipe, out) = {
            let s = self.setup.lock().unwrap_or_else(|p| p.into_inner());
            match (&s.recipe, &s.out) {
                (Some((_, r)), Some(o)) if !s.files.is_empty() => {
                    (s.files.clone(), r.clone(), o.clone())
                }
                _ => return Err("choose the files, the recipe and the output folder".into()),
            }
        };
        let recipe = recipe.answered(answers).map_err(|e| e.to_string())?;
        // The files chosen are never written over.
        if files
            .iter()
            .any(|f| f.parent().is_some_and(|d| watch::same_dir(d, &out)))
        {
            return Err(
                "choose an output folder other than the one the files are in, so they are never changed"
                    .into(),
            );
        }
        let mut names = std::collections::HashSet::new();
        for f in &files {
            if !names.insert(name_of(f).to_lowercase()) {
                return Err(format!(
                    "two of the files are called {}; their results would land on each other",
                    name_of(f)
                ));
            }
        }
        self.stop.store(false, Ordering::Relaxed);
        let state = self.state.clone();
        let stop = self.stop.clone();
        {
            let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
            st.running = true;
            st.results.clear();
            st.current = None;
        }
        std::thread::spawn(move || {
            for f in &files {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let name = name_of(f);
                state.lock().unwrap_or_else(|p| p.into_inner()).current = Some(name.clone());
                let r = match watch::run_one(f, &out, &recipe, force, &recent) {
                    Ok(done) => FileResult {
                        name,
                        ok: true,
                        rows_in: done.rows_in,
                        rows_out: done.rows_out,
                        message: format!("written to {}", done.target.display()),
                    },
                    Err(e) => FileResult {
                        name,
                        ok: false,
                        rows_in: 0,
                        rows_out: 0,
                        message: e,
                    },
                };
                state
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .results
                    .push(r);
            }
            let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
            st.running = false;
            st.current = None;
        });
        Ok(())
    }

    /// Stop after the file being run.
    pub fn stop_now(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// The table files in a folder (not its subfolders), by name.
pub fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && watch::is_table(p))
                .take(MAX_FILES)
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use natendo_engine::recipe::{RecipeOp, RECIPE_VERSION};

    #[test]
    fn runs_every_file_and_never_changes_them() {
        let dir = std::env::temp_dir().join(format!("nde-batch-{}", std::process::id()));
        let (src, out) = (dir.join("in"), dir.join("out"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(src.join("a.csv"), "name,x\r\nann,1\r\n").unwrap();
        std::fs::write(src.join("b.csv"), "name,x\r\nbo,2\r\ncy,3\r\n").unwrap();
        std::fs::write(src.join("c.csv"), "other\r\n1\r\n").unwrap();
        let b = Batch::default();
        {
            let mut s = b.setup.lock().unwrap();
            s.files = files_in(&src);
            assert_eq!(s.files.len(), 3);
            s.recipe = Some((
                dir.join("r.json"),
                Recipe {
                    version: RECIPE_VERSION,
                    name: String::new(),
                    columns: vec!["name".into(), "x".into()],
                    header: true,
                    steps: vec![RecipeOp::DeleteCols { at: 1, count: 1 }],
                    left_out: Vec::new(),
                    questions: Vec::new(),
                },
            ));
            s.out = Some(src.clone());
        }
        let recent = Arc::new(Recent::default());
        // Not into the files' own folder.
        assert!(b.start(&[], false, recent.clone()).is_err());
        b.setup.lock().unwrap().out = Some(out.clone());
        b.start(&[], false, recent).unwrap();
        while b.status().running {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let st = b.status();
        assert_eq!((st.done, st.failed), (2, 1), "{:?}", st.results);
        assert_eq!(st.results[1].rows_out, 3);
        assert_eq!(
            std::fs::read_to_string(out.join("b.csv")).unwrap(),
            "name\r\nbo\r\ncy\r\n"
        );
        assert_eq!(
            std::fs::read_to_string(src.join("b.csv")).unwrap(),
            "name,x\r\nbo,2\r\ncy,3\r\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
