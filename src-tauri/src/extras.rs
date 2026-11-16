//! Recent files, the folder panel's listing, and saved view files.
//!
//! Paths stay here in the shell: the webview gets names and list numbers,
//! and asks to open "item 3 of the last listing", never a path. Anything
//! read from disk (the recent list, view files) is untrusted and bounded.

use std::hash::BuildHasher;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use natendo_engine::filter::FilterSpec;
use natendo_engine::highlight::{self, Highlight};
use natendo_engine::sort::SortSpec;
use serde::{Deserialize, Serialize};

use crate::docs::{CmdError, CmdResult};

/// Most recent files kept.
pub const MAX_RECENT: usize = 20;
/// Largest recent-files list file, in bytes.
const MAX_RECENT_FILE: u64 = 64 << 10;
/// Most entries listed from a folder.
pub const MAX_LISTED: usize = 2_000;
/// Largest view file, in bytes.
pub const MAX_VIEW_FILE: u64 = 1 << 20;
pub const VIEW_EXT: &str = "ndview";

fn io(what: &str) -> impl Fn(std::io::Error) -> CmdError + '_ {
    move |e| CmdError::new("io", format!("{what}: {e}"))
}

// ---- recent files ----

/// Recently opened files (newest first), kept in the app's data folder;
/// and the files and folders in private file mode (never listed here).
#[derive(Default)]
pub struct Recent {
    file: Mutex<Option<PathBuf>>,
    list: Mutex<Vec<PathBuf>>,
    private_file: Mutex<Option<PathBuf>>,
    private: Mutex<Vec<PathBuf>>,
    /// Random key for the recent files' ids (new each run, so an id says
    /// nothing about the path and cannot be guessed from it).
    ids: std::collections::hash_map::RandomState,
}

/// Most files and folders in private file mode.
pub const MAX_PRIVATE: usize = 500;

/// Private file mode of one file: set for the file itself, or for its folder.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct PrivateState {
    pub file: bool,
    pub folder: bool,
}

fn read_list(file: &Path, max: usize) -> Vec<PathBuf> {
    let small = std::fs::metadata(file).is_ok_and(|m| m.len() <= MAX_RECENT_FILE);
    if !small {
        return Vec::new();
    }
    std::fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .map(|paths| {
            paths
                .into_iter()
                .take(max)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .collect()
        })
        .unwrap_or_default()
}

