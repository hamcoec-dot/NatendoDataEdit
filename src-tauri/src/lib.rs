//! Tauri shell for Natendo Data Edit. Holds IPC commands only; all data logic
//! lives in `natendo-engine`, and the boundary rules live in `docs`.
//!
//! Security rule: no command accepts a file path from the webview. Open and
//! Save As show the native dialog here, in Rust, and use the path it returns.

mod agent;
mod applog;
mod assoc;
mod batch;
mod diskwatch;
mod docs;
mod extras;
mod imports;
mod links;
#[cfg(test)]
mod lockaudit;
mod recovery;
mod session;
#[cfg(test)]
mod testdir;
mod watch;
mod windows;
mod zips;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use agent::{Agent, AgentStatus};

use docs::{
    CmdError, CmdResult, DataOp, DocInfo, Docs, Found, JobInfo, Layout, LayoutChoices, OpReport,
    ReplaceReport, SaveAs, TableOp,
};
use extras::{Folders, Listing, Recent, RecentItem, View};
use imports::{ImportStart, Imports};
use natendo_engine::export::{ExportOptions, ExportReport};
use natendo_engine::filter::{FilterSpec, ValueList};
use natendo_engine::find::{FindAll, FindSpec, Pattern};
use natendo_engine::import::{ImportFormat, ImportInfo, ImportOptions};
use natendo_engine::ops::Dir;
use natendo_engine::pivot::{
    ItemList, PivotField, PivotGrid, PivotSpec, SavedPivot, MAX_SAVED_PIVOT_BYTES,
};
use natendo_engine::sort::SortSpec;
use serde::Serialize;
use tauri::{Emitter, Manager, State, WebviewWindow};

#[derive(Serialize)]
struct AppInfo {
    name: &'static str,
    version: &'static str,
    engine_version: &'static str,
}

type DocsState<'a> = State<'a, Arc<Docs>>;
type ImportsState<'a> = State<'a, Arc<Imports>>;
type RecentState<'a> = State<'a, Arc<Recent>>;
type SessionState<'a> = State<'a, Arc<session::Session>>;
type FoldersState<'a> = State<'a, Arc<Folders>>;

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        name: "Natendo Data Edit",
        version: env!("CARGO_PKG_VERSION"),
        engine_version: natendo_engine::version(),
    }
}

/// Run `f` on a worker thread (file dialogs, opening, saving, big edits) so
/// the app's main thread never waits on the disk.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> CmdResult<T> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| CmdError::new("error", e.to_string()))?
}

const FILE_TYPES: &[&str] = &["csv", "tsv", "tab", "txt", "psv", "dat", "ssv"];
const SHEET_TYPES: &[&str] = &["xlsx", "xlsm", "ods"];
/// A shapefile opens its attribute table.
const SHAPE_TYPES: &[&str] = &["shp"];
/// Zipped and gzipped tables (opened without unpacking to disk).
const PACKED_TYPES: &[&str] = &["gz", "zip"];
const OTHER_TYPES: &[&str] = &[
    "json", "jsonl", "ndjson", "xml", "html", "htm", "md", "log", "prn", "fwf", "dbf",
];

fn is_import_type(path: &std::path::Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|e| SHEET_TYPES.contains(&e.as_str()) || OTHER_TYPES.contains(&e.as_str()))
}

/// What Open did: a document, or an import screen for a spreadsheet or
/// another format.
#[derive(Serialize)]
struct Opened {
    doc: Option<DocInfo>,
    import: Option<ImportStart>,
    /// A zip file holding several tables: pick which to open.
    #[serde(skip_serializing_if = "Option::is_none")]
    zip: Option<zips::ZipList>,
    /// A database or GeoPackage: pick which table to open.
    #[serde(skip_serializing_if = "Option::is_none")]
    sqlite: Option<(String, Vec<natendo_engine::sqlite::DbTable>)>,
}

fn is_database(path: &std::path::Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|e| natendo_engine::sqlite::OPENED_EXTENSIONS.contains(&e.as_str()))
}

/// The tables of the database at `path`, which becomes the picked one.
fn pick_database(
    path: &std::path::Path,
) -> CmdResult<(String, Vec<natendo_engine::sqlite::DbTable>)> {
    let list = natendo_engine::sqlite::tables(path)?;
    *SQLITE_PICKED.lock().unwrap_or_else(|p| p.into_inner()) = Some(path.to_path_buf());
    Ok((path.display().to_string(), list))
}

/// Open a path the shell chose (dialog, recent list, folder listing):
/// spreadsheets and other formats go to the import screen.
fn open_any(
    docs: &Docs,
    imports: &Imports,
    recent: &Recent,
    path: &std::path::Path,
) -> CmdResult<Opened> {
    let mut head = [0u8; 8];
    let looks_encrypted = std::fs::File::open(path)
        .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
        .is_ok_and(|n| natendo_engine::crypt::is_encrypted(&head[..n]));
    if looks_encrypted {
        return Err(CmdError::new(
            "encrypted",
            "this file is encrypted: open it with File > Open an encrypted file, which asks for its password",
        ));
    }
    // A shapefile's parts open its attribute table.
    if let Some(table) = natendo_engine::dbf::shapefile_table(path) {
        let mut start = imports.open_path(&table)?;
        let name = |p: &std::path::Path| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        start.note = format!(
            "Opened {} (the attribute table of {}); the shapes are not shown.",
            name(&table),
            name(&path.with_extension("shp"))
        );
        recent.add(&table);
        return Ok(Opened {
            doc: None,
            import: Some(start),
            zip: None,
            sqlite: None,
        });
    }
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("shp"))
    {
        return Err(CmdError::new(
            "error",
            "this shapefile has no .dbf file beside it, so there is no attribute table to show",
        ));
    }
    let opened = if is_database(path) {
        Opened {
            doc: None,
            import: None,
            zip: None,
            sqlite: Some(pick_database(path)?),
        }
    } else if zips::is_zip_path(path) {
        // A zip of tables: one opens at once; several are listed to pick.
        match zips::ZIPS.list(path)? {
            zips::Listed::One(name) => Opened {
                doc: zips::ZIPS.open_entry(docs, path, &name)?.into_iter().next(),
                import: None,
                zip: None,
                sqlite: None,
            },
            zips::Listed::Many(list) => Opened {
                doc: None,
                import: None,
                zip: Some(list),
                sqlite: None,
            },
        }
    } else if is_import_type(path) {
        Opened {
            doc: None,
            import: Some(imports.open_path(path)?),
            zip: None,
            sqlite: None,
        }
    } else {
        let info = docs.open_path(path, false)?;
        Opened {
            doc: Some(info),
            import: None,
            zip: None,
            sqlite: None,
        }
    };
    recent.add(path);
    // Private files' names are not written to the app log.
    if !recent.is_private(path) {
        applog::info(&format!(
            "opened {}",
            path.file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default()
        ));
    }
    Ok(opened)
}

/// A file Windows, a second launch, a drop or the agent link asked to open.
#[derive(Serialize)]
struct OsOpen {
    name: String,
    opened: Option<Opened>,
    /// Already open in this tab: switch to it.
    existing: Option<u32>,
    error: Option<String>,
}

/// Opens waiting for the window to show them.
#[derive(Default)]
struct Pending(Mutex<Vec<OsOpen>>);

/// Most files opened from one launch, drop or request.
const MAX_OS_OPENS: usize = 64;

/// Open paths that came from outside the app (command line, a second
/// launch, files dropped on the window, the agent link). Each must be an
/// existing regular file; then it goes the same way as a file picked in
/// the Open dialog. A file already open just gets its tab shown. Results
/// wait in `Pending` and the window is told to take them.
pub(crate) fn open_paths(app: &tauri::AppHandle, paths: Vec<PathBuf>, focus: bool) {
    let app = app.clone();
    std::thread::spawn(move || {
        let docs = app.state::<Arc<Docs>>().inner().clone();
        let imports = app.state::<Arc<Imports>>().inner().clone();
        let recent = app.state::<Arc<Recent>>().inner().clone();
        let mut results = Vec::new();
        for path in paths.into_iter().take(MAX_OS_OPENS) {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let ok_file = std::fs::metadata(&path).is_ok_and(|m| m.is_file());
            let r = if !ok_file {
                OsOpen {
                    name,
                    opened: None,
                    existing: None,
                    error: Some("not a file that can be opened".into()),
                }
            } else if let Some(id) = docs.find_open(&path) {
                OsOpen {
                    name,
                    opened: None,
                    existing: Some(id),
                    error: None,
                }
            } else {
                match open_any(&docs, &imports, &recent, &path) {
                    Ok(o) => OsOpen {
                        name,
                        opened: Some(o),
                        existing: None,
                        error: None,
                    },
                    Err(e) => {
                        applog::warn(&format!("could not open {name}: {}", e.message));
                        OsOpen {
                            name,
                            opened: None,
                            existing: None,
                            error: Some(e.message),
                        }
                    }
                }
            };
            results.push(r);
        }
        if results.is_empty() {
            return;
        }
        app.state::<Pending>()
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .extend(results);
        // The main window gets it (any open window if main is gone: never
        // nobody).
        if let Some(label) = windows::event_target(&windows::open_labels(&app)) {
            if focus {
                if let Some(w) = app.get_webview_window(&label) {
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            }
            let _ = app.emit_to(label, "opened", ());
        }
    });
}

/// File paths among command-line arguments (flags skipped), made absolute
/// against the folder the launch came from.
fn arg_paths(args: &[String], cwd: Option<&std::path::Path>) -> Vec<PathBuf> {
    args.iter()
        .filter(|a| !a.starts_with('-') && !a.is_empty())
        .map(|a| {
            let p = PathBuf::from(a);
            match cwd {
                Some(c) if p.is_relative() => c.join(p),
                _ => p,
            }
        })
        .collect()
}

/// Files opened from outside since the last call.
#[tauri::command]
fn take_opened(pending: State<'_, Pending>) -> Vec<OsOpen> {
    std::mem::take(&mut *pending.0.lock().unwrap_or_else(|p| p.into_inner()))
}

/// Recently opened files (names and folder names; paths stay here), each
/// with the open tab showing it, matched by full path. Off the main thread:
/// matching looks at the files, which can be on a slow network drive.
#[tauri::command]
async fn recent_files(docs: DocsState<'_>, recent: RecentState<'_>) -> CmdResult<Vec<RecentItem>> {
    let (docs, recent) = (docs.inner().clone(), recent.inner().clone());
    blocking(move || Ok(recent.items_with(|l| docs.find_open_many(l)))).await
}

/// For tabs whose files share a name: the folder that tells each apart, as
/// the recent list shows it (`2024\Data`, `C:\…\Data`); "" for the rest.
#[tauri::command]
async fn tab_folders(docs: DocsState<'_>, ids: Vec<u32>) -> CmdResult<Vec<String>> {
    if ids.len() > 1_000 {
        return Err(CmdError::new("bad_request", "too many tabs"));
    }
    let docs = docs.inner().clone();
    blocking(move || {
        let paths = docs.paths_of(&ids);
        let files: Vec<PathBuf> = paths.iter().flatten().cloned().collect();
        let name = |p: &std::path::Path| p.file_name().map(|n| n.to_string_lossy().to_lowercase());
        let mut labels = extras::folder_labels(&files).into_iter();
        Ok(paths
            .iter()
            .map(|p| match p {
                Some(p) => {
                    let label = labels.next().unwrap_or_default();
                    let twin = files.iter().filter(|q| name(q) == name(p)).count() > 1;
                    if twin {
                        label
                    } else {
                        String::new()
                    }
                }
                None => String::new(),
            })
            .collect())
    })
    .await
}

/// A column's name: its cell in the table's first row (the header row).
#[tauri::command]
async fn first_row_cell(docs: DocsState<'_>, doc: u32, col: u32) -> CmdResult<String> {
    let docs = docs.inner().clone();
    blocking(move || docs.first_row_cell(doc, col)).await
}

/// Rename a column: set its cell in the table's first row (one undo step).
#[tauri::command]
async fn set_first_row_cell(
    docs: DocsState<'_>,
    doc: u32,
    col: u32,
    value: String,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.set_first_row_cell(doc, col, value)).await
}

/// Copy column names: the names of columns `cols` as one line, split by
/// `sep` (a tab, or the file's delimiter).
#[tauri::command]
async fn column_names_line(
    docs: DocsState<'_>,
    doc: u32,
    cols: Vec<u32>,
    sep: char,
) -> CmdResult<String> {
    let docs = docs.inner().clone();
    blocking(move || docs.column_names_line(doc, &cols, sep)).await
}

/// Paste as column names: set first-row cells (column, name), one undo step.
#[tauri::command]
async fn set_column_names(
    docs: DocsState<'_>,
    doc: u32,
    names: Vec<(u32, String)>,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.set_column_names(doc, names)).await
}

/// The names of the columns (at most 20,000, each cut to 200 characters):
/// the table's first row, shown or not.
#[tauri::command]
async fn first_row_names(docs: DocsState<'_>, doc: u32) -> CmdResult<Vec<String>> {
    let docs = docs.inner().clone();
    blocking(move || docs.first_row_names(doc, docs::MAX_NAME_COLS, NAME_CHARS)).await
}

/// Names longer than this come back cut (the UI then names such a column
/// by number in rules).
const NAME_CHARS: usize = 200;

/// Open a recent file by its id (from `recent_files`). A file already open
/// in a tab is not opened again: that tab's document comes back.
#[tauri::command]
async fn open_recent(
    docs: DocsState<'_>,
    imports: ImportsState<'_>,
    recent: RecentState<'_>,
    id: String,
) -> CmdResult<Opened> {
    let (docs, imports, recent) = (
        docs.inner().clone(),
        imports.inner().clone(),
        recent.inner().clone(),
    );
    blocking(move || {
        let path = recent.path(&id)?;
        if let Some(doc) = docs.find_open(&path) {
            return Ok(Opened {
                doc: Some(docs.info(doc)?),
                import: None,
                zip: None,
                sqlite: None,
            });
        }
        let r = open_any(&docs, &imports, &recent, &path);
        if r.is_err() && !path.exists() {
            recent.forget(&path);
        }
        r
    })
    .await
}

/// Tidy column titles: what would change (column, now, after).
#[tauri::command]
async fn tidy_titles_preview(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::titles::TitleSpec,
) -> CmdResult<Vec<(u32, String, String)>> {
    let docs = docs.inner().clone();
    blocking(move || docs.tidy_titles_preview(doc, &spec)).await
}

/// Tidy every column title (one undo step).
#[tauri::command]
async fn tidy_titles(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::titles::TitleSpec,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.tidy_titles(doc, spec)).await
}

/// Protect columns from changes (and no others).
#[tauri::command]
async fn set_protected(docs: DocsState<'_>, doc: u32, cols: Vec<u32>) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.set_protected(doc, cols)).await
}

/// Open tables picked from a zip file (each read-only to the zip: a tab
/// with no file, so Save asks where to save it).
#[tauri::command]
async fn zip_open(docs: DocsState<'_>, token: u32, names: Vec<String>) -> CmdResult<Vec<DocInfo>> {
    let docs = docs.inner().clone();
    blocking(move || zips::ZIPS.open_picked(&docs, token, &names)).await
}

/// Remember the open tabs for next time (paths and views only; private,
/// encrypted and unsaved tabs are left out).
#[tauri::command]
async fn session_save(
    docs: DocsState<'_>,
    recent: RecentState<'_>,
    session: SessionState<'_>,
    windows: State<'_, windows::Windows>,
    mut tabs: Vec<session::SessionTab>,
) -> CmdResult<usize> {
    // The files window 2 holds are remembered as ordinary tabs (they reopen
    // in the main window).
    for h in windows.owned_by_second() {
        if !tabs.iter().any(|t| t.doc == h.doc) {
            tabs.push(session::SessionTab {
                doc: h.doc,
                view: h.view,
                row: h.row,
                col: h.col,
                active: false,
            });
        }
    }
    let (docs, recent, session) = (
        docs.inner().clone(),
        recent.inner().clone(),
        session.inner().clone(),
    );
    blocking(move || {
        session.save(tabs, |doc| {
            if docs.encrypted_from(doc).ok().flatten().is_some() {
                return None;
            }
            let path = docs.paths_of(&[doc]).pop().flatten()?;
            (!recent.is_private(&path)).then_some(path)
        })
    })
    .await
}

/// The tabs open when the app last closed (names only).
#[tauri::command]
async fn session_list(
    docs: DocsState<'_>,
    recent: RecentState<'_>,
    session: SessionState<'_>,
    launch: State<'_, LaunchPaths>,
) -> CmdResult<Vec<session::SessionItem>> {
    let (docs, recent, session) = (
        docs.inner().clone(),
        recent.inner().clone(),
        session.inner().clone(),
    );
    // Read once: a later call (none today) must not hide a launch file
    // closed since.
    let launch = std::mem::take(&mut *launch.0.lock().unwrap_or_else(|p| p.into_inner()));
    blocking(move || {
        Ok(session.items(
            |p| recent.is_private(p),
            |paths| docs.open_or_launching(&launch, paths),
        ))
    })
    .await
}

/// The files Windows asked this run to open at start (they may still be
/// opening when the window asks what to offer back).
#[derive(Default)]
struct LaunchPaths(Mutex<Vec<PathBuf>>);

/// A remembered tab reopened, and where it was.
#[derive(Serialize)]
struct SessionOpened {
    opened: Opened,
    place: session::SessionPlace,
}

/// Reopen remembered tab `index`.
#[tauri::command]
async fn session_open(
    docs: DocsState<'_>,
    imports: ImportsState<'_>,
    recent: RecentState<'_>,
    session: SessionState<'_>,
    index: usize,
) -> CmdResult<SessionOpened> {
    let (docs, imports, recent, session) = (
        docs.inner().clone(),
        imports.inner().clone(),
        recent.inner().clone(),
        session.inner().clone(),
    );
    blocking(move || {
        let (path, place) = session.place(index)?;
        if recent.is_private(&path) {
            return Err(CmdError::new(
                "invalid",
                "that file is in private file mode, so it is not reopened",
            ));
        }
        let opened = match docs.find_open(&path) {
            Some(doc) => Opened {
                doc: Some(docs.info(doc)?),
                import: None,
                zip: None,
                sqlite: None,
            },
            None => open_any(&docs, &imports, &recent, &path)?,
        };
        Ok(SessionOpened { opened, place })
    })
    .await
}

/// Forget the remembered tabs.
#[tauri::command]
fn session_clear(session: SessionState<'_>) {
    session.clear();
}

/// The files next to a document's file, for the Files panel.
#[tauri::command]
async fn folder_list(
    docs: DocsState<'_>,
    folders: FoldersState<'_>,
    doc: u32,
) -> CmdResult<Listing> {
    let (docs, folders) = (docs.inner().clone(), folders.inner().clone());
    blocking(move || {
        let path = docs
            .path_of(doc)?
            .ok_or_else(|| CmdError::new("bad_request", "this table has no file yet"))?;
        folders.list(doc, &path, |p| {
            p.extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .is_some_and(|e| {
                    FILE_TYPES.contains(&e.as_str())
                        || SHEET_TYPES.contains(&e.as_str())
                        || OTHER_TYPES.contains(&e.as_str())
                })
        })
    })
    .await
}

/// Open item `index` of the last listing of `doc`'s folder.
#[tauri::command]
async fn folder_open(
    docs: DocsState<'_>,
    imports: ImportsState<'_>,
    recent: RecentState<'_>,
    folders: FoldersState<'_>,
    doc: u32,
    index: u32,
) -> CmdResult<Opened> {
    let (docs, imports, recent, folders) = (
        docs.inner().clone(),
        imports.inner().clone(),
        recent.inner().clone(),
        folders.inner().clone(),
    );
    blocking(move || {
        let path = folders.path(doc, index)?;
        open_any(&docs, &imports, &recent, &path)
    })
    .await
}

/// Save a view to a file chosen in the Save dialog; its name, or `None`.
#[tauri::command]
async fn view_save(
    window: WebviewWindow,
    docs: DocsState<'_>,
    view: View,
) -> CmdResult<Option<String>> {
    let docs = docs.inner().clone();
    blocking(move || {
        view.check()?;
        let bytes =
            serde_json::to_vec_pretty(&view).map_err(|e| CmdError::new("error", e.to_string()))?;
        let name = view
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || " -_".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .take(80)
            .collect::<String>();
        let name = if name.trim().is_empty() {
            "view".to_string()
        } else {
            name.trim().to_string()
        };
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save view")
            .set_file_name(format!("{name}.{}", extras::VIEW_EXT))
            .add_filter("Natendo view", &[extras::VIEW_EXT])
            .set_parent(&window)
            .save_file()
        else {
            return Ok(None);
        };
        docs.check_writable(&path)?;
        extras::write_atomic(&path, &bytes)?;
        Ok(Some(
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ))
    })
    .await
}

