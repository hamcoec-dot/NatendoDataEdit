//! Tauri shell for Natendo Data Edit. Holds IPC commands only; all data logic
//! lives in `natendo-engine`, and the boundary rules live in `docs`.
//!
//! Security rule: no command accepts a file path from the webview. Open and
//! Save As show the native dialog here, in Rust, and use the path it returns.

mod agent;
mod applog;
mod assoc;
mod docs;
mod extras;
mod imports;
mod links;
mod recovery;
mod watch;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Set when the user agreed to quit with unsaved changes.
#[derive(Default)]
struct Quit(AtomicBool);

type DocsState<'a> = State<'a, Arc<Docs>>;
type ImportsState<'a> = State<'a, Arc<Imports>>;
type RecentState<'a> = State<'a, Arc<Recent>>;
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
const OTHER_TYPES: &[&str] = &[
    "json", "jsonl", "ndjson", "xml", "html", "htm", "md", "log", "prn", "fwf",
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
    let opened = if is_import_type(path) {
        Opened {
            doc: None,
            import: Some(imports.open_path(path)?),
        }
    } else {
        let info = docs.open_path(path, false)?;
        Opened {
            doc: Some(info),
            import: None,
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
        if let Some(w) = app.get_webview_window("main") {
            if focus {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
            let _ = w.emit("opened", ());
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
fn set_read_only(docs: DocsState<'_>, doc: u32, on: bool) -> CmdResult<DocInfo> {
    docs.set_read_only(doc, on)
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
            .copied()
            .collect();
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open")
            .add_filter("Tables", &all)
            .add_filter("Delimited text", FILE_TYPES)
            .add_filter("Spreadsheets", SHEET_TYPES)
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
    use natendo_engine::clean::{Cleaner, MAX_CLEANER_BYTES};
    blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open cleaner")
            .add_filter("Natendo cleaner", &[CLEANER_EXT])
            .add_filter("All files", &["*"])
            .set_parent(&window)
            .pick_file()
        else {
            return Ok(None);
        };
        let bytes = extras::read_capped(&path, MAX_CLEANER_BYTES as u64, "a cleaner file")?;
        Ok(Some(Cleaner::from_json(&bytes)?))
    })
    .await
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
        docs.export_file(doc, &path, &options).map(Some)
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
            });
        }
        open_any(&docs, &imports, &recent, &path)
    })
    .await
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
) -> CmdResult<Option<u64>> {
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
        let n = docs.save_change_report(doc, &path)?;
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
        let list = natendo_engine::sqlite::tables(&path)?;
        *SQLITE_PICKED.lock().unwrap_or_else(|p| p.into_inner()) = Some(path.clone());
        Ok(Some((path.display().to_string(), list)))
    })
    .await
}

/// Open a table of the picked database in a new tab.
#[tauri::command]
async fn sqlite_open(docs: DocsState<'_>, table: String) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || {
        let path = SQLITE_PICKED
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| CmdError::new("invalid", "choose the database first"))?;
        docs.open_sqlite(&path, &table)
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
) -> CmdResult<watch::WatchStatus> {
    WATCHER
        .start(force, existing, recent.inner().clone())
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
    now: f64,
    force: bool,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.run_recipe(doc, &recipe, now, force)).await
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
fn close_document(docs: DocsState<'_>, folders: FoldersState<'_>, doc: u32) {
    docs.close(doc);
    folders.forget(doc);
}

#[tauri::command]
fn doc_info(docs: DocsState<'_>, doc: u32) -> CmdResult<DocInfo> {
    docs.info(doc)
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
async fn set_cells(
    docs: DocsState<'_>,
    doc: u32,
    cells: Vec<(u64, u32, String)>,
) -> CmdResult<DocInfo> {
    let docs = docs.inner().clone();
    blocking(move || docs.set_cells(doc, cells)).await
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
        Structure::InsertCols { at, count } => docs.insert_cols(doc, at, count),
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
        let info = docs.save(doc, path, overwrite_changed, allow_lossy, options.as_ref())?;
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

/// Close the window after the user agreed to discard unsaved changes.
#[tauri::command]
fn job_progress(docs: DocsState<'_>, doc: u32) -> CmdResult<Option<JobInfo>> {
    docs.job(doc)
}

#[tauri::command]
fn cancel_job(docs: DocsState<'_>, doc: u32) -> CmdResult<()> {
    docs.cancel_job(doc)
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
fn filter_spec(docs: DocsState<'_>, doc: u32) -> CmdResult<Option<FilterSpec>> {
    docs.filter_spec(doc)
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
) -> CmdResult<ReplaceReport> {
    let docs = docs.inner().clone();
    blocking(move || docs.replace_confirm(doc, spec, replacement)).await
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

#[tauri::command]
fn quit_app(window: WebviewWindow, quit: State<'_, Quit>) {
    quit.0.store(true, Ordering::Relaxed);
    let _ = window.close();
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
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            } else {
                open_paths(app, paths, true);
            }
        }))
        .manage(Arc::new(Docs::default()))
        .manage(Arc::new(Agent::default()))
        .manage(Pending::default())
        .manage(Arc::new(Imports::default()))
        .manage(Arc::new(Recent::default()))
        .manage(Arc::new(Folders::default()))
        .setup(|app| {
            if let Ok(dir) = app.path().app_log_dir() {
                applog::init(dir);
            }
            if let Ok(dir) = app.path().app_local_data_dir() {
                recovery::init(dir);
            }
            applog::info(&format!(
                "Natendo Data Edit {} started",
                env!("CARGO_PKG_VERSION")
            ));
            if let Ok(dir) = app.path().app_data_dir() {
                app.state::<Arc<Recent>>().load(&dir);
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
                open_paths(app.handle(), paths, false);
            }
            Ok(())
        })
        .manage(Quit::default())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
                open_paths(window.app_handle(), paths.clone(), false);
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let agreed = app.state::<Quit>().0.load(Ordering::Relaxed);
                if !agreed && app.state::<Arc<Docs>>().any_modified() {
                    api.prevent_close();
                    let _ = window.emit("close-requested", ());
                }
            }
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
            set_cells,
            fill_cells,
            move_cells,
            copy_cells,
            change,
            save,
            job_progress,
            cancel_job,
            sort_rows,
            set_filter,
            clear_filter,
            filter_spec,
            filter_values,
            find_next,
            find_all,
            find_count,
            find_marks,
            set_doc_title,
            find_count_cancel,
            replace_confirm,
            batch_preview,
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
            import_finish,
            import_progress,
            import_cancel,
            import_close,
            export_preview,
            recent_files,
            open_recent,
            first_row_cell,
            first_row_names,
            set_first_row_cell,
            tab_folders,
            folder_list,
            folder_open,
            view_save,
            view_open,
            set_read_only,
            column_types,
            column_summary,
            selection_totals,
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
            cleaner_save,
            cleaner_open,
            export_file,
            export_text,
            take_opened,
            compute_preview,
            join_preview,
            value_tally,
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
            label_sheets,
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
            check_marks,
            checks_save,
            checks_open,
            file_types,
            set_file_types,
            open_default_apps,
            agent_status,
            set_agent_link,
            log_event,
            open_log_folder,
            quit_app
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
        let dir = std::env::temp_dir().join(format!("natendo-pcopy-{}", std::process::id()));
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
        docs.save_change_report(id, &report).unwrap();
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