/// Write a list of paths (errors are ignored: a later change writes again).
fn write_list(file: &Path, list: &[PathBuf]) {
    let strings: Vec<String> = list
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    if let Ok(bytes) = serde_json::to_vec(&strings) {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = file.with_extension("json.saving");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, file);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RecentItem {
    /// Stable for the file while the app runs (the list's order changes as
    /// files open, so the list is never used by position). Opaque: a keyed
    /// hash of the full path, which stays in the backend.
    pub id: String,
    pub name: String,
    /// The folder's own name (not its full path); when two listed files
    /// would look the same, as many folders above as tell them apart
    /// (`2024\Data`, `2025\Data`).
    pub folder: String,
    /// The open tab showing this file, if any (matched by full path here).
    pub open_doc: Option<u32>,
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The folders above a file, top first: the drive (`C:`) or network share
/// (`\\server\share`), then each folder's name.
fn folder_parts(p: &Path) -> Vec<String> {
    use std::path::{Component, Prefix};
    let Some(dir) = p.parent() else {
        return Vec::new();
    };
    dir.components()
        .filter_map(|c| match c {
            Component::Prefix(pre) => Some(match pre.kind() {
                Prefix::Disk(d) | Prefix::VerbatimDisk(d) => format!("{}:", d as char),
                Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => format!(
                    "\\\\{}\\{}",
                    server.to_string_lossy(),
                    share.to_string_lossy()
                ),
                Prefix::Verbatim(s) | Prefix::DeviceNS(s) => s.to_string_lossy().into_owned(),
            }),
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

/// Each file's folder as shown: its own folder's name; where files of the
/// same name in folders of the same name need telling apart, also the
/// nearest folder (or drive) that differs, with "…" for any folders in
/// between: `2024\Data` and `2025\Data`, or `C:\…\Data` and `D:\…\Data`.
/// Only the differing folders are shown, never the rest of the path (it
/// usually holds the Windows user name).
pub(crate) fn folder_labels(list: &[PathBuf]) -> Vec<String> {
    let parts: Vec<Vec<String>> = list.iter().map(|p| folder_parts(p)).collect();
    let low = |s: &str| s.to_lowercase();
    // The part `d` places up from the file (1: its own folder).
    let at = |i: usize, d: usize| -> Option<&String> {
        parts[i].len().checked_sub(d).map(|k| &parts[i][k])
    };
    let same_look = |i: usize, j: usize| {
        low(&name_of(&list[i])) == low(&name_of(&list[j]))
            && at(i, 1).map(|s| low(s)) == at(j, 1).map(|s| low(s))
    };
    (0..list.len())
        .map(|i| {
            let mut need = std::collections::BTreeSet::from([1usize]);
            for j in (0..list.len()).filter(|&j| j != i && same_look(i, j)) {
                let far = parts[i].len().max(parts[j].len());
                if let Some(d) =
                    (2..=far).find(|&d| at(i, d).map(|s| low(s)) != at(j, d).map(|s| low(s)))
                {
                    if at(i, d).is_some() {
                        need.insert(d);
                    }
                }
            }
            // Top first, "…" where folders are left out.
            let mut out: Vec<&str> = Vec::new();
            let mut last: Option<usize> = None;
            for d in need.iter().rev() {
                let Some(s) = at(i, *d) else { continue };
                if last.is_some_and(|l| l - d > 1) {
                    out.push("…");
                }
                out.push(s);
                last = Some(*d);
            }
            out.join("\\")
        })
        .collect()
}

impl Recent {
    /// Load the list from `dir` (missing or unreadable: empty).
    pub fn load(&self, dir: &Path) {
        let file = dir.join("recent.json");
        let mut list: Vec<PathBuf> = Vec::new();
        let small = std::fs::metadata(&file).is_ok_and(|m| m.len() <= MAX_RECENT_FILE);
        if small {
            if let Ok(bytes) = std::fs::read(&file) {
                if let Ok(paths) = serde_json::from_slice::<Vec<String>>(&bytes) {
                    list = paths
                        .into_iter()
                        .take(MAX_RECENT)
                        .map(PathBuf::from)
                        .filter(|p| p.is_absolute())
                        .collect();
                }
            }
        }
        *self.list.lock().unwrap_or_else(|p| p.into_inner()) = list;
        *self.file.lock().unwrap_or_else(|p| p.into_inner()) = Some(file);
        let pfile = dir.join("private.json");
        *self.private.lock().unwrap_or_else(|p| p.into_inner()) = read_list(&pfile, MAX_PRIVATE);
        *self.private_file.lock().unwrap_or_else(|p| p.into_inner()) = Some(pfile);
        // A file put in private mode leaves the recent list.
        self.drop_private();
    }

    /// Whether `path` is in private file mode (itself, or a folder above it).
    pub fn private_state(&self, path: &Path) -> PrivateState {
        let Some(path) = place(path) else {
            return PrivateState::default();
        };
        let list = self.private.lock().unwrap_or_else(|p| p.into_inner());
        let list: Vec<PathBuf> = list.iter().map(|p| local_share(p)).collect();
        PrivateState {
            file: list.contains(&path),
            folder: list.iter().any(|p| *p != path && path.starts_with(p)),
        }
    }

    pub fn is_private(&self, path: &Path) -> bool {
        let s = self.private_state(path);
        s.file || s.folder
    }

    /// Put a file or folder in private file mode, or take it out. Paths are
    /// kept resolved (links, junctions, mapped network drives) and a
    /// computer's own administrative share (`\\localhost\C$\...`) as its
    /// drive, so the same place matches however it was opened.
    pub fn set_private(&self, path: &Path, on: bool) -> CmdResult<()> {
        let path = place(path).ok_or_else(|| {
            CmdError::new(
                "invalid",
                "private file mode: that file or folder cannot be found",
            )
        })?;
        let snapshot = {
            let mut list = self.private.lock().unwrap_or_else(|p| p.into_inner());
            list.retain(|p| *p != path);
            if on {
                if list.len() >= MAX_PRIVATE {
                    return Err(CmdError::new(
                        "invalid",
                        format!("at most {MAX_PRIVATE} files and folders can be private"),
                    ));
                }
                list.push(path);
            }
            list.clone()
        };
        let file = self
            .private_file
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(file) = file {
            write_list(&file, &snapshot);
        }
        if on {
            self.drop_private();
        }
        Ok(())
    }

    /// Take private files off the recent list (and save it if it changed).
    fn drop_private(&self) {
        let snapshot = {
            let mut list = self.list.lock().unwrap_or_else(|p| p.into_inner());
            let before = list.len();
            let private = self.private.lock().unwrap_or_else(|p| p.into_inner());
            list.retain(|f| !private.iter().any(|p| f.starts_with(p)));
            (list.len() != before).then(|| list.clone())
        };
        let file = self.file.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if let (Some(list), Some(file)) = (snapshot, file) {
            write_list(&file, &list);
        }
    }

    /// Put `path` first and save the list (errors saving are ignored: the
    /// list is a convenience).
    pub fn add(&self, path: &Path) {
        let Ok(path) = std::fs::canonicalize(path) else {
            return;
        };
        if self.is_private(&path) {
            return;
        }
        let snapshot = {
            let mut list = self.list.lock().unwrap_or_else(|p| p.into_inner());
            list.retain(|p| *p != path);
            list.insert(0, path);
            list.truncate(MAX_RECENT);
            list.clone()
        };
        let file = self.file.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(file) = file {
            let strings: Vec<String> = snapshot
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            if let Ok(bytes) = serde_json::to_vec(&strings) {
                if let Some(dir) = file.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let tmp = file.with_extension("json.saving");
                if std::fs::write(&tmp, bytes).is_ok() {
                    let _ = std::fs::rename(&tmp, &file);
                }
            }
        }
    }

    /// Folders where saves may have left temporary files: those of the
    /// recent files and of the files and folders in private file mode.
    pub fn temp_sweep_dirs(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        let list = self.list.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let private = self
            .private
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        for p in list.iter().chain(private.iter()) {
            let d = if p.is_dir() {
                p.clone()
            } else if let Some(parent) = p.parent() {
                parent.to_path_buf()
            } else {
                continue;
            };
            if !out.contains(&d) {
                out.push(d);
            }
        }
        out
    }

    fn id_of(&self, p: &Path) -> String {
        format!("{:016x}", self.ids.hash_one(p))
    }

    #[cfg(test)]
    pub fn items(&self) -> Vec<RecentItem> {
        self.items_with(|l| vec![None; l.len()])
    }

    /// The list, each with the open tab showing it (`open` is asked once,
    /// with all the full paths; it is not held under the list's lock).
    pub fn items_with(&self, open: impl FnOnce(&[PathBuf]) -> Vec<Option<u32>>) -> Vec<RecentItem> {
        let list = self.list.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let folders = folder_labels(&list);
        let docs = open(&list);
        list.iter()
            .zip(folders)
            .enumerate()
            .map(|(i, (p, folder))| RecentItem {
                id: self.id_of(p),
                name: name_of(p),
                folder,
                open_doc: docs.get(i).copied().flatten(),
            })
            .collect()
    }

    /// The file with this id (from `items`).
    pub fn path(&self, id: &str) -> CmdResult<PathBuf> {
        let list = self.list.lock().unwrap_or_else(|p| p.into_inner());
        list.iter()
            .find(|p| self.id_of(p) == id)
            .cloned()
            .ok_or_else(|| CmdError::new("bad_request", "that recent file is no longer listed"))
    }

    /// Forget a file (it could not be opened).
    pub fn forget(&self, path: &Path) {
        let mut list = self.list.lock().unwrap_or_else(|p| p.into_inner());
        list.retain(|p| p != path);
    }
}

// ---- folder listing ----

/// The last listing of each document's folder, so the webview can open an
/// item by its number.
#[derive(Default)]
pub struct Folders {
    lists: Mutex<std::collections::HashMap<u32, Vec<PathBuf>>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FolderItem {
    pub index: u32,
    pub name: String,
    pub size: u64,
    /// Opened here as a table (a known extension).
    pub table: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Listing {
    /// The folder's own name.
    pub folder: String,
    pub items: Vec<FolderItem>,
    /// More files than were listed.
    pub more: bool,
}

impl Folders {
    /// List the files next to `file` (folders and hidden files left out).
    pub fn list(
        &self,
        doc: u32,
        file: &Path,
        is_table: impl Fn(&Path) -> bool,
    ) -> CmdResult<Listing> {
        let dir = file
            .parent()
            .ok_or_else(|| CmdError::new("bad_request", "this file has no folder"))?;
        let mut paths: Vec<(String, PathBuf, u64)> = Vec::new();
        let mut more = false;
        for e in std::fs::read_dir(dir).map_err(io("could not list the folder"))? {
            let Ok(e) = e else { continue };
            let Ok(meta) = e.metadata() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            if !meta.is_file() || name.starts_with('.') {
                continue;
            }
            if paths.len() >= MAX_LISTED {
                more = true;
                break;
            }
            paths.push((name, e.path(), meta.len()));
        }
        paths.sort_by_key(|a| a.0.to_lowercase());
        let items = paths
            .iter()
            .enumerate()
            .map(|(i, (name, p, size))| FolderItem {
                index: i as u32,
                name: name.clone(),
                size: *size,
                table: is_table(p),
            })
            .collect();
        self.lists
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(doc, paths.into_iter().map(|x| x.1).collect());
        Ok(Listing {
            folder: name_of(dir),
            items,
            more,
        })
    }

    /// Drop a closed document's listing.
    pub fn forget(&self, doc: u32) {
        self.lists
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&doc);
    }

    pub fn path(&self, doc: u32, index: u32) -> CmdResult<PathBuf> {
        self.lists
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&doc)
            .and_then(|l| l.get(index as usize))
            .cloned()
            .ok_or_else(|| CmdError::new("bad_request", "list the folder again"))
    }
}

// ---- saved views ----

/// One column of a view, matched to a file's columns by title.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewColumn {
    pub title: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub hidden: bool,
    /// Text wraps onto more lines (view only).
    #[serde(default)]
    pub wrap: bool,
    /// How values show (view only; the file never changes).
    #[serde(default)]
    pub format: Option<DisplayFormat>,
}

/// What a display format does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormatKind {
    Number,
    Percent,
    Currency,
    Date,
    Checkbox,
}

/// How dates show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateStyle {
    Iso,
    Mdy,
    Dmy,
    Long,
    Medium,
}

/// A column's display format (view only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayFormat {
    pub kind: FormatKind,
    pub decimals: u8,
    pub thousands: bool,
    pub symbol: String,
    pub date: DateStyle,
}

/// A saved view: layout, sort, filter and highlight rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub name: String,
    /// The first row holds titles.
    #[serde(default)]
    pub header: bool,
    /// Columns in the order shown.
    #[serde(default)]
    pub columns: Vec<ViewColumn>,
    #[serde(default)]
    pub freeze_rows: u32,
    #[serde(default)]
    pub freeze_cols: u32,
    #[serde(default)]
    pub sort: Option<SortSpec>,
    #[serde(default)]
    pub filter: Option<FilterSpec>,
    #[serde(default)]
    pub highlights: Vec<Highlight>,
    /// Data checks (older view files have none).
    #[serde(default)]
    pub checks: Vec<natendo_engine::validate::Check>,
    /// Alternating row colours.
    #[serde(default)]
    pub banded: bool,
    /// Lines of text per row (1 to 10; 0 in older views means 1).
    #[serde(default)]
    pub row_lines: u8,
}