/// Open a view file (untrusted; checked as it is read). `None` if cancelled.
#[tauri::command]
async fn view_open(window: WebviewWindow) -> CmdResult<Option<View>> {
    blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open view")
            .add_filter("Natendo view", &[extras::VIEW_EXT])
            .add_filter("All files", &["*"])
            .set_parent(&window)
            .pick_file()
        else {
            return Ok(None);
        };
        let bytes = extras::read_capped(&path, extras::MAX_VIEW_FILE, "a view file")?;
        View::from_json(&bytes).map(Some)
    })
    .await
}

#[tauri::command]
async fn set_read_only(docs: DocsState<'_>, doc: u32, on: bool) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.set_read_only(doc, on)).await
}

#[tauri::command]
async fn column_types(
    docs: DocsState<'_>,
    doc: u32,
    skip_first: bool,
    read: natendo_engine::value::ReadOptions,
) -> CmdResult<Vec<natendo_engine::stats::ColType>> {
    let docs = docs.inner().clone();
    blocking(move || docs.column_types(doc, skip_first, read)).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn selection_totals(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    rows: u64,
    col: u32,
    cols: u32,
    read: natendo_engine::value::ReadOptions,
    distinct: Option<bool>,
) -> CmdResult<docs::TotalsOut> {
    let docs = docs.inner().clone();
    let distinct = distinct.unwrap_or(false);
    blocking(move || docs.selection_totals(doc, row, rows, col, cols, read, distinct)).await
}

/// The totals row: each of these columns over the shown rows.
#[tauri::command]
async fn column_totals(
    docs: DocsState<'_>,
    doc: u32,
    cols: Vec<u32>,
    header: bool,
    read: natendo_engine::value::ReadOptions,
) -> CmdResult<docs::ColumnTotalsOut> {
    let docs = docs.inner().clone();
    blocking(move || docs.column_totals(doc, cols, header, read)).await
}

/// Totals of several selected blocks (Ctrl+click), each cell once.
#[tauri::command]
async fn selection_totals_areas(
    docs: DocsState<'_>,
    doc: u32,
    areas: Vec<natendo_engine::areas::Area>,
    read: natendo_engine::value::ReadOptions,
) -> CmdResult<docs::TotalsOut> {
    let docs = docs.inner().clone();
    blocking(move || docs.selection_totals_areas(doc, areas, read)).await
}

/// A new table and what a comparison found.
#[tauri::command]
async fn table_report(docs: DocsState<'_>, doc: u32, op: TableOp) -> CmdResult<docs::TableReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.new_table_report(doc, op)).await
}

/// The next shown row whose cell differs.
#[tauri::command]
async fn find_change(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    col: u32,
    up: bool,
) -> CmdResult<Option<u64>> {
    let docs = docs.inner().clone();
    blocking(move || docs.find_change(doc, row, col, up)).await
}

/// The row of a column's largest or smallest number.
#[tauri::command]
async fn find_extreme(
    docs: DocsState<'_>,
    doc: u32,
    col: u32,
    largest: bool,
    keep_first: bool,
    read: natendo_engine::value::ReadOptions,
) -> CmdResult<Option<u64>> {
    let docs = docs.inner().clone();
    blocking(move || docs.find_extreme(doc, col, largest, keep_first, read)).await
}

/// Compare two columns: counts, and the rows where they differ.
#[tauri::command]
async fn compare_cols(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::colcompare::CompareCols,
) -> CmdResult<natendo_engine::colcompare::CompareColsReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.compare_cols(doc, &spec)).await
}

/// Go To Special: blank, number, text or row-difference cells of an area.
#[tauri::command]
async fn special_cells(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::special::SpecialSpec,
) -> CmdResult<natendo_engine::special::SpecialCells> {
    let docs = docs.inner().clone();
    blocking(move || docs.special_cells(doc, spec)).await
}

/// Smallest and largest number of a column.
#[tauri::command]
async fn number_range(
    docs: DocsState<'_>,
    doc: u32,
    col: u32,
    keep_first: bool,
    read: natendo_engine::value::ReadOptions,
) -> CmdResult<Option<(f64, f64)>> {
    let docs = docs.inner().clone();
    blocking(move || docs.number_range(doc, col, keep_first, read)).await
}

/// Type-ahead suggestion for a cell being typed.
#[tauri::command]
async fn typeahead(
    docs: DocsState<'_>,
    doc: u32,
    col: u32,
    prefix: String,
    keep_first: bool,
) -> CmdResult<Option<String>> {
    let docs = docs.inner().clone();
    blocking(move || docs.typeahead(doc, col, &prefix, keep_first)).await
}

/// Rename the open file: the new name is chosen in the Save dialog, in the
/// file's folder. `None` if cancelled.
#[tauri::command]
async fn rename_file(
    window: WebviewWindow,
    docs: DocsState<'_>,
    doc: u32,
) -> CmdResult<Option<DocInfo>> {
    let docs = docs.inner().clone();
    blocking(move || {
        let from = docs
            .path_of(doc)?
            .ok_or_else(|| CmdError::new("no_path", "this document has no file yet"))?;
        let mut dialog = rfd::FileDialog::new()
            .set_title("Rename file")
            .set_parent(&window);
        if let Some(dir) = from.parent() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(name) = from.file_name() {
            dialog = dialog.set_file_name(name.to_string_lossy());
        }
        let Some(to) = dialog.save_file() else {
            return Ok(None);
        };
        docs.rename_file(doc, &to).map(Some)
    })
    .await
}

/// The folder picked for the last split into files, per document, so a
/// "replace the files?" answer does not ask for the folder again.
static SPLIT_FOLDER: std::sync::Mutex<Option<(u32, PathBuf)>> = std::sync::Mutex::new(None);

/// Split shown rows into files in a folder chosen in a native dialog.
/// With `replace` the folder picked last time for this document is used
/// and files already there are replaced. `None` if cancelled.
#[tauri::command]
async fn split_to_files(
    window: WebviewWindow,
    docs: DocsState<'_>,
    recent: RecentState<'_>,
    doc: u32,
    spec: natendo_engine::tools::SplitParts,
    replace: bool,
) -> CmdResult<Option<docs::SplitReport>> {
    let docs = docs.inner().clone();
    let recent = recent.inner().clone();
    blocking(move || {
        let path = docs.path_of(doc)?;
        let dir = if replace {
            match SPLIT_FOLDER
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
            {
                // Used once: a later "replace" must come from a new folder pick.
                Some((d, dir)) if d == doc => dir,
                _ => return Err(CmdError::new("bad_request", "choose the folder again")),
            }
        } else {
            let mut dialog = rfd::FileDialog::new()
                .set_title("Choose the folder for the new files")
                .set_parent(&window);
            if let Some(dir) = path.as_deref().and_then(std::path::Path::parent) {
                dialog = dialog.set_directory(dir);
            }
            let Some(dir) = dialog.pick_folder() else {
                return Ok(None);
            };
            *SPLIT_FOLDER.lock().unwrap_or_else(|p| p.into_inner()) = Some((doc, dir.clone()));
            dir
        };
        let base = path
            .as_deref()
            .and_then(std::path::Path::file_stem)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "part".into());
        // From a private tab, each new file goes into private file mode as
        // soon as it is written. Into a folder that was empty, the folder
        // itself is marked instead (it holds only the split's files), so
        // big splits do not fill the list.
        let private = docs.is_private(doc)?;
        let was_empty = std::fs::read_dir(&dir).is_ok_and(|mut d| d.next().is_none());
        let whole_folder = private && was_empty && !recent.is_private(&dir);
        let marked = std::sync::atomic::AtomicU64::new(0);
        let missed = std::sync::atomic::AtomicU64::new(0);
        let on_written = |p: &std::path::Path| {
            if private && !whole_folder && !recent.is_private(p) {
                let n = if recent.set_private(p, true).is_ok() {
                    &marked
                } else {
                    &missed
                };
                n.fetch_add(1, Ordering::Relaxed);
            }
        };
        if whole_folder {
            recent.set_private(&dir, true)?;
        }
        let mut r = docs.split_to_files(doc, &spec, &dir, &base, replace, &on_written)?;
        if whole_folder {
            r.private = r.files;
            r.private_folder = true;
        }
        r.private = marked.into_inner();
        r.private_missed = missed.into_inner();
        Ok(Some(r))
    })
    .await
}

/// Stack several delimited files, picked in the Open dialog, into a new
/// tab. `None` if cancelled.
#[tauri::command]
async fn combine_files(
    window: WebviewWindow,
    docs: DocsState<'_>,
    spec: natendo_engine::tools::Combine,
) -> CmdResult<Option<DocInfo>> {
    let docs = docs.inner().clone();
    blocking(move || {
        let Some(paths) = rfd::FileDialog::new()
            .set_title("Choose the files to combine")
            .add_filter("Text tables", &["csv", "tsv", "tab", "txt", "psv", "dat"])
            .add_filter("All files", &["*"])
            .set_parent(&window)
            .pick_files()
        else {
            return Ok(None);
        };
        docs.combine_files(&paths, &spec).map(Some)
    })
    .await
}

/// Combine with a column map: pick the files (system dialog) and read their heads. `None` if cancelled.
#[tauri::command]
async fn combine_pick(
    window: WebviewWindow,
    docs: DocsState<'_>,
    spec: natendo_engine::tools::Combine,
) -> CmdResult<Option<docs::CombineStart>> {
    let docs = docs.inner().clone();
    blocking(move || {
        let Some(paths) = rfd::FileDialog::new()
            .set_title("Choose the files to combine")
            .add_filter("Text tables", &["csv", "tsv", "tab", "txt", "psv", "dat"])
            .add_filter("All files", &["*"])
            .set_parent(&window)
            .pick_files()
        else {
            return Ok(None);
        };
        docs.combine_start(paths, &spec).map(Some)
    })
    .await
}

#[tauri::command]
async fn combine_check(
    docs: DocsState<'_>,
    token: u64,
    plan: natendo_engine::combine::CombinePlan,
) -> CmdResult<docs::CombineChecked> {
    let docs = docs.inner().clone();
    blocking(move || docs.combine_check(token, plan)).await
}

#[tauri::command]
async fn combine_refresh(docs: DocsState<'_>, token: u64) -> CmdResult<docs::CombineStart> {
    let docs = docs.inner().clone();
    blocking(move || docs.combine_refresh(token)).await
}

#[tauri::command]
async fn combine_run(
    docs: DocsState<'_>,
    token: u64,
    plan: natendo_engine::combine::CombinePlan,
    source: bool,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.combine_run(token, plan, source)).await
}

#[tauri::command]
async fn combine_cancel(docs: DocsState<'_>, token: u64) -> CmdResult<()> {
    docs.inner().combine_cancel(token);
    Ok(())
}

/// Save the column map (titles only) where the person picks. `false` if cancelled.
#[tauri::command]
async fn combine_save_mapping(
    window: WebviewWindow,
    docs: DocsState<'_>,
    token: u64,
    plan: natendo_engine::combine::CombinePlan,
    extras: natendo_engine::combine::Extras,
) -> CmdResult<bool> {
    let docs = docs.inner().clone();
    blocking(move || {
        let bytes = docs.combine_mapping(token, &plan, extras)?;
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save the column map")
            .set_file_name("column map.ndcombine")
            .add_filter("Column maps", &["ndcombine"])
            .set_parent(&window)
            .save_file()
        else {
            return Ok(false);
        };
        docs.check_writable(&path)?;
        natendo_engine::save::write_atomic(&path, |w| {
            use std::io::Write;
            Ok(w.write_all(&bytes)?)
        })?;
        Ok(true)
    })
    .await
}

/// Load a saved column map and fit it to the picked files. `None` if cancelled.
#[tauri::command]
async fn combine_load_mapping(
    window: WebviewWindow,
    docs: DocsState<'_>,
    token: u64,
) -> CmdResult<Option<docs::CombineFit>> {
    let docs = docs.inner().clone();
    blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Use a saved column map")
            .add_filter("Column maps", &["ndcombine"])
            .set_parent(&window)
            .pick_file()
        else {
            return Ok(None);
        };
        let size = std::fs::metadata(&path)
            .map_err(natendo_engine::Error::from)?
            .len();
        if size > natendo_engine::combine::MAX_MAPPING_BYTES as u64 {
            return Err(CmdError::new("invalid", "a column map is at most 1 MB"));
        }
        let bytes = std::fs::read(&path).map_err(natendo_engine::Error::from)?;
        docs.combine_apply_mapping(token, &bytes).map(Some)
    })
    .await
}

/// Show the document's file selected in File Explorer.
#[tauri::command]
async fn show_in_folder(docs: DocsState<'_>, doc: u32) -> CmdResult<()> {
    let docs = docs.inner().clone();
    blocking(move || {
        let path = docs
            .path_of(doc)?
            .ok_or_else(|| CmdError::new("no_path", "this document has no file yet"))?;
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let explorer = std::env::var_os("SystemRoot")
                .map(|r| PathBuf::from(r).join("explorer.exe"))
                .unwrap_or_else(|| PathBuf::from(r"C:\Windows\explorer.exe"));
            // The document's own path (never one from the webview); a
            // Windows path cannot hold a double quote, so quoting is safe.
            let p = path.to_string_lossy();
            if p.contains('"') {
                return Err(CmdError::new("error", "this file name cannot be shown"));
            }
            std::process::Command::new(explorer)
                .raw_arg(format!("/select,\"{p}\""))
                .spawn()
                .map(|_| ())
                .map_err(|e| CmdError::new("error", e.to_string()))
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Err(CmdError::new(
                "error",
                "open the folder from the file manager",
            ))
        }
    })
    .await
}

#[tauri::command]
async fn clean_suggestions(
    docs: DocsState<'_>,
    doc: u32,
    skip_first: bool,
    read: natendo_engine::value::ReadOptions,
) -> CmdResult<natendo_engine::suggest::Suggestions> {
    let docs = docs.inner().clone();
    blocking(move || docs.suggestions(doc, skip_first, read)).await
}

#[tauri::command]
async fn column_summary(
    docs: DocsState<'_>,
    doc: u32,
    col: u32,
    skip_first: bool,
    read: natendo_engine::value::ReadOptions,
) -> CmdResult<natendo_engine::stats::Summary> {
    let docs = docs.inner().clone();
    blocking(move || docs.summary(doc, col, skip_first, read)).await
}

#[tauri::command]
async fn chart_data(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::stats::ChartSpec,
) -> CmdResult<natendo_engine::stats::ChartData> {
    let docs = docs.inner().clone();
    blocking(move || docs.chart_data(doc, &spec)).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn highlights(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    rows: u64,
    col: u32,
    cols: u32,
    rules: Vec<natendo_engine::highlight::Highlight>,
    skip_first: bool,
    read: natendo_engine::value::ReadOptions,
) -> CmdResult<Vec<natendo_engine::highlight::Mark>> {
    let docs = docs.inner().clone();
    blocking(move || docs.highlights(doc, row, rows, col, cols, &rules, skip_first, read)).await
}

/// Show the Open dialog and open the chosen file. `None` if cancelled.
#[tauri::command]
async fn open_file(
    window: WebviewWindow,
    docs: DocsState<'_>,
    imports: ImportsState<'_>,
    recent: RecentState<'_>,
) -> CmdResult<Option<Opened>> {
    let docs = docs.inner().clone();
    let imports = imports.inner().clone();
    let recent = recent.inner().clone();
    blocking(move || {
        let all: Vec<&str> = FILE_TYPES
            .iter()
            .chain(SHEET_TYPES)
            .chain(OTHER_TYPES)
            .chain(PACKED_TYPES)
            .chain(SHAPE_TYPES)
            .chain(natendo_engine::sqlite::OPENED_EXTENSIONS)
            .copied()
            .collect();
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open")
            .add_filter("Tables", &all)
            .add_filter("Delimited text", FILE_TYPES)
            .add_filter("Spreadsheets", SHEET_TYPES)
            .add_filter("Zipped tables (.gz, .zip)", PACKED_TYPES)
            .add_filter(
                "Databases and GeoPackages",
                natendo_engine::sqlite::OPENED_EXTENSIONS,
            )
            .add_filter("Shapefile attribute tables (.dbf, .shp)", &["dbf", "shp"])
            .add_filter("All files", &["*"])
            .set_parent(&window)
            .pick_file()
        else {
            return Ok(None);
        };
        open_any(&docs, &imports, &recent, &path).map(Some)
    })
    .await
}

/// Show the Import dialog (spreadsheets and other formats). `None` if
/// cancelled.
#[tauri::command]
async fn import_file(
    window: WebviewWindow,
    imports: ImportsState<'_>,
) -> CmdResult<Option<ImportStart>> {
    let imports = imports.inner().clone();
    blocking(move || {
        let all: Vec<&str> = SHEET_TYPES.iter().chain(OTHER_TYPES).copied().collect();
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import")
            .add_filter("Spreadsheets and other tables", &all)
            .add_filter("Excel and OpenDocument", SHEET_TYPES)
            .add_filter("JSON", &["json", "jsonl", "ndjson"])
            .add_filter("XML", &["xml"])
            .add_filter("Web page", &["html", "htm"])
            .add_filter("Markdown", &["md"])
            .add_filter("All files", &["*"])
            .set_parent(&window)
            .pick_file()
        else {
            return Ok(None);
        };
        imports.open_path(&path).map(Some)
    })
    .await
}

/// Start an import from clipboard text (read by the UI).
#[tauri::command]
async fn import_text(imports: ImportsState<'_>, text: String) -> CmdResult<ImportStart> {
    let imports = imports.inner().clone();
    blocking(move || imports.open_text(text)).await
}

#[tauri::command]
async fn import_inspect(
    imports: ImportsState<'_>,
    id: u32,
    format: Option<ImportFormat>,
) -> CmdResult<ImportInfo> {
    let imports = imports.inner().clone();
    blocking(move || imports.inspect(id, format)).await
}

#[tauri::command]
async fn import_preview(
    imports: ImportsState<'_>,
    id: u32,
    options: ImportOptions,
) -> CmdResult<Vec<Vec<String>>> {
    let imports = imports.inner().clone();
    blocking(move || imports.preview(id, &options)).await
}

#[tauri::command]
async fn import_notes(
    docs: DocsState<'_>,
    imports: ImportsState<'_>,
    id: u32,
    options: ImportOptions,
) -> CmdResult<imports::ImportNotes> {
    let docs = docs.inner().clone();
    let imports = imports.inner().clone();
    blocking(move || imports.notes(id, &options, &docs)).await
}

#[tauri::command]
async fn import_finish(
    docs: DocsState<'_>,
    imports: ImportsState<'_>,
    id: u32,
    options: ImportOptions,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    let imports = imports.inner().clone();
    blocking(move || imports.finish(id, &options, &docs)).await
}

#[tauri::command]
fn import_progress(imports: ImportsState<'_>, id: u32) -> CmdResult<Option<f64>> {
    imports.progress(id)
}

#[tauri::command]
fn import_cancel(imports: ImportsState<'_>, id: u32) -> CmdResult<()> {
    imports.cancel(id)
}

#[tauri::command]
fn import_close(imports: ImportsState<'_>, id: u32) {
    imports.close(id)
}

/// A page of the changes a cleaner would make, with exact counts over everything it would look at.
#[tauri::command]
async fn clean_preview_page(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::clean::CleanSpec,
    now: f64,
    offset: u64,
    want: usize,
) -> CmdResult<natendo_engine::clean::CleanPreview> {
    let docs = docs.inner().clone();
    blocking(move || docs.clean_preview_page(doc, &spec, now, offset, want)).await
}

/// The first changes a cleaner would make.
#[tauri::command]
async fn clean_preview(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::clean::CleanSpec,
    now: f64,
) -> CmdResult<natendo_engine::clean::CleanPreview> {
    let docs = docs.inner().clone();
    blocking(move || docs.clean_preview(doc, &spec, now)).await
}

/// Save a cleaner to a file chosen in the Save dialog. Returns the file's
/// name, `None` if cancelled.
#[tauri::command]
async fn cleaner_save(
    window: WebviewWindow,
    docs: DocsState<'_>,
    cleaner: natendo_engine::clean::Cleaner,
) -> CmdResult<Option<String>> {
    let docs = docs.inner().clone();
    blocking(move || {
        let bytes = cleaner.to_json()?;
        let name = if cleaner.name.trim().is_empty() {
            "cleaner".to_string()
        } else {
            safe_file_name(&cleaner.name)
        };
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save cleaner")
            .set_file_name(format!("{name}.{CLEANER_EXT}"))
            .add_filter("Natendo cleaner", &[CLEANER_EXT])
            .set_parent(&window)
            .save_file()
        else {
            return Ok(None);
        };
        docs.check_writable(&path)?;
        extras::write_atomic(&path, &bytes)?;
        Ok(Some(
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ))
    })
    .await
}

/// Open a cleaner file chosen in the Open dialog. The file is untrusted and
/// checked as it is read. `None` if cancelled.
#[tauri::command]
async fn cleaner_open(window: WebviewWindow) -> CmdResult<Option<natendo_engine::clean::Cleaner>> {
    blocking(move || Ok(pick_cleaner(&window)?.map(|(c, _)| c))).await
}

/// A cleaner and the name of the file it was read from.
#[derive(serde::Serialize)]
struct NamedCleaner {
    cleaner: natendo_engine::clean::Cleaner,
    file: String,
}

/// [`cleaner_open`] with the file's name (no folder), for the clean matrix.
#[tauri::command]
async fn cleaner_open_named(window: WebviewWindow) -> CmdResult<Option<NamedCleaner>> {
    blocking(move || {
        Ok(pick_cleaner(&window)?.map(|(cleaner, path)| NamedCleaner {
            cleaner,
            file: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }))
    })
    .await
}

fn pick_cleaner(
    window: &WebviewWindow,
) -> CmdResult<Option<(natendo_engine::clean::Cleaner, std::path::PathBuf)>> {
    use natendo_engine::clean::{Cleaner, MAX_CLEANER_BYTES};
    let Some(path) = rfd::FileDialog::new()
        .set_title("Open cleaner")
        .add_filter("Natendo cleaner", &[CLEANER_EXT])
        .add_filter("All files", &["*"])
        .set_parent(window)
        .pick_file()
    else {
        return Ok(None);
    };
    let bytes = extras::read_capped(&path, MAX_CLEANER_BYTES as u64, "a cleaner file")?;
    Ok(Some((Cleaner::from_json(&bytes)?, path)))
}

/// The extension of cleaner files.
const CLEANER_EXT: &str = "ndclean";

/// A cleaner's name made safe as a file name.
fn safe_file_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        "cleaner".into()
    } else {
        s
    }
}

