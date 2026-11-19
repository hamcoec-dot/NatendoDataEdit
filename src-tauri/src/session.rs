//! Pick up where you left off: the files open when the app last closed,
//! each with its view (filter, sort, hidden columns, widths) and active cell.
//! Only file paths and view settings are kept, never cell values. Files in
//! private file mode, encrypted files and tabs without a file are left out.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::docs::{CmdError, CmdResult};

/// Most tabs remembered.
pub const MAX_SESSION_TABS: usize = 50;
/// Largest view kept for one tab (bytes of JSON); a bigger one is dropped
/// and the file reopens with the plain view.
pub const MAX_VIEW_BYTES: usize = 512 << 10;
/// Largest session file read back.
const MAX_SESSION_FILE: u64 = 32 << 20;

/// One remembered tab, as stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    path: String,
    /// The file's size and last change (seconds since 1970) when it was
    /// remembered: a file changed since reopens with the plain view.
    size: u64,
    modified: u64,
    #[serde(default)]
    view: Option<serde_json::Value>,
    #[serde(default)]
    row: u64,
    #[serde(default)]
    col: u32,
    #[serde(default)]
    active: bool,
}

/// One tab to remember, from the window.
#[derive(Debug, Clone, Deserialize)]
pub struct SessionTab {
    pub doc: u32,
    /// The tab's view as the window keeps it (opaque here).
    #[serde(default)]
    pub view: Option<serde_json::Value>,
    #[serde(default)]
    pub row: u64,
    #[serde(default)]
    pub col: u32,
    #[serde(default)]
    pub active: bool,
}

/// A remembered tab as the window sees it (no path).
#[derive(Debug, Clone, Serialize)]
pub struct SessionItem {
    pub index: usize,
    pub name: String,
    /// The folder's own name.
    pub folder: String,
    /// The file changed since (reopens with the plain view).
    pub changed: bool,
    /// The file is gone.
    pub missing: bool,
    /// The file is open already (or being opened, say from Windows at
    /// start): nothing to offer back.
    pub open: bool,
    pub active: bool,
}

/// What the window needs to put a reopened tab back as it was.
#[derive(Debug, Clone, Serialize)]
pub struct SessionPlace {
    /// `None` when the file changed since.
    pub view: Option<serde_json::Value>,
    pub row: u64,
    pub col: u32,
    pub changed: bool,
}

#[derive(Default)]
pub struct Session {
    file: Mutex<Option<PathBuf>>,
    entries: Mutex<Vec<Entry>>,
}

fn stamp(path: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(path).ok()?;
    if !m.is_file() {
        return None;
    }
    let t = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    Some((m.len(), t))
}