fn one() -> u32 {
    1
}

impl View {
    /// Every bound, for views from the webview or from a file.
    pub fn check(&self) -> CmdResult<()> {
        let bad = |m: &str| Err(CmdError::new("bad_request", m.to_string()));
        if self.version > 1 {
            return bad("this view was saved by a newer version of Natendo Data Edit");
        }
        if self.name.chars().count() > 200 {
            return bad("a view's name can be at most 200 characters");
        }
        if self.columns.len() > 16_384
            || self
                .columns
                .iter()
                .any(|c| c.title.chars().count() > 1_000 || c.width.is_some_and(|w| w > 5_000))
        {
            return bad("the view lists too many or too long columns");
        }
        if self.columns.iter().any(|c| {
            c.format
                .as_ref()
                .is_some_and(|f| f.decimals > 10 || f.symbol.chars().count() > 4)
        }) {
            return bad("a column's display format is not valid");
        }
        if self.row_lines > 10 {
            return bad("rows can be at most 10 lines tall");
        }
        if self.freeze_rows > 1_000 || self.freeze_cols > 1_000 {
            return bad("the view freezes too many rows or columns");
        }
        if self.sort.as_ref().is_some_and(|s| s.keys.len() > 64) {
            return bad("the view sorts by too many columns");
        }
        if let Some(f) = &self.filter {
            if f.columns.len() > 16_384 {
                return bad("the view filters too many columns");
            }
            crate::docs::check_filter(f)?;
        }
        highlight::check(&self.highlights, Default::default())?;
        natendo_engine::validate::check_spec(&natendo_engine::validate::CheckSpec {
            checks: self.checks.clone(),
            ..Default::default()
        })?;
        Ok(())
    }