/// The first rows of an export, as text.
#[tauri::command]
async fn export_preview(
    docs: DocsState<'_>,
    doc: u32,
    options: ExportOptions,
) -> CmdResult<String> {
    let docs = docs.inner().clone();
    blocking(move || docs.export_preview(doc, &options)).await
}

/// Export to a file chosen in the native Save dialog. `None` if cancelled.
#[tauri::command]
async fn export_file(
    window: WebviewWindow,
    docs: DocsState<'_>,
    doc: u32,
    options: ExportOptions,
) -> CmdResult<Option<ExportReport>> {
    let docs = docs.inner().clone();
    blocking(move || {
        let ext = options.format.extension();
        let base = docs.info(doc)?.name;
        let base = base
            .rsplit_once('.')
            .map_or(base.as_str(), |(b, _)| b)
            .to_string();
        let mut dialog = rfd::FileDialog::new()
            .set_title("Export")
            .set_file_name(format!("{base}.{ext}"))
            .add_filter(ext.to_ascii_uppercase(), &[ext])
            .add_filter("All files", &["*"])
            .set_parent(&window);
        if let Some(dir) = docs.path_of(doc)?.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return Ok(None);
        };
        // Parts go beside the chosen name: ask before replacing any.
        let (same, other) = docs.xlsx_part_clash(doc, &path, &options)?;
        if !same.is_empty() || !other.is_empty() {
            let folder = path
                .parent()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let mut text = String::new();
            if !same.is_empty() {
                text.push_str(&format!(
                    "{} already in {folder}: {}{}. Exporting replaces {}.",
                    if same.len() == 1 { "1 part file is".to_string() } else { format!("{} part files are", same.len()) },
                    same[0],
                    if same.len() > 1 { format!(" to {}", same[same.len() - 1]) } else { String::new() },
                    if same.len() == 1 { "it" } else { "them" },
                ));
            }
            if !other.is_empty() {
                if !text.is_empty() {
                    text.push_str("

");
                }
                text.push_str(&format!(
                    "{} from an earlier export in another number of parts ({}{}) {} there too; {} left as {}.",
                    if other.len() == 1 { "1 file".to_string() } else { format!("{} files", other.len()) },
                    other[0],
                    if other.len() > 1 { ", …" } else { "" },
                    if other.len() == 1 { "is" } else { "are" },
                    if other.len() == 1 { "it is" } else { "they are" },
                    if other.len() == 1 { "it is" } else { "they are" },
                ));
            }
            let go = rfd::MessageDialog::new()
                .set_title(if same.is_empty() { "Earlier parts are there" } else { "Replace the part files?" })
                .set_description(text)
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::OkCancelCustom(
                    if same.is_empty() { "Export".into() } else { "Replace".into() },
                    "Cancel".into(),
                ))
                .set_parent(&window)
                .show();
            if !matches!(go, rfd::MessageDialogResult::Ok | rfd::MessageDialogResult::Yes)
                && !matches!(&go, rfd::MessageDialogResult::Custom(s) if s == "Replace" || s == "Export")
            {
                return Ok(None);
            }
        }
        docs.export_file(doc, &path, &options).map(Some)
    })
    .await
}

/// Several tabs as the sheets of one Excel workbook, saved where the
/// native Save dialog says. `None` if cancelled.
#[tauri::command]
async fn export_book(
    window: WebviewWindow,
    docs: DocsState<'_>,
    docs_ids: Vec<u32>,
    options: ExportOptions,
    widths: Option<Vec<Vec<u32>>>,
) -> CmdResult<Option<ExportReport>> {
    let docs = docs.inner().clone();
    blocking(move || {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save the tabs as one Excel workbook")
            .set_file_name("Workbook.xlsx")
            .add_filter("XLSX", &["xlsx"])
            .set_parent(&window);
        if let Some(&first) = docs_ids.first() {
            if let Some(dir) = docs.path_of(first)?.as_ref().and_then(|p| p.parent()) {
                dialog = dialog.set_directory(dir);
            }
        }
        let Some(path) = dialog.save_file() else {
            return Ok(None);
        };
        docs.export_book(&docs_ids, &path, &options, &widths.unwrap_or_default())
            .map(Some)
    })
    .await
}

/// Whether a tab's file is in private file mode (itself, or its folder).
#[tauri::command]
fn private_state(
    docs: DocsState<'_>,
    recent: RecentState<'_>,
    doc: u32,
) -> CmdResult<extras::PrivateState> {
    let mut state = docs
        .path_of(doc)?
        .map(|p| recent.private_state(&p))
        .unwrap_or_default();
    // A tab made from a private one (a sample, an import) has no file of
    // its own in the list but is private all the same.
    if !state.file && !state.folder && docs.is_private(doc)? {
        state.file = true;
    }
    Ok(state)
}

/// Put a tab's file (or its folder, with `folder`) in private file mode, or
/// take it out. Tabs with no file yet are private for this session only.
#[tauri::command]
fn set_private(
    docs: DocsState<'_>,
    recent: RecentState<'_>,
    doc: u32,
    folder: bool,
    on: bool,
) -> CmdResult<extras::PrivateState> {
    let state = match docs.path_of(doc)? {
        Some(p) => {
            let target = if folder {
                p.parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or(p.clone())
            } else {
                p.clone()
            };
            recent.set_private(&target, on)?;
            // Every open tab under it follows (old recovery copies and
            // earlier saves are deleted only when the user says so).
            docs.private_changed();
            recent.private_state(&p)
        }
        None => extras::PrivateState {
            file: on,
            folder: false,
        },
    };
    docs.set_private(doc, state.file || state.folder)?;
    Ok(state)
}

/// The file (or with `folder`, the folder) a private file mode switch is about.
fn private_target(docs: &Docs, doc: u32, folder: bool) -> CmdResult<PathBuf> {
    let p = docs
        .path_of(doc)?
        .ok_or_else(|| CmdError::new("no_path", "this tab has no file yet"))?;
    Ok(if folder {
        p.parent().map(std::path::Path::to_path_buf).unwrap_or(p)
    } else {
        p
    })
}

/// Earlier saves and recovery copies kept for the file (or its folder):
/// (earlier saves, recovery copies).
#[tauri::command]
async fn private_leftovers(docs: DocsState<'_>, doc: u32, folder: bool) -> CmdResult<(u64, u64)> {
    let docs = docs.inner().clone();
    blocking(move || Ok(recovery::count_path(&private_target(&docs, doc, folder)?))).await
}

/// Delete them (the user said yes). Only for a file or folder in private
/// file mode. Returns how many went.
#[tauri::command]
async fn private_forget(
    docs: DocsState<'_>,
    recent: RecentState<'_>,
    doc: u32,
    folder: bool,
) -> CmdResult<u64> {
    let (docs, recent) = (docs.inner().clone(), recent.inner().clone());
    blocking(move || {
        let target = private_target(&docs, doc, folder)?;
        if !recent.is_private(&target) {
            return Err(CmdError::new("invalid", "that is not in private file mode"));
        }
        Ok(recovery::forget_path(&target))
    })
    .await
}

/// Forget the kept scramble key (the cleaning and share-safe screens
/// closed).
#[tauri::command]
fn forget_scramble_key() {
    natendo_engine::privacy::forget_keys();
}

/// Columns that look like they hold private data.
#[tauri::command]
async fn find_private(
    docs: DocsState<'_>,
    doc: u32,
    header: bool,
    read: natendo_engine::value::ReadOptions,
    today: f64,
) -> CmdResult<Vec<natendo_engine::privacy::Finding>> {
    let docs = docs.inner().clone();
    blocking(move || docs.find_private(doc, header, read, today)).await
}

/// The running folder search, and the files the last one found (only
/// those can be opened from its results).
static FOLDER_SEARCH: Mutex<Option<Arc<natendo_engine::document::Progress>>> = Mutex::new(None);
static FOLDER_FOUND: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Pick the folder to search in the native dialog. `None` if cancelled.
#[tauri::command]
async fn pick_search_folder(
    window: WebviewWindow,
    docs: DocsState<'_>,
    doc: Option<u32>,
) -> CmdResult<Option<String>> {
    let docs = docs.inner().clone();
    blocking(move || {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Choose the folder to search")
            .set_parent(&window);
        if let Some(dir) = doc
            .and_then(|d| docs.path_of(d).ok().flatten())
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        {
            dialog = dialog.set_directory(dir);
        }
        Ok(dialog.pick_folder().map(|p| p.display().to_string()))
    })
    .await
}

/// Search the table files in a folder (see `natendo_engine::folder`).
#[tauri::command]
async fn search_folder(
    dir: String,
    spec: natendo_engine::folder::FolderSearch,
) -> CmdResult<natendo_engine::folder::FolderReport> {
    blocking(move || {
        let dir = PathBuf::from(dir);
        if !dir.is_dir() {
            return Err(CmdError::new("invalid", "choose a folder to search"));
        }
        let progress = Arc::new(natendo_engine::document::Progress::new());
        {
            let mut running = FOLDER_SEARCH.lock().unwrap_or_else(|p| p.into_inner());
            if running.is_some() {
                return Err(CmdError::new("busy", "a folder search is already running"));
            }
            *running = Some(progress.clone());
        }
        let r = natendo_engine::folder::search_folder(&dir, &spec, &progress);
        *FOLDER_SEARCH.lock().unwrap_or_else(|p| p.into_inner()) = None;
        let r = r?;
        let mut found: Vec<PathBuf> = r.hits.iter().map(|h| h.path.clone()).collect();
        found.dedup();
        *FOLDER_FOUND.lock().unwrap_or_else(|p| p.into_inner()) = found;
        Ok(r)
    })
    .await
}

/// How far the folder search is (0 to 1), if one runs.
#[tauri::command]
fn folder_search_progress() -> Option<f64> {
    FOLDER_SEARCH
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .map(|p| p.fraction())
}

/// Stop the folder search (before its next file).
#[tauri::command]
fn folder_search_cancel() {
    if let Some(p) = FOLDER_SEARCH
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
    {
        p.cancel.store(true, Ordering::Relaxed);
    }
}

/// Open a file the last folder search found (or show its tab).
#[tauri::command]
async fn open_folder_hit(
    docs: DocsState<'_>,
    imports: ImportsState<'_>,
    recent: RecentState<'_>,
    path: String,
) -> CmdResult<Opened> {
    let (docs, imports, recent) = (
        docs.inner().clone(),
        imports.inner().clone(),
        recent.inner().clone(),
    );
    blocking(move || {
        let path = PathBuf::from(path);
        if !FOLDER_FOUND
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&path)
        {
            return Err(CmdError::new("invalid", "search the folder again"));
        }
        if let Some(id) = docs.find_open(&path) {
            return Ok(Opened {
                doc: Some(docs.info(id)?),
                import: None,
                zip: None,
                sqlite: None,
            });
        }
        open_any(&docs, &imports, &recent, &path)
    })
    .await
}

static DISK_WATCH: std::sync::LazyLock<diskwatch::DiskWatch> =
    std::sync::LazyLock::new(Default::default);

/// Look whether another program changed an open file. Waits at most two
/// seconds, off the window's thread.
#[tauri::command]
async fn disk_check(docs: DocsState<'_>) -> CmdResult<Vec<diskwatch::DiskEvent>> {
    let docs = docs.inner().clone();
    blocking(move || Ok(DISK_WATCH.round(docs.disk_marks(), |id| docs.disk_mark(id)))).await
}

/// Compare a tab's file as it was last read or saved with the file on disk
/// now: opens the disk's version and the differences as new tabs.
#[tauri::command]
async fn disk_compare(
    docs: DocsState<'_>,
    doc: u32,
    header: bool,
) -> CmdResult<(DocInfo, DocInfo)> {
    let docs = docs.inner().clone();
    blocking(move || docs.disk_compare(doc, header)).await
}

/// What changed since the file was opened or last saved.
#[tauri::command]
async fn change_list(
    docs: DocsState<'_>,
    doc: u32,
) -> CmdResult<natendo_engine::changes::ChangeList> {
    let docs = docs.inner().clone();
    blocking(move || docs.change_list(doc, 2_000)).await
}

/// Cells on screen that differ from the file.
#[tauri::command]
async fn edited_cells(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    rows: u32,
    col: u32,
    cols: u32,
) -> CmdResult<Vec<(u64, u32)>> {
    let docs = docs.inner().clone();
    blocking(move || docs.edited_cells(doc, row, rows, col, cols)).await
}

/// Save the list of changes as a CSV report chosen in the Save dialog.
/// `None` if cancelled; else the number of changes written.
#[tauri::command]
async fn save_change_report(
    window: WebviewWindow,
    docs: DocsState<'_>,
    recent: RecentState<'_>,
    doc: u32,
    guard: Option<bool>,
) -> CmdResult<Option<docs::Written>> {
    let docs = docs.inner().clone();
    let recent = recent.inner().clone();
    blocking(move || {
        let base = docs.info(doc)?.name;
        let stem = base.rsplit_once('.').map_or(base.as_str(), |(b, _)| b);
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save the list of changes")
            .set_file_name(format!("{stem} (changes).csv"))
            .add_filter("CSV", &["csv"])
            .set_parent(&window);
        if let Some(dir) = docs.path_of(doc)?.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return Ok(None);
        };
        let n = docs.save_change_report(doc, &path, guard.unwrap_or(true))?;
        private_copy(&docs, &recent, doc, &path)?;
        Ok(Some(n))
    })
    .await
}

/// A recipe with each step in words.
#[derive(serde::Serialize)]
struct RecipeView {
    recipe: natendo_engine::recipe::Recipe,
    described: Vec<String>,
    /// Steps that hold values from the file (filter ticks, texts, examples).
    holds_values: Vec<bool>,
}

fn described(r: &natendo_engine::recipe::Recipe) -> Vec<String> {
    r.steps.iter().map(|s| s.describe()).collect()
}

/// A file written from a private tab (a report, labels, a recipe) goes into
/// private file mode too. Returns whether it did.
fn private_copy(docs: &Docs, recent: &Recent, doc: u32, path: &std::path::Path) -> CmdResult<bool> {
    if !docs.is_private(doc)? || recent.is_private(path) {
        return Ok(false);
    }
    recent.set_private(path, true)?;
    docs.private_changed();
    Ok(true)
}

/// The database file last picked to open (only that one can be read).
static SQLITE_PICKED: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Pick a database file; its tables. `None` if cancelled.
#[tauri::command]
async fn sqlite_pick(
    window: WebviewWindow,
) -> CmdResult<Option<(String, Vec<natendo_engine::sqlite::DbTable>)>> {
    blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open a table from a database")
            .add_filter("SQLite database", natendo_engine::sqlite::EXTENSIONS)
            .set_parent(&window)
            .pick_file()
        else {
            return Ok(None);
        };
        pick_database(&path).map(Some)
    })
    .await
}

/// A table opened from a database: the tab, and a sentence about numbers SQLite keeps as binary
/// (empty when there is nothing to say).
#[derive(serde::Serialize)]
struct SqliteOpened {
    info: DocInfo,
    note: String,
}

/// Open a table of the picked database in a new tab.
#[tauri::command]
async fn sqlite_open(docs: DocsState<'_>, table: String) -> CmdResult<SqliteOpened> {
    let docs = docs.inner().clone();
    blocking(move || {
        let path = SQLITE_PICKED
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| CmdError::new("invalid", "choose the database first"))?;
        let (info, note) = docs.open_sqlite_report(&path, &table)?;
        Ok(SqliteOpened { info, note })
    })
    .await
}

/// Save a tab as a table of a database file chosen in the Save dialog
/// (an existing file is added to). `None` if cancelled.
#[tauri::command]
async fn sqlite_save(
    window: WebviewWindow,
    docs: DocsState<'_>,
    recent: RecentState<'_>,
    doc: u32,
    table: String,
    header: bool,
    replace: bool,
) -> CmdResult<Option<u64>> {
    let docs = docs.inner().clone();
    let recent = recent.inner().clone();
    blocking(move || {
        let base = docs.info(doc)?.name;
        let stem = base.rsplit_once('.').map_or(base.as_str(), |(b, _)| b);
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save as a table in a database")
            .set_file_name(format!("{stem}.sqlite"))
            .add_filter("SQLite database", natendo_engine::sqlite::EXTENSIONS)
            .set_parent(&window);
        if let Some(dir) = docs.path_of(doc)?.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return Ok(None);
        };
        let n = docs.save_sqlite(doc, &path, &table, header, replace)?;
        private_copy(&docs, &recent, doc, &path)?;
        Ok(Some(n))
    })
    .await
}

/// The watched folder (one per app).
static WATCHER: std::sync::LazyLock<watch::Watcher> =
    std::sync::LazyLock::new(watch::Watcher::default);

/// Choose the watched folder (`which` "folder" or "out") or the recipe
/// ("recipe") in the native dialog; the new status.
#[tauri::command]
async fn watch_pick(window: WebviewWindow, which: String) -> CmdResult<watch::WatchStatus> {
    blocking(move || {
        match which.as_str() {
            "folder" | "out" => {
                let title = if which == "folder" {
                    "Choose the folder to watch"
                } else {
                    "Choose the folder for the results"
                };
                if let Some(p) = rfd::FileDialog::new()
                    .set_title(title)
                    .set_parent(&window)
                    .pick_folder()
                {
                    let mut s = WATCHER.setup.lock().unwrap_or_else(|e| e.into_inner());
                    if which == "folder" {
                        s.folder = Some(p);
                    } else {
                        s.out = Some(p);
                    }
                }
            }
            "recipe" => {
                if let Some(p) = rfd::FileDialog::new()
                    .set_title("Choose the recipe to run")
                    .add_filter("Recipe", &["json"])
                    .set_parent(&window)
                    .pick_file()
                {
                    let r = docs::read_recipe(&p)?;
                    WATCHER
                        .setup
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .recipe = Some((p, r));
                }
            }
            _ => return Err(CmdError::new("invalid", "unknown choice")),
        }
        Ok(WATCHER.status())
    })
    .await
}

#[tauri::command]
fn watch_status() -> watch::WatchStatus {
    WATCHER.status()
}

/// Start watching (`force`: run even when columns differ; `existing`: run
/// the files already there too).
#[tauri::command]
fn watch_start(
    recent: RecentState<'_>,
    force: bool,
    existing: bool,
    receipt: Option<watch::ReceiptAsk>,
) -> CmdResult<watch::WatchStatus> {
    WATCHER
        .start(force, existing, recent.inner().clone(), receipt)
        .map_err(|e| CmdError::new("invalid", e))?;
    Ok(WATCHER.status())
}

#[tauri::command]
fn watch_stop() -> watch::WatchStatus {
    WATCHER.stop_now();
    WATCHER.status()
}

/// Notes settings: (key columns, header, notes count).
#[tauri::command]
async fn notes_info(docs: DocsState<'_>, doc: u32) -> CmdResult<(Vec<u32>, bool, usize)> {
    let docs = docs.inner().clone();
    blocking(move || docs.notes_info(doc)).await
}

#[tauri::command]
async fn note_marks(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    rows: u64,
) -> CmdResult<Vec<natendo_engine::notes::NoteMark>> {
    let docs = docs.inner().clone();
    blocking(move || docs.note_marks(doc, row, rows)).await
}

/// Set a row's note and flag (written beside the file at once).
#[tauri::command]
async fn note_set(
    docs: DocsState<'_>,
    recent: RecentState<'_>,
    doc: u32,
    row: u64,
    flag: u8,
    text: String,
) -> CmdResult<()> {
    let docs = docs.inner().clone();
    let recent = recent.inner().clone();
    blocking(move || {
        let side = docs.note_set(doc, row, flag, &text)?;
        if side.exists() {
            private_copy(&docs, &recent, doc, &side)?;
        }
        Ok(())
    })
    .await
}