impl Session {
    /// Read the remembered tabs from `dir` (missing or damaged: none).
    pub fn load(&self, dir: &Path) {
        let file = dir.join("session.json");
        let small = std::fs::metadata(&file).is_ok_and(|m| m.len() <= MAX_SESSION_FILE);
        let entries: Vec<Entry> = if small {
            std::fs::read(&file)
                .ok()
                .and_then(|b| serde_json::from_slice::<Vec<Entry>>(&b).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let entries = entries
            .into_iter()
            .filter(|e| Path::new(&e.path).is_absolute())
            .take(MAX_SESSION_TABS)
            .collect();
        *self.entries.lock().unwrap_or_else(|p| p.into_inner()) = entries;
        *self.file.lock().unwrap_or_else(|p| p.into_inner()) = Some(file);
    }

    /// Remember these tabs (`path_of` gives a tab's file, or `None` to
    /// leave it out). Returns how many were kept.
    pub fn save(
        &self,
        tabs: Vec<SessionTab>,
        path_of: impl Fn(u32) -> Option<PathBuf>,
    ) -> CmdResult<usize> {
        if tabs.len() > 1_000 {
            return Err(CmdError::new("bad_request", "too many tabs"));
        }
        let mut entries = Vec::new();
        for t in tabs {
            if entries.len() >= MAX_SESSION_TABS {
                break;
            }
            let Some(path) = path_of(t.doc) else { continue };
            let Some((size, modified)) = stamp(&path) else {
                continue;
            };
            let view = t
                .view
                .map(layout_only)
                .filter(|v| serde_json::to_vec(v).is_ok_and(|b| b.len() <= MAX_VIEW_BYTES));
            entries.push(Entry {
                path: path.to_string_lossy().into_owned(),
                size,
                modified,
                view,
                row: t.row,
                col: t.col,
                active: t.active,
            });
        }
        let n = entries.len();
        let file = self.file.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(file) = file {
            if let Ok(bytes) = serde_json::to_vec(&entries) {
                if let Some(dir) = file.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let tmp = file.with_extension("json.saving");
                if std::fs::write(&tmp, bytes).is_ok() {
                    let _ = std::fs::rename(&tmp, &file);
                }
            }
        }
        *self.entries.lock().unwrap_or_else(|p| p.into_inner()) = entries;
        Ok(n)
    }

    /// The remembered tabs, by name (files now private are left out).
    pub fn items(
        &self,
        is_private: impl Fn(&Path) -> bool,
        is_open: impl FnOnce(&[PathBuf]) -> Vec<bool>,
    ) -> Vec<SessionItem> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let paths: Vec<PathBuf> = entries.iter().map(|e| PathBuf::from(&e.path)).collect();
        let folders = crate::extras::folder_labels(&paths);
        let kept: Vec<_> = entries
            .iter()
            .zip(paths.iter())
            .zip(folders)
            .enumerate()
            .filter(|(_, ((_, p), _))| !is_private(p))
            .collect();
        // Open already (or being opened): one pass over the open tabs.
        let shown: Vec<PathBuf> = kept.iter().map(|(_, ((_, p), _))| (*p).clone()).collect();
        let open = is_open(&shown);
        kept.into_iter()
            .zip(open)
            .map(|((index, ((e, p), folder)), open)| {
                let now = stamp(p);
                SessionItem {
                    index,
                    name: p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    folder,
                    changed: now.is_some_and(|s| s != (e.size, e.modified)),
                    missing: now.is_none(),
                    open,
                    active: e.active,
                }
            })
            .collect()
    }

    /// The file and place of remembered tab `index`.
    pub fn place(&self, index: usize) -> CmdResult<(PathBuf, SessionPlace)> {
        let entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let e = entries
            .get(index)
            .ok_or_else(|| CmdError::new("invalid", "that file is no longer in the list"))?;
        let path = PathBuf::from(&e.path);
        let changed = stamp(&path).is_some_and(|s| s != (e.size, e.modified));
        Ok((
            path,
            SessionPlace {
                view: if changed { None } else { e.view.clone() },
                row: e.row,
                col: e.col,
                changed,
            },
        ))
    }

    /// Forget every remembered tab.
    pub fn clear(&self) {
        let _ = self.save(Vec::new(), |_| None);
    }
}

/// A view with only its layout: filters, highlight rules and data checks
/// can hold values from the file (a picked name, "contains Smith"), and
/// nothing from the cells is ever written here; sorting is not replayed.
fn layout_only(mut v: serde_json::Value) -> serde_json::Value {
    if let Some(o) = v.as_object_mut() {
        o.insert("filter".into(), serde_json::Value::Null);
        o.insert("sort".into(), serde_json::Value::Null);
        o.insert("highlights".into(), serde_json::json!([]));
        o.insert("checks".into(), serde_json::json!([]));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_keeps_values_from_the_file() {
        let dir = std::env::temp_dir().join(format!("nde-session-v-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let a = dir.join("a.csv");
        std::fs::write(
            &a, "x,y
1,2
",
        )
        .unwrap();
        let s = Session::default();
        s.load(&dir);
        let view = serde_json::json!({
            "name": "v",
            "filter": {"columns": [{"col": 0, "rule": {"Values": {"values": ["Red Bank"]}}}]},
            "highlights": [{"text": "Smith"}],
            "checks": [{"col": 1, "kind": {"type": "list", "values": ["Lee"]}}],
            "columns": [{"width": 90}],
        });
        let tabs = vec![SessionTab {
            doc: 1,
            view: Some(view),
            row: 0,
            col: 0,
            active: true,
        }];
        s.save(tabs, |_| Some(a.clone())).unwrap();
        let disk = std::fs::read_to_string(dir.join("session.json")).unwrap();
        for v in ["Red Bank", "Smith", "Lee"] {
            assert!(!disk.contains(v), "{v} was written to the session file");
        }
        assert!(disk.contains("\"width\":90"));
    }

    #[test]
    fn remembers_paths_and_views_only() {
        let dir = std::env::temp_dir().join(format!("nde-session-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let a = dir.join("a.csv");
        std::fs::write(&a, "x,y\n1,2\n").unwrap();
        let s = Session::default();
        s.load(&dir);
        let tabs = vec![
            SessionTab {
                doc: 1,
                view: Some(serde_json::json!({"name": "v"})),
                row: 5,
                col: 2,
                active: true,
            },
            // No file: left out.
            SessionTab {
                doc: 2,
                view: None,
                row: 0,
                col: 0,
                active: false,
            },
        ];
        let kept = s.save(tabs, |d| (d == 1).then(|| a.clone())).unwrap();
        assert_eq!(kept, 1);
        // Read back by a new run.
        let s2 = Session::default();
        s2.load(&dir);
        let items = s2.items(|_| false, |ps| vec![false; ps.len()]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "a.csv");
        assert!(!items[0].changed && !items[0].missing && !items[0].open && items[0].active);
        // A file open already is marked, so the window doesn't offer it.
        assert!(s2.items(|_| false, |ps| ps.iter().map(|p| *p == a).collect())[0].open);
        let (p, place) = s2.place(0).unwrap();
        assert_eq!(p, a);
        assert_eq!((place.row, place.col), (5, 2));
        assert!(place.view.is_some());
        // Private files are not listed.
        assert!(s2.items(|_| true, |ps| vec![false; ps.len()]).is_empty());
        // A changed file reopens with the plain view.
        std::fs::write(&a, "x,y\n1,2\n3,4\n").unwrap();
        let (_, place) = s2.place(0).unwrap();
        assert!(place.changed && place.view.is_none());
        // Nothing of the cells is stored.
        let text = std::fs::read_to_string(dir.join("session.json")).unwrap();
        assert!(!text.contains("1,2"));
        s2.clear();
        assert!(s2.items(|_| false, |ps| vec![false; ps.len()]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