    /// A view from a file's bytes (untrusted).
    pub fn from_json(bytes: &[u8]) -> CmdResult<View> {
        if bytes.len() as u64 > MAX_VIEW_FILE {
            return Err(CmdError::new(
                "too_large",
                "a view file can be at most 1 MB",
            ));
        }
        let v: View = serde_json::from_slice(bytes).map_err(|e| {
            CmdError::new(
                "bad_request",
                format!("this is not a view file Natendo can read: {e}"),
            )
        })?;
        v.check()?;
        Ok(v)
    }
}

/// Write `bytes` next to `path`, then move it over (a half-written file
/// never replaces a good one).
/// Read at most `max` bytes of a file; longer files are refused (the file
/// may grow after its size was checked, so the size alone is not trusted).
pub fn read_capped(path: &Path, max: u64, what: &str) -> CmdResult<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path).map_err(io("could not open the file"))?;
    let mut bytes = Vec::new();
    f.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(io("could not read the file"))?;
    if bytes.len() as u64 > max {
        return Err(CmdError::new(
            "too_large",
            format!("{what} can be at most {} KB", max / 1024),
        ));
    }
    Ok(bytes)
}

/// The temporary file is new (never one of the user's: a name already taken
/// is skipped).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> CmdResult<()> {
    use std::io::Write;
    write_atomic_with(path, |f| {
        f.write_all(bytes).map_err(|e| io("could not save")(e))
    })
}