#[tauri::command]
async fn notes_set_keys(
    docs: DocsState<'_>,
    doc: u32,
    cols: Vec<u32>,
    header: bool,
) -> CmdResult<()> {
    let docs = docs.inner().clone();
    blocking(move || docs.notes_set_keys(doc, cols, header)).await
}

#[tauri::command]
async fn note_list(
    docs: DocsState<'_>,
    doc: u32,
) -> CmdResult<(Vec<natendo_engine::notes::NoteMark>, u64)> {
    let docs = docs.inner().clone();
    blocking(move || docs.note_list(doc, 5_000)).await
}

#[tauri::command]
async fn notes_show(docs: DocsState<'_>, doc: u32, flag: u8) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.notes_show(doc, flag)).await
}

/// Save the shown rows as an encrypted file chosen in the Save dialog.
/// `None` if cancelled; else the rows written.
#[tauri::command]
async fn save_encrypted(
    window: WebviewWindow,
    docs: DocsState<'_>,
    doc: u32,
    password: String,
    same: bool,
) -> CmdResult<Option<u64>> {
    let docs = docs.inner().clone();
    blocking(move || {
        if password.chars().count() < natendo_engine::crypt::MIN_PASSWORD_CHARS {
            return Err(CmdError::new(
                "invalid",
                format!(
                    "the password must be at least {} characters",
                    natendo_engine::crypt::MIN_PASSWORD_CHARS
                ),
            ));
        }
        if same {
            return docs.save_encrypted_again(doc, &password).map(Some);
        }
        let base = docs.info(doc)?.name;
        let stem = base.rsplit_once('.').map_or(base.as_str(), |(b, _)| b);
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save an encrypted copy")
            .set_file_name(format!("{stem}.ndenc"))
            .add_filter("Encrypted table", &["ndenc"])
            .set_parent(&window);
        if let Some(dir) = docs.path_of(doc)?.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return Ok(None);
        };
        docs.save_encrypted(doc, &path, &password).map(Some)
    })
    .await
}

/// The encrypted file a tab saves to (opened from or saved to), if any.
#[tauri::command]
fn encrypted_origin(docs: DocsState<'_>, doc: u32) -> CmdResult<Option<String>> {
    Ok(docs.encrypted_from(doc)?.map(|p| p.display().to_string()))
}

/// Where to save a patch of tabs `a` (`None` if cancelled).
#[tauri::command]
async fn patch_pick_save(
    window: WebviewWindow,
    docs: DocsState<'_>,
    doc: u32,
) -> CmdResult<Option<String>> {
    let docs = docs.inner().clone();
    blocking(move || {
        let base = docs.info(doc)?.name;
        let stem = base.rsplit_once('.').map_or(base.as_str(), |(b, _)| b);
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save changes as a patch")
            .set_file_name(format!("{stem}.ndpatch"))
            .add_filter("Natendo patch", &["ndpatch"])
            .set_parent(&window);
        if let Some(dir) = docs.path_of(doc)?.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        Ok(dialog.save_file().map(|p| p.display().to_string()))
    })
    .await
}

/// A patch file to apply, and whether it needs a phrase.
#[tauri::command]
async fn patch_pick(window: WebviewWindow) -> CmdResult<Option<String>> {
    blocking(move || {
        Ok(rfd::FileDialog::new()
            .set_title("Apply a patch")
            .add_filter("Natendo patch", &["ndpatch"])
            .set_parent(&window)
            .pick_file()
            .map(|p| p.display().to_string()))
    })
    .await
}

/// Whether a patch file needs a phrase.
#[tauri::command]
async fn patch_protected(path: PathBuf) -> CmdResult<bool> {
    blocking(move || Ok(natendo_engine::patch::Patch::is_protected(&path)?)).await
}

/// Write the differences from tab `a` to tab `b` as a patch file.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn patch_make(
    docs: DocsState<'_>,
    recent: RecentState<'_>,
    a: u32,
    b: u32,
    spec: natendo_engine::patch::PatchSpec,
    path: PathBuf,
    phrase: Option<String>,
    plain: bool,
) -> CmdResult<docs::patching::PatchSaved> {
    let docs = docs.inner().clone();
    let recent = recent.inner().clone();
    blocking(move || {
        let saved = docs.patch_make(a, b, &spec, &path, phrase, plain)?;
        private_copy(&docs, &recent, a, &path)?;
        private_copy(&docs, &recent, b, &path)?;
        Ok(saved)
    })
    .await
}

/// The dry run of a patch file on a tab.
#[tauri::command]
async fn patch_preview(
    docs: DocsState<'_>,
    doc: u32,
    path: PathBuf,
    phrase: Option<String>,
) -> CmdResult<docs::patching::PatchPreview> {
    let docs = docs.inner().clone();
    blocking(move || docs.patch_preview(doc, &path, phrase)).await
}

#[tauri::command]
async fn patch_apply(docs: DocsState<'_>, doc: u32) -> CmdResult<docs::patching::PatchApplied> {
    let docs = docs.inner().clone();
    blocking(move || docs.patch_apply(doc)).await
}

/// Save the dry run's lines as CSV (`None` if cancelled).
#[tauri::command]
async fn patch_report_save(
    window: WebviewWindow,
    docs: DocsState<'_>,
    recent: RecentState<'_>,
    doc: u32,
) -> CmdResult<Option<String>> {
    let docs = docs.inner().clone();
    let recent = recent.inner().clone();
    blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save the patch report")
            .set_file_name("patch report.csv")
            .add_filter("CSV", &["csv"])
            .set_parent(&window)
            .save_file()
        else {
            return Ok(None);
        };
        docs.patch_report_save(doc, &path)?;
        private_copy(&docs, &recent, doc, &path)?;
        Ok(Some(path.display().to_string()))
    })
    .await
}

#[tauri::command]
async fn patch_close(docs: DocsState<'_>, doc: u32) -> CmdResult<()> {
    docs.patch_close(doc);
    Ok(())
}

/// Pick an encrypted file to open (`None` if cancelled).
#[tauri::command]
async fn pick_encrypted(window: WebviewWindow) -> CmdResult<Option<String>> {
    blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open an encrypted file")
            .add_filter("Encrypted table", &["ndenc"])
            .set_parent(&window)
            .pick_file()
        else {
            return Ok(None);
        };
        *ENCRYPTED_PICKED.lock().unwrap_or_else(|p| p.into_inner()) = Some(path.clone());
        Ok(Some(path.display().to_string()))
    })
    .await
}

/// The encrypted file last picked (only that one can be opened).
static ENCRYPTED_PICKED: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Open the picked encrypted file with its password.
#[tauri::command]
async fn open_encrypted(docs: DocsState<'_>, password: String) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || {
        let path = ENCRYPTED_PICKED
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| CmdError::new("invalid", "choose the file first"))?;
        let info = docs.open_encrypted(&path, &password)?;
        *ENCRYPTED_PICKED.lock().unwrap_or_else(|p| p.into_inner()) = None;
        Ok(info)
    })
    .await
}

/// Compare with a newer copy (rows paired by key columns).
#[tauri::command]
async fn updates_find(
    docs: DocsState<'_>,
    doc: u32,
    other: u32,
    spec: natendo_engine::updates::UpdateSpec,
) -> CmdResult<natendo_engine::updates::UpdateList> {
    let docs = docs.inner().clone();
    blocking(move || docs.updates_find(doc, other, &spec)).await
}

/// Apply the accepted changes from the newer copy.
#[tauri::command]
async fn updates_accept(
    docs: DocsState<'_>,
    doc: u32,
    choice: natendo_engine::updates::UpdateChoice,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.updates_accept(doc, &choice)).await
}

#[tauri::command]
fn updates_close(docs: DocsState<'_>, doc: u32) {
    docs.updates_close(doc);
}

/// What a street lookup did.
#[derive(serde::Serialize)]
struct StreetDone {
    info: DocInfo,
    report: natendo_engine::streets::StreetReport,
}

/// Street range lookup from another tab.
#[tauri::command]
async fn street_lookup(
    docs: DocsState<'_>,
    doc: u32,
    other: u32,
    cols: natendo_engine::streets::IndexCols,
    spec: natendo_engine::streets::StreetSpec,
) -> CmdResult<StreetDone> {
    let docs = docs.inner().clone();
    blocking(move || {
        let (info, report) = docs.street_lookup(doc, other, &cols, &spec)?;
        Ok(StreetDone { info, report })
    })
    .await
}

/// Match people: pairs of rows that look like the same person.
#[tauri::command]
async fn match_find(
    docs: DocsState<'_>,
    doc: u32,
    other: Option<u32>,
    spec: natendo_engine::fuzzy::MatchSpec,
    bring: Vec<u32>,
) -> CmdResult<natendo_engine::fuzzy::MatchReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.match_find(doc, other, &spec, &bring)).await
}

/// Add columns for the accepted pairs.
#[tauri::command]
async fn match_accept(
    docs: DocsState<'_>,
    doc: u32,
    accepted: Vec<(u32, u64, f32)>,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.match_accept(doc, &accepted)).await
}

#[tauri::command]
fn match_close(docs: DocsState<'_>, doc: u32) {
    docs.match_close(doc);
}

/// Preview golden records from a one-list match's accepted pairs.
#[tauri::command]
async fn golden_plan(
    docs: DocsState<'_>,
    doc: u32,
    accepted: Vec<(u32, u64, f32)>,
    spec: natendo_engine::golden::GoldenSpec,
) -> CmdResult<docs::golden::GoldenSummary> {
    let docs = docs.inner().clone();
    blocking(move || docs.golden_plan(doc, &accepted, spec)).await
}

#[tauri::command]
async fn golden_page(
    docs: DocsState<'_>,
    doc: u32,
    page: usize,
) -> CmdResult<Vec<natendo_engine::golden::ClusterView>> {
    let docs = docs.inner().clone();
    blocking(move || docs.golden_page(doc, page)).await
}

#[tauri::command]
async fn golden_create(
    docs: DocsState<'_>,
    doc: u32,
    away: bool,
) -> CmdResult<docs::golden::GoldenMade> {
    let docs = docs.inner().clone();
    blocking(move || docs.golden_create(doc, away)).await
}

#[tauri::command]
async fn golden_close(docs: DocsState<'_>, doc: u32) -> CmdResult<()> {
    docs.golden_close(doc);
    Ok(())
}

/// The steps done on a tab so far, as a recipe.
#[tauri::command]
async fn recipe_get(docs: DocsState<'_>, doc: u32, header: bool) -> CmdResult<RecipeView> {
    let docs = docs.inner().clone();
    blocking(move || {
        let recipe = docs.recipe_of(doc, header, "")?;
        Ok(RecipeView {
            described: described(&recipe),
            holds_values: recipe.steps.iter().map(|s| s.holds_values()).collect(),
            recipe,
        })
    })
    .await
}

/// Save a tab's steps as a recipe file chosen in the Save dialog. `None`
/// if cancelled; else the file's path. Secret phrases are never saved.
#[tauri::command]
async fn recipe_save(
    window: WebviewWindow,
    docs: DocsState<'_>,
    recent: RecentState<'_>,
    doc: u32,
    header: bool,
    name: String,
) -> CmdResult<Option<String>> {
    let docs = docs.inner().clone();
    let recent = recent.inner().clone();
    blocking(move || {
        let recipe = docs.recipe_of(doc, header, &name)?;
        if recipe.steps.is_empty() {
            return Err(CmdError::new("invalid", "there are no steps to save yet"));
        }
        let file = if recipe.name.trim().is_empty() {
            "recipe".to_string()
        } else {
            recipe
                .name
                .chars()
                .map(|c| if r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
                .collect()
        };
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save the recipe")
            .set_file_name(format!("{}.recipe.json", file.trim()))
            .add_filter("Recipe", &["json"])
            .set_parent(&window);
        if let Some(dir) = docs.path_of(doc)?.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return Ok(None);
        };
        docs.save_recipe(doc, &recipe, &path)?;
        private_copy(&docs, &recent, doc, &path)?;
        Ok(Some(path.display().to_string()))
    })
    .await
}

/// A recipe file opened to run, with how its columns differ from the tab's.
#[derive(serde::Serialize)]
struct RecipeOpened {
    path: String,
    recipe: natendo_engine::recipe::Recipe,
    described: Vec<String>,
    holds_values: Vec<bool>,
    problems: Vec<String>,
}

/// Pick a recipe file in the Open dialog and read it. `None` if cancelled.
#[tauri::command]
async fn recipe_open(
    window: WebviewWindow,
    docs: DocsState<'_>,
    doc: u32,
) -> CmdResult<Option<RecipeOpened>> {
    let docs = docs.inner().clone();
    blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open a recipe")
            .add_filter("Recipe", &["json"])
            .set_parent(&window)
            .pick_file()
        else {
            return Ok(None);
        };
        let recipe = docs::read_recipe(&path)?;
        let problems = docs.recipe_problems(doc, &recipe)?;
        Ok(Some(RecipeOpened {
            path: path.display().to_string(),
            described: described(&recipe),
            holds_values: recipe.steps.iter().map(|s| s.holds_values()).collect(),
            recipe,
            problems,
        }))
    })
    .await
}

/// Run a recipe on a tab, each step one undo step. Refused when the
/// columns differ, unless `force`.
#[tauri::command]
async fn recipe_run(
    docs: DocsState<'_>,
    doc: u32,
    recipe: natendo_engine::recipe::Recipe,
    answers: Option<Vec<(String, String)>>,
    now: f64,
    force: bool,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    let answers = answers.unwrap_or_default();
    blocking(move || docs.run_recipe(doc, &recipe, &answers, now, force)).await
}

/// The steps panel: a tab's steps and where the table is among them.
#[tauri::command]
async fn steps_view(docs: DocsState<'_>, doc: u32) -> CmdResult<docs::StepsView> {
    let docs = docs.inner().clone();
    blocking(move || docs.steps_view(doc)).await
}

/// Show the table as it was after step `k` (`None`: before the first).
#[tauri::command]
async fn steps_goto(docs: DocsState<'_>, doc: u32, k: Option<usize>) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.steps_goto(doc, k)).await
}

/// The tab's checkpoints, each with whether the undo history still reaches it.
#[tauri::command]
async fn checkpoint_list(docs: DocsState<'_>, doc: u32) -> CmdResult<Vec<docs::CheckpointInfo>> {
    let docs = docs.inner().clone();
    blocking(move || docs.checkpoint_list(doc)).await
}

/// Name this moment.
#[tauri::command]
async fn checkpoint_add(
    docs: DocsState<'_>,
    doc: u32,
    name: String,
) -> CmdResult<Vec<docs::CheckpointInfo>> {
    let docs = docs.inner().clone();
    blocking(move || docs.checkpoint_add(doc, &name)).await
}

/// Forget a checkpoint (the table does not change).
#[tauri::command]
async fn checkpoint_remove(
    docs: DocsState<'_>,
    doc: u32,
    checkpoint: u32,
) -> CmdResult<Vec<docs::CheckpointInfo>> {
    let docs = docs.inner().clone();
    blocking(move || docs.checkpoint_remove(doc, checkpoint)).await
}

/// What changed since a checkpoint: the counts and one page of the changed cells (a job with progress and Cancel).
#[tauri::command]
async fn checkpoint_since(
    docs: DocsState<'_>,
    doc: u32,
    checkpoint: u32,
    header: bool,
    offset: u64,
    want: usize,
) -> CmdResult<docs::SinceView> {
    let docs = docs.inner().clone();
    blocking(move || docs.checkpoint_since(doc, checkpoint, header, offset, want)).await
}

/// Save what changed since a checkpoint as a CSV report chosen in the Save dialog; `None` if cancelled.
#[tauri::command]
async fn checkpoint_since_report(
    window: WebviewWindow,
    docs: DocsState<'_>,
    recent: RecentState<'_>,
    doc: u32,
    checkpoint: u32,
    header: bool,
    guard: Option<bool>,
) -> CmdResult<Option<docs::Written>> {
    let docs = docs.inner().clone();
    let recent = recent.inner().clone();
    blocking(move || {
        let base = docs.info(doc)?.name;
        let stem = base.rsplit_once('.').map_or(base.as_str(), |(b, _)| b);
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save what changed since the checkpoint")
            .set_file_name(format!("{stem} (changes since checkpoint).csv"))
            .add_filter("CSV", &["csv"])
            .set_parent(&window);
        if let Some(dir) = docs.path_of(doc)?.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return Ok(None);
        };
        let guard = guard.unwrap_or(true);
        let n = docs.checkpoint_since_report(doc, checkpoint, header, &path, guard)?;
        private_copy(&docs, &recent, doc, &path)?;
        Ok(Some(n))
    })
    .await
}

/// Go to the state a checkpoint names.
#[tauri::command]
async fn checkpoint_restore(docs: DocsState<'_>, doc: u32, checkpoint: u32) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.checkpoint_restore(doc, checkpoint)).await
}

/// Change a step; the steps after it run again.
#[tauri::command]
async fn steps_change(
    docs: DocsState<'_>,
    doc: u32,
    k: usize,
    change: docs::StepChange,
    now: f64,
    force: bool,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.steps_change(doc, k, change, now, force)).await
}

/// Make a value in a step a question asked each time the recipe runs.
#[tauri::command]
async fn steps_ask(
    docs: DocsState<'_>,
    doc: u32,
    k: usize,
    path: String,
    name: String,
    prompt: String,
) -> CmdResult<docs::StepsView> {
    let docs = docs.inner().clone();
    blocking(move || docs.steps_ask(doc, k, &path, &name, &prompt)).await
}

static BATCH: std::sync::LazyLock<batch::Batch> = std::sync::LazyLock::new(batch::Batch::default);

/// Run recipe on files: choose the files ("files" or a "folder"), the
/// "recipe" or the "out" folder, in the app's own dialogs.
#[tauri::command]
async fn batch_pick(
    window: WebviewWindow,
    recent: RecentState<'_>,
    which: String,
) -> CmdResult<batch::BatchStatus> {
    let recent = recent.inner().clone();
    blocking(move || {
        match which.as_str() {
            "files" => {
                if let Some(ps) = rfd::FileDialog::new()
                    .set_title("Choose the files to run the recipe on")
                    .add_filter("Tables", &["csv", "tsv", "txt", "psv", "tab"])
                    .set_parent(&window)
                    .pick_files()
                {
                    let mut ps = ps;
                    ps.truncate(batch::MAX_FILES);
                    BATCH.setup.lock().unwrap_or_else(|e| e.into_inner()).files = ps;
                }
            }
            "folder" => {
                if let Some(p) = rfd::FileDialog::new()
                    .set_title("Choose a folder of files to run the recipe on")
                    .set_parent(&window)
                    .pick_folder()
                {
                    BATCH.setup.lock().unwrap_or_else(|e| e.into_inner()).files =
                        batch::files_in(&p);
                }
            }
            "out" => {
                if let Some(p) = rfd::FileDialog::new()
                    .set_title("Choose the folder for the results")
                    .set_parent(&window)
                    .pick_folder()
                {
                    BATCH.setup.lock().unwrap_or_else(|e| e.into_inner()).out = Some(p);
                }
            }
            "recipe" => {
                if let Some(p) = rfd::FileDialog::new()
                    .set_title("Choose the recipe to run")
                    .add_filter("Recipe", &["json"])
                    .set_parent(&window)
                    .pick_file()
                {
                    let r = docs::read_recipe(&p)?;
                    BATCH.setup.lock().unwrap_or_else(|e| e.into_inner()).recipe = Some((p, r));
                }
            }
            _ => return Err(CmdError::new("invalid", "unknown choice")),
        }
        {
            // Private files: their answers are not remembered by the window.
            let mut s = BATCH.setup.lock().unwrap_or_else(|e| e.into_inner());
            s.private = s.files.iter().any(|f| recent.is_private(f));
        }
        Ok(BATCH.status())
    })
    .await
}

#[tauri::command]
fn batch_status() -> batch::BatchStatus {
    BATCH.status()
}

/// Run the recipe on every chosen file, with the answers to its questions
/// (`force`: run even when a file's columns differ).
#[tauri::command]
fn batch_start(
    recent: RecentState<'_>,
    answers: Vec<(String, String)>,
    force: bool,
    existing: Option<batch::Existing>,
    receipt: Option<watch::ReceiptAsk>,
) -> CmdResult<batch::BatchStatus> {
    BATCH
        .start(&answers, force, existing, recent.inner().clone(), receipt)
        .map_err(|(kind, e)| CmdError::new(kind, e))?;
    Ok(BATCH.status())
}

#[tauri::command]
fn batch_stop() -> batch::BatchStatus {
    BATCH.stop_now();
    BATCH.status()
}

/// Label sheets to choose from (name, sizes).
#[tauri::command]
fn label_sheets() -> Vec<(String, natendo_engine::mail::LabelSheet)> {
    natendo_engine::mail::LABEL_SHEETS
        .iter()
        .map(|(n, s)| ((*n).to_string(), *s))
        .collect()
}

/// The first few labels or letters as text.
#[tauri::command]
async fn mail_preview(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::mail::MailSpec,
) -> CmdResult<Vec<Vec<String>>> {
    let docs = docs.inner().clone();
    blocking(move || docs.mail_preview(doc, &spec)).await
}

/// Write labels or letters to a PDF chosen in the native Save dialog.
/// `None` if cancelled.
#[tauri::command]
async fn save_mail(
    window: WebviewWindow,
    docs: DocsState<'_>,
    recent: RecentState<'_>,
    doc: u32,
    spec: natendo_engine::mail::MailSpec,
) -> CmdResult<Option<natendo_engine::mail::MailReport>> {
    let docs = docs.inner().clone();
    let recent = recent.inner().clone();
    blocking(move || {
        let base = docs.info(doc)?.name;
        let stem = base.rsplit_once('.').map_or(base.as_str(), |(b, _)| b);
        let what = match spec.kind {
            natendo_engine::mail::MailKind::Labels => "labels",
            natendo_engine::mail::MailKind::Letters => "letters",
        };
        let mut dialog = rfd::FileDialog::new()
            .set_title(format!("Save the {what} as PDF"))
            .set_file_name(format!("{stem} ({what}).pdf"))
            .add_filter("PDF", &["pdf"])
            .set_parent(&window);
        if let Some(dir) = docs.path_of(doc)?.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return Ok(None);
        };
        let rep = docs.save_mail(doc, &path, &spec)?;
        private_copy(&docs, &recent, doc, &path)?;
        Ok(Some(rep))
    })
    .await
}

/// Write a share-safe copy to a file chosen in the native Save dialog.
/// `None` if cancelled.
#[tauri::command]
async fn save_safe_copy(
    window: WebviewWindow,
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::privacy::SafeCopy,
    now: f64,
) -> CmdResult<Option<natendo_engine::privacy::SafeCopyReport>> {
    let docs = docs.inner().clone();
    blocking(move || {
        let base = docs.info(doc)?.name;
        let (stem, ext) = base
            .rsplit_once('.')
            .map_or((base.as_str(), "csv"), |(b, e)| (b, e));
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save a share-safe copy")
            .set_file_name(format!("{stem} (share-safe).{ext}"))
            .add_filter(ext.to_ascii_uppercase(), &[ext])
            .add_filter("All files", &["*"])
            .set_parent(&window);
        if let Some(dir) = docs.path_of(doc)?.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return Ok(None);
        };
        docs.save_safe_copy(doc, &path, &spec, now).map(Some)
    })
    .await
}

/// Export as text for the clipboard (written there by the UI).
#[tauri::command]
async fn export_text(docs: DocsState<'_>, doc: u32, options: ExportOptions) -> CmdResult<String> {
    let docs = docs.inner().clone();
    blocking(move || docs.export_text(doc, &options)).await
}

/// Open the document's own file again, optionally reading quoted cells line
/// by line (for files where a stray quote swallowed rows) or with a layout
/// the user chose (`None`: detect it again). Returns the new document; the
/// old one stays open until the UI closes it.
#[tauri::command]
async fn reopen(
    docs: DocsState<'_>,
    doc: u32,
    line_by_line: bool,
    layout: Option<Layout>,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || {
        let path = docs
            .path_of(doc)?
            .ok_or_else(|| CmdError::new("no_path", "this document has no file yet"))?;
        docs.open_with(&path, line_by_line, layout.as_ref())
    })
    .await
}

/// Open the file of tab `doc` again as another encoding (a new document; the old one stays open until the screen
/// closes it, so Cancel or an error keeps it). Refused with unsaved changes: they would be lost.
#[tauri::command]
async fn reopen_encoding(
    docs: DocsState<'_>,
    doc: u32,
    encoding: String,
    line_by_line: bool,
    layout: Option<Layout>,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || {
        let path = docs
            .path_of(doc)?
            .ok_or_else(|| CmdError::new("no_path", "this document has no file yet"))?;
        if docs.info(doc)?.modified {
            return Err(CmdError::new(
                "modified",
                "Save or undo your changes first: reopening reads the file again from disk.",
            ));
        }
        let enc = natendo_engine::encoding::Encoding::from_label(&encoding)
            .ok_or_else(|| CmdError::new("invalid", "that encoding is not known"))?;
        docs.open_with_encoding(&path, line_by_line, layout.as_ref(), Some(enc))
    })
    .await
}

/// The encodings the picker offers.
#[tauri::command]
fn encoding_choices() -> Vec<docs::EncodingChoice> {
    docs::Docs::encoding_choices()
}

/// The first rows of the file read as another encoding (the picker's scrub preview).
#[tauri::command]
async fn encoding_preview(
    docs: DocsState<'_>,
    doc: u32,
    encoding: String,
    rows: usize,
) -> CmdResult<Vec<Vec<String>>> {
    let docs = docs.inner().clone();
    blocking(move || docs.encoding_preview(doc, &encoding, rows)).await
}

/// Read the file as another ASCII-compatible encoding without reopening (one undo step).
#[tauri::command]
async fn encoding_apply(docs: DocsState<'_>, doc: u32, encoding: String) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.encoding_apply(doc, &encoding)).await
}

/// Look for rows whose bytes are in another encoding (counts only).
#[tauri::command]
async fn scan_encodings(
    docs: DocsState<'_>,
    doc: u32,
    keep_first: bool,
) -> CmdResult<natendo_engine::encscan::EncodingScan> {
    let docs = docs.inner().clone();
    blocking(move || docs.scan_encodings(doc, keep_first)).await
}

/// Show only the rows the encoding scan flagged.
#[tauri::command]
async fn encoding_show_rows(docs: DocsState<'_>, doc: u32, keep_first: bool) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.encoding_show_rows(doc, keep_first)).await
}

/// The likely layouts of a document's file, with previews.
#[tauri::command]
async fn layout_choices(docs: DocsState<'_>, doc: u32) -> CmdResult<LayoutChoices> {
    let docs = docs.inner().clone();
    blocking(move || docs.layout_choices(doc)).await
}

/// The first rows of a document's file read with another layout.
#[tauri::command]
async fn preview_layout(
    docs: DocsState<'_>,
    doc: u32,
    layout: Layout,
) -> CmdResult<Vec<Vec<String>>> {
    let docs = docs.inner().clone();
    blocking(move || docs.preview_layout(doc, &layout)).await
}

#[tauri::command]
fn new_document(docs: DocsState<'_>) -> CmdResult<DocInfo> {
    docs.new_document()
}

#[tauri::command]
fn close_document(
    app: tauri::AppHandle,
    docs: DocsState<'_>,
    folders: FoldersState<'_>,
    wins: State<'_, windows::Windows>,
    doc: u32,
) {
    docs.close(doc);
    folders.forget(doc);
    wins.forget(doc);
    windows::maybe_close_second(&app);
}

#[tauri::command]
async fn doc_info(docs: DocsState<'_>, doc: u32) -> CmdResult<DocInfo> {
    // Never on the window's thread: while a change holds the document this answers at once from the
    // last reading (marked stale); only the very first question can wait, on the blocking pool.
    let docs = docs.inner().clone();
    blocking(move || docs.info_now(doc)).await
}

/// A window of cells in the engine's binary form (no JSON on the hot path).
#[tauri::command]
async fn viewport(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    rows: u32,
    col: u32,
    cols: u32,
    max_chars: u32,
) -> CmdResult<tauri::ipc::Response> {
    docs.viewport(doc, row, rows, col, cols, max_chars)
        .map(tauri::ipc::Response::new)
}

#[derive(Serialize)]
struct CellText {
    text: String,
    truncated: bool,
}

#[tauri::command]
async fn cell_text(docs: DocsState<'_>, doc: u32, row: u64, col: u32) -> CmdResult<CellText> {
    let (text, truncated) = docs.cell_text(doc, row, col)?;
    Ok(CellText { text, truncated })
}

#[tauri::command]
async fn date_scan(
    docs: DocsState<'_>,
    doc: u32,
    col: u32,
    keep_first: bool,
    sample: Option<u64>,
) -> CmdResult<natendo_engine::datescan::DateScan> {
    let docs = docs.inner().clone();
    blocking(move || docs.date_scan(doc, col, keep_first, sample)).await
}

#[tauri::command]
async fn date_marks(
    docs: DocsState<'_>,
    doc: u32,
    col: u32,
    keep_first: bool,
    row: u64,
    rows: u64,
) -> CmdResult<Vec<u64>> {
    let docs = docs.inner().clone();
    blocking(move || docs.date_marks(doc, col, keep_first, row, rows)).await
}

#[tauri::command]
async fn date_list(
    docs: DocsState<'_>,
    doc: u32,
    col: u32,
    keep_first: bool,
) -> CmdResult<Vec<(u64, u64, String)>> {
    let docs = docs.inner().clone();
    blocking(move || docs.date_list(doc, col, keep_first)).await
}

/// Report notes (C4): why rows cannot be set aside (`None`: they can).
#[tauri::command]
async fn set_aside_check(
    docs: DocsState<'_>,
    doc: u32,
    at: u64,
    count: u64,
    place: natendo_engine::document::report::Place,
) -> CmdResult<Option<String>> {
    let docs = docs.inner().clone();
    blocking(move || docs.set_aside_check(doc, at, count, place)).await
}

#[tauri::command]
async fn set_aside(
    docs: DocsState<'_>,
    doc: u32,
    at: u64,
    count: u64,
    place: natendo_engine::document::report::Place,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.set_aside(doc, at, count, place)).await
}

#[tauri::command]
async fn fill_blanks_count(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::tools::FillBlanks,
) -> CmdResult<natendo_engine::tools::FillCount> {
    let docs = docs.inner().clone();
    blocking(move || docs.fill_blanks_count(doc, &spec)).await
}

#[tauri::command]
async fn fill_blanks_count_stop(docs: DocsState<'_>, doc: u32) -> CmdResult<()> {
    let docs = docs.inner().clone();
    blocking(move || docs.fill_blanks_count_stop(doc)).await
}

#[tauri::command]
async fn fill_marks(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    rows: u64,
) -> CmdResult<Vec<(u64, u32)>> {
    let docs = docs.inner().clone();
    blocking(move || docs.fill_marks(doc, row, rows)).await
}

#[tauri::command]
async fn fill_marks_next(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    col: u32,
    back: bool,
) -> CmdResult<Option<(u64, u32)>> {
    let docs = docs.inner().clone();
    blocking(move || docs.fill_marks_next(doc, row, col, back)).await
}

#[tauri::command]
async fn fill_marks_clear(docs: DocsState<'_>, doc: u32) -> CmdResult<()> {
    let docs = docs.inner().clone();
    blocking(move || docs.fill_marks_clear(doc)).await
}

#[tauri::command]
async fn total_row(
    docs: DocsState<'_>,
    doc: u32,
    header: bool,
    read: natendo_engine::value::ReadOptions,
) -> CmdResult<Option<natendo_engine::document::report::TotalRow>> {
    let docs = docs.inner().clone();
    blocking(move || docs.total_row(doc, header, read)).await
}

#[tauri::command]
async fn report_notes(
    docs: DocsState<'_>,
    doc: u32,
) -> CmdResult<natendo_engine::document::report::ReportNotes> {
    let docs = docs.inner().clone();
    blocking(move || docs.report_notes(doc)).await
}

#[tauri::command]
async fn put_back_notes(docs: DocsState<'_>, doc: u32) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.put_back_notes(doc)).await
}

#[tauri::command]
async fn keep_notes(docs: DocsState<'_>, doc: u32, keep: bool) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.keep_notes(doc, keep)).await
}

#[tauri::command]
async fn gap_hidden(docs: DocsState<'_>, doc: u32, row: u64) -> CmdResult<u64> {
    let docs = docs.inner().clone();
    blocking(move || docs.gap_hidden(doc, row)).await
}

#[tauri::command]
async fn date_find(
    docs: DocsState<'_>,
    doc: u32,
    col: u32,
    keep_first: bool,
    from: u64,
    forward: bool,
) -> CmdResult<Option<u64>> {
    let docs = docs.inner().clone();
    blocking(move || docs.date_find(doc, col, keep_first, from, forward)).await
}

#[tauri::command]
async fn ragged_list(
    docs: DocsState<'_>,
    doc: u32,
    keep_first: bool,
) -> CmdResult<docs::RaggedList> {
    let docs = docs.inner().clone();
    blocking(move || docs.ragged_list(doc, keep_first)).await
}

#[tauri::command]
async fn row_text(
    docs: DocsState<'_>,
    doc: u32,
    row: Option<u64>,
    row_id: Option<u32>,
    keep_first: bool,
) -> CmdResult<natendo_engine::document::repair::RowText> {
    let docs = docs.inner().clone();
    blocking(move || docs.row_text(doc, row, row_id, keep_first)).await
}

#[tauri::command]
async fn row_width(docs: DocsState<'_>, doc: u32, row: u64) -> CmdResult<(u64, u32)> {
    let docs = docs.inner().clone();
    blocking(move || docs.row_width(doc, row)).await
}

#[tauri::command]
async fn same_shape(
    docs: DocsState<'_>,
    doc: u32,
    keep_first: bool,
    fields: u32,
    groups: Vec<u32>,
) -> CmdResult<natendo_engine::document::repair::SameShape> {
    let docs = docs.inner().clone();
    blocking(move || docs.same_shape(doc, keep_first, fields, &groups)).await
}

#[tauri::command]
async fn fix_all(
    docs: DocsState<'_>,
    doc: u32,
    keep_first: bool,
    revision: u64,
    fields: u32,
    groups: Vec<u32>,
    skip: Vec<u32>,
) -> CmdResult<(u64, DocInfo)> {
    let docs = docs.inner().clone();
    blocking(move || docs.fix_all(doc, keep_first, revision, fields, groups, &skip)).await
}

#[tauri::command]
async fn empty_extra_columns(
    docs: DocsState<'_>,
    doc: u32,
    keep_first: bool,
) -> CmdResult<(u32, u64)> {
    let docs = docs.inner().clone();
    blocking(move || docs.empty_extra_columns(doc, keep_first)).await
}

#[tauri::command]
async fn remove_empty_extra_columns(
    docs: DocsState<'_>,
    doc: u32,
    keep_first: bool,
    revision: u64,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.remove_empty_extra_columns(doc, keep_first, revision)).await
}

#[tauri::command]
async fn repair_row(
    docs: DocsState<'_>,
    doc: u32,
    row_id: u32,
    revision: u64,
    repair: natendo_engine::document::repair::Repair,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.repair_row(doc, row_id, revision, &repair)).await
}

#[tauri::command]
async fn set_cells(
    docs: DocsState<'_>,
    doc: u32,
    cells: Vec<(u64, u32, String)>,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.set_cells(doc, cells)).await
}

/// Say what a column holds (Text, Number, Date, Id, or Auto to guess again).
#[tauri::command]
async fn set_column_type(
    docs: DocsState<'_>,
    doc: u32,
    col: u32,
    kind: natendo_engine::coltype::ColumnType,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.set_column_type(doc, col, kind)).await
}

/// Put all the column types back (Undo, a saved view, a session).
#[tauri::command]
async fn set_column_types(
    docs: DocsState<'_>,
    doc: u32,
    types: natendo_engine::coltype::ColumnTypes,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.set_column_types(doc, types)).await
}

#[tauri::command]
async fn hidden_between(docs: DocsState<'_>, doc: u32, row: u64, rows: u64) -> CmdResult<u64> {
    let docs = docs.inner().clone();
    blocking(move || docs.hidden_between(doc, row, rows)).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn fill_cells(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    rows: u64,
    col: u32,
    cols: u32,
    value: String,
    skip: Option<Vec<u32>>,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    let skip = skip.unwrap_or_default();
    blocking(move || docs.fill_skipping(doc, row, rows, col, cols, value, &skip)).await
}

/// One value into every cell of several selected blocks, one undo step.
#[tauri::command]
async fn fill_areas(
    docs: DocsState<'_>,
    doc: u32,
    areas: Vec<natendo_engine::areas::Area>,
    value: String,
    skip: Option<Vec<u32>>,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    let skip = skip.unwrap_or_default();
    blocking(move || docs.fill_areas(doc, areas, value, &skip)).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn move_cells(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    rows: u64,
    col: u32,
    cols: u32,
    to_row: u64,
    to_col: u32,
    copy: bool,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.move_cells(doc, row, rows, col, cols, to_row, to_col, copy)).await
}

#[tauri::command]
async fn copy_cells(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    rows: u64,
    col: u32,
    cols: u32,
    skip: Option<Vec<u32>>,
) -> CmdResult<String> {
    let docs = docs.inner().clone();
    let skip = skip.unwrap_or_default();
    blocking(move || docs.copy_skipping(doc, row, rows, col, cols, &skip)).await
}

/// Row and column structure changes and undo/redo, in one command.
#[derive(serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Structure {
    InsertRows {
        at: u64,
        count: u64,
    },
    DeleteRows {
        at: u64,
        count: u64,
    },
    MoveRows {
        at: u64,
        count: u64,
        to: u64,
    },
    InsertCols {
        at: u32,
        count: u32,
        /// Part of something that is not a recipe step (Split into columns).
        #[serde(default)]
        unrecorded: bool,
    },
    DeleteCols {
        at: u32,
        count: u32,
    },
    MoveCols {
        at: u32,
        count: u32,
        to: u32,
    },
    HideRows {
        at: u64,
        count: u64,
    },
    ShowRows {
        which: natendo_engine::rows::ShowRows,
    },
    ShuffleRows {
        at: u64,
        count: u64,
    },
    FlipRows {
        at: u64,
        count: u64,
    },
    DeleteColList {
        cols: Vec<u32>,
    },
    Undo,
    Redo,
}

#[tauri::command]
async fn change(docs: DocsState<'_>, doc: u32, change: Structure) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || match change {
        Structure::InsertRows { at, count } => docs.insert_rows(doc, at, count),
        Structure::DeleteRows { at, count } => docs.delete_rows(doc, at, count),
        Structure::MoveRows { at, count, to } => docs.move_rows(doc, at, count, to),
        Structure::InsertCols {
            at,
            count,
            unrecorded,
        } => {
            if unrecorded {
                docs.insert_cols_unrecorded(doc, at, count)
            } else {
                docs.insert_cols(doc, at, count)
            }
        }
        Structure::DeleteCols { at, count } => docs.delete_cols(doc, at, count),
        Structure::MoveCols { at, count, to } => docs.move_cols(doc, at, count, to),
        Structure::HideRows { at, count } => docs.hide_rows(doc, at, count),
        Structure::ShowRows { which } => docs.show_rows(doc, which),
        Structure::ShuffleRows { at, count } => docs.shuffle_rows(doc, at, count),
        Structure::FlipRows { at, count } => docs.flip_rows(doc, at, count),
        Structure::DeleteColList { cols } => docs.delete_col_list(doc, cols),
        Structure::Undo => docs.undo(doc),
        Structure::Redo => docs.redo(doc),
    })
    .await
}

/// Save. `save_as` shows the Save dialog here; the webview never names a
/// path. `overwrite_changed` and `allow_lossy` are passed only after the
/// user agreed to the matching prompt. `None` means the dialog was cancelled.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn save(
    window: WebviewWindow,
    docs: DocsState<'_>,
    recent: RecentState<'_>,
    doc: u32,
    save_as: bool,
    overwrite_changed: bool,
    allow_lossy: bool,
    options: Option<SaveAs>,
    receipt: Option<watch::ReceiptAsk>,
) -> CmdResult<Option<DocInfo>> {
    let docs = docs.inner().clone();
    let recent = recent.inner().clone();
    blocking(move || {
        let current = docs.path_of(doc)?;
        let path: Option<PathBuf> = if save_as || current.is_none() {
            let name = match current.as_ref().and_then(|p| p.file_name()) {
                Some(n) => n.to_string_lossy().into_owned(),
                None => format!("{}.csv", docs::file_name_of(&docs.info(doc)?.name)),
            };
            let mut dialog = rfd::FileDialog::new()
                .set_title("Save As")
                .set_file_name(name)
                .add_filter("Delimited text", FILE_TYPES)
                .add_filter("All files", &["*"])
                .set_parent(&window);
            if let Some(dir) = current.as_ref().and_then(|p| p.parent()) {
                dialog = dialog.set_directory(dir);
            }
            match dialog.save_file() {
                Some(p) => Some(p),
                None => return Ok(None),
            }
        } else {
            None
        };
        let info = match receipt {
            Some(ask) => docs.save_with_receipt(
                doc,
                path,
                overwrite_changed,
                allow_lossy,
                options.as_ref(),
                ask,
            )?,
            None => docs.save(doc, path, overwrite_changed, allow_lossy, options.as_ref())?,
        };
        // A private tab saved somewhere new (a sample of a private list
        // saved as its own file, say) puts that file in private file mode,
        // so it reopens private.
        if docs.is_private(doc)? {
            if let Some(p) = docs.path_of(doc)? {
                if !recent.is_private(&p) {
                    recent.set_private(&p, true)?;
                    docs.private_changed();
                }
            }
        }
        Ok(Some(info))
    })
    .await
}