/// [`write_atomic`] with the contents written by `fill` straight into the
/// temporary file (nothing held in memory), which is synced and then moved
/// into place; on any failure the temporary file is removed.
pub fn write_atomic_with(
    path: &Path,
    fill: impl FnOnce(&mut std::fs::File) -> CmdResult<()>,
) -> CmdResult<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut found = None;
    for n in 0..100u32 {
        let tmp = dir.join(format!(
            "{}{}-{n}",
            natendo_engine::save::TEMP_PREFIX,
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(f) => {
                found = Some((tmp, f));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io("could not save")(e)),
        }
    }
    let Some((tmp, mut f)) = found else {
        return Err(CmdError::new(
            "io",
            "could not save: no free temporary name",
        ));
    };
    let written = fill(&mut f).and_then(|()| f.sync_all().map_err(|e| io("could not save")(e)));
    drop(f);
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(io("could not save")(e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_files_are_checked() {
        let v = View::from_json(br#"{"name":"Mine","header":true,"columns":[{"title":"a","width":80}],"highlights":[]}"#)
            .unwrap();
        assert_eq!(v.columns[0].width, Some(80));
        for bad in [
            &br#"{"version":9}"#[..],
            br#"{"columns":[{"title":"a","width":99999}]}"#,
            br#"{"unknown":1}"#,
            br#"{"freeze_rows":5000}"#,
            br#"{"highlights":[{"col":0,"rule":{"type":"values","values":["x"]},"color":99}]}"#,
            b"not json",
        ] {
            assert!(
                View::from_json(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        let big = vec![b' '; MAX_VIEW_FILE as usize + 1];
        assert!(View::from_json(&big).is_err());
    }

    #[test]
    fn recent_list_is_bounded_and_untrusted() {
        let dir = std::env::temp_dir().join(format!("natendo-recent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A hostile list: relative paths dropped, length capped.
        let many: Vec<String> = (0..100).map(|i| format!("rel{i}.csv")).collect();
        std::fs::write(dir.join("recent.json"), serde_json::to_vec(&many).unwrap()).unwrap();
        let r = Recent::default();
        r.load(&dir);
        assert!(r.items().is_empty());
        let f = dir.join("one.csv");
        std::fs::write(&f, b"a\r\n").unwrap();
        for _ in 0..3 {
            r.add(&f);
        }
        assert_eq!(r.items().len(), 1);
        assert_eq!(r.items()[0].name, "one.csv");
        let r2 = Recent::default();
        r2.load(&dir);
        assert_eq!(r2.items().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two files with the same name in folders of the same name (2024\Data
    /// and 2025\Data) look alike in the list, but each id opens its own
    /// file, before and after the list reorders.
    #[test]
    fn recent_ids_tell_same_named_files_apart() {
        let dir = std::env::temp_dir().join(format!("natendo-recent-ids-{}", std::process::id()));
        let (old, new) = (dir.join("2024").join("Data"), dir.join("2025").join("Data"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        let (a, b) = (old.join("voters.csv"), new.join("voters.csv"));
        std::fs::write(&a, b"x\r\n").unwrap();
        std::fs::write(&b, b"y\r\n").unwrap();
        let r = Recent::default();
        r.load(&dir);
        r.add(&a);
        r.add(&b);
        let items = r.items();
        assert_eq!(items.len(), 2);
        // Shown with enough of the path to tell them apart.
        assert_eq!(
            (items[0].folder.as_str(), items[1].folder.as_str()),
            ("2025\\Data", "2024\\Data")
        );
        assert_ne!(items[0].id, items[1].id);
        let older = items[1].id.clone();
        assert!(same_file::is_same_file(r.path(&older).unwrap(), &a).unwrap());
        // Opening the newer one again reorders the list; the id still holds.
        r.add(&b);
        assert!(same_file::is_same_file(r.path(&older).unwrap(), &a).unwrap());
        // Which tab shows a file is matched by its full path.
        let ca = std::fs::canonicalize(&a).unwrap();
        let open = r.items_with(|l| l.iter().map(|p| (*p == ca).then_some(7)).collect());
        let mark: Vec<Option<u32>> = open.iter().map(|i| i.open_doc).collect();
        assert_eq!(mark, vec![None, Some(7)]);
        r.forget(&ca);
        assert!(r.path(&older).is_err());
        // With only one left, its folder's own name is enough again.
        assert_eq!(r.items()[0].folder, "Data");
        assert!(r.path("not-an-id").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Labels show only the folder that tells look-alike files apart, never
    /// the rest of the path (the user name in it stays off the screen).
    #[cfg(windows)]
    #[test]
    fn recent_labels_show_only_the_differing_folder() {
        let p = |s: &str| PathBuf::from(s);
        let list = [
            p(r"C:\Users\Nate\Docs\2024\Data\voters.csv"),
            p(r"C:\Users\Nate\Docs\2025\Data\voters.csv"),
            p(r"C:\Users\Nate\Elections\Data\voters.csv"),
            p(r"D:\Users\Nate\Elections\Data\voters.csv"),
            p(r"\\?\E:\Users\Nate\Elections\Archive\voters.csv"),
            p(r"\\server\share\Users\Nate\Data\other.csv"),
            p(r"C:\top.csv"),
        ];
        let l = folder_labels(&list);
        assert_eq!(l[0], r"2024\Data");
        assert_eq!(l[1], r"2025\Data");
        assert_eq!(l[2], r"C:\…\Elections\Data");
        assert_eq!(l[3], r"D:\…\Elections\Data");
        assert_eq!(l[4], "Archive");
        assert_eq!(l[5], "Data");
        assert_eq!(l[6], "C:");
        assert!(l.iter().all(|s| !s.contains("Nate")));
        // Identical but for the share: the share tells them apart.
        let unc = [
            p(r"\\a\s\Users\Nate\Data\v.csv"),
            p(r"\\b\s\Users\Nate\Data\v.csv"),
        ];
        assert_eq!(folder_labels(&unc), vec![r"\\a\s\…\Data", r"\\b\s\…\Data"]);
    }

    /// Private file mode: a file or its folder; private files leave and
    /// never join the recent list; the setting survives a restart.
    #[test]
    fn private_files_stay_off_the_recent_list() {
        let dir = std::env::temp_dir().join(format!("natendo-private-{}", std::process::id()));
        let sub = dir.join("voters");
        std::fs::create_dir_all(&sub).unwrap();
        let (a, b) = (dir.join("a.csv"), sub.join("b.csv"));
        std::fs::write(&a, b"x\r\n").unwrap();
        std::fs::write(&b, b"x\r\n").unwrap();
        let r = Recent::default();
        r.load(&dir);
        r.add(&a);
        r.add(&b);
        assert_eq!(r.items().len(), 2);
        r.set_private(&a, true).unwrap();
        assert_eq!(r.items().len(), 1);
        r.add(&a);
        assert_eq!(r.items()[0].name, "b.csv");
        r.set_private(&sub, true).unwrap();
        assert!(r.items().is_empty());
        let s = r.private_state(&b);
        assert!(!s.file && s.folder);
        assert!(r.private_state(&a).file);
        let r2 = Recent::default();
        r2.load(&dir);
        assert!(r2.is_private(&a) && r2.is_private(&b));
        r2.set_private(&sub, false).unwrap();
        assert!(!r2.is_private(&b));
        r2.add(&b);
        assert_eq!(r2.items().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A path as the one place it names: resolved by Windows (links, junctions,
/// mapped drives become `\\server\share`), and this computer's own
/// administrative shares (`\\localhost\C$`, `\\127.0.0.1\C$`,
/// `\\<this computer>\C$`) as the drive itself.
pub(crate) fn place(p: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(p).ok().map(|c| local_share(&c))
}

/// [`place`]'s rule for administrative shares of this computer.
pub(crate) fn local_share(p: &Path) -> PathBuf {
    let text = p.to_string_lossy();
    let Some(rest) = text
        .strip_prefix(r"\\?\UNC\")
        .or_else(|| text.strip_prefix(r"\\"))
        .filter(|_| !text.starts_with(r"\\?\") || text.starts_with(r"\\?\UNC\"))
    else {
        return p.to_path_buf();
    };
    let mut parts = rest.splitn(3, '\\');
    let (Some(server), Some(share)) = (parts.next(), parts.next()) else {
        return p.to_path_buf();
    };
    let here = std::env::var("COMPUTERNAME").unwrap_or_default();
    let local = ["localhost", "127.0.0.1", "::1", "[::1]"]
        .iter()
        .any(|h| server.eq_ignore_ascii_case(h))
        || (!here.is_empty() && server.eq_ignore_ascii_case(&here));
    let drive = share
        .strip_suffix('$')
        .filter(|d| d.len() == 1 && d.chars().all(|c| c.is_ascii_alphabetic()));
    match (local, drive) {
        (true, Some(d)) => {
            let tail = parts.next().unwrap_or("");
            PathBuf::from(format!(r"\\?\{}:\{tail}", d.to_ascii_uppercase()))
        }
        _ => p.to_path_buf(),
    }
}

#[cfg(test)]
mod place_tests {
    use super::local_share;
    use std::path::{Path, PathBuf};

    #[test]
    fn admin_shares_are_their_drive() {
        assert_eq!(
            local_share(Path::new(r"\\?\UNC\localhost\c$\Data\a.csv")),
            PathBuf::from(r"\\?\C:\Data\a.csv")
        );
        assert_eq!(
            local_share(Path::new(r"\\127.0.0.1\D$\x")),
            PathBuf::from(r"\\?\D:\x")
        );
        // Other servers and shares stay as they are.
        let other = Path::new(r"\\?\UNC\fileserver\c$\a.csv");
        assert_eq!(local_share(other), other.to_path_buf());
        let share = Path::new(r"\\?\UNC\localhost\shared\a.csv");
        assert_eq!(local_share(share), share.to_path_buf());
        let plain = Path::new(r"\\?\C:\a.csv");
        assert_eq!(local_share(plain), plain.to_path_buf());
    }
}