/// C11: whether the tab can be saved as JSON (and why not), and whether saving asks about blank cells.
#[tauri::command]
async fn json_check(
    docs: DocsState<'_>,
    doc: u32,
) -> CmdResult<natendo_engine::jsonshape::JsonCheck> {
    let docs = docs.inner().clone();
    blocking(move || docs.json_check(doc)).await
}

/// C11: save as JSON Lines or JSON. `save_as` shows the Save dialog here (the webview never names a
/// path), writing JSON Lines (`lines`) or a JSON array (`pretty`: over several lines); else the tab
/// saves back to the JSON file it came from, in that file's own layout. `overwrite_changed` is passed
/// only after the user agreed to save over a file another program changed. `None`: the dialog was
/// cancelled.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn save_json(
    window: WebviewWindow,
    docs: DocsState<'_>,
    recent: RecentState<'_>,
    doc: u32,
    save_as: bool,
    lines: bool,
    pretty: bool,
    blanks: natendo_engine::jsonshape::Blanks,
    overwrite_changed: bool,
) -> CmdResult<Option<docs::JsonSaved>> {
    let docs = docs.inner().clone();
    let recent = recent.inner().clone();
    blocking(move || {
        let (path, form) = if save_as {
            let ext = if lines { "jsonl" } else { "json" };
            let mut dialog = rfd::FileDialog::new()
                .set_title(if lines {
                    "Save As JSON Lines"
                } else {
                    "Save As JSON"
                })
                .set_file_name(format!("{}.{ext}", docs::file_name_of(&docs.stem(doc))))
                .set_parent(&window);
            dialog = if lines {
                dialog.add_filter("JSON Lines", &["jsonl", "ndjson"])
            } else {
                dialog.add_filter("JSON", &["json"])
            };
            dialog = dialog.add_filter("All files", &["*"]);
            if let Some(dir) = docs.path_of(doc)?.as_ref().and_then(|p| p.parent()) {
                dialog = dialog.set_directory(dir);
            }
            match dialog.save_file() {
                Some(p) => (
                    Some(p),
                    Some(natendo_engine::jsonshape::JsonForm {
                        lines,
                        indent: (!lines && pretty).then(|| "  ".to_string()),
                        crlf: false,
                        bom: false,
                        final_newline: true,
                    }),
                ),
                None => return Ok(None),
            }
        } else {
            (None, None)
        };
        let written = path.clone();
        let saved = docs.save_json(doc, path, form, blanks, overwrite_changed)?;
        // A private tab saved as a new file puts that file in private file mode.
        if let Some(p) = written {
            if docs.is_private(doc)? && !recent.is_private(&p) {
                recent.set_private(&p, true)?;
                docs.private_changed();
            }
        }
        Ok(Some(saved))
    })
    .await
}

/// What the last save with a receipt said about the receipt (once).
#[tauri::command]
fn receipt_note(docs: DocsState<'_>, doc: u32) -> Option<String> {
    docs.take_receipt_note(doc)
}

/// Tools > Verify a receipt: the receipt and the files to check against it are picked here, in
/// the app's own dialogs (the webview never names a path); paths written inside the receipt
/// are never opened. `None`: a dialog was cancelled.
#[tauri::command]
async fn receipt_verify(
    window: WebviewWindow,
    docs: DocsState<'_>,
    with_files: bool,
    chain: bool,
) -> CmdResult<Option<natendo_engine::receipt::VerifyReport>> {
    let docs = docs.inner().clone();
    blocking(move || {
        let Some(receipt) = rfd::FileDialog::new()
            .set_title("Verify a receipt")
            .add_filter("Receipts", &["json"])
            .set_parent(&window)
            .pick_file()
        else {
            return Ok(None);
        };
        let (mut inputs, mut outputs) = (Vec::new(), Vec::new());
        if with_files {
            let dir = receipt.parent().map(std::path::Path::to_path_buf);
            let pick = |title: &str| {
                let mut d = rfd::FileDialog::new().set_title(title).set_parent(&window);
                if let Some(dir) = &dir {
                    d = d.set_directory(dir);
                }
                d.pick_files().unwrap_or_default()
            };
            inputs = pick("The files that went in (Cancel for none)");
            outputs = pick("The files that came out (Cancel for none)");
        }
        docs.receipt_verify(&receipt, &inputs, &outputs, chain)
            .map(Some)
    })
    .await
}

/// Close the window after the user agreed to discard unsaved changes.
#[tauri::command]
fn job_progress(docs: DocsState<'_>, doc: u32) -> CmdResult<Option<JobInfo>> {
    docs.job(doc)
}

/// Stop the running long operation; with `label`, only if it is that one.
#[tauri::command]
fn cancel_job(docs: DocsState<'_>, doc: u32, label: Option<String>) -> CmdResult<()> {
    docs.cancel_job(doc, label.as_deref())
}

#[tauri::command]
async fn sort_rows(docs: DocsState<'_>, doc: u32, spec: SortSpec) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.sort(doc, spec)).await
}

#[tauri::command]
async fn set_filter(docs: DocsState<'_>, doc: u32, spec: FilterSpec) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.filter(doc, spec)).await
}

#[tauri::command]
async fn clear_filter(docs: DocsState<'_>, doc: u32) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.clear_filter(doc)).await
}

#[tauri::command]
async fn filter_spec(docs: DocsState<'_>, doc: u32) -> CmdResult<Option<FilterSpec>> {
    let docs = docs.inner().clone();
    blocking(move || docs.filter_spec(doc)).await
}

#[tauri::command]
async fn filter_chips(
    docs: DocsState<'_>,
    doc: u32,
) -> CmdResult<natendo_engine::chips::FilterChips> {
    let docs = docs.inner().clone();
    blocking(move || docs.filter_chips(doc)).await
}

#[tauri::command]
async fn filter_chip_counts(
    docs: DocsState<'_>,
    doc: u32,
) -> CmdResult<natendo_engine::chips::FilterChips> {
    let docs = docs.inner().clone();
    blocking(move || docs.filter_chip_counts(doc)).await
}

#[tauri::command]
fn filter_chip_counts_stop(docs: DocsState<'_>, doc: u32) -> CmdResult<()> {
    docs.filter_chip_counts_stop(doc)
}

#[tauri::command]
async fn filter_change(
    docs: DocsState<'_>,
    doc: u32,
    change: natendo_engine::chips::ChipChange,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.filter_change(doc, change)).await
}

#[tauri::command]
async fn filter_values(
    docs: DocsState<'_>,
    doc: u32,
    col: u32,
    spec: FilterSpec,
    search: Option<String>,
) -> CmdResult<ValueList> {
    let docs = docs.inner().clone();
    blocking(move || docs.filter_values(doc, col, spec, search)).await
}

#[tauri::command]
async fn find_next(
    docs: DocsState<'_>,
    doc: u32,
    spec: FindSpec,
    from: Option<(u64, u32)>,
    backward: bool,
) -> CmdResult<Option<Found>> {
    let docs = docs.inner().clone();
    blocking(move || docs.find_next(doc, spec, from, backward)).await
}

#[tauri::command]
async fn replace_page(
    docs: DocsState<'_>,
    doc: u32,
    offset: u64,
    count: usize,
) -> CmdResult<Vec<natendo_engine::find::Change>> {
    let docs = docs.inner().clone();
    blocking(move || docs.replace_page(doc, offset, count)).await
}

#[tauri::command]
async fn column_pairs(
    docs: DocsState<'_>,
    doc: u32,
    find_col: u32,
    with_col: u32,
    skip_first: bool,
) -> CmdResult<Vec<(String, String)>> {
    let docs = docs.inner().clone();
    blocking(move || docs.column_pairs(doc, find_col, with_col, skip_first)).await
}

#[tauri::command]
async fn show_changed(docs: DocsState<'_>, doc: u32, keep_first: bool) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.show_changed(doc, keep_first)).await
}

#[tauri::command]
async fn find_rows(
    docs: DocsState<'_>,
    doc: u32,
    spec: FindSpec,
    action: docs::FindRowsAction,
    revision: Option<u64>,
) -> CmdResult<docs::FindRowsReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.find_rows(doc, spec, action, revision)).await
}

#[tauri::command]
async fn find_notes(
    docs: DocsState<'_>,
    doc: u32,
    pattern: natendo_engine::find::Pattern,
) -> CmdResult<Vec<natendo_engine::notes::NoteMark>> {
    let docs = docs.inner().clone();
    blocking(move || docs.find_notes(doc, pattern)).await
}

#[tauri::command]
async fn replace_titles(
    docs: DocsState<'_>,
    doc: u32,
    spec: FindSpec,
    replacement: String,
) -> CmdResult<Vec<(u32, String)>> {
    let docs = docs.inner().clone();
    blocking(move || docs.replace_titles(doc, spec, replacement)).await
}

#[tauri::command]
async fn batch_preview(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::find::BatchReplace,
) -> CmdResult<ReplaceReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.batch_preview(doc, spec)).await
}

#[tauri::command]
async fn batch_confirm(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::find::BatchReplace,
) -> CmdResult<ReplaceReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.batch_confirm(doc, spec)).await
}

#[tauri::command]
async fn replace_cell(
    docs: DocsState<'_>,
    doc: u32,
    spec: FindSpec,
    replacement: String,
) -> CmdResult<ReplaceReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.replace_cell(doc, spec, replacement)).await
}

#[tauri::command]
async fn replace_confirm(
    docs: DocsState<'_>,
    doc: u32,
    spec: FindSpec,
    replacement: String,
    exclude: Option<Vec<(u64, u32)>>,
) -> CmdResult<ReplaceReport> {
    let docs = docs.inner().clone();
    let exclude = exclude.unwrap_or_default();
    blocking(move || docs.replace_confirm_except(doc, spec, replacement, exclude)).await
}

#[tauri::command]
fn find_count_cancel(docs: DocsState<'_>, doc: u32) -> CmdResult<()> {
    docs.find_count_cancel(doc)
}

#[tauri::command]
async fn set_doc_title(docs: DocsState<'_>, doc: u32, title: String) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.set_title(doc, &title)).await
}

#[tauri::command]
async fn find_count(
    docs: DocsState<'_>,
    doc: u32,
    spec: FindSpec,
    at: Option<(u64, u32)>,
) -> CmdResult<natendo_engine::find::FindCount> {
    let docs = docs.inner().clone();
    blocking(move || docs.find_count(doc, spec, at)).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn find_marks(
    docs: DocsState<'_>,
    doc: u32,
    spec: FindSpec,
    row: u64,
    rows: u64,
    col: u32,
    cols: u32,
) -> CmdResult<Vec<(u64, u32)>> {
    let docs = docs.inner().clone();
    blocking(move || docs.find_marks(doc, spec, row, rows, col, cols)).await
}

/// What Replace all would make of the cells on screen (the ghost diff).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn replace_ghosts(
    docs: DocsState<'_>,
    doc: u32,
    spec: FindSpec,
    replacement: String,
    row: u64,
    rows: u64,
    col: u32,
    cols: u32,
) -> CmdResult<natendo_engine::find::GhostWindow> {
    let docs = docs.inner().clone();
    blocking(move || docs.replace_ghosts(doc, spec, &replacement, row, rows, col, cols)).await
}

/// How many cells Replace all would change.
#[tauri::command]
async fn replace_count(
    docs: DocsState<'_>,
    doc: u32,
    spec: FindSpec,
    replacement: String,
) -> CmdResult<u64> {
    let docs = docs.inner().clone();
    blocking(move || docs.replace_count(doc, spec, &replacement)).await
}

/// Stop the count behind the ghost diff.
#[tauri::command]
fn replace_count_cancel(docs: DocsState<'_>, doc: u32) -> CmdResult<()> {
    docs.replace_count_cancel(doc)
}

#[tauri::command]
async fn find_all(docs: DocsState<'_>, doc: u32, spec: FindSpec) -> CmdResult<FindAll> {
    let docs = docs.inner().clone();
    blocking(move || docs.find_all(doc, spec)).await
}

#[tauri::command]
async fn replace_preview(
    docs: DocsState<'_>,
    doc: u32,
    spec: FindSpec,
    replacement: String,
) -> CmdResult<ReplaceReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.replace_preview(doc, spec, replacement)).await
}

#[tauri::command]
async fn replace_all(
    docs: DocsState<'_>,
    doc: u32,
    spec: FindSpec,
    replacement: String,
) -> CmdResult<ReplaceReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.replace_all(doc, spec, replacement)).await
}

#[tauri::command]
async fn find_in_text(text: String, pattern: Pattern) -> CmdResult<Vec<(u32, u32)>> {
    blocking(move || docs::find_in_text(&text, &pattern)).await
}

/// Fill, Paste Special, duplicates, blanks, split/merge, sort columns.
#[tauri::command]
async fn data_op(docs: DocsState<'_>, doc: u32, op: DataOp) -> CmdResult<OpReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.data_op(doc, op)).await
}

/// Pivot or unpivot into a new document.
#[tauri::command]
async fn new_table(docs: DocsState<'_>, doc: u32, op: TableOp) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.new_table(doc, op)).await
}

/// A pivot table's first rows and columns (live preview). `sample`: read
/// only that many rows first, for a quick look at a big file.
#[tauri::command]
async fn pivot_preview(
    docs: DocsState<'_>,
    doc: u32,
    spec: PivotSpec,
    sample: Option<u64>,
) -> CmdResult<PivotGrid> {
    let docs = docs.inner().clone();
    blocking(move || docs.pivot_preview(doc, &spec, sample)).await
}

/// The whole pivot table in a new tab.
#[tauri::command]
async fn pivot_table(docs: DocsState<'_>, doc: u32, spec: PivotSpec) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.pivot_table(doc, &spec)).await
}

/// The whole pivot table as tab-separated text.
#[tauri::command]
async fn pivot_text(docs: DocsState<'_>, doc: u32, spec: PivotSpec) -> CmdResult<String> {
    let docs = docs.inner().clone();
    blocking(move || docs.pivot_text(doc, &spec)).await
}

/// The source rows behind a pivot cell, in a new tab.
#[tauri::command]
async fn pivot_drill(
    docs: DocsState<'_>,
    doc: u32,
    spec: PivotSpec,
    row_path: Vec<String>,
    col_path: Vec<String>,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.pivot_drill(doc, &spec, &row_path, &col_path)).await
}

/// The items of one pivot field (for its pick list).
#[tauri::command]
async fn pivot_items(
    docs: DocsState<'_>,
    doc: u32,
    field: PivotField,
    spec: PivotSpec,
) -> CmdResult<ItemList> {
    let docs = docs.inner().clone();
    blocking(move || docs.pivot_items(doc, &field, &spec)).await
}

/// Forget the pivot data kept for a tab (the pivot window closed).
#[tauri::command]
fn pivot_close(docs: DocsState<'_>, doc: u32) -> CmdResult<()> {
    docs.pivot_forget(doc)
}

/// The extension of saved pivot layouts.
const PIVOT_EXT: &str = "ndpivot";

/// Save a pivot layout to a file chosen in the Save dialog. `None` if
/// cancelled.
#[tauri::command]
async fn pivot_save(
    window: WebviewWindow,
    docs: DocsState<'_>,
    doc: u32,
    name: String,
    spec: PivotSpec,
) -> CmdResult<Option<String>> {
    let docs = docs.inner().clone();
    blocking(move || {
        let titles = docs.titles_of(doc, spec.keep_first)?;
        let bytes = SavedPivot::new(&name, &spec, &titles).to_json()?;
        let base = if name.trim().is_empty() {
            "pivot".to_string()
        } else {
            safe_file_name(&name)
        };
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save pivot layout")
            .set_file_name(format!("{base}.{PIVOT_EXT}"))
            .add_filter("Natendo pivot layout", &[PIVOT_EXT])
            .set_parent(&window)
            .save_file()
        else {
            return Ok(None);
        };
        docs.check_writable(&path)?;
        extras::write_atomic(&path, &bytes)?;
        Ok(Some(
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ))
    })
    .await
}

/// Open a saved pivot layout (an untrusted file) and fit it to a tab's
/// columns. `None` if cancelled. Columns whose titles are not found (after
/// a rename, say) take the column in the same place and are listed, so the
/// person is asked before the layout is used.
#[tauri::command]
async fn pivot_open(
    window: WebviewWindow,
    docs: DocsState<'_>,
    doc: u32,
    keep_first: bool,
) -> CmdResult<Option<(String, PivotSpec, Vec<natendo_engine::pivot::PlaceSwap>)>> {
    let docs = docs.inner().clone();
    blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open pivot layout")
            .add_filter("Natendo pivot layout", &[PIVOT_EXT])
            .add_filter("All files", &["*"])
            .set_parent(&window)
            .pick_file()
        else {
            return Ok(None);
        };
        let bytes = extras::read_capped(&path, MAX_SAVED_PIVOT_BYTES as u64, "a pivot layout")?;
        let saved = SavedPivot::from_json(&bytes)?;
        let titles = docs.titles_of(doc, keep_first)?;
        let (spec, swaps) = saved.fit_by_place(&titles)?;
        Ok(Some((saved.name.clone(), spec, swaps)))
    })
    .await
}

/// The shown row for a row number of the whole file.
#[tauri::command]
async fn shown_row_of(
    docs: DocsState<'_>,
    doc: u32,
    number: u64,
) -> CmdResult<Option<(u64, bool)>> {
    let docs = docs.inner().clone();
    blocking(move || docs.shown_row_of(doc, number)).await
}

/// The block of data around a cell.
#[tauri::command]
async fn current_region(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    col: u32,
    first: u64,
) -> CmdResult<Option<(u64, u64, u32, u32)>> {
    let docs = docs.inner().clone();
    blocking(move || docs.current_region(doc, row, col, first)).await
}

/// Where Ctrl+arrow lands.
#[tauri::command]
async fn data_edge(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    col: u32,
    dir: Dir,
) -> CmdResult<(u64, u32)> {
    let docs = docs.inner().clone();
    blocking(move || docs.data_edge(doc, row, col, dir)).await
}

/// The filter panel's counts, every card in one pass.
#[tauri::command]
async fn facet_counts(
    docs: DocsState<'_>,
    doc: u32,
    cards: Vec<natendo_engine::filter::FacetCard>,
    spec: natendo_engine::filter::FilterSpec,
) -> CmdResult<natendo_engine::filter::Facets> {
    let docs = docs.inner().clone();
    blocking(move || docs.facet_counts(doc, &cards, &spec)).await
}

/// Free the filter panel's counts for a tab.
#[tauri::command]
async fn facet_forget(docs: DocsState<'_>, doc: u32) -> CmdResult<()> {
    let docs = docs.inner().clone();
    blocking(move || docs.facet_forget(doc)).await
}

/// The column insight strip under the headings (a sample).
#[tauri::command]
async fn column_insights(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::insight::InsightSpec,
) -> CmdResult<natendo_engine::insight::Insights> {
    let docs = docs.inner().clone();
    blocking(move || docs.column_insights(doc, &spec)).await
}

/// A column's profile: shapes, histogram, odd values.
#[tauri::command]
async fn column_profile(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::profile::ProfileSpec,
) -> CmdResult<natendo_engine::profile::Profile> {
    let docs = docs.inner().clone();
    blocking(move || docs.column_profile(doc, &spec)).await
}

/// Run a formula typed in a cell (or filled with Ctrl+Enter).
#[tauri::command]
async fn cell_formula(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::cellformula::CellFormula,
) -> CmdResult<docs::FormulaReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.cell_formula(doc, &spec)).await
}

/// The formula a cell's value came from, if it still holds that value.
#[tauri::command]
async fn cell_formula_of(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    col: u32,
) -> CmdResult<Option<String>> {
    let docs = docs.inner().clone();
    blocking(move || docs.cell_formula_of(doc, row, col)).await
}

/// The first results of a computed column.
#[tauri::command]
async fn compute_preview(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::expr::ComputeSpec,
) -> CmdResult<natendo_engine::expr::ComputePreview> {
    let docs = docs.inner().clone();
    blocking(move || docs.compute_preview(doc, &spec)).await
}

/// Each distinct value of a column with its count (the value editor).
#[tauri::command]
async fn value_tally(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::editing::TallySpec,
) -> CmdResult<natendo_engine::editing::ValueTally> {
    let docs = docs.inner().clone();
    blocking(move || docs.value_tally(doc, &spec)).await
}

/// Group the values of a column that look alike by sound or spelling.
#[tauri::command]
async fn lookalike_groups(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::editing::LookAlikeSpec,
) -> CmdResult<natendo_engine::editing::LookAlikes> {
    let docs = docs.inner().clone();
    blocking(move || docs.lookalike_groups(doc, &spec)).await
}

// ---- explode cells (C7) ----

/// What a column's cells look like (JSON, lists, key=value), from a sample.
#[tauri::command]
async fn explode_detect(
    docs: DocsState<'_>,
    doc: u32,
    col: u32,
    keep_first: bool,
) -> CmdResult<natendo_engine::explode::ExplodeDetect> {
    let docs = docs.inner().clone();
    blocking(move || docs.explode_detect(doc, col, keep_first)).await
}

/// Read the whole column: keys or tags with counts, cells that did not parse.
#[tauri::command]
async fn explode_scan(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::explode::Explode,
) -> CmdResult<natendo_engine::explode::ExplodeScan> {
    let docs = docs.inner().clone();
    blocking(move || docs.explode_scan(doc, &spec)).await
}

/// Before and after for the first 20 shown rows.
#[tauri::command]
async fn explode_preview(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::explode::Explode,
) -> CmdResult<natendo_engine::explode::ExplodePreview> {
    let docs = docs.inner().clone();
    blocking(move || docs.explode_preview(doc, &spec)).await
}

// ---- value dictionaries (C6) ----

/// The Dictionaries folder in the app's data folder (made when missing).
fn dictionary_folder(app: &tauri::AppHandle) -> CmdResult<PathBuf> {
    use tauri::Manager;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|_| docs::CmdError::new("io", "the app's data folder could not be found"))?
        .join("Dictionaries");
    std::fs::create_dir_all(&dir)
        .map_err(|_| docs::CmdError::new("io", "the Dictionaries folder could not be made"))?;
    Ok(dir)
}

/// A dictionary in the Dictionaries folder.
#[derive(Serialize)]
struct DictionaryItem {
    /// The name inside the file.
    name: String,
    /// The file's own name (shown when two share a name).
    file: String,
    path: String,
    pairs: u64,
    /// The file could not be read: why.
    problem: Option<String>,
}

#[tauri::command]
async fn dictionary_list(app: tauri::AppHandle) -> CmdResult<(String, Vec<DictionaryItem>)> {
    blocking(move || {
        let dir = dictionary_folder(&app)?;
        let mut out = Vec::new();
        let read = std::fs::read_dir(&dir)
            .map_err(|_| docs::CmdError::new("io", "the Dictionaries folder could not be read"))?;
        for e in read.flatten().take(1_000) {
            let p = e.path();
            if p.extension()
                .and_then(|x| x.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
                != Some("ndmap")
            {
                continue;
            }
            let file = p
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("")
                .to_string();
            let read = extras::read_capped(
                &p,
                natendo_engine::dictionary::MAX_DICTIONARY_BYTES as u64,
                "A dictionary",
            )
            .and_then(|b| Ok(natendo_engine::dictionary::Dictionary::parse(&b)?));
            out.push(match read {
                Ok(r) => DictionaryItem {
                    name: if r.dict.name.trim().is_empty() {
                        file.trim_end_matches(".ndmap").to_string()
                    } else {
                        r.dict.name.clone()
                    },
                    file,
                    path: p.to_string_lossy().into_owned(),
                    pairs: r.dict.pairs.len() as u64,
                    problem: None,
                },
                Err(err) => DictionaryItem {
                    name: file.trim_end_matches(".ndmap").to_string(),
                    file,
                    path: p.to_string_lossy().into_owned(),
                    pairs: 0,
                    problem: Some(err.message),
                },
            });
        }
        out.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.file.cmp(&b.file))
        });
        Ok((dir.to_string_lossy().into_owned(), out))
    })
    .await
}

/// Dictionary files the person picked in a file window this session (only
/// those, and files in the Dictionaries folder, are read or written).
static DICT_PICKED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// A path the dictionary commands may use: in the Dictionaries folder, or
/// picked by the person.
fn dictionary_path_ok(app: &tauri::AppHandle, path: &std::path::Path) -> CmdResult<()> {
    let dir = dictionary_folder(app)?;
    let picked = DICT_PICKED
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .any(|p| p == path);
    let inside = path.parent().is_some_and(|p| p == dir);
    if picked || inside {
        Ok(())
    } else {
        Err(docs::CmdError::new(
            "bad_request",
            "choose the dictionary file in the file window",
        ))
    }
}

// ---- file contracts (C9) ----

/// The contracts folder in the app's data folder (made when first needed).
fn contracts_folder(app: &tauri::AppHandle) -> CmdResult<PathBuf> {
    use tauri::Manager;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|_| docs::CmdError::new("io", "the app's data folder could not be found"))?
        .join("Contracts");
    std::fs::create_dir_all(&dir)
        .map_err(|_| docs::CmdError::new("io", "the Contracts folder could not be made"))?;
    Ok(dir)
}

#[tauri::command]
async fn contracts_list(
    app: tauri::AppHandle,
    docs: DocsState<'_>,
) -> CmdResult<Vec<docs::contracts::ContractItem>> {
    let docs = docs.inner().clone();
    blocking(move || docs.contracts_list(&contracts_folder(&app)?)).await
}

#[tauri::command]
async fn contract_get(
    app: tauri::AppHandle,
    docs: DocsState<'_>,
    name: String,
) -> CmdResult<natendo_engine::contract::Contract> {
    let docs = docs.inner().clone();
    blocking(move || docs.contract_get(&contracts_folder(&app)?, &name)).await
}

/// The contract a tab looks like (file name and titles only).
#[tauri::command]
async fn contract_match(
    app: tauri::AppHandle,
    docs: DocsState<'_>,
    doc: u32,
    keep_first: bool,
) -> CmdResult<Option<docs::contracts::ContractMatch>> {
    let docs = docs.inner().clone();
    blocking(move || docs.contract_match(&contracts_folder(&app)?, doc, keep_first)).await
}

/// Check a tab against a contract (a count any change stops).
#[tauri::command]
async fn contract_drift(
    app: tauri::AppHandle,
    docs: DocsState<'_>,
    doc: u32,
    name: String,
    opts: natendo_engine::contract::DriftOptions,
) -> CmdResult<natendo_engine::contract::DriftReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.contract_drift(&contracts_folder(&app)?, doc, &name, &opts)).await
}

#[tauri::command]
fn contract_drift_stop(docs: DocsState<'_>, doc: u32) -> CmdResult<()> {
    docs.contract_drift_stop(doc)
}

/// Show the rows behind one count of a drift report.
#[tauri::command]
async fn contract_show_rows(
    app: tauri::AppHandle,
    docs: DocsState<'_>,
    doc: u32,
    name: String,
    what: natendo_engine::contract::DriftRows,
    opts: natendo_engine::contract::DriftOptions,
) -> CmdResult<docs::OpReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.contract_show_rows(&contracts_folder(&app)?, doc, &name, what, opts))
        .await
}

/// Fix what I can: titles and column order as one undo step.
#[tauri::command]
async fn contract_fix(
    app: tauri::AppHandle,
    docs: DocsState<'_>,
    doc: u32,
    name: String,
    keep_first: bool,
) -> CmdResult<docs::contracts::ContractFixed> {
    let docs = docs.inner().clone();
    blocking(move || docs.contract_fix(&contracts_folder(&app)?, doc, &name, keep_first)).await
}

/// A contract's draft from a tab (refused for a private tab).
#[tauri::command]
async fn contract_draft(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::contract::DraftSpec,
) -> CmdResult<natendo_engine::contract::Draft> {
    let docs = docs.inner().clone();
    blocking(move || docs.contract_draft(doc, &spec)).await
}

/// Save a contract into the Contracts folder.
#[tauri::command]
async fn contract_save(
    app: tauri::AppHandle,
    docs: DocsState<'_>,
    contract: natendo_engine::contract::Contract,
    was: Option<String>,
    from_tab: Option<u32>,
) -> CmdResult<Vec<docs::contracts::ContractItem>> {
    let docs = docs.inner().clone();
    blocking(move || {
        docs.contract_save(
            &contracts_folder(&app)?,
            &contract,
            was.as_deref(),
            from_tab,
        )
    })
    .await
}

#[tauri::command]
async fn contract_delete(
    app: tauri::AppHandle,
    docs: DocsState<'_>,
    name: String,
) -> CmdResult<Vec<docs::contracts::ContractItem>> {
    let docs = docs.inner().clone();
    blocking(move || docs.contract_delete(&contracts_folder(&app)?, &name)).await
}

/// Export a contract to a file the person picks. `false` when the dialog was closed.
#[tauri::command]
async fn contract_export(
    app: tauri::AppHandle,
    window: WebviewWindow,
    docs: DocsState<'_>,
    name: String,
) -> CmdResult<bool> {
    let docs = docs.inner().clone();
    blocking(move || {
        let folder = contracts_folder(&app)?;
        let Some(mut path) = rfd::FileDialog::new()
            .set_title("Export a contract")
            .set_file_name(natendo_engine::contract::file_name(&safe_file_name(&name)))
            .add_filter("Natendo contract", &[natendo_engine::contract::EXTENSION])
            .set_parent(&window)
            .save_file()
        else {
            return Ok(false);
        };
        if path
            .extension()
            .and_then(|x| x.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
            != Some(natendo_engine::contract::EXTENSION)
        {
            path.set_extension(natendo_engine::contract::EXTENSION);
        }
        docs.contract_export(&folder, &name, &path)?;
        Ok(true)
    })
    .await
}

/// Import a contract from a file the person picks. `None` when the dialog was closed.
#[tauri::command]
async fn contract_import(
    app: tauri::AppHandle,
    window: WebviewWindow,
    docs: DocsState<'_>,
) -> CmdResult<Option<Vec<docs::contracts::ContractItem>>> {
    let docs = docs.inner().clone();
    blocking(move || {
        let folder = contracts_folder(&app)?;
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import a contract")
            .add_filter("Natendo contract", &[natendo_engine::contract::EXTENSION])
            .set_parent(&window)
            .pick_file()
        else {
            return Ok(None);
        };
        docs.contract_import(&folder, &path).map(Some)
    })
    .await
}

/// Pick a dictionary file to load.
#[tauri::command]
async fn dictionary_pick(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> CmdResult<Option<String>> {
    blocking(move || {
        let mut d = rfd::FileDialog::new()
            .set_title("Load a dictionary")
            .add_filter("Natendo dictionary", &["ndmap"])
            .set_parent(&window);
        if let Ok(dir) = dictionary_folder(&app) {
            d = d.set_directory(dir);
        }
        let Some(path) = d.pick_file() else {
            return Ok(None);
        };
        DICT_PICKED
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(path.clone());
        Ok(Some(path.display().to_string()))
    })
    .await
}

/// Pick where to save a dictionary (the Dictionaries folder to begin with).
#[tauri::command]
async fn dictionary_pick_save(
    app: tauri::AppHandle,
    window: WebviewWindow,
    name: String,
) -> CmdResult<Option<String>> {
    blocking(move || {
        let stem = if name.trim().is_empty() {
            "dictionary".to_string()
        } else {
            safe_file_name(&name)
        };
        let mut d = rfd::FileDialog::new()
            .set_title("Save as dictionary")
            .set_file_name(format!("{stem}.ndmap"))
            .add_filter("Natendo dictionary", &["ndmap"])
            .set_parent(&window);
        if let Ok(dir) = dictionary_folder(&app) {
            d = d.set_directory(dir);
        }
        let Some(mut path) = d.save_file() else {
            return Ok(None);
        };
        if path
            .extension()
            .and_then(|x| x.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
            != Some("ndmap")
        {
            path.set_extension("ndmap");
        }
        DICT_PICKED
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(path.clone());
        Ok(Some(path.display().to_string()))
    })
    .await
}

#[tauri::command]
async fn dictionary_read(
    app: tauri::AppHandle,
    path: String,
) -> CmdResult<natendo_engine::dictionary::ReadDictionary> {
    blocking(move || {
        dictionary_path_ok(&app, std::path::Path::new(&path))?;
        let bytes = extras::read_capped(
            std::path::Path::new(&path),
            natendo_engine::dictionary::MAX_DICTIONARY_BYTES as u64,
            "A dictionary",
        )?;
        Ok(natendo_engine::dictionary::Dictionary::parse(&bytes)?)
    })
    .await
}

/// Save a dictionary made from tab `doc`. A private or encrypted tab's values
/// are only written once the person said so (`confirmed`); never automatic.
#[tauri::command]
async fn dictionary_write(
    app: tauri::AppHandle,
    docs: DocsState<'_>,
    doc: u32,
    path: String,
    dict: natendo_engine::dictionary::Dictionary,
    confirmed: bool,
) -> CmdResult<String> {
    let docs = docs.inner().clone();
    blocking(move || {
        if !confirmed && docs.dictionary_sensitive(doc)? {
            return Err(docs::CmdError::new(
                "private",
                "This dictionary holds values from a private file",
            ));
        }
        let p = PathBuf::from(&path);
        if !p.is_absolute()
            || p.extension()
                .and_then(|x| x.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
                != Some("ndmap")
        {
            return Err(docs::CmdError::new(
                "bad_request",
                "choose where to save the dictionary",
            ));
        }
        dictionary_path_ok(&app, &p)?;
        docs.check_writable(&p)?;
        let text = dict.to_json()?;
        extras::write_atomic(&p, text.as_bytes())?;
        Ok(p.to_string_lossy().into_owned())
    })
    .await
}

#[tauri::command]
async fn dictionary_unmatched(
    docs: DocsState<'_>,
    doc: u32,
    col: u32,
    keep_first: bool,
    dict: natendo_engine::dictionary::Dictionary,
) -> CmdResult<docs::UnmatchedOut> {
    let docs = docs.inner().clone();
    blocking(move || docs.dictionary_unmatched(doc, col, keep_first, &dict)).await
}

/// How the first rows would look after a join, with counts.
#[tauri::command]
async fn join_preview(
    docs: DocsState<'_>,
    doc: u32,
    other: Option<u32>,
    list: Option<String>,
    list_id: Option<u64>,
    spec: natendo_engine::join::JoinSpec,
) -> CmdResult<natendo_engine::join::JoinPreview> {
    let docs = docs.inner().clone();
    blocking(move || docs.join_preview(doc, other, list.as_deref(), list_id, &spec)).await
}

/// Keep a pasted list of keys for the Join screen; returns its number.
#[tauri::command]
async fn join_list(docs: DocsState<'_>, doc: u32, text: String) -> CmdResult<u64> {
    let docs = docs.inner().clone();
    blocking(move || docs.join_list(doc, text)).await
}

/// A tab's Join screen closed.
#[tauri::command]
fn join_close(docs: DocsState<'_>, doc: u32) {
    docs.join_close(doc);
}

/// The computed-column screen closed.
#[tauri::command]
fn lookup_close(docs: DocsState<'_>) {
    docs.lookup_close();
}

/// Run data checks over the shown rows.
#[tauri::command]
async fn run_checks(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::validate::CheckSpec,
) -> CmdResult<natendo_engine::validate::CheckReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.run_checks(doc, &spec)).await
}

/// Compare 2 to 5 lists by key (a job on tab `owner`).
#[tauri::command]
async fn sets_run(
    docs: DocsState<'_>,
    owner: u32,
    tabs: Vec<u32>,
    spec: natendo_engine::sets::SetsSpec,
) -> CmdResult<docs::SetsRun> {
    let docs = docs.inner().clone();
    blocking(move || docs.sets_run(owner, &tabs, &spec)).await
}

/// A slice of a comparison in a new tab.
#[tauri::command]
async fn sets_open(
    docs: DocsState<'_>,
    id: u64,
    mask: u8,
    list: usize,
    keys: bool,
) -> CmdResult<docs::DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.sets_open(id, mask, list, keys)).await
}

/// A comparison's counts in a new tab.
#[tauri::command]
async fn sets_table(docs: DocsState<'_>, id: u64) -> CmdResult<docs::DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.sets_table(id)).await
}

/// Forget a comparison.
#[tauri::command]
async fn sets_close(docs: DocsState<'_>, id: u64) -> CmdResult<()> {
    docs.sets_close(id);
    Ok(())
}

/// What a clean matrix would change (counts and a few examples).
#[tauri::command]
async fn clean_matrix_preview(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::clean::CleanMatrix,
    now: f64,
) -> CmdResult<natendo_engine::clean::MatrixPreview> {
    let docs = docs.inner().clone();
    blocking(move || docs.clean_matrix_preview(doc, &spec, now)).await
}

/// Look for relationships among columns.
#[tauri::command]
async fn find_relations(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::relations::RelationsSpec,
) -> CmdResult<natendo_engine::relations::RelationsReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.find_relations(doc, &spec)).await
}

/// Find the numbers missing from a column of numbers or IDs.
#[tauri::command]
async fn gap_audit(
    docs: DocsState<'_>,
    doc: u32,
    spec: natendo_engine::gaps::GapsSpec,
) -> CmdResult<natendo_engine::gaps::GapReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.gap_audit(doc, &spec).map(|r| (*r).clone())).await
}

/// X-ray the shown rows for look-alike characters (counts only).
#[tauri::command]
async fn xray_report(
    docs: DocsState<'_>,
    doc: u32,
    cols: Vec<u32>,
    keep_first: bool,
) -> CmdResult<natendo_engine::xray::XrayReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.xray_report(doc, &cols, keep_first)).await
}

/// Show only the rows an X-ray finds gremlins in.
#[tauri::command]
async fn xray_show(
    docs: DocsState<'_>,
    doc: u32,
    cols: Vec<u32>,
    keep_first: bool,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.xray_show(doc, &cols, keep_first)).await
}

/// Problem marks for an area of the screen.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn check_marks(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    rows: u64,
    col: u32,
    cols: u32,
    spec: natendo_engine::validate::CheckSpec,
) -> CmdResult<Vec<natendo_engine::validate::CheckMark>> {
    let docs = docs.inner().clone();
    blocking(move || docs.check_marks(doc, row, rows, col, cols, &spec)).await
}

/// The next (previous) problem cell after a cell.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn next_problem(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    col: u32,
    backward: bool,
    skip_cols: Vec<u32>,
    spec: natendo_engine::validate::CheckSpec,
) -> CmdResult<natendo_engine::validate::JumpResult> {
    let docs = docs.inner().clone();
    blocking(move || docs.next_problem(doc, row, col, backward, &spec, &skip_cols)).await
}

/// Rules a column's values suggest.
#[tauri::command]
async fn suggest_rules(
    docs: DocsState<'_>,
    doc: u32,
    col: u32,
    keep_first: bool,
) -> CmdResult<Vec<natendo_engine::lintsuggest::Suggestion>> {
    let docs = docs.inner().clone();
    blocking(move || docs.suggest_rules(doc, col, keep_first)).await
}

/// The rules file beside the table, if any.
#[tauri::command]
async fn checks_sidecar_load(
    docs: DocsState<'_>,
    doc: u32,
) -> CmdResult<Option<docs::SidecarRules>> {
    let docs = docs.inner().clone();
    blocking(move || docs.checks_sidecar_load(doc)).await
}

/// Keep the rules in a file beside the table (empty list removes the file).
#[tauri::command]
async fn checks_sidecar_keep(
    docs: DocsState<'_>,
    doc: u32,
    checks: Vec<natendo_engine::validate::Check>,
    table_checks: Option<Vec<natendo_engine::tablecheck::TableCheck>>,
    keep_first: bool,
) -> CmdResult<String> {
    let docs = docs.inner().clone();
    let table_checks = table_checks.unwrap_or_default();
    blocking(move || docs.checks_sidecar_keep(doc, checks, table_checks, keep_first)).await
}

/// Compare two tabs by key and open the differences.
#[tauri::command]
async fn diff_files(
    docs: DocsState<'_>,
    a: u32,
    b: u32,
    spec: natendo_engine::diff::DiffSpec,
) -> CmdResult<docs::DiffReportOut> {
    let docs = docs.inner().clone();
    blocking(move || docs.diff_files(a, b, spec)).await
}

/// The differences in a window of a tab that is in a diff.
#[tauri::command]
async fn diff_marks(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    rows: u64,
    col: u32,
    cols: u32,
) -> CmdResult<docs::DiffMarksOut> {
    let docs = docs.inner().clone();
    blocking(move || docs.diff_marks(doc, row, rows, col, cols)).await
}

/// The next (previous) difference.
#[tauri::command]
async fn diff_next(
    docs: DocsState<'_>,
    doc: u32,
    row: u64,
    backward: bool,
) -> CmdResult<Option<docs::DiffJump>> {
    let docs = docs.inner().clone();
    blocking(move || docs.diff_next(doc, row, backward)).await
}

/// Take the differences off a tab (and the tabs compared with it).
#[tauri::command]
async fn diff_clear(docs: DocsState<'_>, doc: u32) -> CmdResult<()> {
    let docs = docs.inner().clone();
    blocking(move || {
        docs.diff_clear(doc);
        Ok(())
    })
    .await
}

/// Largest checks file read.
const MAX_CHECKS_FILE: u64 = 4 << 20;

/// Save data checks to a JSON file chosen in the Save dialog (the command
/// line reads the same file). Returns the file's name, `None` if cancelled.
#[tauri::command]
async fn checks_save(
    window: WebviewWindow,
    docs: DocsState<'_>,
    checks: Vec<natendo_engine::validate::Check>,
) -> CmdResult<Option<String>> {
    let docs = docs.inner().clone();
    blocking(move || {
        let spec = natendo_engine::validate::CheckSpec {
            checks,
            ..Default::default()
        };
        natendo_engine::validate::check_spec(&spec)?;
        let bytes = serde_json::to_vec_pretty(&serde_json::json!({ "checks": spec.checks }))
            .map_err(|e| CmdError::new("error", e.to_string()))?;
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save data checks")
            .set_file_name("checks.json")
            .add_filter("Data checks", &["json"])
            .set_parent(&window)
            .save_file()
        else {
            return Ok(None);
        };
        docs.check_writable(&path)?;
        extras::write_atomic(&path, &bytes)?;
        Ok(Some(
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ))
    })
    .await
}

/// Open data checks from a JSON file (untrusted: size-capped and checked).
#[tauri::command]
async fn checks_open(
    window: WebviewWindow,
) -> CmdResult<Option<Vec<natendo_engine::validate::Check>>> {
    blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open data checks")
            .add_filter("Data checks", &["json"])
            .add_filter("All files", &["*"])
            .set_parent(&window)
            .pick_file()
        else {
            return Ok(None);
        };
        let bytes = extras::read_capped(&path, MAX_CHECKS_FILE, "a checks file")?;
        #[derive(serde::Deserialize)]
        struct File {
            checks: Vec<natendo_engine::validate::Check>,
        }
        let f: File = serde_json::from_slice(&bytes)
            .map_err(|e| CmdError::new("invalid", format!("not a data checks file: {e}")))?;
        let spec = natendo_engine::validate::CheckSpec {
            checks: f.checks,
            ..Default::default()
        };
        natendo_engine::validate::check_spec(&spec)?;
        Ok(Some(spec.checks))
    })
    .await
}

/// The file types this app handles (Windows).
#[tauri::command]
async fn file_types() -> CmdResult<Vec<assoc::TypeState>> {
    blocking(|| Ok(assoc::status())).await
}

/// Handle exactly these file types (from the offered list).
#[tauri::command]
async fn set_file_types(on: Vec<String>) -> CmdResult<Vec<assoc::TypeState>> {
    blocking(move || {
        let on: Vec<String> = on
            .into_iter()
            .map(|e| e.to_ascii_lowercase())
            .filter(|e| assoc::known(e))
            .collect();
        assoc::set(&on).map_err(|e| CmdError::new("error", e.to_string()))?;
        applog::info(&format!("file types set: {}", on.join(", ")));
        Ok(assoc::status())
    })
    .await
}

/// Open the Windows Settings page for choosing this app as a default.
#[tauri::command]
fn open_default_apps() -> CmdResult<()> {
    assoc::open_settings().map_err(|e| CmdError::new("error", e.to_string()))
}

#[tauri::command]
fn agent_status(agent: State<'_, Arc<Agent>>) -> AgentStatus {
    agent.status()
}

/// Switch the agent link on or off.
#[tauri::command]
async fn set_agent_link(
    app: tauri::AppHandle,
    agent: State<'_, Arc<Agent>>,
    on: bool,
) -> CmdResult<AgentStatus> {
    let agent = agent.inner().clone();
    blocking(move || agent.set(&app, on)).await
}

/// Write a line from the window to the app's log (cleaned and cut there).
#[tauri::command]
fn log_event(level: String, text: String) {
    let level = match level.as_str() {
        "warn" | "error" => level.as_str(),
        _ => "info",
    };
    applog::write(level, &text);
}

/// Show the app's log folder in File Explorer.
#[tauri::command]
fn open_log_folder() -> CmdResult<()> {
    let dir = applog::folder().ok_or_else(|| CmdError::new("error", "there is no log folder"))?;
    #[cfg(windows)]
    {
        std::process::Command::new("explorer.exe")
            .arg(&dir)
            .spawn()
            .map(|_| ())
            .map_err(|e| CmdError::new("error", e.to_string()))
    }
    #[cfg(not(windows))]
    {
        let _ = dir;
        Err(CmdError::new(
            "error",
            "open the log folder from the file manager",
        ))
    }
}

/// Keep a recovery copy of a document's unsaved changes.
#[tauri::command]
async fn autosave(docs: DocsState<'_>, doc: u32) -> CmdResult<recovery::Autosaved> {
    let docs = docs.inner().clone();
    blocking(move || docs.autosave(doc)).await
}

/// Recovery copies left by an earlier run that stopped without saving.
#[tauri::command]
async fn recovery_list() -> CmdResult<Vec<recovery::RecoveryItem>> {
    blocking(recovery::list).await
}

#[tauri::command]
async fn recovery_open(docs: DocsState<'_>, index: u32) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.recovery_open(index)).await
}

#[tauri::command]
async fn recovery_discard(index: u32) -> CmdResult<()> {
    blocking(move || recovery::discard(index)).await
}

/// Keep earlier saves or not (the Options switch).
#[tauri::command]
fn versions_keep(on: bool) {
    recovery::set_keep_versions(on);
}

/// Delete every earlier save of every file; returns how many.
#[tauri::command]
async fn versions_clear() -> CmdResult<u64> {
    blocking(recovery::clear_versions).await
}

/// Earlier saves of a document's file.
#[tauri::command]
async fn versions_list(docs: DocsState<'_>, doc: u32) -> CmdResult<Vec<recovery::VersionItem>> {
    let docs = docs.inner().clone();
    blocking(move || docs.versions(doc)).await
}

#[tauri::command]
async fn version_open(
    docs: DocsState<'_>,
    doc: u32,
    index: u32,
    label: String,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.version_open(doc, index, &label)).await
}

/// Open a web or e-mail address from a cell (checked again in `links`).
#[tauri::command]
fn open_link(url: String) -> CmdResult<()> {
    links::open_link(&url)
}

/// What stands between the app and a quit: unsaved documents (either
/// window), documents something holds right now, long jobs.
#[tauri::command]
fn modified_docs(docs: DocsState<'_>) -> docs::ModifiedReport {
    docs.modified_report()
}

/// A document as the quit question listed it.
#[derive(serde::Deserialize)]
struct KnownDoc {
    id: u32,
    generation: u64,
}

/// Quit the app. `known` is the list the question showed (document and
/// generation): if anything is modified now that was not on it, or has been
/// edited since, or something is held (a save), the app does not quit and the
/// question is asked again. Only the main window can quit.
#[tauri::command]
fn quit_app(
    app: tauri::AppHandle,
    window: WebviewWindow,
    docs: DocsState<'_>,
    known: Vec<KnownDoc>,
) -> &'static str {
    if window.label() != windows::MAIN {
        return "ignored";
    }
    let known: Vec<(u32, u64)> = known.iter().map(|k| (k.id, k.generation)).collect();
    if docs.modified_report().changed_since(&known) {
        let _ = app.emit_to(windows::MAIN, "close-requested", ());
        return "ask_again";
    }
    app.exit(0);
    "exiting"
}

/// The main window's close button: the app ends. Nothing unsaved and nothing
/// running: at once; otherwise the main window asks first. It never waits on
/// window 2's page.
fn request_quit(app: &tauri::AppHandle) {
    if app.state::<Arc<Docs>>().modified_report().is_clear() {
        app.exit(0);
    } else {
        let _ = app.emit_to(windows::MAIN, "close-requested", ());
    }
}

/// Move a tab to window 2 (opened on demand).
#[tauri::command]
async fn window_move(
    app: tauri::AppHandle,
    window: WebviewWindow,
    docs: DocsState<'_>,
    wins: State<'_, windows::Windows>,
    h: windows::HandOver,
) -> CmdResult<()> {
    if window.label() != windows::MAIN {
        return Err(CmdError::new(
            "window",
            "tabs are moved to the second window from the main window",
        ));
    }
    if docs.job(h.doc)?.is_some() {
        return Err(CmdError::new(
            "busy",
            "this file is busy: wait until it has finished, then move it",
        ));
    }
    let doc = h.doc;
    wins.move_to_second(h, std::time::Instant::now());
    if let Err(e) = windows::ensure_second(&app) {
        wins.forget(doc);
        return Err(CmdError::new("window", e));
    }
    let _ = app.emit_to(windows::SECOND, "handover", ());
    Ok(())
}

/// A window takes the tabs waiting for it.
#[tauri::command]
fn window_take(window: WebviewWindow, wins: State<'_, windows::Windows>) -> Vec<windows::HandOver> {
    wins.take(window.label(), std::time::Instant::now())
}

/// Window 2 hands a tab back to the main window.
#[tauri::command]
fn window_move_back(
    app: tauri::AppHandle,
    window: WebviewWindow,
    wins: State<'_, windows::Windows>,
    h: windows::HandOver,
) -> CmdResult<()> {
    if window.label() != windows::SECOND {
        return Err(CmdError::new(
            "window",
            "only the second window hands tabs back",
        ));
    }
    wins.move_back(h);
    windows::tell_main(&app);
    windows::maybe_close_second(&app);
    Ok(())
}

/// Window 2 made a document itself (a result tab): it holds it.
#[tauri::command]
fn window_claim(window: WebviewWindow, wins: State<'_, windows::Windows>, h: windows::HandOver) {
    if window.label() == windows::SECOND {
        wins.claim(h, std::time::Instant::now());
    }
}

/// Window 2 reports the state of its tabs (and that it is alive).
#[tauri::command]
fn window_sync(
    window: WebviewWindow,
    wins: State<'_, windows::Windows>,
    items: Vec<windows::HandOver>,
) {
    if window.label() == windows::SECOND {
        wins.sync(items, std::time::Instant::now());
    }
}

/// Window 2 asks to be closed: its tabs go back to the main window.
#[tauri::command]
fn window_close_self(app: tauri::AppHandle, window: WebviewWindow) {
    if window.label() == windows::SECOND {
        windows::close_second(&app, None);
    }
}

/// Setup and uninstall run the app with `--register-file-types` or
/// `--unregister-file-types`: do that and exit without a window.
pub fn file_types_from_args() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    assoc::from_args(&args)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        // First: a second launch hands its files to this window and exits.
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            let args: Vec<String> = argv.into_iter().skip(1).collect();
            let paths = arg_paths(&args, Some(std::path::Path::new(&cwd)));
            if paths.is_empty() {
                if let Some(label) = windows::event_target(&windows::open_labels(app)) {
                    if let Some(w) = app.get_webview_window(&label) {
                        let _ = w.unminimize();
                        let _ = w.set_focus();
                    }
                }
            } else {
                open_paths(app, paths, true);
            }
        }))
        .manage(Arc::new(Docs::default()))
        .manage(Arc::new(Agent::default()))
        .manage(Pending::default())
        .manage(LaunchPaths::default())
        .manage(Arc::new(Imports::default()))
        .manage(Arc::new(Recent::default()))
        .manage(Arc::new(session::Session::default()))
        .manage(Arc::new(Folders::default()))
        .setup(|app| {
            if let Ok(dir) = app.path().app_log_dir() {
                applog::init(dir);
            }
            windows::watchdog(app.handle().clone());
            if let Ok(dir) = app.path().app_local_data_dir() {
                recovery::init(dir);
            }
            applog::info(&format!(
                "Natendo Data Edit {} started",
                env!("CARGO_PKG_VERSION")
            ));
            if let Ok(dir) = app.path().app_data_dir() {
                app.state::<Arc<Recent>>().load(&dir);
                app.state::<Arc<session::Session>>().load(&dir);
            }
            {
                let recent = app.state::<Arc<Recent>>().inner().clone();
                app.state::<Arc<Docs>>()
                    .set_private_check(Arc::new(move |p| recent.is_private(p)));
            }
            {
                // Temporary files left by a save that was cut off (the app
                // closed or the computer stopped mid-save), in the folders
                // the app saves to. An hour old at least, so a save still
                // running in another window is left alone.
                let mut dirs = app.state::<Arc<Recent>>().temp_sweep_dirs();
                dirs.extend(app.path().app_data_dir().ok());
                std::thread::spawn(move || {
                    let n: usize = dirs
                        .iter()
                        .map(|d| {
                            natendo_engine::save::sweep_temps(
                                d,
                                std::time::Duration::from_secs(3600),
                            )
                        })
                        .sum();
                    if n > 0 {
                        applog::info(&format!(
                            "removed {n} temporary file(s) left by an earlier save"
                        ));
                    }
                });
            }
            Agent::clear_stale(app.handle());
            let args: Vec<String> = std::env::args().skip(1).collect();
            let cwd = std::env::current_dir().ok();
            let paths = arg_paths(&args, cwd.as_deref());
            if !paths.is_empty() {
                *app.state::<LaunchPaths>()
                    .0
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()) =
                    paths.iter().take(MAX_OS_OPENS).cloned().collect();
                open_paths(app.handle(), paths, false);
            }
            Ok(())
        })
        .manage(windows::Windows::default())
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) => {
                open_paths(window.app_handle(), paths.clone(), false);
            }
            // Closing the main window ends the app (asking first when
            // anything is unsaved); closing window 2 hands its tabs back.
            tauri::WindowEvent::CloseRequested { api, .. } => match window.label() {
                windows::MAIN => {
                    api.prevent_close();
                    request_quit(window.app_handle());
                }
                windows::SECOND => {
                    api.prevent_close();
                    windows::begin_close_second(window.app_handle());
                }
                _ => {}
            },
            // Window 2 gone, however it went: its files are main's again.
            tauri::WindowEvent::Destroyed if window.label() == windows::SECOND => {
                windows::close_second(window.app_handle(), None);
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            open_file,
            reopen,
            layout_choices,
            preview_layout,
            new_document,
            close_document,
            doc_info,
            viewport,
            cell_text,
            ragged_list,
            date_scan,
            date_marks,
            date_find,
            date_list,
            gap_hidden,
            set_aside_check,
            set_aside,
            report_notes,
            total_row,
            dictionary_list,
            contracts_list,
            contract_get,
            contract_match,
            contract_drift,
            contract_drift_stop,
            contract_show_rows,
            contract_fix,
            contract_draft,
            contract_save,
            contract_delete,
            contract_export,
            contract_import,
            dictionary_pick,
            dictionary_pick_save,
            dictionary_read,
            dictionary_write,
            dictionary_unmatched,
            fill_blanks_count,
            fill_blanks_count_stop,
            fill_marks,
            fill_marks_next,
            fill_marks_clear,
            put_back_notes,
            keep_notes,
            row_text,
            repair_row,
            same_shape,
            fix_all,
            empty_extra_columns,
            remove_empty_extra_columns,
            row_width,
            set_cells,
            set_column_type,
            set_column_types,
            fill_cells,
            hidden_between,
            move_cells,
            copy_cells,
            change,
            save,
            json_check,
            save_json,
            receipt_note,
            receipt_verify,
            job_progress,
            cancel_job,
            sort_rows,
            set_filter,
            clear_filter,
            filter_spec,
            filter_chips,
            filter_chip_counts,
            filter_chip_counts_stop,
            filter_change,
            filter_values,
            find_next,
            find_all,
            find_count,
            find_marks,
            replace_ghosts,
            replace_count,
            replace_count_cancel,
            diff_files,
            diff_marks,
            diff_next,
            diff_clear,
            set_doc_title,
            find_count_cancel,
            replace_confirm,
            batch_preview,
            find_rows,
            show_changed,
            column_pairs,
            find_notes,
            replace_titles,
            batch_confirm,
            replace_cell,
            replace_page,
            replace_preview,
            replace_all,
            find_in_text,
            data_op,
            new_table,
            pivot_preview,
            pivot_table,
            pivot_text,
            pivot_drill,
            pivot_items,
            pivot_close,
            pivot_save,
            pivot_open,
            table_report,
            find_change,
            find_extreme,
            special_cells,
            compare_cols,
            number_range,
            typeahead,
            rename_file,
            split_to_files,
            combine_files,
            show_in_folder,
            data_edge,
            current_region,
            shown_row_of,
            import_file,
            import_text,
            import_inspect,
            import_preview,
            import_notes,
            import_finish,
            import_progress,
            import_cancel,
            import_close,
            export_preview,
            recent_files,
            zip_open,
            set_protected,
            tidy_titles_preview,
            tidy_titles,
            session_save,
            session_list,
            session_open,
            session_clear,
            open_recent,
            first_row_cell,
            first_row_names,
            set_first_row_cell,
            column_names_line,
            set_column_names,
            tab_folders,
            folder_list,
            folder_open,
            view_save,
            view_open,
            set_read_only,
            column_types,
            column_summary,
            selection_totals,
            selection_totals_areas,
            column_totals,
            fill_areas,
            open_link,
            clean_suggestions,
            autosave,
            recovery_list,
            recovery_open,
            recovery_discard,
            versions_keep,
            versions_clear,
            versions_list,
            version_open,
            chart_data,
            highlights,
            clean_preview,
            clean_preview_page,
            cleaner_save,
            cleaner_open,
            cleaner_open_named,
            export_book,
            export_file,
            export_text,
            take_opened,
            compute_preview,
            cell_formula,
            cell_formula_of,
            column_profile,
            column_insights,
            facet_counts,
            facet_forget,
            join_preview,
            value_tally,
            lookalike_groups,
            explode_detect,
            explode_scan,
            explode_preview,
            join_list,
            join_close,
            lookup_close,
            set_private,
            private_leftovers,
            forget_scramble_key,
            private_forget,
            private_state,
            find_private,
            save_safe_copy,
            combine_pick,
            combine_check,
            combine_refresh,
            combine_run,
            combine_cancel,
            combine_save_mapping,
            combine_load_mapping,
            label_sheets,
            disk_check,
            disk_compare,
            change_list,
            pick_search_folder,
            search_folder,
            folder_search_progress,
            folder_search_cancel,
            open_folder_hit,
            edited_cells,
            save_change_report,
            street_lookup,
            encrypted_origin,
            sqlite_pick,
            sqlite_open,
            sqlite_save,
            watch_pick,
            watch_status,
            watch_start,
            watch_stop,
            steps_view,
            steps_goto,
            checkpoint_list,
            checkpoint_add,
            checkpoint_remove,
            checkpoint_restore,
            checkpoint_since,
            checkpoint_since_report,
            steps_change,
            steps_ask,
            batch_pick,
            batch_status,
            batch_start,
            batch_stop,
            notes_info,
            note_marks,
            note_set,
            notes_set_keys,
            note_list,
            notes_show,
            save_encrypted,
            pick_encrypted,
            open_encrypted,
            updates_find,
            golden_plan,
            golden_page,
            golden_create,
            golden_close,
            patch_pick_save,
            patch_pick,
            patch_protected,
            patch_make,
            patch_preview,
            patch_apply,
            patch_report_save,
            patch_close,
            updates_accept,
            updates_close,
            match_find,
            match_accept,
            match_close,
            recipe_get,
            recipe_save,
            recipe_open,
            recipe_run,
            mail_preview,
            save_mail,
            run_checks,
            gap_audit,
            find_relations,
            sets_run,
            sets_open,
            sets_table,
            sets_close,
            clean_matrix_preview,
            reopen_encoding,
            encoding_choices,
            encoding_preview,
            encoding_apply,
            scan_encodings,
            encoding_show_rows,
            xray_report,
            xray_show,
            check_marks,
            next_problem,
            suggest_rules,
            checks_sidecar_load,
            checks_sidecar_keep,
            checks_save,
            checks_open,
            file_types,
            set_file_types,
            open_default_apps,
            agent_status,
            set_agent_link,
            log_event,
            open_log_folder,
            quit_app,
            modified_docs,
            window_move,
            window_take,
            window_move_back,
            window_sync,
            window_claim,
            window_close_self
        ])
        .build(tauri::generate_context!())
        .expect("error while starting Natendo Data Edit");
    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            app.state::<Arc<Agent>>().shutdown();
            // Closed normally: this run's recovery copies are not needed.
            recovery::forget_session();
            applog::info("closed");
        }
    });
}

#[cfg(test)]
mod private_copy_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn files_written_from_a_private_tab_are_private() {
        let dir = crate::testdir::path("natendo-pcopy");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("voters.csv");
        std::fs::write(&src, b"name,ssn\r\nAnn,1\r\n").unwrap();
        let recent = Arc::new(Recent::default());
        recent.load(&dir);
        recent.set_private(&src, true).unwrap();
        let docs = Docs::default();
        let r2 = recent.clone();
        docs.set_private_check(Arc::new(move |p: &Path| r2.is_private(p)));
        let id = docs.open_path(&src, false).unwrap().id;
        while docs.info(id).map(|i| !i.indexed).unwrap_or(false) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        docs.set_cells(id, vec![(1, 0, "Bo".into())]).unwrap();
        // A change report and a recipe written from it go on the list.
        let report = dir.join("changes.csv");
        docs.save_change_report(id, &report, true).unwrap();
        assert!(private_copy(&docs, &recent, id, &report).unwrap());
        assert!(recent.is_private(&report));
        let recipe = dir.join("r.recipe.json");
        docs.delete_cols(id, 1, 1).unwrap();
        let r = docs.recipe_of(id, true, "").unwrap();
        docs.save_recipe(id, &r, &recipe).unwrap();
        assert!(private_copy(&docs, &recent, id, &recipe).unwrap());
        assert!(recent.is_private(&recipe));
        // From a tab that is not private, nothing is added.
        let other = dir.join("plain.csv");
        std::fs::write(&other, b"a\r\n1\r\n").unwrap();
        let j = docs.open_path(&other, false).unwrap().id;
        let out = dir.join("plain-changes.csv");
        std::fs::write(&out, b"x").unwrap();
        assert!(!private_copy(&docs, &recent, j, &out).unwrap());
        assert!(!recent.is_private(&out));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
