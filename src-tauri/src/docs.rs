//! Open documents and the rules at the UI-to-engine boundary.
//!
//! The webview is treated as untrusted input:
//! - It never names a file. Paths come only from native file dialogs shown by
//!   this process (see `lib.rs`); the webview refers to documents by id.
//! - Every read is capped: viewports by cell count and characters per cell,
//!   single-cell reads at [`MAX_EDIT_CHARS`], copies by cells and bytes.
//! - Every write is capped: cells per call, characters per value and in total.
//!
//! Indexing runs on a background thread in steps, releasing the document's
//! lock between steps so the first screen can be drawn while a big file is
//! still being read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::recovery::Autosaved;
use natendo_engine::clean::{CleanPreview, CleanSpec, SplitName};
use natendo_engine::editing::{
    Arrange, Extract, GroupCalc, MapValues, TallySpec, ValueTally, MAX_PAIRS,
};
use natendo_engine::example::ExampleSpec;
use natendo_engine::export::{ExportFormat, ExportOptions, ExportReport};
use natendo_engine::expr::Lookups;
use natendo_engine::expr::{ComputePreview, ComputeSpec};
use natendo_engine::filter::{FilterSpec, Rule, ValueList};
use natendo_engine::find::{Change, FindAll, FindSpec, Matcher, Pattern, ReplacePlan};
use natendo_engine::highlight::{Highlight, Mark};
use natendo_engine::join::{
    AppendSource, AppendSpec, JoinPreview, JoinReport, JoinSource, JoinSpec, LookupTable,
};
use natendo_engine::ops::{
    self, ColPlan, Dir, FillSpec, MergeColumns, PasteSpec, SplitColumn, MAX_BLOCK_CELLS,
};
use natendo_engine::pivot::{Cube, GridLimit, ItemList, PivotField, PivotGrid, PivotSpec};
use natendo_engine::recipe::{Recipe, RecipeOp};
use natendo_engine::reshape::{
    Blanks, Duplicates, MergeRows, Pivot, RowPlan, SplitRows, Table, Unpivot,
};
use natendo_engine::rows::{ShowRows, Totals};
use natendo_engine::sort::SortSpec;
use natendo_engine::stats::{ChartData, ChartSpec, ColType, SubtotalSpec, Summary};
use natendo_engine::tools::{
    Breaks, Combine, CompareIndex, CompareReport, CompareSpec, FillBlanks, Repeats, Reshape,
    Sample, SplitParts, Stack, MAX_COMBINE,
};
use natendo_engine::validate::{CheckMark, CheckReport, CheckSpec};
use natendo_engine::value::ReadOptions;
use natendo_engine::{
    scan, Dialect, Document, Encoding, Error, Format, LineBreak, OpenOptions, Progress, Quoting,
    SaveOptions,
};
use serde::{Deserialize, Serialize};

/// Most documents open at once.
pub const MAX_DOCS: usize = 64;
/// Label of pivot jobs (a newer preview cancels an older one).
const PIVOT_JOB: &str = "Pivot";
/// Rows and value columns a pivot preview shows.
pub const PIVOT_PREVIEW_ROWS: usize = 2_000;
pub const PIVOT_PREVIEW_COLS: usize = 400;
/// Largest pivot copied as text.
const MAX_CLIP_TEXT: usize = MAX_COPY_BYTES;
/// Bytes indexed per step; the lock is released between steps.
pub const INDEX_STEP: usize = 8 << 20;
/// Characters per cell sent for display.
pub const DISPLAY_CHARS: usize = 1024;
/// Total characters one viewport may carry (about 16 MB worst case).
pub const VIEWPORT_CHAR_BUDGET: usize = 4 << 20;
/// Cells in one viewport request.
pub const MAX_VIEWPORT_CELLS: u64 = 20_000;
/// Longest value that can be read into the editor or written as one cell.
pub const MAX_EDIT_CHARS: usize = 1 << 20;
/// Cells changed by one command (paste, clear, fill).
pub const MAX_WRITE_CELLS: u64 = 100_000;
/// Characters written by one command, all cells together.
pub const MAX_WRITE_CHARS: usize = 64 << 20;
/// Rows inserted by one command.
pub const MAX_INSERT_ROWS: u64 = 100_000;
/// Cells copied to the clipboard at once.
pub const MAX_COPY_CELLS: u64 = 1_000_000;
/// Bytes of clipboard text produced at once.
pub const MAX_COPY_BYTES: usize = 64 << 20;
/// Cells read per step while copying.
pub const COPY_BAND_CELLS: u64 = 1024;
/// Characters read per step while copying, all cells together (bounds the
/// memory one step can use, whatever the cells hold).
pub const COPY_BAND_CHARS: usize = 16 << 20;
/// Columns inserted by one command.
pub const MAX_INSERT_COLS: u32 = 1_000;
/// Longest pasted list of keys for a join (characters).
pub const MAX_JOIN_LIST_CHARS: usize = 64 << 20;
/// Most bytes of a translation list read from another tab.
pub const MAX_MAP_BYTES: usize = 256 << 20;

/// Where a translation list comes from: two columns of another tab.
#[derive(Debug, Clone, Deserialize)]
pub struct MapFrom {
    pub doc: u32,
    pub old: u32,
    pub new: u32,
    #[serde(default)]
    pub header: bool,
}
/// Columns a document may have after inserting columns.
pub const MAX_COLS_AFTER_INSERT: u32 = 16_384;
/// Cells (rows x columns) a document may have after inserting columns, so
/// one click cannot turn a save into terabytes.
pub const MAX_CELLS_AFTER_INSERT: u64 = 2_000_000_000;

/// Rows, columns and characters per cell in a layout preview.
pub const PREVIEW_ROWS: usize = 8;
pub const PREVIEW_COLS: usize = 12;
pub const PREVIEW_CHARS: usize = 40;
/// Characters of the lines above the table sent to the UI.
pub const PREAMBLE_CHARS: usize = 4096;
/// Most lines the user may set aside above the table.
pub const MAX_SKIP_LINES: u32 = 1000;
/// Hits listed by find-all.
pub const MAX_FIND_HITS: usize = 10_000;
/// Changes shown in a replace preview.
pub const PREVIEW_CHANGES: usize = 50;
/// Values in all of a filter's value lists together, and their total size.
pub const MAX_FILTER_VALUES: usize = 200_000;
pub const MAX_FILTER_BYTES: usize = 64 << 20;
/// Longest search text for a value list.
pub const MAX_SEARCH_CHARS: usize = 1000;
/// Matches listed by a search in the edit panel.
pub const MAX_TEXT_MATCHES: usize = 100_000;
/// Times a sort or filter is computed again when an edit made meanwhile
/// left its plan stale.
pub const COMPUTE_ATTEMPTS: usize = 3;

/// How a file is laid out, as the UI shows and edits it.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Layout {
    /// 1 to 4 characters (tab, space and control characters allowed).
    pub delimiter: String,
    pub quote: Option<String>,
    /// Inside quotes, makes the next character literal (backslash).
    pub escape: Option<String>,
    /// Runs of the separator count as one (columns lined up with spaces).
    pub merge: bool,
    /// Lines above the table, kept as they are.
    pub skip_lines: u32,
}

impl Layout {
    pub fn of(d: &Dialect, skip_lines: u32) -> Layout {
        let ch = |b: u8| (b as char).to_string();
        Layout {
            delimiter: d.delimiter_str().to_string(),
            quote: d.quote().map(ch),
            escape: d.escape().map(ch),
            merge: d.merges(),
            skip_lines,
        }
    }

    /// The engine dialect, checked (the UI's input is never trusted).
    pub fn dialect(&self) -> CmdResult<Dialect> {
        let one = |s: &Option<String>| -> CmdResult<Option<u8>> {
            match s.as_deref() {
                None => Ok(None),
                Some(t) if t.len() == 1 && t.is_ascii() => Ok(Some(t.as_bytes()[0])),
                Some(_) => Err(CmdError::new(
                    "bad_layout",
                    "quote and escape are one character",
                )),
            }
        };
        if !self.delimiter.is_ascii() {
            return Err(CmdError::new(
                "bad_layout",
                "the separator must be plain ASCII characters",
            ));
        }
        if self.skip_lines > MAX_SKIP_LINES {
            return Err(CmdError::new(
                "bad_layout",
                format!("at most {MAX_SKIP_LINES} lines can be set aside"),
            ));
        }
        Dialect::custom(
            self.delimiter.as_bytes(),
            one(&self.quote)?,
            one(&self.escape)?,
            self.merge,
        )
        .map_err(|e| CmdError::new("bad_layout", e.to_string()))
    }
}

/// Save As choices: how values are quoted, the qualifier (quote)
/// character, separator, encoding, byte-order mark and line breaks.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct SaveAs {
    pub delimiter: String,
    pub quote: Option<String>,
    pub quoting: Quoting,
    /// An encoding name the engine knows (UTF-8, UTF-16LE, windows-1252, ...).
    pub encoding: String,
    pub bom: bool,
    /// "CRLF", "LF" or "CR".
    pub line_break: String,
    /// Give every row this line break (else only new and edited rows).
    pub convert_line_breaks: bool,
}

impl SaveAs {
    /// The engine format, checked (the UI's input is never trusted).
    pub fn format(&self) -> CmdResult<(Format, bool, Quoting)> {
        let bad = |m: &str| CmdError::new("bad_layout", m);
        if !self.delimiter.is_ascii() || self.delimiter.is_empty() || self.delimiter.len() > 4 {
            return Err(bad("the separator must be 1 to 4 plain ASCII characters"));
        }
        let quote = match self.quote.as_deref() {
            None | Some("") => None,
            Some(t) if t.len() == 1 && t.is_ascii() => Some(t.as_bytes()[0]),
            Some(_) => return Err(bad("the qualifier is one plain character")),
        };
        if quote.is_none() && self.quoting != Quoting::Keep && self.quoting != Quoting::Needed {
            return Err(bad("choose a qualifier character to put values in quotes"));
        }
        let dialect = Dialect::custom(self.delimiter.as_bytes(), quote, None, false)
            .map_err(|e| bad(&e.to_string()))?;
        let encoding = Encoding::from_label(&self.encoding)
            .ok_or_else(|| bad("that encoding is not known"))?;
        let line_break = match self.line_break.as_str() {
            "CRLF" => LineBreak::CrLf,
            "LF" => LineBreak::Lf,
            "CR" => LineBreak::Cr,
            _ => return Err(bad("line breaks are CRLF, LF or CR")),
        };
        let bom = self.bom && !encoding.bom().is_empty();
        Ok((
            Format {
                encoding,
                bom,
                dialect,
                line_break,
            },
            self.convert_line_breaks,
            self.quoting,
        ))
    }
}

/// One way to read a file, with a preview.
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct LayoutChoice {
    pub layout: Layout,
    pub columns: u32,
    pub preview: Vec<Vec<String>>,
}

/// What detection found, for the layout picker.
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct LayoutChoices {
    /// 0 to 1.
    pub confidence: f32,
    pub sep_line: bool,
    pub current: Layout,
    /// Best first; the current layout is among them when detection found it.
    pub choices: Vec<LayoutChoice>,
    /// The lines above the table now set aside (cut to PREAMBLE_CHARS).
    pub preamble: String,
}

/// An error the UI can act on: `kind` picks the prompt, `message` is shown.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct CmdError {
    pub kind: &'static str,
    pub message: String,
    pub row: Option<u64>,
}

impl CmdError {
    pub fn new(kind: &'static str, message: impl Into<String>) -> CmdError {
        CmdError {
            kind,
            message: message.into(),
            row: None,
        }
    }
}

impl From<Error> for CmdError {
    fn from(e: Error) -> CmdError {
        let (kind, row) = match &e {
            Error::ChangedOnDisk => ("changed_on_disk", None),
            Error::Lossy { row } => ("lossy", *row),
            Error::NoPath => ("no_path", None),
            Error::StillLoading => ("still_loading", None),
            Error::Cancelled => ("cancelled", None),
            Error::Unmappable { row, .. } => ("unmappable", Some(*row)),
            Error::CannotRepresent { row, .. } => ("cannot_represent", Some(*row)),
            Error::VerifyFailed { row } => ("verify_failed", Some(*row)),
            Error::SaveIncomplete { .. } => ("save_incomplete", None),
            Error::SavedButReloadFailed { .. } => ("reload_failed", None),
            Error::RowTooWide { row, .. } => ("row_too_wide", Some(*row)),
            Error::OutOfMemory(_) => ("out_of_memory", None),
            Error::OutOfRange => ("out_of_range", None),
            Error::BadPattern(_) => ("bad_pattern", None),
            Error::FilterActive => ("filter_active", None),
            Error::Changed => ("changed", None),
            Error::Invalid(_) => ("invalid", None),
            _ => ("error", None),
        };
        CmdError {
            kind,
            message: e.to_string(),
            row,
        }
    }
}

pub type CmdResult<T> = Result<T, CmdError>;

/// A long operation running on a document (one at a time).
struct Job {
    label: &'static str,
    progress: Arc<Progress>,
}

/// Progress of a document's long operation, for the UI.
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct JobInfo {
    pub label: &'static str,
    /// 0 to 1.
    pub fraction: f64,
}

/// One open document.
pub struct Entry {
    /// Read/write lock: long operations compute under a read lock, so the
    /// grid keeps drawing, and apply under the write lock.
    doc: RwLock<Document>,
    job: Mutex<Option<Job>>,
    /// The last replace computed for a preview, reused by "Replace all"
    /// when nothing changed since (request as JSON, plan).
    replace: Mutex<Option<(String, ReplacePlan)>>,
    cancel: AtomicBool,
    /// Opened with quoted cells never spanning lines (cleared once a save
    /// has reloaded the file normally).
    pub line_by_line: AtomicBool,
    /// Indexing failed (shown once; the document stays usable as far as read).
    error: Mutex<Option<String>>,
    /// Name shown for a document with no file yet (imports).
    title: Mutex<Option<String>>,
    /// Read-only: every change and every save over the file is refused.
    locked: AtomicBool,
    /// The last data-check run: the checks (as JSON), the document revision
    /// and, per unique check, the rows whose value repeats.
    checks: Mutex<Option<LastChecks>>,
    /// The quick totals being worked out for the selection, if any: a newer
    /// selection or any change cancels them.
    totals: Mutex<Option<Arc<Progress>>>,
    /// Type-ahead values of one column, for one revision (memory only).
    #[allow(clippy::type_complexity)]
    typeahead: Mutex<Option<(u64, u32, Arc<Vec<(String, String)>>)>>,
    /// Changes waiting for (or taking) the write lock. Quick totals and
    /// recovery copies, which hold the read lock, stop when this is above
    /// zero, so a change never waits for them.
    writers: AtomicU32,
    /// Set when the tab is closed: a recovery copy being written stops.
    closed: AtomicBool,
    /// When the document last changed (milliseconds since 1970).
    changed_at: AtomicU64,
    /// A recovery copy is being written (only one at a time).
    autosaving: AtomicBool,
    /// Rows times columns when the document was last too large for a
    /// recovery copy (0: not). No copy is tried again until it has shrunk.
    too_large: AtomicU64,
    /// The document revision of the last recovery copy (`u64::MAX`: none).
    autosaved: AtomicU64,
    /// A recovery copy was stopped by a change: wait for a pause next time.
    copy_cancelled: AtomicBool,
    /// Pivot cubes read lately (scan key, revision, quick look, cube): the
    /// last quick preview and the last full one.
    #[allow(clippy::type_complexity)]
    pivots: Mutex<Vec<(String, u64, bool, Arc<Cube>)>>,
    /// When a due copy was first put off for a pause (0: not waiting).
    waiting_since: AtomicU64,
    /// Private file mode: no recovery copies and no earlier saves.
    private: AtomicBool,
    /// Private because of where its data came from (a sample of, a join
    /// from or an import of something private), whatever its own path.
    private_origin: AtomicBool,
    /// The encrypted file this tab was opened from or last saved to
    /// encrypted (Save keeps it encrypted there).
    encrypted_from: Mutex<Option<PathBuf>>,
    /// The steps done, for a recipe (see `Docs::recorded`).
    recipe: Mutex<RecipeLog>,
    /// The other list of the last "match people" run (for accepting).
    matched: Mutex<Option<MatchKept>>,
    /// The last comparison with a newer file (for accepting).
    updates: Mutex<Option<UpdatesKept>>,
    /// Notes and flags (loaded from beside the file when first asked).
    notes: Mutex<Option<natendo_engine::notes::NoteBook>>,
}

/// A big document (or one whose last copy was stopped by a change) gets a
/// recovery copy only once it has been left alone this long, so a copy that
/// takes seconds is not started and thrown away while the user is still
/// editing. Small documents are copied at once (it takes milliseconds).
const AUTOSAVE_IDLE_MS: u64 = 30_000;
/// The pause is not waited for longer than this: steady editing still gets
/// a copy (which a change stops at once if it comes during the copy).
const AUTOSAVE_MAX_WAIT_MS: u64 = 5 * 60_000;
/// "Big" for the pause: a file over this many bytes...
const AUTOSAVE_BIG_BYTES: u64 = 20 << 20;
/// ...or over this many cells.
const AUTOSAVE_BIG_CELLS: u64 = 2_000_000;

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Clears a flag when dropped.
struct ClearOnDrop<'a>(&'a AtomicBool);
impl Drop for ClearOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Entry {
    // A panic while holding the lock leaves the engine's own state
    // consistent (every op is applied whole), so keep going.
    fn read(&self) -> RwLockReadGuard<'_, Document> {
        self.doc.read().unwrap_or_else(|p| p.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, Document> {
        // Quick totals and recovery copies hold a read lock; never make a
        // change wait for them. The change is announced before the lock is
        // asked for: a reader that got the lock first sees it (in its own
        // checks) and stops, and quick totals are also cancelled directly.
        self.writers.fetch_add(1, Ordering::SeqCst);
        self.cancel_totals();
        let g = self.doc.write().unwrap_or_else(|p| p.into_inner());
        self.writers.fetch_sub(1, Ordering::SeqCst);
        self.changed_at.store(now_millis(), Ordering::Relaxed);
        g
    }

    /// Whether a due recovery copy waits for a pause in editing: only a
    /// `big` document (see [`AUTOSAVE_BIG_BYTES`]) changed in the last
    /// [`AUTOSAVE_IDLE_MS`], and not once it has waited
    /// [`AUTOSAVE_MAX_WAIT_MS`].
    fn put_off_copy(&self, big: bool, now: u64) -> bool {
        if !big || now.saturating_sub(self.changed_at.load(Ordering::Relaxed)) >= AUTOSAVE_IDLE_MS {
            return false;
        }
        let since = match self.waiting_since.load(Ordering::Relaxed) {
            0 => {
                self.waiting_since.store(now, Ordering::Relaxed);
                now
            }
            t => t,
        };
        now.saturating_sub(since) < AUTOSAVE_MAX_WAIT_MS
    }

    /// A change is waiting for the document (or the tab was closed): a
    /// reader running for its own sake (quick totals, a recovery copy)
    /// should stop.
    fn change_waiting(&self) -> bool {
        self.writers.load(Ordering::SeqCst) > 0 || self.closed.load(Ordering::SeqCst)
    }

    fn cancel_totals(&self) {
        if let Some(p) = self.totals.lock().unwrap_or_else(|p| p.into_inner()).take() {
            p.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn jobs(&self) -> MutexGuard<'_, Option<Job>> {
        self.job.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Start a long operation; refused while another one runs.
    fn start(&self, label: &'static str) -> CmdResult<JobGuard<'_>> {
        let mut j = self.jobs();
        if j.is_some() {
            return Err(CmdError::new(
                "busy",
                "another operation is still running on this file",
            ));
        }
        let progress = Arc::new(Progress::new());
        *j = Some(Job {
            label,
            progress: progress.clone(),
        });
        Ok(JobGuard {
            entry: self,
            progress,
        })
    }
}

/// The last data-check run: checks (as JSON), revision, unique repeats.
type LastChecks = (String, u64, Vec<(u32, Vec<u64>)>);

/// Ends a job when dropped (also on errors and panics).
struct JobGuard<'a> {
    entry: &'a Entry,
    progress: Arc<Progress>,
}

impl Drop for JobGuard<'_> {
    fn drop(&mut self) {
        *self.entry.jobs() = None;
    }
}

/// A cell found by find-next.
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Found {
    pub row: u64,
    pub col: u32,
    /// The search went past the end and started over.
    pub wrapped: bool,
}

/// What a replace would do (or did).
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ReplaceReport {
    pub cells: u64,
    pub hits: u64,
    pub preview: Vec<Change>,
    /// After "Replace all": the document's new state.
    pub info: Option<DocInfo>,
}

/// A data operation from the Data and Edit menus (Milestone 4b). Every
/// field is bounded: text by [`MAX_WRITE_CHARS`], cell counts and column
/// lists by the engine's caps, separators by length.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DataOp {
    /// Paste or Paste Special of clipboard text (tab-separated).
    Paste {
        spec: PasteSpec,
        text: String,
    },
    Fill {
        spec: FillSpec,
    },
    ShowDuplicates {
        spec: Duplicates,
    },
    RemoveDuplicates {
        spec: Duplicates,
    },
    RemoveBlankRows {
        spec: Blanks,
    },
    RemoveBlankCols {
        spec: Blanks,
    },
    SplitColumn {
        spec: SplitColumn,
    },
    MergeColumns {
        spec: MergeColumns,
    },
    SplitRows {
        spec: SplitRows,
    },
    MergeRows {
        spec: MergeRows,
    },
    SortColumns {
        col: u32,
        count: u32,
        descending: bool,
    },
    /// Bring in another open document's values by matching keys (or keep
    /// or drop the rows whose key is on a pasted list).
    Join {
        #[serde(default)]
        other: Option<u32>,
        #[serde(default)]
        list: Option<String>,
        /// A list kept by [`Docs::join_list`].
        #[serde(default)]
        list_id: Option<u64>,
        spec: JoinSpec,
    },
    /// Add another open document's rows at the end.
    Append {
        other: u32,
        spec: AppendSpec,
    },
    /// Run a cleaner (one tool or a saved list). `now`: the local date and
    /// time as days since 1970-01-01, for "add date and time".
    Clean {
        spec: CleanSpec,
        now: f64,
    },
    SplitName {
        spec: SplitName,
    },
    /// Fill a column's empty cells from the examples typed in it.
    FillExample {
        spec: ExampleSpec,
    },
    /// Add a computed column, or write over one.
    Compute {
        spec: ComputeSpec,
    },
    /// Show only the rows that fail data checks.
    ShowProblems {
        spec: CheckSpec,
        /// Show the rows that pass instead.
        #[serde(default)]
        passing: bool,
    },
    /// Put the rows that fail the checks first.
    SortProblems {
        spec: CheckSpec,
    },
    /// Add a "Check result" column (OK, or the checks the row fails).
    CheckColumn {
        spec: CheckSpec,
        #[serde(default)]
        title: String,
    },
    /// Fill empty cells from above or below, or with a text.
    FillBlanks {
        spec: FillBlanks,
    },
    /// A new column counting each key's rows.
    CountRepeats {
        spec: Repeats,
    },
    /// Empty the key cells of rows that repeat an earlier key.
    BlankRepeats {
        spec: Duplicates,
    },
    /// Blank rows between groups.
    Breaks {
        spec: Breaks,
    },
    /// Delete the rows the user hid.
    DeleteHidden,
    /// Replace whole cells by a list of old → new values (the value
    /// editor), the list given or read from two columns of another tab.
    MapValues {
        spec: MapValues,
        #[serde(default)]
        from: Option<MapFrom>,
    },
    /// A new column with a figure for each row's group.
    GroupCalc {
        spec: GroupCalc,
    },
    /// Keep, order and rename columns in one step.
    Arrange {
        spec: Arrange,
    },
    /// Pull a piece of each cell into a new column.
    Extract {
        spec: Extract,
    },
}

/// Check a cleaner run from the webview: the moment (years 1900 to 9999,
/// as days since 1970) and the column list (the cleaner itself is checked
/// by the engine).
fn check_clean(spec: &CleanSpec, now: f64) -> CmdResult<()> {
    if !now.is_finite() || !(-25_567.0..=2_932_896.0).contains(&now) {
        return Err(CmdError::new(
            "bad_request",
            "the date and time of the run is not valid",
        ));
    }
    if let natendo_engine::find::Scope::Columns { cols } = &spec.scope {
        if cols.len() > 16_384 {
            return Err(CmdError::new("bad_request", "too many columns chosen"));
        }
    }
    Ok(())
}

/// A seed for random text: differs between runs.
fn run_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
}

/// A new table built from a document (opened in a new tab).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TableOp {
    Pivot {
        spec: Pivot,
    },
    Unpivot {
        spec: Unpivot,
    },
    Subtotals {
        spec: SubtotalSpec,
    },
    /// One column's cells laid out several to a row.
    Reshape {
        spec: Reshape,
    },
    /// Several columns stacked into one.
    Stack {
        spec: Stack,
    },
    /// A sample of rows.
    Sample {
        spec: Sample,
    },
    /// This document compared with another open one.
    Compare {
        other: u32,
        spec: CompareSpec,
    },
}

/// A new table and, for comparisons, what was found.
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct TableReport {
    pub info: DocInfo,
    pub compare: Option<CompareReport>,
}

/// What a comparison with a newer file keeps for accepting changes.
struct UpdatesKept {
    updates: natendo_engine::updates::Updates,
    other_tab: u32,
}

/// What a "match people" run keeps for accepting pairs.
struct MatchKept {
    /// The other list (`None`: near-duplicates in the tab itself).
    other: Option<natendo_engine::fuzzy::MatchSide>,
    other_tab: Option<u32>,
    bring_titles: Vec<String>,
    keep_first: bool,
    /// The tab's revision when run.
    revision: u64,
}

/// A step noted for a recipe.
enum Logged {
    Step(RecipeOp),
    /// Done but cannot be repeated by a recipe (in words).
    LeftOut(String),
}

/// What a tab's recipe has so far: the first row before the first step
/// (the column titles), and the steps with their place in the history.
#[derive(Default)]
struct RecipeLog {
    first_row: Option<Vec<String>>,
    steps: Vec<(u64, Logged)>,
}

/// The recipe step for a data operation, or why it cannot be one.
fn recipe_step(op: &DataOp) -> Logged {
    let s = match op {
        DataOp::RemoveDuplicates { spec } => RecipeOp::RemoveDuplicates { spec: spec.clone() },
        DataOp::RemoveBlankRows { spec } => RecipeOp::RemoveBlankRows { spec: spec.clone() },
        DataOp::RemoveBlankCols { spec } => RecipeOp::RemoveBlankCols { spec: spec.clone() },
        DataOp::SplitColumn { spec } => RecipeOp::SplitColumn { spec: spec.clone() },
        DataOp::MergeColumns { spec } => RecipeOp::MergeColumns { spec: spec.clone() },
        DataOp::SplitRows { spec } => RecipeOp::SplitRows { spec: spec.clone() },
        DataOp::MergeRows { spec } => RecipeOp::MergeRows { spec: spec.clone() },
        DataOp::SortColumns {
            col,
            count,
            descending,
        } => RecipeOp::SortColumns {
            col: *col,
            count: *count,
            descending: *descending,
        },
        DataOp::Clean { spec, .. } => RecipeOp::Clean { spec: spec.clone() },
        DataOp::SplitName { spec } => RecipeOp::SplitName { spec: spec.clone() },
        DataOp::FillExample { spec } => RecipeOp::FillExample { spec: spec.clone() },
        DataOp::Compute { spec } if spec.expr.to_ascii_uppercase().contains("LOOKUP(") => {
            return Logged::LeftOut("a computed column that looks up another tab".into())
        }
        DataOp::Compute { spec } => RecipeOp::Compute { spec: spec.clone() },
        DataOp::FillBlanks { spec } => RecipeOp::FillBlanks { spec: spec.clone() },
        DataOp::CountRepeats { spec } => RecipeOp::CountRepeats { spec: spec.clone() },
        DataOp::BlankRepeats { spec } => RecipeOp::BlankRepeats { spec: spec.clone() },
        DataOp::Breaks { spec } => RecipeOp::Breaks { spec: spec.clone() },
        DataOp::DeleteHidden => RecipeOp::DeleteHidden,
        DataOp::MapValues { spec, from: None } => RecipeOp::MapValues { spec: spec.clone() },
        DataOp::MapValues { .. } => {
            return Logged::LeftOut("changing values from a list in another tab".into())
        }
        DataOp::GroupCalc { spec } => RecipeOp::GroupCalc { spec: spec.clone() },
        DataOp::Arrange { spec } => RecipeOp::Arrange { spec: spec.clone() },
        DataOp::Extract { spec } => RecipeOp::Extract { spec: spec.clone() },
        DataOp::Paste { .. } => return Logged::LeftOut("a paste".into()),
        DataOp::Fill { .. } => return Logged::LeftOut("a fill of the selection".into()),
        DataOp::SortProblems { spec } => RecipeOp::SortProblems { spec: spec.clone() },
        DataOp::CheckColumn { spec, title } => RecipeOp::CheckColumn {
            spec: spec.clone(),
            title: title.clone(),
        },
        DataOp::ShowDuplicates { .. } | DataOp::ShowProblems { .. } => {
            return Logged::LeftOut("showing duplicates or problems (a view, not a change)".into())
        }
        DataOp::Join { .. } => return Logged::LeftOut("a join with another tab or a list".into()),
        DataOp::Append { .. } => return Logged::LeftOut("appending another tab".into()),
    };
    Logged::Step(s)
}

/// The data operation that does a recipe step (not for the steps that have
/// their own command: sort, filter, columns, Replace All).
fn recipe_data_op(s: RecipeOp, now: f64) -> DataOp {
    match s {
        RecipeOp::RemoveDuplicates { spec } => DataOp::RemoveDuplicates { spec },
        RecipeOp::RemoveBlankRows { spec } => DataOp::RemoveBlankRows { spec },
        RecipeOp::RemoveBlankCols { spec } => DataOp::RemoveBlankCols { spec },
        RecipeOp::SplitColumn { spec } => DataOp::SplitColumn { spec },
        RecipeOp::MergeColumns { spec } => DataOp::MergeColumns { spec },
        RecipeOp::SplitRows { spec } => DataOp::SplitRows { spec },
        RecipeOp::MergeRows { spec } => DataOp::MergeRows { spec },
        RecipeOp::SortColumns {
            col,
            count,
            descending,
        } => DataOp::SortColumns {
            col,
            count,
            descending,
        },
        RecipeOp::Clean { spec } => DataOp::Clean { spec, now },
        RecipeOp::SplitName { spec } => DataOp::SplitName { spec },
        RecipeOp::FillExample { spec } => DataOp::FillExample { spec },
        RecipeOp::Compute { spec } => DataOp::Compute { spec },
        RecipeOp::FillBlanks { spec } => DataOp::FillBlanks { spec },
        RecipeOp::CountRepeats { spec } => DataOp::CountRepeats { spec },
        RecipeOp::BlankRepeats { spec } => DataOp::BlankRepeats { spec },
        RecipeOp::Breaks { spec } => DataOp::Breaks { spec },
        RecipeOp::DeleteHidden => DataOp::DeleteHidden,
        RecipeOp::MapValues { spec } => DataOp::MapValues { spec, from: None },
        RecipeOp::GroupCalc { spec } => DataOp::GroupCalc { spec },
        RecipeOp::Arrange { spec } => DataOp::Arrange { spec },
        RecipeOp::Extract { spec } => DataOp::Extract { spec },
        RecipeOp::SortProblems { mut spec } => {
            spec.now = now;
            DataOp::SortProblems { spec }
        }
        RecipeOp::CheckColumn { mut spec, title } => {
            spec.now = now;
            DataOp::CheckColumn { spec, title }
        }
        RecipeOp::Sort { .. }
        | RecipeOp::Filter { .. }
        | RecipeOp::ClearFilter
        | RecipeOp::InsertCols { .. }
        | RecipeOp::DeleteCols { .. }
        | RecipeOp::MoveCols { .. }
        | RecipeOp::ReplaceAll { .. } => unreachable!("handled by its own command"),
    }
}

/// What a split into files wrote.
#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
pub struct SplitReport {
    pub files: u64,
    pub rows: u64,
    /// Files that were there before and were replaced.
    pub replaced: u64,
    /// From a private tab: new files put in private file mode, and those
    /// that could not be (the list of private files was full).
    pub private: u64,
    pub private_missed: u64,
    /// The folder was empty, so the folder itself was made private.
    pub private_folder: bool,
}

/// A cell value or name as a safe Windows file name part: no path
/// characters, control characters, reserved device names or dots and
/// spaces at the end; at most 100 characters.
pub(crate) fn part_name(name: &str, empty: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(100)
        .collect();
    while s.ends_with(['.', ' ']) {
        s.pop();
    }
    let s = s.trim_start().to_string();
    if s.is_empty() {
        return empty.to_string();
    }
    let stem = s
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        format!("_{s}")
    } else {
        s
    }
}

/// Quick totals, with the distinct count when asked for.
#[derive(Debug, Serialize, Clone, Copy, PartialEq)]
pub struct TotalsOut {
    #[serde(flatten)]
    pub totals: Totals,
    pub distinct: Option<u64>,
    /// False when there were more distinct values than counted.
    pub distinct_exact: bool,
}

impl From<Totals> for TotalsOut {
    fn from(totals: Totals) -> Self {
        TotalsOut {
            totals,
            distinct: None,
            distinct_exact: true,
        }
    }
}

/// What a data operation did.
#[derive(Debug, Serialize, Clone, PartialEq, Default)]
pub struct OpReport {
    pub info: Option<DocInfo>,
    /// Cells changed.
    pub cells: u64,
    /// Rows removed, added or shown.
    pub rows: u64,
    /// Groups found (duplicates, merges).
    pub groups: u64,
    /// Columns added or removed.
    pub cols: u32,
    /// Cells left alone (Paste Special: not a number, division by zero).
    pub skipped: u64,
    /// Join and append: what matched.
    pub join: Option<JoinReport>,
    /// Cleaners: cells a step could not read (not a date or number);
    /// keep newest/oldest: copies whose date could not be read.
    pub left: u64,
    /// Cleaners: dates read the other way round from the chosen day order.
    pub other_order: u64,
    /// Fill by example: the pattern used, in words.
    pub pattern: Option<String>,
    /// Fill by example: examples learned from.
    pub examples: u64,
    /// Computed columns: cells that came out as error codes.
    pub errors: u64,
}

/// Changes listed, and rows looked at, by a cleaner preview.
const CLEAN_PREVIEW_CHANGES: usize = 50;
const CLEAN_PREVIEW_ROWS: u64 = 20_000;

/// The other table of a join or append, copied out of its document.
enum Source {
    Join(Arc<JoinSource>),
    Append(AppendSource),
}

/// The planned form of a [`DataOp`].
enum Planned {
    Clean(natendo_engine::clean::CleanPlan),
    Join(natendo_engine::join::JoinPlan),
    Cells(ops::CellPlan),
    Cols(ColPlan),
    Rows(RowPlan),
    Sort(natendo_engine::sort::SortPlan),
}

/// What the UI shows about a document.
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct DocInfo {
    pub id: u32,
    pub name: String,
    /// Shown in tooltips only; never accepted back from the UI.
    pub path: Option<String>,
    /// Rows shown (all rows, or those a filter shows).
    pub rows: u64,
    /// Every row, shown or not.
    pub total_rows: u64,
    pub filtered: bool,
    /// Columns with a filter rule.
    pub filter_cols: Vec<u32>,
    /// A filter rule sits on a deleted column (its rows stay hidden).
    pub filter_on_deleted: bool,
    pub cols: u32,
    pub indexed: bool,
    pub progress: f64,
    pub modified: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Changes done minus changes undone: tells an undo of a column move
    /// from an undo of something else, so the view can follow.
    pub history_pos: u64,
    pub encoding: &'static str,
    pub bom: bool,
    pub delimiter: String,
    pub quote: Option<String>,
    pub escape: Option<String>,
    pub merge: bool,
    /// Lines above the table, kept as they are.
    pub preamble_lines: u32,
    /// How sure layout detection was (0 to 1); `None` when the layout was
    /// chosen by the user.
    pub layout_confidence: Option<f32>,
    /// Read-only (file lock): changes and saving over the file are refused.
    pub read_only: bool,
    /// Rows the user hid (still saved).
    pub hidden_rows: u64,
    /// A filter is on (`filtered` is also true while rows are only hidden).
    pub has_filter: bool,
    pub line_break: &'static str,
    pub mixed_line_breaks: bool,
    /// Decoding replaced bytes: saving needs consent.
    pub lossy: bool,
    pub invalid_bytes: bool,
    pub line_by_line: bool,
    pub column_changes: bool,
    pub repair_count: u64,
    pub unclosed_quotes: bool,
    pub long_quoted_cells: bool,
    /// First row read line by line after a quote that never closed.
    pub line_bound_from: Option<u64>,
    pub error: Option<String>,
}

/// All open documents.
#[derive(Default)]
pub struct Docs {
    next: AtomicU32,
    map: Mutex<HashMap<u32, Arc<Entry>>>,
    /// Kept while a Join screen is open, one per tab the screen is for
    /// (so two windows never share one): the indexed other table (or
    /// list) and pasted lists, so previews do not copy them again.
    joins: Mutex<HashMap<u32, JoinCache>>,
    /// Numbers given to pasted lists.
    next_list: std::sync::atomic::AtomicU64,
    /// LOOKUP tables by tab, its revision and the two columns.
    lookups: Mutex<Vec<(String, Arc<LookupTable>)>>,
    /// Whether a path is in private file mode (set when the app starts;
    /// the recent list keeps the private files and folders).
    private_check: Mutex<Option<PrivateCheck>>,
}

/// See [`Docs::set_private_check`].
pub type PrivateCheck = Arc<dyn Fn(&Path) -> bool + Send + Sync>;

/// See [`Docs::joins`].
#[derive(Default)]
struct JoinCache {
    source: Option<(String, Arc<JoinSource>)>,
    lists: Vec<(u64, Arc<String>)>,
    /// The running preview, cancelled when a newer one starts.
    preview: Option<Arc<Progress>>,
}

/// Pasted lists kept at once.
const MAX_JOIN_LISTS: usize = 4;
/// LOOKUP tables kept at once.
const MAX_LOOKUP_TABLES: usize = 8;

/// The options that decide how the other table is indexed (cache key).
fn index_key(spec: &JoinSpec) -> String {
    serde_json::to_string(&(
        &spec.other_keys,
        &spec.columns,
        spec.other_header,
        spec.match_case,
        spec.trim,
        spec.loose,
        spec.key_match,
        spec.repeats,
        &spec.separator,
        spec.keys.len(),
    ))
    .unwrap_or_default()
}

/// Text fields of a join are bounded like other written text.
fn check_join_text(spec: &JoinSpec) -> Result<(), Error> {
    if spec.not_found.chars().count() > MAX_EDIT_CHARS
        || spec.prefix.chars().count() > MAX_EDIT_CHARS
    {
        return Err(Error::Invalid(
            "the not-found text and the name start can be at most 1,048,576 characters".into(),
        ));
    }
    Ok(())
}

fn line_break_name(b: LineBreak) -> &'static str {
    match b {
        LineBreak::CrLf => "CRLF",
        LineBreak::Lf => "LF",
        LineBreak::Cr => "CR",
    }
}

fn byte_label(b: u8) -> String {
    (b as char).to_string()
}

fn preview_of(d: &Document, dialect: Dialect, skip: u32) -> Vec<Vec<String>> {
    d.preview(dialect, skip, PREVIEW_ROWS, PREVIEW_COLS, PREVIEW_CHARS)
}

impl Docs {
    fn map(&self) -> MutexGuard<'_, HashMap<u32, Arc<Entry>>> {
        self.map.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn get(&self, id: u32) -> CmdResult<Arc<Entry>> {
        self.map()
            .get(&id)
            .cloned()
            .ok_or_else(|| CmdError::new("no_document", "that document is not open"))
    }

    /// Some document has unsaved changes. Never waits: a document whose
    /// lock is held (a save or another change is running) counts as
    /// modified, so closing the window asks instead of freezing.
    pub fn any_modified(&self) -> bool {
        let entries: Vec<_> = self.map().values().cloned().collect();
        entries.iter().any(|e| match e.doc.try_read() {
            Ok(d) => d.is_modified(),
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner().is_modified(),
            Err(std::sync::TryLockError::WouldBlock) => true,
        })
    }

    fn insert(&self, doc: Document, line_by_line: bool) -> CmdResult<(u32, Arc<Entry>)> {
        let mut map = self.map();
        if map.len() >= MAX_DOCS {
            return Err(CmdError::new(
                "too_many_documents",
                format!("at most {MAX_DOCS} files can be open at once"),
            ));
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let entry = Arc::new(Entry {
            doc: RwLock::new(doc),
            job: Mutex::new(None),
            replace: Mutex::new(None),
            cancel: AtomicBool::new(false),
            line_by_line: AtomicBool::new(line_by_line),
            error: Mutex::new(None),
            title: Mutex::new(None),
            locked: AtomicBool::new(false),
            checks: Mutex::new(None),
            totals: Mutex::new(None),
            typeahead: Mutex::new(None),
            writers: AtomicU32::new(0),
            closed: AtomicBool::new(false),
            changed_at: AtomicU64::new(0),
            autosaving: AtomicBool::new(false),
            private: AtomicBool::new(false),
            private_origin: AtomicBool::new(false),
            encrypted_from: Mutex::new(None),
            recipe: Mutex::new(RecipeLog::default()),
            matched: Mutex::new(None),
            updates: Mutex::new(None),
            notes: Mutex::new(None),
            too_large: AtomicU64::new(0),
            copy_cancelled: AtomicBool::new(false),
            pivots: Mutex::new(Vec::new()),
            waiting_since: AtomicU64::new(0),
            autosaved: AtomicU64::new(u64::MAX),
        });
        map.insert(id, entry.clone());
        Ok((id, entry))
    }

    /// Open a file the user chose in a native dialog. Detection runs now;
    /// indexing continues on a background thread.
    pub fn open_path(&self, path: &Path, line_by_line: bool) -> CmdResult<DocInfo> {
        self.open_with(path, line_by_line, None)
    }

    /// Open a file, with a layout the user chose (`None`: detect it). The
    /// encoding is detected as usual.
    pub fn open_with(
        &self,
        path: &Path,
        line_by_line: bool,
        layout: Option<&Layout>,
    ) -> CmdResult<DocInfo> {
        let mut opts = OpenOptions {
            line_by_line,
            ..Default::default()
        };
        if let Some(l) = layout {
            opts.dialect = Some(l.dialect()?);
            opts.skip_lines = Some(l.skip_lines);
        }
        let doc = Document::open(path, opts)?;
        // A file locked in another tab (reopened with another layout, or
        // opened again from Recent files or the Files panel) opens locked.
        let locked = self.is_locked_file(path);
        let (id, entry) = self.insert(doc, line_by_line)?;
        entry.locked.store(locked, Ordering::Relaxed);
        // Every open, reopen and revert goes through here.
        if self.is_private_path(path) {
            entry.private.store(true, Ordering::SeqCst);
        }
        let worker = entry.clone();
        std::thread::Builder::new()
            .name(format!("index-{id}"))
            .spawn(move || index_in_steps(&worker))
            .map_err(|e| CmdError::new("error", e.to_string()))?;
        self.info(id)
    }

    /// A new, empty document (one empty row) with no file yet.
    pub fn new_document(&self) -> CmdResult<DocInfo> {
        let doc = Document::from_bytes(b"\r\n".to_vec(), OpenOptions::default(), Some("csv"))?;
        let (id, _) = self.insert(doc, false)?;
        self.info(id)
    }

    pub fn close(&self, id: u32) {
        if let Some(e) = self.map().remove(&id) {
            e.cancel.store(true, Ordering::Relaxed);
            e.closed.store(true, Ordering::SeqCst);
        }
        // Let go of anything kept from this tab: its Join screen, indexes
        // of it kept by other tabs' Join screens, and LOOKUP tables of it.
        {
            let mut joins = self.joins.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(p) = joins.remove(&id).and_then(|j| j.preview) {
                p.cancel.store(true, Ordering::Relaxed);
            }
            let mine = format!("doc:{id}:");
            for j in joins.values_mut() {
                if j.source.as_ref().is_some_and(|(k, _)| k.starts_with(&mine)) {
                    j.source = None;
                }
            }
        }
        let mine = format!("{id}:");
        self.lookups
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(k, _)| !k.starts_with(&mine));
        crate::recovery::forget(id);
    }

    /// How to tell whether a path is in private file mode.
    pub fn set_private_check(&self, check: PrivateCheck) {
        *self.private_check.lock().unwrap_or_else(|p| p.into_inner()) = Some(check);
    }

    /// Whether `path` is in private file mode.
    pub fn is_private_path(&self, path: &Path) -> bool {
        let check = self
            .private_check
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        check.is_some_and(|c| c(path))
    }

    /// Whether a tab is private.
    pub fn is_private(&self, id: u32) -> CmdResult<bool> {
        Ok(self.get(id)?.private.load(Ordering::SeqCst))
    }

    /// A new tab made from `from` (or from files at `paths`) is private
    /// when any of them is.
    fn inherit_private(&self, id: u32, from: &[u32], paths: &[&Path]) {
        let private = from
            .iter()
            .any(|f| self.get(*f).is_ok_and(|e| e.private.load(Ordering::SeqCst)))
            || paths.iter().any(|p| self.is_private_path(p));
        if private {
            if let Ok(e) = self.get(id) {
                e.private.store(true, Ordering::SeqCst);
                e.private_origin.store(true, Ordering::SeqCst);
            }
        }
    }

    /// Mark a tab private when the file it came from (an import, an earlier
    /// save) is.
    pub fn private_if_from(&self, id: u32, path: &Path) {
        self.inherit_private(id, &[], &[path]);
    }

    /// Private file mode changed: every open tab of a file follows the
    /// list. Kept earlier saves and recovery copies of other runs are not
    /// touched here (the user is asked first).
    pub fn private_changed(&self) {
        let entries: Vec<(u32, Arc<Entry>)> =
            self.map().iter().map(|(k, v)| (*k, v.clone())).collect();
        for (id, e) in entries {
            let Some(p) = e.read().path().map(Path::to_path_buf) else {
                continue;
            };
            // Data from a private origin keeps the tab private whatever
            // happens to the list (only the user turns that off).
            let private = self.is_private_path(&p) || e.private_origin.load(Ordering::SeqCst);
            e.private.store(private, Ordering::SeqCst);
            if private {
                crate::recovery::forget(id);
            }
        }
    }

    /// Private file mode on or off for a tab: no recovery copies and no
    /// earlier saves while on; turning it on deletes the tab's recovery copy.
    pub fn set_private(&self, id: u32, on: bool) -> CmdResult<()> {
        let entry = self.get(id)?;
        entry.private.store(on, Ordering::SeqCst);
        if !on {
            // The user turned it off for this tab.
            entry.private_origin.store(false, Ordering::SeqCst);
        }
        if on {
            crate::recovery::forget(id);
        }
        Ok(())
    }

    /// What changed since the file was opened or last saved; at most
    /// `show` cell changes are sent to the screen (the counts are whole).
    pub fn change_list(
        &self,
        id: u32,
        show: usize,
    ) -> CmdResult<natendo_engine::changes::ChangeList> {
        let entry = self.get(id)?;
        let job = entry.start("Listing your changes")?;
        let d = entry.read();
        let mut l = d.change_list(&job.progress)?;
        l.cells
            .truncate(show.min(natendo_engine::changes::MAX_LISTED));
        Ok(l)
    }

    /// Cells on screen that differ from the file.
    pub fn edited_cells(
        &self,
        id: u32,
        row: u64,
        rows: u32,
        col: u32,
        cols: u32,
    ) -> CmdResult<Vec<(u64, u32)>> {
        if u64::from(rows) * u64::from(cols) > MAX_VIEWPORT_CELLS {
            return Err(CmdError::new(
                "too_large",
                "too many cells requested at once",
            ));
        }
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.edited_cells(row..row + u64::from(rows), col..col.saturating_add(cols))?)
    }

    /// Write the list of changes as a CSV report to `path`.
    pub fn save_change_report(&self, id: u32, path: &Path) -> CmdResult<u64> {
        self.check_writable(path)?;
        let entry = self.get(id)?;
        let job = entry.start("Writing the list of changes")?;
        let d = entry.read();
        if d.path()
            .is_some_and(|p| natendo_engine::save::same_path(p, path))
        {
            return Err(Error::Invalid("choose another name: that is the open file".into()).into());
        }
        let titles: Vec<String> = (0..d.col_count())
            .map(|c| {
                d.cell(0, c)
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| format!("Column {}", c + 1))
            })
            .collect();
        let list = d.save_change_report(path, &titles, &job.progress)?;
        Ok(list.cells_total + list.added_total + list.deleted_total)
    }

    /// The first few labels or letters as text.
    pub fn mail_preview(
        &self,
        id: u32,
        spec: &natendo_engine::mail::MailSpec,
    ) -> CmdResult<Vec<Vec<String>>> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.mail_preview(spec, 6)?)
    }

    /// Write labels or letters as a PDF to `path`.
    pub fn save_mail(
        &self,
        id: u32,
        path: &Path,
        spec: &natendo_engine::mail::MailSpec,
    ) -> CmdResult<natendo_engine::mail::MailReport> {
        self.check_writable(path)?;
        let entry = self.get(id)?;
        let job = entry.start(match spec.kind {
            natendo_engine::mail::MailKind::Labels => "Making labels",
            natendo_engine::mail::MailKind::Letters => "Making letters",
        })?;
        let d = entry.read();
        Ok(d.save_mail(path, spec, &job.progress)?)
    }

    /// Columns that look like they hold private data (first 20,000 rows).
    pub fn find_private(
        &self,
        id: u32,
        header: bool,
        read: ReadOptions,
        today: f64,
    ) -> CmdResult<Vec<natendo_engine::privacy::Finding>> {
        let entry = self.get(id)?;
        let job = entry.start("Looking for private data")?;
        let d = entry.read();
        Ok(d.find_private(header, read, today, &job.progress)?)
    }

    /// Write a share-safe copy (columns dropped or masked) to `path`; the
    /// open document does not change.
    pub fn save_safe_copy(
        &self,
        id: u32,
        path: &Path,
        spec: &natendo_engine::privacy::SafeCopy,
        now: f64,
    ) -> CmdResult<natendo_engine::privacy::SafeCopyReport> {
        if spec.columns.len() > natendo_engine::document::MAX_MAPPED_COLS as usize {
            return Err(Error::Invalid("too many columns".into()).into());
        }
        for c in &spec.columns {
            if let natendo_engine::privacy::ColumnAction::Clean { cleaner } = c {
                cleaner.check()?;
            }
        }
        self.check_writable(path)?;
        let entry = self.get(id)?;
        let job = entry.start("Writing a share-safe copy")?;
        let d = entry.read();
        Ok(d.save_safe_copy(path, spec, now, &job.progress)?)
    }

    /// Keep a recovery copy of a document's unsaved changes (see
    /// `recovery`). Never waits for the document: a busy one is skipped.
    pub fn autosave(&self, id: u32) -> CmdResult<Autosaved> {
        let entry = self.get(id)?;
        if entry.jobs().is_some() || entry.private.load(Ordering::SeqCst) {
            return Ok(Autosaved::Skipped);
        }
        // One copy at a time per document.
        if entry.autosaving.swap(true, Ordering::SeqCst) {
            return Ok(Autosaved::Skipped);
        }
        let _running = ClearOnDrop(&entry.autosaving);
        let title = entry
            .title
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let d = match entry.doc.try_read() {
            Ok(d) => d,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Ok(Autosaved::Skipped),
        };
        if entry.change_waiting() {
            return Ok(Autosaved::Skipped);
        }
        if !d.is_indexed() || !d.is_modified() {
            return Ok(Autosaved::Unchanged);
        }
        let rev = d.revision();
        if entry.autosaved.load(Ordering::Relaxed) == rev {
            return Ok(Autosaved::Unchanged);
        }
        // Too large last time: not written again until it is clearly
        // smaller (a twentieth fewer cells), so a big file is not copied
        // and thrown away every few minutes.
        let size = d
            .total_rows()
            .saturating_mul(u64::from(d.col_count().max(1)))
            .max(1);
        let was = entry.too_large.load(Ordering::Relaxed);
        if was != 0 && size.saturating_mul(20) > was.saturating_mul(19) {
            return Ok(Autosaved::TooLarge);
        }
        // A big document still being edited: wait for a pause, but not
        // for ever.
        let big = d.source_bytes() > AUTOSAVE_BIG_BYTES
            || size > AUTOSAVE_BIG_CELLS
            || entry.copy_cancelled.load(Ordering::Relaxed);
        if entry.put_off_copy(big, now_millis()) {
            return Ok(Autosaved::Skipped);
        }
        let name = d
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .or(title)
            .unwrap_or_else(|| "Untitled".into());
        let r = crate::recovery::write_copy(&d, id, &name, &|| entry.change_waiting())?;
        match r {
            Autosaved::Written { .. } => {
                entry.autosaved.store(rev, Ordering::Relaxed);
                entry.too_large.store(0, Ordering::Relaxed);
                entry.copy_cancelled.store(false, Ordering::Relaxed);
                entry.waiting_since.store(0, Ordering::Relaxed);
            }
            Autosaved::TooLarge => entry.too_large.store(size, Ordering::Relaxed),
            Autosaved::Skipped => {
                // Stopped by a change: wait again (for a pause, or at most
                // another 5 minutes) instead of retrying every tick.
                entry.copy_cancelled.store(true, Ordering::Relaxed);
                entry.waiting_since.store(now_millis(), Ordering::Relaxed);
            }
            _ => {}
        }
        Ok(r)
    }

    /// Open a recovery copy left by an earlier run as a new unsaved tab.
    pub fn recovery_open(&self, index: u32) -> CmdResult<DocInfo> {
        let (mut doc, title) = crate::recovery::open(index)?;
        doc.mark_modified();
        let info = self.insert_titled(doc, Some(title))?;
        crate::recovery::adopt(index, info.id);
        Ok(info)
    }

    /// Earlier saves of a document's file.
    pub fn versions(&self, id: u32) -> CmdResult<Vec<crate::recovery::VersionItem>> {
        let entry = self.get(id)?;
        let path = entry.read().path().map(Path::to_path_buf);
        match path {
            Some(p) => crate::recovery::versions(id, &p),
            None => Ok(Vec::new()),
        }
    }

    /// Open an earlier save of a document's file as a new tab, read with
    /// the document's layout. `label` (when it was saved, as the UI words
    /// it) goes into the tab's name.
    pub fn version_open(&self, id: u32, index: u32, label: &str) -> CmdResult<DocInfo> {
        let entry = self.get(id)?;
        let (layout, ext, name) = {
            let d = entry.read();
            let path = d
                .path()
                .ok_or_else(|| CmdError::new("no_path", "this file has no earlier saves"))?;
            (
                Layout::of(&d.format().dialect, d.preamble_lines()),
                path.extension().map(|e| e.to_string_lossy().into_owned()),
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )
        };
        let doc = crate::recovery::open_version(id, index, &layout, ext.as_deref())?;
        let label: String = label
            .chars()
            .filter(|c| !c.is_control())
            .take(100)
            .collect();
        let info = self.insert_titled(doc, Some(format!("{name} ({label})")))?;
        self.inherit_private(info.id, &[id], &[]);
        Ok(info)
    }

    /// The likely layouts of a document's file, each with a preview.
    pub fn layout_choices(&self, id: u32) -> CmdResult<LayoutChoices> {
        let entry = self.get(id)?;
        let d = entry.read();
        let ext = d
            .path()
            .and_then(|p| p.extension())
            .map(|e| e.to_string_lossy().into_owned());
        let found = d.detect_layout(ext.as_deref());
        let current = Layout::of(&d.format().dialect, d.preamble_lines());
        let choices = found
            .choices
            .iter()
            .map(|c| LayoutChoice {
                layout: Layout::of(&c.dialect, c.skip_lines),
                columns: c.columns,
                preview: preview_of(&d, c.dialect, c.skip_lines),
            })
            .collect();
        Ok(LayoutChoices {
            confidence: found.confidence,
            sep_line: found.sep_line,
            current,
            choices,
            preamble: d.preamble_text().chars().take(PREAMBLE_CHARS).collect(),
        })
    }

    /// The first rows of a document's file read with another layout.
    pub fn preview_layout(&self, id: u32, layout: &Layout) -> CmdResult<Vec<Vec<String>>> {
        let dialect = layout.dialect()?;
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(preview_of(&d, dialect, layout.skip_lines))
    }

    /// Open documents, in the order they were opened.
    pub fn ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.map().keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// The open document showing this file, if any.
    pub fn find_open(&self, path: &Path) -> Option<u32> {
        let entries: Vec<(u32, Arc<Entry>)> =
            self.map().iter().map(|(k, v)| (*k, v.clone())).collect();
        entries.into_iter().find_map(|(id, e)| {
            let d = e.doc.try_read().ok()?;
            d.path().filter(|p| same_file(p, path)).map(|_| id)
        })
    }

    /// Shown rows as text, at most `max_cells` cells and 16M characters in
    /// all (cells are cut to 32,768 characters).
    pub fn read_rows(
        &self,
        id: u32,
        row: u64,
        count: u64,
        max_cells: u64,
    ) -> CmdResult<Vec<Vec<String>>> {
        const CELL: usize = 32_768;
        const TOTAL: usize = 16 << 20;
        let entry = self.get(id)?;
        let d = entry.read();
        if !d.is_indexed() {
            return Err(Error::StillLoading.into());
        }
        let cols = u64::from(d.col_count().max(1));
        let count = count.min(max_cells / cols).max(1);
        let end = row.saturating_add(count).min(d.row_count());
        let mut out = Vec::new();
        let mut total = 0usize;
        for r in row..end {
            let mut vals = d.row_values(r)?;
            for v in vals.iter_mut() {
                if v.len() > CELL {
                    let mut cut = CELL;
                    while !v.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    v.truncate(cut);
                }
                total += v.len();
            }
            if total > TOTAL {
                break;
            }
            out.push(vals);
        }
        Ok(out)
    }

    pub fn path_of(&self, id: u32) -> CmdResult<Option<PathBuf>> {
        Ok(self.get(id)?.read().path().map(Path::to_path_buf))
    }

    pub fn info(&self, id: u32) -> CmdResult<DocInfo> {
        let entry = self.get(id)?;
        let error = entry
            .error
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let title = entry
            .title
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let d = entry.read();
        let mut info = describe(id, &d, entry.line_by_line.load(Ordering::Relaxed), error);
        info.read_only = entry.locked.load(Ordering::Relaxed);
        if let (None, Some(t)) = (&info.path, title) {
            info.name = t;
        }
        Ok(info)
    }

    /// Add a new document (no file yet) under a name, as a new tab.
    pub fn insert_titled(&self, doc: Document, title: Option<String>) -> CmdResult<DocInfo> {
        let (id, entry) = self.insert(doc, false)?;
        *entry.title.lock().unwrap_or_else(|p| p.into_inner()) = title;
        self.info(id)
    }

    /// Cells for display, as the engine's binary form.
    pub fn viewport(
        &self,
        id: u32,
        row: u64,
        rows: u32,
        col: u32,
        cols: u32,
        max_chars: u32,
    ) -> CmdResult<Vec<u8>> {
        let cells = rows as u64 * cols as u64;
        if cells > MAX_VIEWPORT_CELLS {
            return Err(CmdError::new(
                "too_large",
                "too many cells requested at once",
            ));
        }
        let per_cell = (VIEWPORT_CHAR_BUDGET / cells.max(1) as usize).max(16);
        let max_chars = (max_chars as usize).clamp(1, DISPLAY_CHARS).min(per_cell);
        let entry = self.get(id)?;
        let d = entry.read();
        let v = d.viewport(
            row..row.saturating_add(rows as u64),
            col..col.saturating_add(cols),
            max_chars,
        )?;
        Ok(v.to_bytes())
    }

    /// One cell's value for the editor, up to [`MAX_EDIT_CHARS`]. A longer
    /// cell comes back cut with `truncated` set and must not be edited.
    pub fn cell_text(&self, id: u32, row: u64, col: u32) -> CmdResult<(String, bool)> {
        let entry = self.get(id)?;
        let d = entry.read();
        if row >= d.row_count() {
            return Err(Error::OutOfRange.into());
        }
        let mut v = d.viewport(row..row + 1, col..col + 1, MAX_EDIT_CHARS)?;
        let truncated = v.truncated.first().copied().unwrap_or(false);
        Ok((v.cells.pop().unwrap_or_default(), truncated))
    }

    /// Set cells, as one undo step.
    pub fn set_cells(&self, id: u32, cells: Vec<(u64, u32, String)>) -> CmdResult<DocInfo> {
        check_write(cells.len() as u64, cells.iter().map(|c| c.2.len()))?;
        let widest = cells.iter().map(|c| c.1 as u64 + 1).max().unwrap_or(0);
        self.edit(id, |d| {
            check_width(d, widest)?;
            d.set_cells(cells)
        })
    }

    /// Set every cell of a rectangle to one value (clear, fill), one undo step.
    pub fn fill(
        &self,
        id: u32,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
        value: String,
    ) -> CmdResult<DocInfo> {
        let n = rows.saturating_mul(cols as u64);
        check_write(n, std::iter::once(value.len()))?;
        if (value.len() as u64).saturating_mul(n) > MAX_WRITE_CHARS as u64 {
            return Err(too_much());
        }
        self.edit(id, |d| {
            let r1 = row.checked_add(rows).ok_or(Error::OutOfRange)?;
            let c1 = col.checked_add(cols).ok_or(Error::OutOfRange)?;
            if rows > 0 {
                check_width(d, c1 as u64)?;
            }
            let mut cells = Vec::with_capacity(n as usize);
            for r in row..r1 {
                for c in col..c1 {
                    cells.push((r, c, value.clone()));
                }
            }
            d.set_cells(cells)
        })
    }

    /// Selected cells as tab-separated text for the clipboard. Values with a
    /// tab, quote or line break are quoted, so pasting round-trips.
    pub fn copy(&self, id: u32, row: u64, rows: u64, col: u32, cols: u32) -> CmdResult<String> {
        let n = rows.saturating_mul(cols as u64);
        if n > MAX_COPY_CELLS {
            return Err(CmdError::new(
                "too_large",
                format!(
                    "copying is limited to {} cells at once",
                    group(MAX_COPY_CELLS)
                ),
            ));
        }
        let entry = self.get(id)?;
        let d = entry.read();
        let r1 = row.saturating_add(rows).min(d.row_count());
        let mut out = String::new();
        // Read in small bands, each with a character budget shared by its
        // cells, so one step never holds more than about 64 MB however long
        // the cells are. A cell over its share is refused, not cut.
        let band = (COPY_BAND_CELLS / cols.max(1) as u64).max(1);
        let band_cells = (band * cols.max(1) as u64).min(n.max(1)) as usize;
        let per_cell = (COPY_BAND_CHARS / band_cells).clamp(1, MAX_EDIT_CHARS);
        if cols > natendo_engine::viewport::MAX_CELLS as u32 {
            return Err(CmdError::new(
                "too_large",
                "too many columns to copy at once",
            ));
        }
        let mut r = row;
        while r < r1 {
            let end = (r + band).min(r1);
            let v = d.viewport(r..end, col..col.saturating_add(cols), per_cell)?;
            for i in 0..v.rows {
                for c in 0..v.cols {
                    let k = (i * v.cols + c) as usize;
                    if c > 0 {
                        out.push('\t');
                    }
                    if v.truncated[k] {
                        // Over its share of the band: read this cell alone.
                        let (rr, cc) = (r + i as u64, col + c);
                        let one = d.viewport(rr..rr + 1, cc..cc + 1, MAX_EDIT_CHARS)?;
                        if one.truncated.first().copied().unwrap_or(true) {
                            return Err(CmdError::new(
                                "too_large",
                                "a cell is too long to copy as text",
                            ));
                        }
                        push_tsv(&one.cells[0], &mut out);
                    } else {
                        push_tsv(&v.cells[k], &mut out);
                    }
                    if out.len() > MAX_COPY_BYTES {
                        return Err(CmdError::new(
                            "too_large",
                            "the selection is too large to copy",
                        ));
                    }
                }
                out.push_str("\r\n");
                if out.len() > MAX_COPY_BYTES {
                    return Err(CmdError::new(
                        "too_large",
                        "the selection is too large to copy",
                    ));
                }
            }
            r = end;
        }
        Ok(out)
    }

    /// Move the cells of a rectangle so its top-left cell lands on
    /// (`to_row`, `to_col`), as one undo step (dragging a selection by its
    /// border). Cells under the new place are overwritten; cells left behind
    /// are cleared, unless `copy` (Ctrl held: copy instead of move). The
    /// whole new place must be inside the table.
    #[allow(clippy::too_many_arguments)]
    pub fn move_cells(
        &self,
        id: u32,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
        to_row: u64,
        to_col: u32,
        copy: bool,
    ) -> CmdResult<DocInfo> {
        let n = rows.saturating_mul(cols as u64);
        // Every moved cell is written, plus (moving) up to as many cleared.
        if n.saturating_mul(if copy { 1 } else { 2 }) > MAX_WRITE_CELLS {
            return Err(too_much());
        }
        if cols > natendo_engine::viewport::MAX_CELLS as u32 {
            return Err(too_much());
        }
        if n == 0 || (row == to_row && col == to_col) {
            return self.info(id);
        }
        let mut too_long = false;
        let done = self.edit(id, |d| {
            let (nr, nc) = (d.row_count(), d.col_count() as u64);
            let fits = |r: u64, c: u32| {
                r.checked_add(rows).is_some_and(|e| e <= nr)
                    && (c as u64).checked_add(cols as u64).is_some_and(|e| e <= nc)
            };
            if !fits(row, col) || !fits(to_row, to_col) {
                return Err(Error::OutOfRange);
            }
            // Read in small bands, each with a character budget shared by
            // its cells (as copy does), so one band never holds more than
            // about COPY_BAND_CHARS; a cell over its share is read on its
            // own. All values together stay under MAX_WRITE_CHARS bytes.
            let band = (COPY_BAND_CELLS / cols.max(1) as u64).max(1);
            let band_cells = (band * cols.max(1) as u64).min(n) as usize;
            let per_cell = (COPY_BAND_CHARS / band_cells.max(1)).clamp(1, MAX_EDIT_CHARS);
            let mut values: Vec<String> = Vec::with_capacity(n as usize);
            let mut bytes = 0usize;
            let mut r = row;
            while r < row + rows {
                let end = (r + band).min(row + rows);
                let v = d.viewport(r..end, col..col + cols, per_cell)?;
                for (k, cell) in v.cells.into_iter().enumerate() {
                    let cell = if v.truncated[k] {
                        let w = v.cols.max(1) as u64;
                        let (rr, cc) = (r + k as u64 / w, col + (k as u64 % w) as u32);
                        let mut one = d.viewport(rr..rr + 1, cc..cc + 1, MAX_EDIT_CHARS)?;
                        if one.truncated.first().copied().unwrap_or(true) {
                            too_long = true;
                            return Err(Error::OutOfRange);
                        }
                        one.cells.pop().unwrap_or_default()
                    } else {
                        cell
                    };
                    bytes += cell.len();
                    if bytes > MAX_WRITE_CHARS {
                        too_long = true;
                        return Err(Error::OutOfRange);
                    }
                    values.push(cell);
                }
                r = end;
            }
            let inside = |r: u64, c: u32| {
                r >= to_row && r < to_row + rows && c >= to_col && c < to_col + cols
            };
            let mut cells = Vec::with_capacity((n as usize).saturating_mul(2));
            for i in 0..rows {
                for j in 0..cols {
                    if !copy && !inside(row + i, col + j) {
                        cells.push((row + i, col + j, String::new()));
                    }
                }
            }
            let mut it = values.into_iter();
            for i in 0..rows {
                for j in 0..cols {
                    cells.push((to_row + i, to_col + j, it.next().unwrap_or_default()));
                }
            }
            d.set_cells(cells)
        });
        if too_long {
            return Err(CmdError::new(
                "too_large",
                "the cells hold too much text to move at once",
            ));
        }
        done
    }

    pub fn insert_rows(&self, id: u32, at: u64, count: u64) -> CmdResult<DocInfo> {
        if count > MAX_INSERT_ROWS {
            return Err(CmdError::new(
                "too_large",
                format!(
                    "at most {} rows can be inserted at once",
                    group(MAX_INSERT_ROWS)
                ),
            ));
        }
        self.edit(id, |d| {
            d.insert_rows(at, vec![vec![String::new()]; count as usize])
        })
    }

    pub fn delete_rows(&self, id: u32, at: u64, count: u64) -> CmdResult<DocInfo> {
        self.edit(id, |d| d.delete_rows(at, count))
    }

    /// Hide shown rows (one undo step; hidden rows are still saved).
    pub fn hide_rows(&self, id: u32, at: u64, count: u64) -> CmdResult<DocInfo> {
        self.edit(id, |d| d.hide_rows(at, count).map(drop))
    }

    /// Show hidden rows again (one undo step).
    pub fn show_rows(&self, id: u32, which: ShowRows) -> CmdResult<DocInfo> {
        self.edit(id, |d| d.show_rows(which).map(drop))
    }

    /// Put shown rows in a random order (one undo step).
    pub fn shuffle_rows(&self, id: u32, at: u64, count: u64) -> CmdResult<DocInfo> {
        let seed = run_seed();
        self.compute_apply(
            id,
            "Shuffling",
            |d, p| d.plan_shuffle(at, count, seed, p),
            |d, plan| d.apply_shuffle(plan),
        )
    }

    /// Likely clean-ups, each with its fix (a job: progress and Cancel).
    pub fn suggestions(
        &self,
        id: u32,
        skip_first: bool,
        read: ReadOptions,
    ) -> CmdResult<natendo_engine::suggest::Suggestions> {
        let entry = self.get(id)?;
        let job = entry.start("Looking for clean-ups")?;
        let d = entry.read();
        Ok(d.suggestions(skip_first, read, &job.progress)?)
    }

    /// Quick totals of a selection. A newer request (or any change to the
    /// document) cancels this one, which then fails with kind `cancelled`.
    #[allow(clippy::too_many_arguments)]
    pub fn selection_totals(
        &self,
        id: u32,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
        read: ReadOptions,
        distinct: bool,
    ) -> CmdResult<TotalsOut> {
        let entry = self.get(id)?;
        let progress = Arc::new(Progress::new());
        {
            let mut slot = entry.totals.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(old) = slot.replace(progress.clone()) {
                old.cancel.store(true, Ordering::Relaxed);
            }
        }
        // Never queue behind a change: if the document is busy being
        // changed, the UI asks again after the change.
        let d = match entry.doc.try_read() {
            Ok(d) => d,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(CmdError::new("busy", "the file is being changed"))
            }
        };
        // A change that asked for the document just before this read got
        // it: give way now (the change also cancels `progress` once it is
        // announced, so one that comes later stops the work below).
        if entry.change_waiting() {
            drop(d);
            return Err(CmdError::new("busy", "the file is being changed"));
        }
        let r = d
            .selection_totals(row, rows, col, cols, read, &progress)
            .and_then(|t| {
                let mut t = TotalsOut::from(t);
                if distinct {
                    let (n, exact) = d.selection_distinct(row, rows, col, cols, &progress)?;
                    t.distinct = Some(n);
                    t.distinct_exact = exact;
                }
                Ok(t)
            });
        drop(d);
        let mut slot = entry.totals.lock().unwrap_or_else(|p| p.into_inner());
        if slot.as_ref().is_some_and(|p| Arc::ptr_eq(p, &progress)) {
            *slot = None;
        }
        Ok(r?)
    }

    pub fn move_rows(&self, id: u32, at: u64, count: u64, to: u64) -> CmdResult<DocInfo> {
        self.edit(id, |d| d.move_rows(at, count, to))
    }

    pub fn insert_cols(&self, id: u32, at: u32, count: u32) -> CmdResult<DocInfo> {
        self.recorded(id, Logged::Step(RecipeOp::InsertCols { at, count }), || {
            self.insert_cols_now(id, at, count)
        })
    }

    fn insert_cols_now(&self, id: u32, at: u32, count: u32) -> CmdResult<DocInfo> {
        if count > MAX_INSERT_COLS {
            return Err(CmdError::new(
                "too_large",
                format!(
                    "at most {} columns can be inserted at once",
                    group(MAX_INSERT_COLS as u64)
                ),
            ));
        }
        self.edit(id, |d| {
            if !within_size(d.col_count() as u64 + count as u64, d.total_rows()) {
                return Err(Error::OutOfMemory(
                    "inserting these columns would make the file too large to save",
                ));
            }
            d.insert_cols(at, count)
        })
    }

    pub fn delete_cols(&self, id: u32, at: u32, count: u32) -> CmdResult<DocInfo> {
        self.recorded(id, Logged::Step(RecipeOp::DeleteCols { at, count }), || {
            self.edit(id, |d| d.delete_cols(at, count))
        })
    }

    pub fn move_cols(&self, id: u32, at: u32, count: u32, to: u32) -> CmdResult<DocInfo> {
        self.recorded(
            id,
            Logged::Step(RecipeOp::MoveCols { at, count, to }),
            || self.edit(id, |d| d.move_cols(at, count, to)),
        )
    }

    pub fn undo(&self, id: u32) -> CmdResult<DocInfo> {
        self.edit(id, |d| {
            d.undo();
            Ok(())
        })
    }

    pub fn redo(&self, id: u32) -> CmdResult<DocInfo> {
        self.edit(id, |d| {
            d.redo();
            Ok(())
        })
    }

    /// Save to the open file, or to `path` (from a Save As dialog).
    pub fn save(
        &self,
        id: u32,
        path: Option<PathBuf>,
        overwrite_changed: bool,
        allow_lossy: bool,
        options: Option<&SaveAs>,
    ) -> CmdResult<DocInfo> {
        let entry = self.get(id)?;
        // A save over a locked file (this tab's or another's, however the
        // path is written) is refused before this document is locked for
        // writing.
        match path.as_deref() {
            None if entry.locked.load(Ordering::Relaxed) => return Err(read_only()),
            Some(p) => self.check_writable(p)?,
            None => {}
        }
        {
            let mut d = entry.write();
            if entry.locked.load(Ordering::Relaxed)
                && path
                    .as_deref()
                    .is_none_or(|p| d.path().is_some_and(|own| same_file(p, own)))
            {
                return Err(read_only());
            }
            let (format, convert_line_breaks, quoting) = match options {
                Some(o) => {
                    let (f, c, q) = o.format()?;
                    (Some(f), c, q)
                }
                None => (None, false, Quoting::Keep),
            };
            // The file as it is now becomes an earlier save (small files).
            // Not in unit tests: the versions folder is shared by the whole
            // test run, and recovery's own test counts what is in it.
            #[cfg(not(test))]
            if let Some(target) = path
                .as_deref()
                .or(d.path())
                .filter(|_| !entry.private.load(Ordering::SeqCst))
            {
                crate::recovery::keep_version(target);
            }
            d.save(SaveOptions {
                path,
                overwrite_changed,
                allow_lossy,
                format,
                convert_line_breaks,
                quoting,
                ..Default::default()
            })?;
        }
        // The saved copy was reopened normally (quotes may span lines).
        entry.line_by_line.store(false, Ordering::Relaxed);
        // Nothing unsaved: the recovery copy goes.
        crate::recovery::forget(id);
        entry.autosaved.store(u64::MAX, Ordering::Relaxed);
        entry.too_large.store(0, Ordering::Relaxed);
        self.info(id)
    }

    /// The running long operation of a document, if any.
    pub fn job(&self, id: u32) -> CmdResult<Option<JobInfo>> {
        let entry = self.get(id)?;
        let j = entry.jobs();
        Ok(j.as_ref().map(|j| JobInfo {
            label: j.label,
            fraction: j.progress.fraction(),
        }))
    }

    /// Ask the running long operation to stop (nothing is changed).
    pub fn cancel_job(&self, id: u32) -> CmdResult<()> {
        let entry = self.get(id)?;
        if let Some(j) = entry.jobs().as_ref() {
            j.progress.cancel.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Compute under the read lock, then apply under the write lock. An
    /// edit made while computing (typing goes on meanwhile) makes the plan
    /// stale; it is then computed again, a few times, before giving up with
    /// a "busy" message. The job slot is held until the apply ends.
    fn compute_apply<P>(
        &self,
        id: u32,
        label: &'static str,
        plan: impl Fn(&Document, &Progress) -> Result<P, Error>,
        apply: impl Fn(&mut Document, P) -> Result<(), Error>,
    ) -> CmdResult<DocInfo> {
        let entry = self.get(id)?;
        let changes = label != "Filtering";
        if changes && entry.locked.load(Ordering::Relaxed) {
            return Err(read_only());
        }
        let job = entry.start(label)?;
        for _ in 0..COMPUTE_ATTEMPTS {
            let p = plan(&entry.read(), &job.progress)?;
            match self.edit_as(id, changes, |d| apply(d, p)) {
                Err(e) if e.kind == "changed" => continue,
                other => return other,
            }
        }
        Err(CmdError::new(
            "busy",
            "the file kept changing while this ran; try again",
        ))
    }

    /// Sort every row (one undo step).
    pub fn sort(&self, id: u32, spec: SortSpec) -> CmdResult<DocInfo> {
        self.recorded(
            id,
            Logged::Step(RecipeOp::Sort { spec: spec.clone() }),
            || {
                self.compute_apply(
                    id,
                    "Sorting",
                    |d, p| d.plan_sort(&spec, p),
                    |d, plan| d.apply_sort(plan),
                )
            },
        )
    }

    /// Turn a filter on (a spec without rules turns it off).
    pub fn filter(&self, id: u32, spec: FilterSpec) -> CmdResult<DocInfo> {
        check_filter(&spec)?;
        self.recorded(
            id,
            Logged::Step(RecipeOp::Filter { spec: spec.clone() }),
            || {
                self.compute_apply(
                    id,
                    "Filtering",
                    |d, p| d.plan_filter(&spec, p),
                    |d, plan| d.apply_filter(plan),
                )
            },
        )
    }

    pub fn clear_filter(&self, id: u32) -> CmdResult<DocInfo> {
        self.recorded(id, Logged::Step(RecipeOp::ClearFilter), || {
            self.edit_as(id, false, |d| {
                d.clear_filter();
                Ok(())
            })
        })
    }

    /// The filter's rules (logical columns), or `None` without a filter.
    pub fn filter_spec(&self, id: u32) -> CmdResult<Option<FilterSpec>> {
        Ok(self.get(id)?.read().filter_spec())
    }

    /// A column's values for its header menu.
    pub fn filter_values(
        &self,
        id: u32,
        col: u32,
        spec: FilterSpec,
        search: Option<String>,
    ) -> CmdResult<ValueList> {
        check_filter(&spec)?;
        if search
            .as_ref()
            .is_some_and(|s| s.chars().count() > MAX_SEARCH_CHARS)
        {
            return Err(CmdError::new("too_large", "the search text is too long"));
        }
        let entry = self.get(id)?;
        let job = entry.start("Listing values")?;
        let d = entry.read();
        Ok(d.filter_values(col, &spec, search.as_deref(), &job.progress)?)
    }

    pub fn find_next(
        &self,
        id: u32,
        spec: FindSpec,
        from: Option<(u64, u32)>,
        backward: bool,
    ) -> CmdResult<Option<Found>> {
        let entry = self.get(id)?;
        let job = entry.start("Searching")?;
        let d = entry.read();
        Ok(d.find_next(&spec, from, backward, &job.progress)?
            .map(|((row, col), wrapped)| Found { row, col, wrapped }))
    }

    pub fn find_all(&self, id: u32, spec: FindSpec) -> CmdResult<FindAll> {
        let entry = self.get(id)?;
        let job = entry.start("Searching")?;
        let d = entry.read();
        Ok(d.find_all(&spec, MAX_FIND_HITS, &job.progress)?)
    }

    /// What "Replace all" would change; the plan is kept for it.
    pub fn replace_preview(
        &self,
        id: u32,
        spec: FindSpec,
        replacement: String,
    ) -> CmdResult<ReplaceReport> {
        let entry = self.get(id)?;
        let key = replace_key(&spec, &replacement);
        let job = entry.start("Looking for matches")?;
        let plan =
            entry
                .read()
                .plan_replace(&spec, &replacement, PREVIEW_CHANGES, &job.progress)?;
        drop(job);
        let report = ReplaceReport {
            cells: plan.cells,
            hits: plan.hits,
            preview: plan.preview.clone(),
            info: None,
        };
        *entry.replace.lock().unwrap_or_else(|p| p.into_inner()) = Some((key, plan));
        Ok(report)
    }

    /// Replace every match, as one undo step. Reuses the previewed plan
    /// when it is for the same request and the document did not change.
    pub fn replace_all(
        &self,
        id: u32,
        spec: FindSpec,
        replacement: String,
    ) -> CmdResult<ReplaceReport> {
        let step = RecipeOp::ReplaceAll {
            spec: spec.clone(),
            with: replacement.clone(),
        };
        self.recorded(id, Logged::Step(step), || {
            self.replace_all_now(id, spec, replacement)
        })
    }

    fn replace_all_now(
        &self,
        id: u32,
        spec: FindSpec,
        replacement: String,
    ) -> CmdResult<ReplaceReport> {
        let entry = self.get(id)?;
        let key = replace_key(&spec, &replacement);
        let kept = entry
            .replace
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .filter(|(k, _)| *k == key)
            .map(|(_, plan)| plan);
        let attempt = |plan: ReplacePlan| -> CmdResult<ReplaceReport> {
            let (cells, hits) = (plan.cells, plan.hits);
            let info = self.edit(id, |d| d.apply_replace(plan))?;
            Ok(ReplaceReport {
                cells,
                hits,
                preview: Vec::new(),
                info: Some(info),
            })
        };
        if let Some(plan) = kept {
            match attempt(plan) {
                Err(e) if e.kind == "changed" => {}
                other => return other,
            }
        }
        let job = entry.start("Replacing")?;
        let plan = entry
            .read()
            .plan_replace(&spec, &replacement, 0, &job.progress)?;
        drop(job);
        attempt(plan)
    }

    /// Run a data operation as one undo step (or, for "show duplicates", a
    /// filter), computed under the read lock with progress and cancel.
    pub fn data_op(&self, id: u32, op: DataOp) -> CmdResult<OpReport> {
        let logged = recipe_step(&op);
        self.recorded(id, logged, || self.data_op_now(id, op))
    }

    // ---- matching people (Milestone 15) ----

    /// Find rows of tab `id` that look like the same person as rows of tab
    /// `other` (or of itself when `None`). `bring`: columns of the other
    /// tab to add for accepted pairs.
    pub fn match_find(
        &self,
        id: u32,
        other: Option<u32>,
        spec: &natendo_engine::fuzzy::MatchSpec,
        bring: &[u32],
    ) -> CmdResult<natendo_engine::fuzzy::MatchReport> {
        use natendo_engine::fuzzy::{find_matches, FieldKind, MatchSide};
        if spec.fields.is_empty() || spec.fields.len() > natendo_engine::fuzzy::MAX_FIELDS {
            return Err(CmdError::new("invalid", "choose the columns to compare"));
        }
        let kinds: Vec<FieldKind> = spec.fields.iter().map(|f| f.kind).collect();
        let here: Vec<u32> = spec.fields.iter().map(|f| f.col).collect();
        let there: Vec<u32> = spec.fields.iter().map(|f| f.other_col).collect();
        let entry = self.get(id)?;
        let job = entry.start("Matching")?;
        // The other list is copied under its own lock first (two tabs are
        // never locked together).
        let (other_side, bring_titles) = match other {
            Some(o) if o != id => {
                let e = self.get(o)?;
                let d = e.read();
                let side = MatchSide::build(
                    &d,
                    &there,
                    &kinds,
                    bring,
                    spec.other_header,
                    spec.read,
                    &job.progress,
                )?;
                (Some(side), d.titles(bring, spec.other_header))
            }
            _ => (None, Vec::new()),
        };
        let d = entry.read();
        let this = MatchSide::build(
            &d,
            &here,
            &kinds,
            &[],
            spec.keep_first,
            spec.read,
            &job.progress,
        )?;
        let revision = d.revision();
        drop(d);
        let rep = find_matches(&this, other_side.as_ref(), spec, &job.progress)?;
        *entry.matched.lock().unwrap_or_else(|p| p.into_inner()) = Some(MatchKept {
            other: other_side,
            other_tab: other.filter(|&o| o != id),
            bring_titles,
            keep_first: spec.keep_first,
            revision,
        });
        Ok(rep)
    }

    /// Add the columns for accepted pairs: (row id here, the other side's
    /// shown row, score). The tab must not have changed since the run.
    pub fn match_accept(&self, id: u32, accepted: &[(u32, u64, f32)]) -> CmdResult<DocInfo> {
        if accepted.is_empty() {
            return Err(CmdError::new("invalid", "accept at least one pair"));
        }
        if accepted.len() > natendo_engine::fuzzy::MAX_LISTED {
            return Err(CmdError::new("invalid", "too many pairs at once"));
        }
        let entry = self.get(id)?;
        let kept = entry
            .matched
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .ok_or_else(|| CmdError::new("invalid", "find the matches again"))?;
        if entry.read().revision() != kept.revision {
            return Err(CmdError::new(
                "changed",
                "the table changed since the matches were found; find them again",
            ));
        }
        let info = self.recorded(
            id,
            Logged::LeftOut("matching people with another list".into()),
            || {
                self.edit(id, |d| {
                    let plan = d.plan_match_columns(
                        accepted,
                        kept.other.as_ref(),
                        &kept.bring_titles,
                        kept.keep_first,
                    )?;
                    d.apply_cols(plan)
                })
            },
        )?;
        if let Some(o) = kept.other_tab {
            self.inherit_private(id, &[o], &[]);
        }
        Ok(info)
    }

    /// The match screen closed: let go of the other list's copy.
    pub fn match_close(&self, id: u32) {
        if let Ok(e) = self.get(id) {
            *e.matched.lock().unwrap_or_else(|p| p.into_inner()) = None;
        }
    }

    // ---- notes and flags (Milestone 15) ----

    /// Run `f` with tab `id`'s notes (read from beside the file the first
    /// time). Notes need a saved file.
    fn with_notes<T>(
        &self,
        id: u32,
        f: impl FnOnce(&Document, &mut natendo_engine::notes::NoteBook, &Path) -> CmdResult<T>,
    ) -> CmdResult<T> {
        use natendo_engine::notes::{notes_path, NoteBook};
        let entry = self.get(id)?;
        let path = self.path_of(id)?.ok_or_else(|| {
            CmdError::new("invalid", "save the file first: notes are kept beside it")
        })?;
        let sidecar = notes_path(&path);
        let mut slot = entry.notes.lock().unwrap_or_else(|p| p.into_inner());
        if slot.is_none() {
            *slot = Some(NoteBook::load(&sidecar)?.unwrap_or_default());
        }
        let d = entry.read();
        f(&d, slot.as_mut().expect("loaded"), &sidecar)
    }

    /// The notes settings (key columns and their titles), and whether any.
    pub fn notes_info(&self, id: u32) -> CmdResult<(Vec<u32>, bool, usize)> {
        self.with_notes(id, |_, b, _| {
            Ok((b.key_cols.clone(), b.header, b.notes.len()))
        })
    }

    /// Noted rows on screen.
    pub fn note_marks(
        &self,
        id: u32,
        row: u64,
        rows: u64,
    ) -> CmdResult<Vec<natendo_engine::notes::NoteMark>> {
        self.with_notes(id, |d, b, _| Ok(d.note_marks(b, row, rows)?))
    }

    /// Set (or clear, with flag 0 and no text) a row's note and flag; the
    /// notes file is written at once. Returns its path.
    pub fn note_set(&self, id: u32, row: u64, flag: u8, text: &str) -> CmdResult<PathBuf> {
        use natendo_engine::notes::{Note, FLAGS, MAX_NOTE_CHARS};
        if flag > FLAGS || text.chars().count() > MAX_NOTE_CHARS {
            return Err(CmdError::new(
                "invalid",
                "that note is too long or the flag is unknown",
            ));
        }
        self.with_notes(id, |d, b, sidecar| {
            let key = d.note_key(b, row)?;
            if key.is_empty() {
                return Err(CmdError::new(
                    "invalid",
                    "this row has nothing in the columns that identify rows, so it cannot have a note",
                ));
            }
            let before = b.notes.clone();
            if flag == 0 && text.trim().is_empty() {
                b.notes.remove(&key);
            } else {
                b.notes.insert(key, Note { flag, text: text.to_string() });
            }
            if let Err(e) = b.save(sidecar) {
                b.notes = before;
                return Err(e.into());
            }
            Ok(sidecar.to_path_buf())
        })
    }

    /// Choose the columns that identify rows (notes made with other key
    /// columns no longer match their rows; the app asks first).
    pub fn notes_set_keys(&self, id: u32, cols: Vec<u32>, header: bool) -> CmdResult<()> {
        self.with_notes(id, |d, b, sidecar| {
            if cols.is_empty() || cols.len() > 8 || cols.iter().any(|&c| c >= d.col_count()) {
                return Err(CmdError::new(
                    "invalid",
                    "choose 1 to 8 columns that identify a row",
                ));
            }
            b.key_titles = d.titles(&cols, header);
            b.key_cols = cols;
            b.header = header;
            b.save(sidecar)?;
            Ok(())
        })
    }

    /// Every noted row (first `limit`) and how many notes match no row.
    pub fn note_list(
        &self,
        id: u32,
        limit: usize,
    ) -> CmdResult<(Vec<natendo_engine::notes::NoteMark>, u64)> {
        let progress = Progress::new();
        self.with_notes(id, |d, b, _| {
            Ok(d.note_list(b, limit.min(100_000), &progress)?)
        })
    }

    /// Show only noted rows (`flag` 0: any; else that flag).
    pub fn notes_show(&self, id: u32, flag: u8) -> CmdResult<DocInfo> {
        let book = self.with_notes(id, |_, b, _| Ok(b.clone()))?;
        self.compute_apply(
            id,
            "Filtering",
            |d, p| d.plan_show_noted(&book, flag, p),
            |d, plan| d.apply_rows(plan),
        )
    }

    // ---- SQLite (Milestone 15) ----

    /// Open a table of a database file in a new tab.
    pub fn open_sqlite(&self, path: &Path, table: &str) -> CmdResult<DocInfo> {
        let progress = Progress::new();
        let doc = natendo_engine::sqlite::read_table(path, table, &progress)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let info = self.insert_titled(doc, Some(format!("{name}: {table}")))?;
        self.private_if_from(info.id, path);
        self.info(info.id)
    }

    /// Save tab `id` as a table of a database file.
    pub fn save_sqlite(
        &self,
        id: u32,
        path: &Path,
        table: &str,
        header: bool,
        replace: bool,
    ) -> CmdResult<u64> {
        self.check_writable(path)?;
        if self.find_open(path).is_some() {
            return Err(CmdError::new(
                "invalid",
                "choose another file: that one is open",
            ));
        }
        let entry = self.get(id)?;
        let job = entry.start("Saving to the database")?;
        let d = entry.read();
        Ok(d.save_sqlite(path, table, header, replace, &job.progress)?)
    }

    // ---- encrypted files (Milestone 15) ----

    /// Save tab `id`'s shown rows as an encrypted file.
    pub fn save_encrypted(&self, id: u32, path: &Path, password: &str) -> CmdResult<u64> {
        self.check_writable(path)?;
        if self.find_open(path).is_some() {
            return Err(CmdError::new(
                "invalid",
                "choose another name: that file is open",
            ));
        }
        let entry = self.get(id)?;
        let job = entry.start("Encrypting")?;
        let pw = natendo_engine::privacy::Secret::new(password);
        let n = entry.read().save_encrypted(path, &pw, &job.progress)?;
        // Without a plain file of its own, Save now goes to this file.
        if entry.read().path().is_none() {
            *entry
                .encrypted_from
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(path.to_path_buf());
        }
        Ok(n)
    }

    /// The encrypted file Save writes to for tab `id` (opened from or last
    /// saved to), if any.
    pub fn encrypted_from(&self, id: u32) -> CmdResult<Option<PathBuf>> {
        Ok(self
            .get(id)?
            .encrypted_from
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone())
    }

    /// Save tab `id` encrypted over the file it came from.
    pub fn save_encrypted_again(&self, id: u32, password: &str) -> CmdResult<u64> {
        let path = self.encrypted_from(id)?.ok_or_else(|| {
            CmdError::new("invalid", "this tab did not come from an encrypted file")
        })?;
        self.save_encrypted(id, &path, password)
    }

    /// Open an encrypted file into a new tab (decrypted in memory only;
    /// the tab is private: no recovery copies, never in the recent files).
    pub fn open_encrypted(&self, path: &Path, password: &str) -> CmdResult<DocInfo> {
        let pw = natendo_engine::privacy::Secret::new(password);
        let doc = Document::open_encrypted(path, &pw)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Encrypted file".into());
        let info = self.insert_titled(doc, Some(name))?;
        if let Ok(e) = self.get(info.id) {
            e.private.store(true, Ordering::SeqCst);
            e.private_origin.store(true, Ordering::SeqCst);
            *e.encrypted_from.lock().unwrap_or_else(|p| p.into_inner()) = Some(path.to_path_buf());
        }
        crate::recovery::forget(info.id);
        Ok(info)
    }

    // ---- changes from a newer file (Milestone 15) ----

    /// Compare tab `id` with a newer copy in tab `other`, rows paired by
    /// key columns; what differs is listed and kept for accepting.
    pub fn updates_find(
        &self,
        id: u32,
        other: u32,
        spec: &natendo_engine::updates::UpdateSpec,
    ) -> CmdResult<natendo_engine::updates::UpdateList> {
        use natendo_engine::updates::{NewerIndex, NewerValues};
        if other == id {
            return Err(CmdError::new(
                "invalid",
                "choose the newer file in another tab",
            ));
        }
        let entry = self.get(id)?;
        let job = entry.start("Comparing with the newer file")?;
        // One table locked at a time: the newer file indexed by hashes,
        // rows paired here, the differing rows' newer values copied, then
        // listed here. Either table changing in between is refused.
        let titles = {
            let d = entry.read();
            let all: Vec<u32> = (0..d.col_count()).collect();
            d.titles(&all, spec.keep_first)
        };
        let oe = self.get(other)?;
        let index = NewerIndex::build(&oe.read(), &titles, spec, &job.progress)?;
        let pairing = entry.read().pair_newer(&index, spec, &job.progress)?;
        let values =
            NewerValues::fetch(&oe.read(), &index, &pairing, &job.progress).map_err(|e| {
                if matches!(e, natendo_engine::Error::Changed) {
                    CmdError::new(
                        "changed",
                        "the newer file changed while comparing; compare again",
                    )
                } else {
                    e.into()
                }
            })?;
        let (updates, list) = entry
            .read()
            .diff_newer(&index, &pairing, values, &job.progress)?;
        *entry.updates.lock().unwrap_or_else(|p| p.into_inner()) = Some(UpdatesKept {
            updates,
            other_tab: other,
        });
        Ok(list)
    }

    /// Apply the accepted changes as one undo step.
    pub fn updates_accept(
        &self,
        id: u32,
        choice: &natendo_engine::updates::UpdateChoice,
    ) -> CmdResult<DocInfo> {
        for c in [&choice.cells, &choice.added, &choice.removed] {
            if c.picked.len() > 20_000_000 {
                return Err(CmdError::new("invalid", "too many changes picked"));
            }
        }
        let entry = self.get(id)?;
        let kept = entry
            .updates
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .ok_or_else(|| CmdError::new("invalid", "compare with the newer file again"))?;
        let r = self.recorded(
            id,
            Logged::LeftOut("changes taken from a newer file".into()),
            || {
                self.compute_apply(
                    id,
                    "Taking the changes",
                    |d, _| d.plan_updates(&kept.updates, choice),
                    |d, plan| d.apply_rows(plan),
                )
            },
        );
        let info = r.map_err(|e| {
            if e.kind == "changed" {
                CmdError::new(
                    "changed",
                    "the table changed since the comparison; compare again",
                )
            } else {
                e
            }
        })?;
        self.inherit_private(id, &[kept.other_tab], &[]);
        Ok(info)
    }

    pub fn updates_close(&self, id: u32) {
        if let Ok(e) = self.get(id) {
            *e.updates.lock().unwrap_or_else(|p| p.into_inner()) = None;
        }
    }

    // ---- street range lookup (Milestone 15) ----

    /// Add the street index's columns (tab `other`) to tab `id` for every
    /// address in one of its ranges.
    pub fn street_lookup(
        &self,
        id: u32,
        other: u32,
        cols: &natendo_engine::streets::IndexCols,
        spec: &natendo_engine::streets::StreetSpec,
    ) -> CmdResult<(DocInfo, natendo_engine::streets::StreetReport)> {
        use natendo_engine::streets::StreetIndex;
        if other == id {
            return Err(CmdError::new(
                "invalid",
                "choose the street index in another tab",
            ));
        }
        let entry = self.get(id)?;
        if entry.locked.load(Ordering::Relaxed) {
            return Err(read_only());
        }
        // The index is copied under its own tab's lock first.
        let index = {
            let job = entry.start("Reading the street index")?;
            let e = self.get(other)?;
            let d = e.read();
            StreetIndex::build(&d, cols, &job.progress)?
        };
        let report = std::cell::RefCell::new(natendo_engine::streets::StreetReport::default());
        let info = self.recorded(
            id,
            Logged::LeftOut("a street range lookup from another tab".into()),
            || {
                self.compute_apply(
                    id,
                    "Looking up streets",
                    |d, p| {
                        let (plan, rep) = d.plan_street_lookup(&index, spec, p)?;
                        *report.borrow_mut() = rep;
                        Ok(plan)
                    },
                    |d, plan| d.apply_cols(plan),
                )
            },
        )?;
        self.inherit_private(id, &[other], &[]);
        Ok((info, report.into_inner()))
    }

    // ---- recipes (Milestone 14) ----

    /// Run `f` (a change to tab `id`) and note it for the tab's recipe. A
    /// step that was undone is dropped when a new one is done.
    fn recorded<T>(
        &self,
        id: u32,
        logged: Logged,
        f: impl FnOnce() -> CmdResult<T>,
    ) -> CmdResult<T> {
        let entry = self.get(id)?;
        let first = {
            let d = entry.read();
            d.row_values(0).ok()
        };
        let out = f()?;
        let pos = entry.read().history_pos();
        let mut log = entry.recipe.lock().unwrap_or_else(|p| p.into_inner());
        // Steps past this point were undone and are now replaced.
        let undoable = !matches!(
            logged,
            Logged::Step(RecipeOp::Filter { .. } | RecipeOp::ClearFilter)
        );
        log.steps
            .retain(|(p, _)| if undoable { *p < pos } else { *p <= pos });
        if log.steps.is_empty() {
            log.first_row = first;
        }
        if log.steps.len() < natendo_engine::recipe::MAX_STEPS {
            log.steps.push((pos, logged));
        }
        Ok(out)
    }

    /// The tab's steps so far (those not undone) as a recipe.
    pub fn recipe_of(&self, id: u32, header: bool, name: &str) -> CmdResult<Recipe> {
        let entry = self.get(id)?;
        let pos = entry.read().history_pos();
        let log = entry.recipe.lock().unwrap_or_else(|p| p.into_inner());
        let mut steps = Vec::new();
        let mut left_out = Vec::new();
        for (p, l) in &log.steps {
            if *p > pos {
                continue;
            }
            match l {
                Logged::Step(s) => steps.push(s.clone()),
                Logged::LeftOut(w) => left_out.push(w.clone()),
            }
        }
        Ok(Recipe {
            version: natendo_engine::recipe::RECIPE_VERSION,
            name: name.chars().filter(|c| !c.is_control()).take(200).collect(),
            columns: if header {
                log.first_row.clone().unwrap_or_default()
            } else {
                Vec::new()
            },
            header,
            steps,
            left_out,
        })
    }

    /// How a tab's columns differ from a recipe's, in words.
    pub fn recipe_problems(&self, id: u32, recipe: &Recipe) -> CmdResult<Vec<String>> {
        Ok(recipe.column_problems(&self.get(id)?.read()))
    }

    /// Write a recipe file (never over a file open in a tab).
    pub fn save_recipe(&self, id: u32, recipe: &Recipe, path: &Path) -> CmdResult<()> {
        self.check_writable(path)?;
        let own = self.path_of(id)?;
        if self.find_open(path).is_some()
            || own.is_some_and(|p| natendo_engine::save::same_path(&p, path))
        {
            return Err(Error::Invalid("choose another name: that file is open".into()).into());
        }
        let bytes = recipe.to_json()?;
        std::fs::write(path, bytes)
            .map_err(|e| CmdError::new("io", format!("could not save the recipe: {e}")))
    }

    /// Run a recipe on tab `id`, each step as one undo step. Refused when
    /// the columns differ from the recipe's, unless `force`.
    pub fn run_recipe(
        &self,
        id: u32,
        recipe: &Recipe,
        now: f64,
        force: bool,
    ) -> CmdResult<DocInfo> {
        if recipe.steps.len() > natendo_engine::recipe::MAX_STEPS {
            return Err(Error::Invalid("the recipe has too many steps".into()).into());
        }
        for s in &recipe.steps {
            if let RecipeOp::Clean { spec } = s {
                check_clean(spec, now)?;
            }
            if let RecipeOp::Filter { spec } = s {
                check_filter(spec)?;
            }
        }
        if !force {
            let entry = self.get(id)?;
            let problems = recipe.column_problems(&entry.read());
            if !problems.is_empty() {
                return Err(CmdError::new(
                    "columns",
                    format!(
                        "this file's columns differ from the recipe's: {}",
                        problems.join("; ")
                    ),
                ));
            }
        }
        // Each step goes through the same command as when done by hand (so
        // it plans under the read lock, keeps the grid live, is noted for
        // the tab's own recipe and is one undo step).
        for (i, s) in recipe.steps.iter().enumerate() {
            let r = match s.clone() {
                RecipeOp::Sort { spec } => self.sort(id, spec).map(|_| ()),
                RecipeOp::Filter { spec } => self.filter(id, spec).map(|_| ()),
                RecipeOp::ClearFilter => self.clear_filter(id).map(|_| ()),
                RecipeOp::InsertCols { at, count } => self.insert_cols(id, at, count).map(|_| ()),
                RecipeOp::DeleteCols { at, count } => self.delete_cols(id, at, count).map(|_| ()),
                RecipeOp::MoveCols { at, count, to } => {
                    self.move_cols(id, at, count, to).map(|_| ())
                }
                RecipeOp::ReplaceAll { spec, with } => self.replace_all(id, spec, with).map(|_| ()),
                other => self.data_op(id, recipe_data_op(other, now)).map(|_| ()),
            };
            r.map_err(|mut e| {
                e.message = format!("step {} ({}): {}", i + 1, s.describe(), e.message);
                e
            })?;
        }
        self.info(id)
    }

    fn data_op_now(&self, id: u32, op: DataOp) -> CmdResult<OpReport> {
        if let DataOp::ShowProblems { spec, .. }
        | DataOp::SortProblems { spec }
        | DataOp::CheckColumn { spec, .. } = &op
        {
            natendo_engine::validate::check_spec(spec)?;
        }
        let grid = match &op {
            DataOp::Paste { text, .. } => {
                if text.len() > MAX_WRITE_CHARS * 4 {
                    return Err(CmdError::new(
                        "too_large",
                        "the clipboard holds too much text to paste at once",
                    ));
                }
                let g = ops::parse_tsv(text, MAX_BLOCK_CELLS)?;
                if g.iter().flatten().any(|c| c.len() > MAX_EDIT_CHARS * 4) {
                    return Err(CmdError::new(
                        "too_large",
                        "a value is too long for one cell",
                    ));
                }
                Some(g)
            }
            _ => None,
        };
        let label = match &op {
            DataOp::Paste { .. } => "Pasting",
            DataOp::Fill { .. } => "Filling",
            DataOp::ShowDuplicates { .. } | DataOp::RemoveDuplicates { .. } => {
                "Looking for duplicates"
            }
            DataOp::RemoveBlankRows { .. } | DataOp::RemoveBlankCols { .. } => "Looking for blanks",
            DataOp::SplitColumn { .. } | DataOp::SplitRows { .. } => "Splitting",
            DataOp::MergeColumns { .. } | DataOp::MergeRows { .. } => "Merging",
            DataOp::SortColumns { .. } => "Sorting columns",
            DataOp::Join { .. } => "Joining",
            DataOp::Append { .. } => "Appending",
            DataOp::Clean { .. } => "Cleaning",
            DataOp::SplitName { .. } => "Splitting names",
            DataOp::FillExample { .. } => "Filling by example",
            DataOp::Compute { .. } => "Computing",
            // Showing rows changes no data, so a locked file allows it.
            DataOp::ShowProblems { .. } => "Filtering",
            DataOp::SortProblems { .. } | DataOp::CheckColumn { .. } => "Checking",
            DataOp::FillBlanks { .. } => "Filling blanks",
            DataOp::CountRepeats { .. } | DataOp::BlankRepeats { .. } => "Looking for repeats",
            DataOp::Breaks { .. } => "Adding breaks",
            DataOp::DeleteHidden => "Deleting hidden rows",
            DataOp::MapValues { .. } => "Changing values",
            DataOp::GroupCalc { .. } => "Working out group figures",
            DataOp::Arrange { .. } => "Arranging columns",
            DataOp::Extract { .. } => "Extracting",
        };
        if let DataOp::Clean { spec, now } = &op {
            check_clean(spec, *now)?;
        }
        let seed = run_seed();
        // The other table is copied out under its own lock first, so two
        // documents are never locked together (it may be this one).
        let source = match &op {
            DataOp::Join {
                other,
                list,
                list_id,
                spec,
            } => {
                check_join_text(spec)?;
                Some(Source::Join(self.join_source(
                    id,
                    *other,
                    list.as_deref(),
                    *list_id,
                    spec,
                    &Progress::new(),
                )?))
            }
            DataOp::Append { other, spec } => {
                let e = self.get(*other)?;
                let d = e.read();
                Some(Source::Append(AppendSource::build(
                    &d,
                    spec,
                    &Progress::new(),
                )?))
            }
            _ => None,
        };
        // A translation list read from two columns of another tab.
        let pairs = match &op {
            DataOp::MapValues { from: Some(f), .. } => Some(self.map_pairs(f)?),
            _ => None,
        };
        // LOOKUP tables of a computed column, copied out the same way.
        let mut read_tabs: Vec<u32> = Vec::new();
        let tables = match &op {
            DataOp::Compute { spec } => self.lookup_tables_from(id, spec, Some(&mut read_tabs))?,
            _ => Lookups::default(),
        };
        // Columns brought in from a private tab make this tab private too
        // (no recovery copies or earlier saves of them).
        match &op {
            DataOp::Join { other: Some(o), .. } | DataOp::Append { other: o, .. } => {
                read_tabs.push(*o)
            }
            DataOp::MapValues { from: Some(f), .. } => read_tabs.push(f.doc),
            _ => {}
        }
        self.inherit_private(id, &read_tabs, &[]);
        if self.is_private(id)? {
            crate::recovery::forget(id);
        }
        let join_report = std::cell::RefCell::new(None);
        // Cleaners and fill by example: (left or missed, pattern, examples).
        let extra = std::cell::RefCell::new((0u64, None::<String>, 0u64));
        let errors = std::cell::Cell::new(0u64);
        let groups = std::cell::Cell::new(0u64);
        let other_order = std::cell::Cell::new(0u64);
        let report = std::cell::RefCell::new(OpReport::default());
        let plan = |d: &Document, p: &Progress| -> Result<Planned, Error> {
            let planned = match &op {
                DataOp::Paste { spec, .. } => {
                    Planned::Cells(d.plan_paste(spec, grid.clone().unwrap_or_default(), p)?)
                }
                DataOp::Fill { spec } => Planned::Cells(d.plan_fill(spec, p)?),
                DataOp::ShowDuplicates { spec } => Planned::Rows(d.plan_show_duplicates(spec, p)?),
                DataOp::RemoveDuplicates { spec } => {
                    Planned::Rows(d.plan_remove_duplicates(spec, p)?)
                }
                DataOp::RemoveBlankRows { spec } => {
                    Planned::Rows(d.plan_remove_blank_rows(spec, p)?)
                }
                DataOp::RemoveBlankCols { spec } => {
                    Planned::Cols(d.plan_remove_blank_cols(spec, p)?)
                }
                DataOp::SplitColumn { spec } => Planned::Cols(d.plan_split_column(spec, p)?),
                DataOp::MergeColumns { spec } => Planned::Cols(d.plan_merge_columns(spec, p)?),
                DataOp::SplitRows { spec } => Planned::Rows(d.plan_split_rows(spec, p)?),
                DataOp::MergeRows { spec } => Planned::Rows(d.plan_merge_rows(spec, p)?),
                DataOp::SortColumns {
                    col,
                    count,
                    descending,
                } => Planned::Cols(d.plan_sort_columns(*col, *count, *descending)?),
                DataOp::Join { spec, .. } => {
                    let Some(Source::Join(src)) = &source else {
                        unreachable!("built above")
                    };
                    let (plan, rep) = d.plan_join(src, spec, p)?;
                    *join_report.borrow_mut() = Some(rep);
                    Planned::Join(plan)
                }
                DataOp::Append { spec, .. } => {
                    let Some(Source::Append(src)) = &source else {
                        unreachable!("built above")
                    };
                    let (plan, rep) = d.plan_append(src, spec)?;
                    *join_report.borrow_mut() = Some(rep);
                    Planned::Cells(plan)
                }
                DataOp::Clean { spec, now } => {
                    let plan = d.plan_clean(spec, *now, seed, 0, p)?;
                    *extra.borrow_mut() = (plan.left, None, 0);
                    other_order.set(plan.other_order);
                    Planned::Clean(plan)
                }
                DataOp::SplitName { spec } => Planned::Cols(d.plan_split_name(spec, p)?),
                DataOp::FillExample { spec } => {
                    let (plan, rep) = d.plan_fill_example(spec, p)?;
                    *extra.borrow_mut() = (rep.missed, Some(rep.pattern), rep.examples);
                    Planned::Cells(plan)
                }
                DataOp::Compute { spec } => {
                    let mut spec = spec.clone();
                    spec.tables = tables.clone();
                    let (plan, rep) = d.plan_compute(&spec, p)?;
                    errors.set(rep.errors);
                    Planned::Cols(plan)
                }
                DataOp::ShowProblems { spec, passing } => {
                    let rep = d.run_checks(spec, p)?;
                    Planned::Rows(d.plan_show_problems(&rep, spec.keep_first, *passing)?)
                }
                DataOp::SortProblems { spec } => {
                    let rep = d.run_checks(spec, p)?;
                    groups.set(rep.rows);
                    Planned::Sort(d.plan_sort_problems(&rep, spec.keep_first)?)
                }
                DataOp::CheckColumn { spec, title } => {
                    let rep = d.run_checks(spec, p)?;
                    groups.set(rep.rows);
                    Planned::Cols(d.plan_check_column(spec, &rep, title, p)?)
                }
                DataOp::FillBlanks { spec } => Planned::Rows(d.plan_fill_blanks(spec, p)?),
                DataOp::CountRepeats { spec } => Planned::Cols(d.plan_count_repeats(spec, p)?),
                DataOp::BlankRepeats { spec } => Planned::Rows(d.plan_blank_repeats(spec, p)?),
                DataOp::Breaks { spec } => Planned::Rows(d.plan_breaks(spec, p)?),
                DataOp::DeleteHidden => Planned::Rows(d.plan_delete_hidden()?),
                DataOp::MapValues { spec, .. } => {
                    let mut spec = spec.clone();
                    if let Some(pairs) = &pairs {
                        spec.pairs.extend(pairs.iter().cloned());
                    }
                    Planned::Cells(d.plan_map_values(&spec, p)?)
                }
                DataOp::GroupCalc { spec } => {
                    let (plan, rep) = d.plan_group_calc(spec, p)?;
                    groups.set(rep.groups);
                    Planned::Cols(plan)
                }
                DataOp::Arrange { spec } => Planned::Cols(d.plan_arrange_columns(spec)?),
                DataOp::Extract { spec } => Planned::Cols(d.plan_extract(spec, p)?),
            };
            let mut r = report.borrow_mut();
            *r = OpReport::default();
            match &planned {
                Planned::Cells(c) => {
                    r.cells = c.cells;
                    r.skipped = c.skipped;
                }
                Planned::Cols(c) => {
                    r.cells = c.cells;
                    r.cols = c.cols;
                }
                Planned::Rows(c) => {
                    r.rows = c.rows;
                    r.groups = c.groups;
                    r.left = c.unreadable;
                }
                Planned::Clean(c) => r.cells = c.changed,
                Planned::Join(c) => r.cells = c.cells,
                Planned::Sort(_) => {}
            }
            Ok(planned)
        };
        // Widening a document with column changes is capped as inserting is.
        let widest = match &op {
            DataOp::Paste { spec, .. } => {
                let g = grid.as_ref().map_or(0, |g| {
                    if spec.transpose {
                        g.len() as u64
                    } else {
                        g.iter().map(Vec::len).max().unwrap_or(0) as u64
                    }
                });
                spec.col as u64 + g
            }
            DataOp::Fill { spec } if spec.dir == Dir::Right => {
                spec.col as u64 + spec.cols as u64 + spec.count
            }
            // The plan itself is capped at 16,384 new columns; the width
            // check below sees the real count once it is planned.
            DataOp::Join { .. } | DataOp::Append { .. } => u64::MAX,
            _ => 0,
        };
        let added = |d: &Document, n: u32| -> Result<(), Error> {
            let cols = d.col_count() as u64 + n as u64;
            if n > 0 && !within_size(cols, d.total_rows()) {
                return Err(Error::OutOfMemory(
                    "the new columns would make the file too large to save",
                ));
            }
            Ok(())
        };
        let apply = |d: &mut Document, planned: Planned| -> Result<(), Error> {
            match planned {
                Planned::Cells(c) => {
                    let widest = if widest == u64::MAX {
                        c.widest() as u64
                    } else {
                        widest
                    };
                    check_width(d, widest)?;
                    d.apply_cells(c)
                }
                Planned::Cols(c) => {
                    added(d, if c.is_grow() { c.cols } else { 0 })?;
                    d.apply_cols(c)
                }
                Planned::Rows(r) => d.apply_rows(r),
                Planned::Sort(s) => d.apply_sort(s),
                Planned::Clean(c) => d.apply_clean(c),
                Planned::Join(j) => {
                    added(d, j.width(d).saturating_sub(d.col_count()))?;
                    d.apply_join(j)
                }
            }
        };
        let info = self.compute_apply(id, label, plan, apply)?;
        let mut r = report.into_inner();
        r.info = Some(info);
        if let Some(j) = join_report.into_inner() {
            r.rows = j.added + j.extra_rows + j.removed;
            r.cols = j.cols;
            r.join = Some(j);
        }
        let (left, pattern, examples) = extra.into_inner();
        r.left = r.left.max(left);
        r.other_order = other_order.get();
        r.pattern = pattern;
        r.examples = examples;
        r.errors = errors.get();
        if groups.get() > 0 {
            r.groups = groups.get();
        }
        Ok(r)
    }

    /// The key index for a join: another open document's rows (copied out
    /// under its own lock, so two documents are never locked together) or
    /// a pasted list of keys.
    fn join_source(
        &self,
        doc: u32,
        other: Option<u32>,
        list: Option<&str>,
        list_id: Option<u64>,
        spec: &JoinSpec,
        progress: &Progress,
    ) -> CmdResult<Arc<JoinSource>> {
        let joins = || self.joins.lock().unwrap_or_else(|p| p.into_inner());
        let opts = index_key(spec);
        let cached = |key: &str| -> Option<Arc<JoinSource>> {
            joins()
                .get(&doc)?
                .source
                .as_ref()
                .filter(|(k, _)| k == key)
                .map(|(_, s)| s.clone())
        };
        let keep = |key: String, src: Arc<JoinSource>| -> Arc<JoinSource> {
            joins().entry(doc).or_default().source = Some((key, src.clone()));
            src
        };
        let from_text = |text: &str, key: String| -> CmdResult<Arc<JoinSource>> {
            if let Some(s) = cached(&key) {
                return Ok(s);
            }
            if text.chars().count() > MAX_JOIN_LIST_CHARS {
                return Err(Error::Invalid("the list is too long".into()).into());
            }
            Ok(keep(key, JoinSource::from_list(text, spec)?))
        };
        match (other, list, list_id) {
            (_, _, Some(lid)) => {
                let text = joins()
                    .get(&doc)
                    .and_then(|j| j.lists.iter().find(|(i, _)| *i == lid))
                    .map(|(_, t)| t.clone())
                    .ok_or_else(|| Error::Invalid("paste the list again".into()))?;
                from_text(&text, format!("list:{lid}:{opts}"))
            }
            (_, Some(text), None) => {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                text.hash(&mut h);
                from_text(text, format!("text:{}:{}:{opts}", h.finish(), text.len()))
            }
            (Some(o), None, None) => {
                let e = self.get(o)?;
                let d = e.read();
                let key = format!("doc:{o}:{}:{opts}", d.revision());
                if let Some(s) = cached(&key) {
                    return Ok(s);
                }
                let src = JoinSource::build(&d, spec, progress)?;
                drop(d);
                Ok(keep(key, src))
            }
            (None, None, None) => {
                Err(Error::Invalid("choose the other table or paste a list".into()).into())
            }
        }
    }

    /// Keep a pasted list for the Join screen; previews and the join name
    /// it by the number returned instead of sending it again.
    pub fn join_list(&self, doc: u32, text: String) -> CmdResult<u64> {
        if text.chars().count() > MAX_JOIN_LIST_CHARS {
            return Err(Error::Invalid("the list is too long".into()).into());
        }
        let id = self.next_list.fetch_add(1, Ordering::Relaxed) + 1;
        let mut joins = self.joins.lock().unwrap_or_else(|p| p.into_inner());
        let j = joins.entry(doc).or_default();
        j.lists.push((id, Arc::new(text)));
        if j.lists.len() > MAX_JOIN_LISTS {
            j.lists.remove(0);
        }
        Ok(id)
    }

    /// A tab's Join screen closed: let go of what it kept and stop its
    /// preview.
    pub fn join_close(&self, doc: u32) {
        let gone = self
            .joins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&doc);
        if let Some(p) = gone.and_then(|j| j.preview) {
            p.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// The computed-column screen closed: let go of its LOOKUP tables.
    pub fn lookup_close(&self) {
        self.lookups
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }

    /// How the first rows would look after a join, and the counts for the
    /// whole table. Not a job (the screen asks again as options change, and
    /// a preview never blocks the real run), but a newer preview cancels an
    /// older one, and the indexed other table is kept between previews.
    pub fn join_preview(
        &self,
        id: u32,
        other: Option<u32>,
        list: Option<&str>,
        list_id: Option<u64>,
        spec: &JoinSpec,
    ) -> CmdResult<JoinPreview> {
        check_join_text(spec)?;
        let progress = Arc::new(Progress::new());
        if let Some(old) = self
            .joins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(id)
            .or_default()
            .preview
            .replace(progress.clone())
        {
            old.cancel.store(true, Ordering::Relaxed);
        }
        let src = self.join_source(id, other, list, list_id, spec, &progress)?;
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.preview_join(&src, spec, &progress)?)
    }

    /// Old → new pairs from two columns of another tab (copied out under
    /// its own lock), capped like a typed list.
    fn map_pairs(&self, f: &MapFrom) -> CmdResult<Vec<(String, String)>> {
        let entry = self.get(f.doc)?;
        let d = entry.read();
        if !d.is_indexed() {
            return Err(Error::StillLoading.into());
        }
        if f.old >= d.col_count() || f.new >= d.col_count() {
            return Err(Error::OutOfRange.into());
        }
        let first = u64::from(f.header && d.row_count() > 0);
        let n = d.row_count().saturating_sub(first);
        if n > MAX_PAIRS as u64 {
            return Err(
                Error::Invalid(format!("a list can hold at most {MAX_PAIRS} values")).into(),
            );
        }
        let mut out = Vec::with_capacity(n as usize);
        let mut size = 0usize;
        for r in first..d.row_count() {
            let old = d.cell(r, f.old).unwrap_or_default();
            let new = d.cell(r, f.new).unwrap_or_default();
            size += old.len() + new.len();
            if size > MAX_MAP_BYTES {
                return Err(Error::Invalid("the list is too long".into()).into());
            }
            if !old.is_empty() {
                out.push((old, new));
            }
        }
        Ok(out)
    }

    /// Each distinct value of a column with its count (a job: progress and
    /// Cancel).
    pub fn value_tally(&self, id: u32, spec: &TallySpec) -> CmdResult<ValueTally> {
        let entry = self.get(id)?;
        let job = entry.start("Counting values")?;
        let d = entry.read();
        Ok(d.value_tally(spec, &job.progress)?)
    }

    /// The first results of a computed column.
    pub fn compute_preview(&self, id: u32, spec: &ComputeSpec) -> CmdResult<ComputePreview> {
        let mut spec = spec.clone();
        spec.tables = self.lookup_tables(id, &spec)?;
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.preview_compute(&spec, 20)?)
    }

    /// The tables an expression's LOOKUPs read, each copied out of its tab
    /// under that tab's own lock (never two at once). A tab is found by its
    /// name, with or without the file extension; columns by their titles.
    pub fn lookup_tables(&self, id: u32, spec: &ComputeSpec) -> CmdResult<Lookups> {
        self.lookup_tables_from(id, spec, None)
    }

    /// [`Docs::lookup_tables`], noting in `used` the tabs it reads.
    fn lookup_tables_from(
        &self,
        id: u32,
        spec: &ComputeSpec,
        mut used: Option<&mut Vec<u32>>,
    ) -> CmdResult<Lookups> {
        let refs = {
            let entry = self.get(id)?;
            let d = entry.read();
            match d.parse_expr(&spec.expr, spec.keep_first) {
                Ok(e) => e.lookups().to_vec(),
                // The real run reports the problem.
                Err(_) => return Ok(Lookups::default()),
            }
        };
        if refs.is_empty() {
            return Ok(Lookups::default());
        }
        let names: Vec<(u32, String)> = self
            .ids()
            .into_iter()
            .filter_map(|i| self.info(i).ok().map(|n| (i, n.name)))
            .collect();
        let mut out = Vec::with_capacity(refs.len());
        for r in refs {
            let want = r.tab.trim().to_lowercase();
            let hits: Vec<u32> = names
                .iter()
                .filter(|(_, n)| {
                    let n = n.to_lowercase();
                    n == want || n.rsplit_once('.').is_some_and(|(stem, _)| stem == want)
                })
                .map(|(i, _)| *i)
                .collect();
            let other = match hits.as_slice() {
                [one] => *one,
                [] => {
                    return Err(
                        Error::Invalid(format!("no open tab is called \"{}\"", r.tab)).into(),
                    )
                }
                _ => {
                    return Err(Error::Invalid(format!(
                        "more than one open tab is called \"{}\"",
                        r.tab
                    ))
                    .into())
                }
            };
            if let Some(u) = used.as_deref_mut() {
                u.push(other);
            }
            let entry = self.get(other)?;
            let d = entry.read();
            let titles = d.expr_names(true);
            let find = |name: &str| {
                let n = name.trim().to_lowercase();
                titles
                    .iter()
                    .position(|t| t.trim().to_lowercase() == n)
                    .map(|c| c as u32)
                    .ok_or_else(|| {
                        Error::Invalid(format!("the tab \"{}\" has no column \"{name}\"", r.tab))
                    })
            };
            let key = find(&r.key)?;
            let value = find(&r.column)?;
            // One table per tab, revision and pair of columns, shared by
            // every LOOKUP that reads it and kept between previews.
            let name = format!("{other}:{}:{key}:{value}", d.revision());
            let hit = {
                let cache = self.lookups.lock().unwrap_or_else(|p| p.into_inner());
                cache
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, t)| t.clone())
            };
            let table = match hit {
                Some(t) => t,
                None => {
                    let t = Arc::new(LookupTable::build(&d, key, value, true, &Progress::new())?);
                    let mut cache = self.lookups.lock().unwrap_or_else(|p| p.into_inner());
                    cache.retain(|(k, _)| {
                        !k.starts_with(&format!("{other}:"))
                            || k.starts_with(&format!("{other}:{}:", d.revision()))
                    });
                    cache.push((name, t.clone()));
                    if cache.len() > MAX_LOOKUP_TABLES {
                        cache.remove(0);
                    }
                    t
                }
            };
            out.push(Some(table));
        }
        Ok(Lookups(out))
    }

    /// Run data checks over the shown rows (a job: progress and Cancel).
    pub fn run_checks(&self, id: u32, spec: &CheckSpec) -> CmdResult<CheckReport> {
        let key = serde_json::to_string(&spec.checks).unwrap_or_default();
        let entry = self.get(id)?;
        let job = entry.start("Checking")?;
        let d = entry.read();
        let r = d.run_checks(spec, &job.progress)?;
        *entry.checks.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((key, d.revision(), r.repeats.clone()));
        Ok(r)
    }

    /// Problem marks for an area of the screen. Unique checks are marked
    /// from the last run while the document has not changed since.
    #[allow(clippy::too_many_arguments)]
    pub fn check_marks(
        &self,
        id: u32,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
        spec: &CheckSpec,
    ) -> CmdResult<Vec<CheckMark>> {
        let key = serde_json::to_string(&spec.checks).unwrap_or_default();
        let entry = self.get(id)?;
        let d = entry.read();
        let last = entry.checks.lock().unwrap_or_else(|p| p.into_inner());
        let repeats: &[(u32, Vec<u64>)] = match last.as_ref() {
            Some((k, rev, reps)) if *k == key && *rev == d.revision() => reps,
            _ => &[],
        };
        Ok(d.check_marks(row, rows, col, cols, spec, repeats)?)
    }

    /// Build a pivot or unpivot table from a document and open it as a new,
    /// unsaved document.
    pub fn new_table(&self, id: u32, op: TableOp) -> CmdResult<DocInfo> {
        Ok(self.new_table_report(id, op)?.info)
    }

    /// [`Docs::new_table`] with what a comparison found.
    pub fn new_table_report(&self, id: u32, op: TableOp) -> CmdResult<TableReport> {
        // A comparison reads the two tables one after the other (each under
        // its own lock, never both at once): fingerprints of the second, then
        // the first, then the second's rows that differ.
        let mut compare = None;
        let table: Table = if let TableOp::Compare { other, spec } = &op {
            let oe = self.get(*other)?;
            let entry = self.get(id)?;
            let job = entry.start("Comparing")?;
            let index = {
                let d = oe.read();
                CompareIndex::build(&d, spec, &job.progress)?
            };
            let plan = {
                let d = entry.read();
                d.compare_rows(&index, spec, &job.progress)?
            };
            let d = oe.read();
            let (t, r) = d
                .compare_finish(&index, plan, spec, &job.progress)
                .map_err(|e| {
                    if matches!(e, Error::Changed) {
                        CmdError::new(
                            "changed",
                            "the second file changed while comparing; try again",
                        )
                    } else {
                        e.into()
                    }
                })?;
            compare = Some(r);
            t
        } else {
            let entry = self.get(id)?;
            let job = entry.start("Building the table")?;
            let d = entry.read();
            match &op {
                TableOp::Pivot { spec } => d.pivot(spec, &job.progress)?,
                TableOp::Unpivot { spec } => d.unpivot(spec, &job.progress)?,
                TableOp::Subtotals { spec } => d.subtotals(spec, &job.progress)?,
                TableOp::Reshape { spec } => d.reshape_column(spec, &job.progress)?,
                TableOp::Stack { spec } => d.stack_columns(spec, &job.progress)?,
                TableOp::Sample { spec } => d.sample(spec, &job.progress)?,
                TableOp::Compare { .. } => unreachable!("handled above"),
            }
        };
        let doc = table.into_document()?;
        let (nid, _) = self.insert(doc, false)?;
        // A sample, comparison or pivot of a private tab is private too.
        let mut from = vec![id];
        if let TableOp::Compare { other, .. } = &op {
            from.push(*other);
        }
        self.inherit_private(nid, &from, &[]);
        Ok(TableReport {
            info: self.info(nid)?,
            compare,
        })
    }

    /// Start a pivot job: a pivot already running on this tab (an older
    /// preview) is cancelled first; anything else makes this "busy".
    fn start_pivot<'a>(&self, entry: &'a Entry) -> CmdResult<JobGuard<'a>> {
        for _ in 0..200 {
            match entry.start(PIVOT_JOB) {
                Ok(j) => return Ok(j),
                Err(e) => {
                    let ours = entry.jobs().as_ref().is_some_and(|j| j.label == PIVOT_JOB);
                    if !ours {
                        return Err(e);
                    }
                    if let Some(j) = entry.jobs().as_ref() {
                        j.progress.cancel.store(true, Ordering::Relaxed);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
        Err(CmdError::new(
            "busy",
            "the pivot is still working; try again",
        ))
    }

    /// The cube for `spec` (read again only when the file or the fields
    /// changed). `sample`: only the first rows, for a quick first look.
    /// The cube for `spec` (read again only when the file or the fields
    /// changed). `sample`: only the first rows, for a quick first look. The
    /// caller holds the pivot job, so Stop or a newer pivot cancels it.
    fn pivot_cube(
        &self,
        entry: &Entry,
        spec: &PivotSpec,
        sample: Option<u64>,
        progress: &Progress,
    ) -> CmdResult<Arc<Cube>> {
        let key = spec.scan_key(sample);
        let quick = sample.is_some();
        let rev = entry.read().revision();
        {
            let mut cache = entry.pivots.lock().unwrap_or_else(|p| p.into_inner());
            if let Some((_, _, _, c)) = cache.iter().find(|(k, r, _, _)| *k == key && *r == rev) {
                return Ok(c.clone());
            }
            // The cube this one replaces is let go before reading: one quick
            // and one full cube at most.
            cache.retain(|(_, r, q, _)| *r == rev && *q != quick);
        }
        let (cube, rev) = {
            let d = entry.read();
            (d.pivot_cube(spec, sample, progress)?, d.revision())
        };
        let cube = Arc::new(cube);
        let mut cache = entry.pivots.lock().unwrap_or_else(|p| p.into_inner());
        cache.retain(|(_, r, q, _)| *r == rev && *q != quick);
        cache.push((key, rev, quick, cube.clone()));
        Ok(cube)
    }

    fn pivot_titles(entry: &Entry, keep_first: bool) -> Vec<String> {
        let d = entry.read();
        let all: Vec<u32> = (0..d.col_count()).collect();
        d.titles(&all, keep_first)
    }

    /// The cube and its layout, under one pivot job.
    fn pivot_grid(
        &self,
        entry: &Entry,
        spec: &PivotSpec,
        sample: Option<u64>,
        limit: Option<GridLimit>,
    ) -> CmdResult<PivotGrid> {
        let job = self.start_pivot(entry)?;
        let cube = self.pivot_cube(entry, spec, sample, &job.progress)?;
        let titles = Self::pivot_titles(entry, spec.keep_first);
        Ok(cube.layout_with(spec, &titles, limit, &job.progress)?)
    }

    /// A pivot table's first rows and columns, for the live preview.
    /// `sample`: read only that many rows (a quick look at a big file).
    pub fn pivot_preview(
        &self,
        id: u32,
        spec: &PivotSpec,
        sample: Option<u64>,
    ) -> CmdResult<PivotGrid> {
        let entry = self.get(id)?;
        let limit = GridLimit {
            rows: PIVOT_PREVIEW_ROWS,
            cols: PIVOT_PREVIEW_COLS,
        };
        self.pivot_grid(&entry, spec, sample, Some(limit))
    }

    /// The whole pivot table in a new tab.
    pub fn pivot_table(&self, id: u32, spec: &PivotSpec) -> CmdResult<DocInfo> {
        let entry = self.get(id)?;
        let table = self
            .pivot_grid(&entry, spec, None, None)?
            .to_table(spec.layout.repeat_labels)?;
        let doc = table.into_document()?;
        let (nid, _) = self.insert(doc, false)?;
        self.inherit_private(nid, &[id], &[]);
        self.info(nid)
    }

    /// The whole pivot table as tab-separated text (for the clipboard).
    pub fn pivot_text(&self, id: u32, spec: &PivotSpec) -> CmdResult<String> {
        let entry = self.get(id)?;
        let grid = self.pivot_grid(&entry, spec, None, None)?;
        // Measured before the text is built (a tab or line break per cell).
        let size: usize = grid
            .head
            .iter()
            .flat_map(|h| h.iter().map(|c| c.text.len() + c.span as usize))
            .chain(
                grid.rows
                    .iter()
                    .flat_map(|r| r.labels.iter().chain(&r.cells).map(|c| c.len() + 1)),
            )
            .sum();
        if size > MAX_CLIP_TEXT {
            return Err(CmdError::new(
                "too_large",
                "the pivot table is too large to copy; put it in a new tab instead",
            ));
        }
        Ok(grid.to_tsv())
    }

    /// The source rows behind one pivot cell, in a new tab.
    pub fn pivot_drill(
        &self,
        id: u32,
        spec: &PivotSpec,
        row_path: &[String],
        col_path: &[String],
    ) -> CmdResult<DocInfo> {
        let entry = self.get(id)?;
        let table = {
            let job = self.start_pivot(&entry)?;
            let cube = self.pivot_cube(&entry, spec, None, &job.progress)?;
            let d = entry.read();
            d.pivot_drill(spec, &cube, row_path, col_path, &job.progress)?
        };
        let doc = table.into_document()?;
        let (nid, _) = self.insert(doc, false)?;
        self.inherit_private(nid, &[id], &[]);
        self.info(nid)
    }

    /// Drop the pivot data kept for a tab.
    pub fn pivot_forget(&self, id: u32) -> CmdResult<()> {
        let entry = self.get(id)?;
        entry
            .pivots
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        Ok(())
    }

    /// Titles of every column of a tab.
    pub fn titles_of(&self, id: u32, keep_first: bool) -> CmdResult<Vec<String>> {
        Ok(Self::pivot_titles(&*self.get(id)?, keep_first))
    }

    /// The items of one pivot field, for its pick list.
    pub fn pivot_items(
        &self,
        id: u32,
        field: &PivotField,
        spec: &PivotSpec,
    ) -> CmdResult<ItemList> {
        let entry = self.get(id)?;
        let job = self.start_pivot(&entry)?;
        let d = entry.read();
        Ok(d.pivot_items(field, spec, &job.progress)?)
    }

    /// Rename the open file on disk to `to` (chosen in a native dialog) and
    /// keep working on it under the new name. Refuses to replace a file.
    pub fn rename_file(&self, id: u32, to: &Path) -> CmdResult<DocInfo> {
        let entry = self.get(id)?;
        if entry.locked.load(Ordering::Relaxed) {
            return Err(read_only());
        }
        self.check_writable(to)?;
        {
            let mut d = entry.write();
            let from = d
                .path()
                .map(Path::to_path_buf)
                .ok_or_else(|| CmdError::new("no_path", "this document has no file yet"))?;
            if same_file(&from, to) && from.file_name() == to.file_name() {
                return self.info(id);
            }
            // Only a change of letter case may point at the same file.
            if to.exists() && !same_file(&from, to) {
                return Err(CmdError::new(
                    "exists",
                    "a file with that name is already there; choose another name",
                ));
            }
            d.rename_to(to).map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    return CmdError::new(
                        "exists",
                        "a file with that name is already there; choose another name",
                    );
                }
                CmdError::new(
                    "error",
                    format!("the file could not be renamed ({e}); close other programs using it, or use Save As"),
                )
            })?;
        }
        // The name shown is the file's own from now on.
        *entry.title.lock().unwrap_or_else(|p| p.into_inner()) = None;
        self.info(id)
    }

    /// Write shown rows into one file per part (a column's values, or every
    /// N rows) inside `dir`, in the document's own format. Names are made
    /// safe for Windows; existing files are only replaced with `replace`
    /// (otherwise the call fails with kind `exists`, listing how many).
    pub fn split_to_files(
        &self,
        id: u32,
        spec: &SplitParts,
        dir: &Path,
        base: &str,
        replace: bool,
        on_written: &dyn Fn(&Path),
    ) -> CmdResult<SplitReport> {
        let entry = self.get(id)?;
        // Files open in any tab (read before this document's lock is taken,
        // so no lock is ever held twice): none of them may be written over.
        let entries: Vec<Arc<Entry>> = self.map().values().cloned().collect();
        let open: Vec<PathBuf> = entries
            .iter()
            .filter_map(|e| e.read().path().map(Path::to_path_buf))
            .collect();
        drop(entries);
        let job = entry.start("Splitting into files")?;
        let d = entry.read();
        let parts = d.split_parts(spec, &job.progress)?;
        let ext = d
            .path()
            .and_then(|p| p.extension())
            .map(|e| e.to_string_lossy().into_owned())
            .filter(|e| !e.is_empty() && e.len() <= 10)
            .unwrap_or_else(|| "csv".into());
        let base = part_name(base, "part");
        let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut paths = Vec::with_capacity(parts.len());
        for p in &parts {
            let stem = format!("{base} - {}", part_name(&p.name, "blank"));
            let mut name = format!("{stem}.{ext}");
            let mut n = 2;
            while !used.insert(name.to_lowercase()) {
                name = format!("{stem} ({n}).{ext}");
                n += 1;
            }
            paths.push(dir.join(name));
        }
        let existing = paths.iter().filter(|p| p.exists()).count();
        if existing > 0 && !replace {
            return Err(CmdError::new(
                "exists",
                format!(
                    "{} of the {} files are already in that folder",
                    group(existing as u64),
                    group(paths.len() as u64)
                ),
            ));
        }
        if let Some(p) = paths.iter().find(|p| open.iter().any(|o| same_file(o, p))) {
            return Err(CmdError::new(
                "invalid",
                format!(
                    "{} is open in a tab; close it or choose another folder",
                    p.file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default()
                ),
            ));
        }
        // A character the encoding cannot store is found before any file is
        // written (only older one-byte and multi-byte encodings can fail).
        if matches!(
            d.format().encoding,
            natendo_engine::encoding::Encoding::Legacy(_)
                | natendo_engine::encoding::Encoding::Utf7
        ) {
            for part in &parts {
                d.write_part(&part.ids, spec.keep_first, &mut std::io::sink())?;
            }
        }
        job.progress.start(parts.len() as u64);
        let mut rows = 0u64;
        for (n, (part, path)) in parts.iter().zip(&paths).enumerate() {
            let done = crate::extras::write_atomic_with(path, |f| {
                let mut w = std::io::BufWriter::with_capacity(1 << 20, f);
                d.write_part(&part.ids, spec.keep_first, &mut w)?;
                std::io::Write::flush(&mut w).map_err(natendo_engine::Error::from)?;
                Ok(())
            })
            .map(|()| on_written(path))
            .and_then(|()| job.progress.step(1).map_err(CmdError::from));
            if let Err(mut e) = done {
                if n > 0 {
                    e.message = format!(
                        "{} ({} of the {} files were written before this)",
                        e.message,
                        group(n as u64),
                        group(paths.len() as u64)
                    );
                }
                return Err(e);
            }
            rows += part.ids.len() as u64;
        }
        Ok(SplitReport {
            files: paths.len() as u64,
            rows,
            replaced: existing as u64,
            private: 0,
            private_missed: 0,
            private_folder: false,
        })
    }

    /// Stack several files (picked in a native dialog) into a new tab.
    pub fn combine_files(&self, paths: &[PathBuf], spec: &Combine) -> CmdResult<DocInfo> {
        if paths.is_empty() || paths.len() > MAX_COMBINE {
            return Err(CmdError::new(
                "bad_request",
                format!("combine 1 to {MAX_COMBINE} files"),
            ));
        }
        // The result cannot be larger than a new table may be, so files that
        // together are larger are refused before any is read (review M1).
        let mut bytes = 0u64;
        for p in paths {
            bytes = bytes.saturating_add(
                std::fs::metadata(p)
                    .map_err(natendo_engine::Error::from)?
                    .len(),
            );
        }
        if bytes > natendo_engine::reshape::MAX_TABLE_BYTES as u64 {
            return Err(CmdError::new(
                "too_large",
                format!(
                    "these files hold {} MB together; at most {} MB can be combined into one new table",
                    bytes >> 20,
                    natendo_engine::reshape::MAX_TABLE_BYTES >> 20
                ),
            ));
        }
        let mut docs = Vec::with_capacity(paths.len());
        for p in paths {
            let mut d = Document::open(p, OpenOptions::default())?;
            while !d.index_step(64 << 20)? {}
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            docs.push((name, d));
        }
        let refs: Vec<(String, &Document)> = docs.iter().map(|(n, d)| (n.clone(), d)).collect();
        let table = natendo_engine::tools::combine(&refs, spec, &Progress::new())?;
        drop(refs);
        drop(docs);
        let (nid, _) = self.insert(table.into_document()?, false)?;
        let paths: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
        self.inherit_private(nid, &[], &paths);
        self.info(nid)
    }

    /// Type-ahead: the first value of column `col` (in its lower-case
    /// order) that starts with `prefix`, ignoring case, longer than it.
    /// Values come from the first 2,000,000 shown rows (at most 50,000
    /// distinct), kept in memory until the document changes. Never waits
    /// behind a change: a busy document gives no suggestion.
    pub fn typeahead(
        &self,
        id: u32,
        col: u32,
        prefix: &str,
        keep_first: bool,
    ) -> CmdResult<Option<String>> {
        if prefix.is_empty() || prefix.chars().count() > 1024 {
            return Ok(None);
        }
        let entry = self.get(id)?;
        let d = match entry.doc.try_read() {
            Ok(d) => d,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
        };
        if entry.change_waiting() || !d.is_indexed() {
            return Ok(None);
        }
        let rev = d.revision();
        let cached = entry
            .typeahead
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .filter(|(r, c, _)| *r == rev && *c == col)
            .map(|(_, _, v)| v.clone());
        let values = match cached {
            Some(v) => v,
            None => {
                let v = Arc::new(d.typeahead_values(col, keep_first, 2_000_000, 50_000)?);
                *entry.typeahead.lock().unwrap_or_else(|p| p.into_inner()) =
                    Some((rev, col, v.clone()));
                v
            }
        };
        drop(d);
        let p = prefix.to_lowercase();
        let i = values.partition_point(|(k, _)| k.as_str() < p.as_str());
        Ok(values
            .get(i)
            .filter(|(k, _)| k.starts_with(&p) && k.len() > p.len())
            .map(|(_, v)| v.clone()))
    }

    /// Delete several columns at once (hidden columns), one undo step.
    pub fn delete_col_list(&self, id: u32, cols: Vec<u32>) -> CmdResult<DocInfo> {
        self.compute_apply(
            id,
            "Deleting columns",
            |d, _| d.plan_delete_cols(&cols),
            |d, plan| d.apply_cols(plan),
        )
    }

    /// Reverse the order of shown rows (one undo step).
    pub fn flip_rows(&self, id: u32, at: u64, count: u64) -> CmdResult<DocInfo> {
        self.compute_apply(
            id,
            "Flipping",
            |d, _| d.plan_flip(at, count),
            |d, plan| d.apply_shuffle(plan),
        )
    }

    /// The next shown row whose cell differs from this one.
    pub fn find_change(&self, id: u32, row: u64, col: u32, up: bool) -> CmdResult<Option<u64>> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.find_change(row, col, up)?)
    }

    /// The row with a column's largest or smallest number.
    pub fn find_extreme(
        &self,
        id: u32,
        col: u32,
        largest: bool,
        keep_first: bool,
        read: ReadOptions,
    ) -> CmdResult<Option<u64>> {
        let entry = self.get(id)?;
        let job = entry.start("Looking")?;
        let d = entry.read();
        Ok(d.find_extreme(col, largest, keep_first, read, &job.progress)?)
    }

    /// Smallest and largest number of a column (colour scales, data bars).
    /// Never waits behind a change: busy documents answer `busy`.
    pub fn number_range(
        &self,
        id: u32,
        col: u32,
        keep_first: bool,
        read: ReadOptions,
    ) -> CmdResult<Option<(f64, f64)>> {
        let entry = self.get(id)?;
        let d = match entry.doc.try_read() {
            Ok(d) => d,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(CmdError::new("busy", "the file is being changed"))
            }
        };
        if entry.change_waiting() {
            return Err(CmdError::new("busy", "the file is being changed"));
        }
        Ok(d.number_range(col, keep_first, read, &Progress::new())?)
    }

    /// The first changes a cleaner would make (looks at the first rows only).
    pub fn clean_preview(&self, id: u32, spec: &CleanSpec, now: f64) -> CmdResult<CleanPreview> {
        check_clean(spec, now)?;
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.preview_clean(
            spec,
            now,
            run_seed(),
            CLEAN_PREVIEW_CHANGES,
            CLEAN_PREVIEW_ROWS,
        )?)
    }

    /// The first rows of an export as text (empty for Excel).
    pub fn export_preview(&self, id: u32, opts: &ExportOptions) -> CmdResult<String> {
        check_export(opts)?;
        if opts.format == ExportFormat::Xlsx {
            return Ok(String::new());
        }
        let entry = self.get(id)?;
        let d = entry.read();
        let mut out = Vec::new();
        d.export(&mut out, opts, Some(EXPORT_PREVIEW_ROWS), &Progress::new())?;
        let mut s = String::from_utf8_lossy(&out).into_owned();
        if s.len() > EXPORT_PREVIEW_BYTES {
            let mut cut = EXPORT_PREVIEW_BYTES;
            while !s.is_char_boundary(cut) {
                cut -= 1;
            }
            s.truncate(cut);
            s.push('…');
        }
        Ok(s)
    }

    /// Export to a file chosen in a native dialog (atomic replace).
    pub fn export_file(
        &self,
        id: u32,
        path: &Path,
        opts: &ExportOptions,
    ) -> CmdResult<ExportReport> {
        check_export(opts)?;
        self.check_writable(path)?;
        let entry = self.get(id)?;
        let job = entry.start("Exporting")?;
        let d = entry.read();
        Ok(d.export_file(path, opts, &job.progress)?)
    }

    /// Export as text for the clipboard (capped; not Excel).
    pub fn export_text(&self, id: u32, opts: &ExportOptions) -> CmdResult<String> {
        check_export(opts)?;
        if opts.format == ExportFormat::Xlsx {
            return Err(CmdError::new(
                "invalid",
                "Excel files cannot go on the clipboard",
            ));
        }
        let entry = self.get(id)?;
        let job = entry.start("Exporting")?;
        let d = entry.read();
        let mut out = CappedWriter {
            buf: Vec::new(),
            cap: MAX_COPY_BYTES,
        };
        match d.export(&mut out, opts, None, &job.progress) {
            Ok(_) => {}
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::OutOfMemory => {
                return Err(too_much());
            }
            Err(e) => return Err(e.into()),
        }
        String::from_utf8(out.buf).map_err(|_| CmdError::new("error", "export was not text"))
    }

    /// Where Ctrl+arrow lands from a cell.
    pub fn data_edge(&self, id: u32, row: u64, col: u32, dir: Dir) -> CmdResult<(u64, u32)> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.edge(row, col, dir)?)
    }

    fn edit(
        &self,
        id: u32,
        f: impl FnOnce(&mut Document) -> Result<(), Error>,
    ) -> CmdResult<DocInfo> {
        self.edit_as(id, true, f)
    }

    /// `changes`: the call changes the data (refused while read-only);
    /// filters only change what is shown.
    fn edit_as(
        &self,
        id: u32,
        changes: bool,
        f: impl FnOnce(&mut Document) -> Result<(), Error>,
    ) -> CmdResult<DocInfo> {
        let entry = self.get(id)?;
        let locked = entry.locked.load(Ordering::Relaxed);
        if changes && locked {
            return Err(read_only());
        }
        let error = entry
            .error
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let mut d = entry.write();
        f(&mut d)?;
        let mut info = describe(id, &d, entry.line_by_line.load(Ordering::Relaxed), error);
        info.read_only = locked;
        Ok(info)
    }

    /// Whether `path` is the file of a locked document (any tab). Takes no
    /// document lock of the caller's, so call it before taking one.
    pub fn is_locked_file(&self, path: &Path) -> bool {
        let locked: Vec<Arc<Entry>> = self
            .map()
            .values()
            .filter(|e| e.locked.load(Ordering::Relaxed))
            .cloned()
            .collect();
        locked
            .iter()
            .any(|e| e.read().path().is_some_and(|p| same_file(p, path)))
    }

    /// Refuse writing over a locked document's file.
    pub fn check_writable(&self, path: &Path) -> CmdResult<()> {
        if self.is_locked_file(path) {
            Err(read_only())
        } else {
            Ok(())
        }
    }

    /// Lock or unlock a document (read-only).
    pub fn set_read_only(&self, id: u32, on: bool) -> CmdResult<DocInfo> {
        self.get(id)?.locked.store(on, Ordering::Relaxed);
        self.info(id)
    }

    /// Guessed column types (from a sample of shown rows).
    pub fn column_types(
        &self,
        id: u32,
        skip_first: bool,
        read: ReadOptions,
    ) -> CmdResult<Vec<ColType>> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.column_types(skip_first, read)?)
    }

    pub fn summary(
        &self,
        id: u32,
        col: u32,
        skip_first: bool,
        read: ReadOptions,
    ) -> CmdResult<Summary> {
        let entry = self.get(id)?;
        let job = entry.start("Summing up")?;
        let d = entry.read();
        Ok(d.summary(col, skip_first, read, &job.progress)?)
    }

    pub fn chart_data(&self, id: u32, spec: &ChartSpec) -> CmdResult<ChartData> {
        let entry = self.get(id)?;
        let job = entry.start("Making the chart")?;
        let d = entry.read();
        Ok(d.chart_data(spec, &job.progress)?)
    }

    /// Highlight marks for an area of the screen.
    #[allow(clippy::too_many_arguments)]
    pub fn highlights(
        &self,
        id: u32,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
        rules: &[Highlight],
        skip_first: bool,
        read: ReadOptions,
    ) -> CmdResult<Vec<Mark>> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.highlights(row, rows, col, cols, rules, skip_first, read)?)
    }
}

/// Whether two paths name the same file. Paths are compared by what they
/// point at (volume and file id, so `\\?\` forms, letter case, short names,
/// links and hard links all match), then, for files that do not exist yet,
/// as resolved text with case ignored on Windows.
pub fn same_file(a: &Path, b: &Path) -> bool {
    if same_file::is_same_file(a, b).unwrap_or(false) {
        return true;
    }
    let norm = |p: &Path| -> String {
        let p = std::fs::canonicalize(p)
            .or_else(|_| std::path::absolute(p))
            .unwrap_or_else(|_| p.to_path_buf());
        let s = p.to_string_lossy();
        let s = match s.strip_prefix(r"\\?\UNC\") {
            Some(r) => format!(r"\\{r}"),
            None => s.strip_prefix(r"\\?\").unwrap_or(&s).to_string(),
        };
        if cfg!(windows) {
            s.replace('/', "\\").to_lowercase()
        } else {
            s
        }
    };
    norm(a) == norm(b)
}

fn read_only() -> CmdError {
    CmdError::new(
        "read_only",
        "this file is locked (read-only); unlock it to make changes",
    )
}

/// Matches of a pattern in a piece of text (the edit panel's search), as
/// UTF-16 offsets.
pub fn find_in_text(text: &str, pattern: &Pattern) -> CmdResult<Vec<(u32, u32)>> {
    if text.len() > MAX_EDIT_CHARS * 4 {
        return Err(CmdError::new("too_large", "the text is too long to search"));
    }
    Ok(Matcher::new(pattern)?.find_in_text(text, MAX_TEXT_MATCHES))
}

fn replace_key(spec: &FindSpec, replacement: &str) -> String {
    serde_json::to_string(&(spec, replacement)).unwrap_or_default()
}

/// Value lists from the UI are bounded in count and size.
pub(crate) fn check_filter(spec: &FilterSpec) -> CmdResult<()> {
    let (mut n, mut bytes) = (0usize, 0usize);
    for c in &spec.columns {
        if let Rule::Values { values, .. } = &c.rule {
            n += values.len();
            bytes += values.iter().map(String::len).sum::<usize>();
        }
    }
    if n > MAX_FILTER_VALUES || bytes > MAX_FILTER_BYTES {
        return Err(CmdError::new(
            "too_large",
            "too many values ticked at once; use a text or number rule instead",
        ));
    }
    Ok(())
}

/// A document of `cols` columns and `rows` rows is small enough to save
/// with every row padded to full width.
fn within_size(cols: u64, rows: u64) -> bool {
    cols <= MAX_COLS_AFTER_INSERT as u64 && cols.saturating_mul(rows) <= MAX_CELLS_AFTER_INSERT
}

/// With column changes, an edit past the last column widens every row on
/// save; apply the same size caps as inserting columns.
fn check_width(d: &Document, widest: u64) -> Result<(), Error> {
    if d.has_column_changes()
        && widest > d.col_count() as u64
        && !within_size(widest, d.total_rows())
    {
        return Err(Error::OutOfMemory(
            "a value that far right would make the file too large to save",
        ));
    }
    Ok(())
}

fn too_much() -> CmdError {
    CmdError::new(
        "too_large",
        format!(
            "one change is limited to {} cells; larger bulk changes arrive with the data tools",
            group(MAX_WRITE_CELLS)
        ),
    )
}

/// Rows and bytes in an export preview.
pub const EXPORT_PREVIEW_ROWS: u64 = 20;
pub const EXPORT_PREVIEW_BYTES: usize = 64 << 10;
/// Longest text option of an export (names, template parts).
pub const MAX_EXPORT_OPTION_CHARS: usize = 1 << 20;

fn check_export(o: &ExportOptions) -> CmdResult<()> {
    let long = [&o.root, &o.record, &o.before, &o.row, &o.after, &o.sheet]
        .iter()
        .any(|s| s.len() > MAX_EXPORT_OPTION_CHARS);
    if long || o.root.len() > 1000 || o.record.len() > 1000 || o.sheet.len() > 1000 {
        return Err(CmdError::new("too_large", "an export setting is too long"));
    }
    Ok(())
}

/// Collects export text, failing once past `cap`.
struct CappedWriter {
    buf: Vec<u8>,
    cap: usize,
}

impl std::io::Write for CappedWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self.buf.len() + b.len() > self.cap {
            return Err(std::io::Error::from(std::io::ErrorKind::OutOfMemory));
        }
        self.buf.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn check_write(cells: u64, lens: impl Iterator<Item = usize>) -> CmdResult<()> {
    if cells > MAX_WRITE_CELLS {
        return Err(too_much());
    }
    let mut total = 0usize;
    for len in lens {
        // Byte length bounds the character count from above.
        if len > MAX_EDIT_CHARS * 4 {
            return Err(CmdError::new(
                "too_large",
                "a value is too long for one cell",
            ));
        }
        total = total.saturating_add(len);
    }
    if total > MAX_WRITE_CHARS * 4 {
        return Err(too_much());
    }
    Ok(())
}

/// Thousands separators for messages.
fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn push_tsv(v: &str, out: &mut String) {
    if v.contains(['\t', '"', '\n', '\r']) {
        out.push('"');
        out.push_str(&v.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(v);
    }
}

fn describe(id: u32, d: &Document, line_by_line: bool, error: Option<String>) -> DocInfo {
    let f = d.format();
    let stats = d.index().stats();
    let mut flags = 0u8;
    for r in &stats.repairs {
        flags |= r.flags;
    }
    let path = d.path();
    DocInfo {
        id,
        name: path
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into()),
        path: path.map(|p| p.display().to_string()),
        rows: d.row_count(),
        total_rows: d.total_rows(),
        filtered: d.is_filtered(),
        filter_cols: d.filtered_columns(),
        filter_on_deleted: d.filter_on_deleted_column(),
        cols: d.col_count(),
        indexed: d.is_indexed(),
        progress: d.index_progress(),
        modified: d.is_modified(),
        can_undo: d.can_undo(),
        can_redo: d.can_redo(),
        history_pos: d.history_pos(),
        encoding: f.encoding.name(),
        bom: f.bom,
        delimiter: f.dialect.delimiter_str().to_string(),
        quote: f.dialect.quote().map(byte_label),
        escape: f.dialect.escape().map(byte_label),
        merge: f.dialect.merges(),
        preamble_lines: d.preamble_lines(),
        layout_confidence: d.detection().map(|x| x.confidence),
        read_only: false,
        hidden_rows: d.hidden_rows(),
        has_filter: d.has_filter(),
        line_break: line_break_name(f.line_break),
        mixed_line_breaks: stats.mixed_line_breaks(),
        lossy: d.is_lossy(),
        invalid_bytes: d.has_invalid_bytes(),
        line_by_line,
        column_changes: d.has_column_changes(),
        repair_count: stats.repair_count,
        unclosed_quotes: flags & scan::UNCLOSED_QUOTE != 0 || d.index().line_bound_from().is_some(),
        long_quoted_cells: flags & scan::LONG_QUOTED_CELL != 0,
        line_bound_from: d.index().line_bound_from(),
        error,
    }
}

fn index_in_steps(entry: &Entry) {
    loop {
        if entry.cancel.load(Ordering::Relaxed) {
            return;
        }
        let step = entry.write().index_step(INDEX_STEP);
        match step {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => {
                *entry.error.lock().unwrap_or_else(|p| p.into_inner()) = Some(e.to_string());
                return;
            }
        }
    }
}

/// Read a recipe file (at most 16 MB).
pub fn read_recipe(path: &Path) -> CmdResult<Recipe> {
    let fail = |e: std::io::Error| CmdError::new("io", format!("could not open the recipe: {e}"));
    let size = std::fs::metadata(path).map_err(fail)?.len();
    if size > 16 << 20 {
        return Err(CmdError::new(
            "invalid",
            "a recipe file can be at most 16 MB",
        ));
    }
    let bytes = std::fs::read(path).map_err(fail)?;
    Ok(Recipe::from_json(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str, body: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nde-docs-{}-{name}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    fn wait(docs: &Docs, id: u32) -> DocInfo {
        for _ in 0..500 {
            let i = docs.info(id).unwrap();
            if i.indexed {
                return i;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("indexing did not finish");
    }

    #[test]
    fn autosave_waits_and_readers_give_way() {
        let p = tmp(
            "idle.csv",
            b"a,b
1,2
",
        );
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        docs.set_cells(id, vec![(1, 0, "9".into())]).unwrap();
        let e = docs.get(id).unwrap();
        // A small document is copied even while it is being edited.
        let now = now_millis();
        assert!(!e.put_off_copy(false, now));
        // A big one (or one whose last copy a change stopped) waits for a
        // pause...
        assert!(e.put_off_copy(true, now));
        assert_eq!(e.waiting_since.load(Ordering::Relaxed), now);
        assert!(e.put_off_copy(true, now + 20_000));
        e.copy_cancelled.store(true, Ordering::Relaxed);
        assert_eq!(docs.autosave(id).unwrap(), Autosaved::Skipped);
        // ...but not for ever: steady editing still gets a copy.
        e.changed_at
            .store(now + AUTOSAVE_MAX_WAIT_MS, Ordering::Relaxed);
        assert!(!e.put_off_copy(true, now + AUTOSAVE_MAX_WAIT_MS));
        // A copy stopped by a change starts the wait again.
        let later = now + AUTOSAVE_MAX_WAIT_MS;
        e.waiting_since.store(later, Ordering::Relaxed);
        assert!(e.put_off_copy(true, later + 1_000));
        // A pause ends the wait too.
        e.waiting_since.store(0, Ordering::Relaxed);
        assert!(!e.put_off_copy(true, now + AUTOSAVE_MAX_WAIT_MS + AUTOSAVE_IDLE_MS));
        e.copy_cancelled.store(false, Ordering::Relaxed);
        // A change waiting for the document stops quick totals that got the
        // document first (it is busy, asked again after the change).
        assert!(!e.change_waiting());
        e.writers.fetch_add(1, Ordering::SeqCst);
        assert!(e.change_waiting());
        let err = docs
            .selection_totals(id, 0, 2, 0, 2, ReadOptions::default(), false)
            .unwrap_err();
        assert_eq!(err.kind, "busy");
        e.writers.fetch_sub(1, Ordering::SeqCst);
        assert!(docs
            .selection_totals(id, 0, 2, 0, 2, ReadOptions::default(), false)
            .is_ok());
        // One copy at a time.
        e.changed_at.store(0, Ordering::Relaxed);
        e.autosaving.store(true, Ordering::SeqCst);
        assert_eq!(docs.autosave(id).unwrap(), Autosaved::Skipped);
        e.autosaving.store(false, Ordering::SeqCst);
        // Too large last time and not smaller since: not written again.
        e.autosaved.store(u64::MAX, Ordering::Relaxed);
        e.too_large.store(1, Ordering::Relaxed);
        assert_eq!(docs.autosave(id).unwrap(), Autosaved::TooLarge);
        // Closing stops a copy in progress.
        docs.close(id);
        assert!(e.change_waiting());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn m10_files_split_combine_rename_and_tools() {
        use natendo_engine::tools::{Combine, SplitParts};
        // Safe part names.
        assert_eq!(part_name("a/b:c*?", "x"), "a_b_c__");
        assert_eq!(part_name(" con.txt", "x"), "_con.txt");
        assert_eq!(part_name("LPT1", "x"), "_LPT1");
        assert_eq!(part_name("COM10", "x"), "COM10");
        assert_eq!(part_name("ok. . ", "x"), "ok");
        assert_eq!(part_name("", "blank"), "blank");
        assert_eq!(part_name("..\\..\\evil", "x"), ".._.._evil");
        let p = tmp("m10.csv", b"team;n\r\nRed;1\r\nBlue;2\r\nred;3\r\n;4\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        // Split by a column into a fresh folder, then refuse to replace.
        let out = p.parent().unwrap().join("split");
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let spec = SplitParts {
            col: Some(0),
            every: 0,
            keep_first: true,
        };
        let r = docs
            .split_to_files(id, &spec, &out, "m10", false, &|_| {})
            .unwrap();
        assert_eq!((r.files, r.rows, r.replaced), (4, 4, 0));
        let red = std::fs::read_to_string(out.join("m10 - Red.csv")).unwrap();
        assert_eq!(red, "team;n\r\nRed;1\r\n");
        // "red" differs from "Red" only in case: it gets its own name.
        assert!(out.join("m10 - red (2).csv").exists());
        assert!(out.join("m10 - blank.csv").exists());
        let e = docs
            .split_to_files(id, &spec, &out, "m10", false, &|_| {})
            .unwrap_err();
        assert_eq!(e.kind, "exists");
        let r = docs
            .split_to_files(id, &spec, &out, "m10", true, &|_| {})
            .unwrap();
        assert_eq!(r.replaced, 4);
        // Combine two of them back, with the source file named.
        let parts = vec![out.join("m10 - Red.csv"), out.join("m10 - Blue.csv")];
        let info = docs
            .combine_files(
                &parts,
                &Combine {
                    header: true,
                    by_title: true,
                    source: true,
                },
            )
            .unwrap();
        assert_eq!(info.rows, 3);
        assert_eq!(info.cols, 3);
        // Rename keeps the tab working on the new name; never replaces.
        let to = p.parent().unwrap().join("m10-renamed.csv");
        let _ = std::fs::remove_file(&to);
        let info = docs.rename_file(id, &to).unwrap();
        assert_eq!(info.path.as_deref(), Some(to.to_string_lossy().as_ref()));
        assert!(!p.exists() && to.exists());
        let other = p.parent().unwrap().join("m10 - Red.csv");
        std::fs::copy(out.join("m10 - Red.csv"), &other).unwrap();
        assert_eq!(docs.rename_file(id, &other).unwrap_err().kind, "exists");
        docs.set_cells(id, vec![(1, 1, "9".into())]).unwrap();
        docs.save(id, None, false, false, None).unwrap();
        assert!(std::fs::read_to_string(&to).unwrap().contains("Red;9"));
        // Type-ahead from the column's own values.
        assert_eq!(
            docs.typeahead(id, 0, "b", true).unwrap().as_deref(),
            Some("Blue")
        );
        assert_eq!(docs.typeahead(id, 0, "blue", true).unwrap(), None);
        assert_eq!(docs.typeahead(id, 0, "", true).unwrap(), None);
        // Totals with the distinct count.
        let t = docs
            .selection_totals(id, 1, 4, 0, 1, ReadOptions::default(), true)
            .unwrap();
        assert_eq!(t.distinct, Some(3));
        // Compare with itself: all the same.
        let rep = docs
            .new_table_report(
                id,
                TableOp::Compare {
                    other: id,
                    spec: CompareSpec {
                        keys: vec![1],
                        keep_first: true,
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        assert_eq!(rep.compare.unwrap().same, 4);
    }

    #[test]
    fn computed_columns_checks_and_agent_reads() {
        use natendo_engine::validate::{Check, CheckKind};
        let p = tmp("m8.csv", b"price,qty\r\n2,3\r\nx,1\r\n2,3\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        assert_eq!(docs.find_open(&p), Some(id));
        let spec = ComputeSpec {
            expr: "[price] * [qty]".into(),
            title: "total".into(),
            ..Default::default()
        };
        let pv = docs.compute_preview(id, &spec).unwrap();
        assert_eq!((pv.rows.len(), pv.errors), (3, 1));
        let r = docs.data_op(id, DataOp::Compute { spec }).unwrap();
        assert_eq!((r.cols, r.errors), (1, 1));
        assert_eq!(docs.cell_text(id, 1, 2).unwrap().0, "6");
        // A locked file refuses a computed column but can show problems.
        let checks = CheckSpec {
            checks: vec![Check {
                col: Some(0),
                kind: CheckKind::Unique { match_case: false },
            }],
            keep_first: true,
            ..Default::default()
        };
        let rep = docs.run_checks(id, &checks).unwrap();
        assert_eq!(rep.rows, 2);
        assert_eq!(docs.check_marks(id, 0, 10, 0, 5, &checks).unwrap().len(), 2);
        // After an edit the old run's repeats are not used for marks.
        docs.set_cells(id, vec![(3, 0, "9".into())]).unwrap();
        assert!(docs
            .check_marks(id, 0, 10, 0, 5, &checks)
            .unwrap()
            .is_empty());
        docs.run_checks(id, &checks).unwrap();
        assert!(docs
            .check_marks(id, 0, 10, 0, 5, &checks)
            .unwrap()
            .is_empty());
        docs.set_cells(id, vec![(3, 0, "2".into())]).unwrap();
        docs.set_read_only(id, true).unwrap();
        assert!(docs
            .data_op(
                id,
                DataOp::Compute {
                    spec: ComputeSpec {
                        expr: "1".into(),
                        ..Default::default()
                    }
                }
            )
            .is_err());
        let r = docs
            .data_op(
                id,
                DataOp::ShowProblems {
                    spec: checks.clone(),
                    passing: false,
                },
            )
            .unwrap();
        assert_eq!(r.rows, 2);
        assert_eq!(docs.info(id).unwrap().rows, 3);
        // M15: rows that pass, failing rows first, a result column.
        docs.clear_filter(id).unwrap();
        let total = docs.info(id).unwrap().rows;
        let r = docs
            .data_op(
                id,
                DataOp::ShowProblems {
                    spec: checks.clone(),
                    passing: true,
                },
            )
            .unwrap();
        assert_eq!(r.rows, total - 1 - 2);
        docs.clear_filter(id).unwrap();
        docs.set_read_only(id, false).unwrap();
        let r = docs
            .data_op(
                id,
                DataOp::SortProblems {
                    spec: checks.clone(),
                },
            )
            .unwrap();
        assert_eq!(r.groups, 2);
        let cols = docs.info(id).unwrap().cols;
        docs.data_op(
            id,
            DataOp::CheckColumn {
                spec: checks.clone(),
                title: String::new(),
            },
        )
        .unwrap();
        assert_eq!(docs.info(id).unwrap().cols, cols + 1);
        assert_eq!(docs.cell_text(id, 0, cols).unwrap().0, "Check result");
        assert!(docs
            .cell_text(id, 1, cols)
            .unwrap()
            .0
            .starts_with("Fails: "));
        docs.undo(id).unwrap();
        docs.undo(id).unwrap();
        docs.set_read_only(id, true).unwrap();
        docs.data_op(
            id,
            DataOp::ShowProblems {
                spec: checks,
                passing: false,
            },
        )
        .unwrap();
        // The agent link's reads are capped by cells.
        let rows = docs.read_rows(id, 0, 100, 4).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(docs.read_rows(id, 0, 100, 10_000).unwrap().len(), 3);
        assert_eq!(docs.ids(), vec![id]);
    }

    #[test]
    fn read_only_refuses_changes_but_not_filters() {
        let a = tmp("m7lock.csv", b"k,v\r\n1,a\r\n2,b\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&a, false).unwrap().id;
        wait(&docs, id);
        assert!(docs.set_read_only(id, true).unwrap().read_only);
        let e = docs.set_cells(id, vec![(1, 1, "x".into())]).unwrap_err();
        assert_eq!(e.kind, "read_only");
        let e = docs
            .sort(
                id,
                SortSpec {
                    keys: vec![],
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert_eq!(e.kind, "read_only");
        assert_eq!(
            docs.save(id, None, false, false, None).unwrap_err().kind,
            "read_only"
        );
        // Filtering only changes what is shown.
        docs.clear_filter(id).unwrap();
        assert!(!docs.set_read_only(id, false).unwrap().read_only);
        docs.set_cells(id, vec![(1, 1, "x".into())]).unwrap();
        assert_eq!(docs.cell_text(id, 1, 1).unwrap().0, "x");
    }

    #[test]
    fn lock_holds_however_the_file_is_named() {
        let a = tmp("m7Lock2.csv", b"k,v\r\n1,a\r\n");
        let original = std::fs::read(&a).unwrap();
        let docs = Docs::default();
        // Recent files and the Files panel carry the resolved form.
        let resolved = std::fs::canonicalize(&a).unwrap();
        let id = docs.open_path(&resolved, false).unwrap().id;
        wait(&docs, id);
        docs.set_read_only(id, true).unwrap();
        let mut names = vec![a.clone(), resolved.clone()];
        if cfg!(windows) {
            let upper = PathBuf::from(a.to_string_lossy().to_uppercase());
            assert!(upper.exists());
            names.push(upper);
            names.push(PathBuf::from(a.to_string_lossy().replace('\\', "/")));
        }
        let other = docs
            .open_path(&tmp("m7other.csv", b"x\r\n"), false)
            .unwrap()
            .id;
        wait(&docs, other);
        let o = ExportOptions {
            format: ExportFormat::Markdown,
            ..Default::default()
        };
        for p in &names {
            // Save As from the locked tab, from another tab, and export.
            for doc in [id, other] {
                let e = docs
                    .save(doc, Some(p.clone()), true, true, None)
                    .unwrap_err();
                assert_eq!(e.kind, "read_only", "save {p:?}");
                let e = docs.export_file(doc, p, &o).unwrap_err();
                assert_eq!(e.kind, "read_only", "export {p:?}");
            }
            assert!(docs.check_writable(p).is_err());
        }
        assert_eq!(std::fs::read(&a).unwrap(), original);
        // Opening the same file again (another layout, line by line, or
        // from Recent files) opens it locked too.
        let again = docs.open_with(&a, true, None).unwrap();
        assert!(again.read_only);
        let e = docs
            .set_cells(again.id, vec![(0, 0, "x".into())])
            .unwrap_err();
        assert_eq!(e.kind, "read_only");
        // Save As to another file is still allowed; unlocking frees it.
        let elsewhere = a.with_file_name("m7lock2-copy.csv");
        let _ = std::fs::remove_file(&elsewhere);
        docs.save(other, Some(elsewhere.clone()), true, true, None)
            .unwrap();
        docs.set_read_only(id, false).unwrap();
        docs.set_read_only(again.id, false).unwrap();
        assert!(docs.check_writable(&a).is_ok());
        assert!(docs.open_path(&a, false).map(|i| !i.read_only).unwrap());
    }

    #[test]
    fn clean_split_name_and_fill_by_example() {
        use natendo_engine::clean::{CaseTo, Cleaner, NamePart, Step};
        use natendo_engine::find::Scope;
        let a = tmp("m6a.csv", b"name,initials\r\n ann  lee ,\r\nbo ray,BR\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&a, false).unwrap().id;
        wait(&docs, id);
        let cell = |r: u64, c: u32| docs.cell_text(id, r, c).unwrap().0;
        let spec = CleanSpec {
            cleaner: Cleaner {
                version: 1,
                name: String::new(),
                steps: vec![
                    Step::Spaces {
                        how: natendo_engine::clean::Spaces::Collapse,
                    },
                    Step::Case { to: CaseTo::Title },
                ],
            },
            scope: Scope::Columns { cols: vec![0] },
            skip_first: true,
            read: Default::default(),
        };
        let p = docs.clean_preview(id, &spec, 20_000.0).unwrap();
        assert_eq!((p.changed, p.changes[0].after.as_str()), (2, "Ann Lee"));
        assert!(docs.clean_preview(id, &spec, f64::NAN).is_err());
        let r = docs
            .data_op(
                id,
                DataOp::Clean {
                    spec,
                    now: 20_000.0,
                },
            )
            .unwrap();
        assert_eq!(r.cells, 2);
        assert_eq!(cell(1, 0), "Ann Lee");
        // Fill by example: "BR" from "Bo Ray" teaches initials.
        let r = docs
            .data_op(
                id,
                DataOp::FillExample {
                    spec: ExampleSpec {
                        col: 1,
                        skip_first: true,
                    },
                },
            )
            .unwrap();
        assert_eq!((r.cells, r.examples), (1, 1));
        assert!(r.pattern.is_some());
        assert_eq!(cell(1, 1), "AL");
        // Split name adds columns.
        let r = docs
            .data_op(
                id,
                DataOp::SplitName {
                    spec: SplitName {
                        col: 0,
                        parts: vec![NamePart::First, NamePart::Last],
                        keep_original: true,
                        keep_first: true,
                    },
                },
            )
            .unwrap();
        assert_eq!(r.cols, 2);
        assert_eq!((cell(0, 2), cell(2, 2)), ("Last name".into(), "Ray".into()));
        docs.undo(id).unwrap();
        docs.undo(id).unwrap();
        docs.undo(id).unwrap();
        assert_eq!(cell(1, 0), " ann  lee ");
    }

    #[test]
    fn sqlite_tables() {
        let a = tmp("m15q_a.csv", b"id,name\r\n1,Ann\r\n");
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        wait(&docs, ia);
        let db = a.with_extension("sqlite");
        let _ = std::fs::remove_file(&db);
        assert_eq!(docs.save_sqlite(ia, &db, "people", true, false).unwrap(), 1);
        let info = docs.open_sqlite(&db, "people").unwrap();
        assert_eq!((info.rows, info.cols), (2, 2));
        assert_eq!(
            info.name,
            format!("{}: people", db.file_name().unwrap().to_string_lossy())
        );
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn notes_beside_the_file() {
        let a = tmp("m15n_a.csv", b"id,name\r\n1,Ann\r\n2,Bo\r\n");
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        wait(&docs, ia);
        let side = docs.note_set(ia, 2, 3, "moved away").unwrap();
        assert!(side.exists());
        let marks = docs.note_marks(ia, 0, 10).unwrap();
        assert_eq!((marks[0].row, marks[0].flag), (2, 3));
        assert!(
            docs.note_set(ia, 0, 1, "x").is_err(),
            "the title row has no key"
        );
        // A second tab of the file reads the notes file.
        docs.close(ia);
        let ib = docs.open_path(&a, false).unwrap().id;
        wait(&docs, ib);
        assert_eq!(docs.note_list(ib, 10).unwrap().0.len(), 1);
        let info = docs.notes_show(ib, 3).unwrap();
        assert_eq!(info.rows, 2);
        docs.note_set(ib, 1, 0, "").unwrap();
        assert!(!side.exists(), "the last note gone: no file left");
    }

    #[test]
    fn encrypted_files_open_private() {
        let a = tmp("m15e_a.csv", b"name,ssn\r\nAnn,1\r\n");
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        wait(&docs, ia);
        let enc = a.with_extension("ndenc");
        assert!(docs.save_encrypted(ia, &enc, "short").is_err());
        assert_eq!(
            docs.save_encrypted(ia, &enc, "a long password here")
                .unwrap(),
            2
        );
        assert!(docs.open_encrypted(&enc, "the wrong password").is_err());
        let info = docs.open_encrypted(&enc, "a long password here").unwrap();
        assert_eq!(info.rows, 2);
        assert!(docs.is_private(info.id).unwrap());
        assert_eq!(docs.cell_text(info.id, 1, 1).unwrap().0, "1");
        // Save keeps it encrypted, in the same file.
        assert_eq!(
            docs.encrypted_from(info.id).unwrap().as_deref(),
            Some(enc.as_path())
        );
        assert_eq!(
            docs.encrypted_from(ia).unwrap(),
            None,
            "a plain file keeps saving plain"
        );
        docs.set_cells(info.id, vec![(1, 0, "Bea".into())]).unwrap();
        docs.save_encrypted_again(info.id, "another long password")
            .unwrap();
        assert!(!std::fs::read(&enc).unwrap().starts_with(b"name"));
        let again = docs.open_encrypted(&enc, "another long password").unwrap();
        assert_eq!(docs.cell_text(again.id, 1, 0).unwrap().0, "Bea");
        let _ = std::fs::remove_file(&enc);
    }

    #[test]
    fn changes_from_a_newer_file() {
        use natendo_engine::updates::{Choice, UpdateChoice, UpdateSpec};
        let a = tmp("m15u_a.csv", b"id,ward\r\n1,4\r\n2,5\r\n");
        let b = tmp("m15u_b.csv", b"id,ward\r\n1,7\r\n2,5\r\n3,9\r\n");
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        let ib = docs.open_path(&b, false).unwrap().id;
        wait(&docs, ia);
        wait(&docs, ib);
        let spec = UpdateSpec {
            keys: vec![0],
            keep_first: true,
            ..Default::default()
        };
        let list = docs.updates_find(ia, ib, &spec).unwrap();
        assert_eq!(
            (list.cells_total, list.added_total, list.removed_total),
            (1, 1, 0)
        );
        let all = Choice {
            all: true,
            picked: vec![],
        };
        let choice = UpdateChoice {
            cells: all.clone(),
            added: all.clone(),
            removed: all,
        };
        let info = docs.updates_accept(ia, &choice).unwrap();
        assert_eq!(info.rows, 4);
        assert_eq!(docs.cell_text(ia, 1, 1).unwrap().0, "7");
        docs.undo(ia).unwrap();
        assert_eq!(docs.info(ia).unwrap().rows, 3);
        // Used once.
        assert!(docs.updates_accept(ia, &choice).is_err());
    }

    #[test]
    fn street_lookup_from_another_tab() {
        use natendo_engine::streets::{IndexCols, StreetSpec};
        let a = tmp(
            "m15s_a.csv",
            b"addr\r\n12 Oak Ave\r\n13 Oak Ave\r\n9 Elm St\r\n",
        );
        let b = tmp(
            "m15s_b.csv",
            b"street,lo,hi,side,pct\r\nOak Avenue,2,98,even,0101\r\nOak Avenue,1,99,odd,0102\r\n",
        );
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        let ib = docs.open_path(&b, false).unwrap().id;
        wait(&docs, ia);
        wait(&docs, ib);
        let cols = IndexCols {
            street: 0,
            low: 1,
            high: 2,
            side: Some(3),
            area: None,
            bring: vec![4],
            header: true,
        };
        let spec = StreetSpec {
            address: 0,
            keep_first: true,
            note: true,
            ..Default::default()
        };
        let (info, rep) = docs.street_lookup(ia, ib, &cols, &spec).unwrap();
        assert_eq!((info.cols, rep.found, rep.no_street), (3, 2, 1));
        let cell = |r: u64, c: u32| docs.cell_text(ia, r, c).unwrap().0;
        assert_eq!(
            (cell(1, 1), cell(2, 1), cell(3, 2)),
            (
                "0101".into(),
                "0102".into(),
                "street not in the index".into()
            )
        );
        // Not with itself.
        assert!(docs.street_lookup(ia, ia, &cols, &spec).is_err());
    }

    #[test]
    fn match_people_across_tabs() {
        use natendo_engine::fuzzy::{FieldKind, MatchField, MatchSpec};
        let a = tmp("m15m_a.csv", b"first,last\r\nWilliam,Smith\r\nAnn,Lee\r\n");
        let b = tmp(
            "m15m_b.csv",
            b"given,family,ward\r\nBill,Smyth,4\r\nZed,Quill,9\r\n",
        );
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        let ib = docs.open_path(&b, false).unwrap().id;
        wait(&docs, ia);
        wait(&docs, ib);
        let spec = MatchSpec {
            fields: vec![
                MatchField {
                    col: 0,
                    other_col: 0,
                    kind: FieldKind::Name,
                    weight: 1.0,
                },
                MatchField {
                    col: 1,
                    other_col: 1,
                    kind: FieldKind::Name,
                    weight: 1.0,
                },
            ],
            min_score: 80.0,
            ..Default::default()
        };
        let rep = docs.match_find(ia, Some(ib), &spec, &[2]).unwrap();
        assert_eq!(rep.total, 1);
        let c = &rep.listed[0];
        assert_eq!((c.row, c.other_row), (1, 1));
        // A change in between: find again.
        docs.set_cells(ia, vec![(2, 0, "Anne".into())]).unwrap();
        assert!(docs
            .match_accept(ia, &[(c.id, c.other_row, c.score)])
            .is_err());
        let rep = docs.match_find(ia, Some(ib), &spec, &[2]).unwrap();
        let c = &rep.listed[0];
        let info = docs
            .match_accept(ia, &[(c.id, c.other_row, c.score)])
            .unwrap();
        assert_eq!(info.cols, 5);
        let cell = |r: u64, col: u32| docs.cell_text(ia, r, col).unwrap().0;
        assert_eq!(
            (cell(0, 2), cell(1, 2), cell(1, 4)),
            ("Match row".into(), "2".into(), "4".into())
        );
        // Once used, the run is gone.
        assert!(docs
            .match_accept(ia, &[(c.id, c.other_row, c.score)])
            .is_err());
    }

    #[test]
    fn recipes_record_and_run() {
        let a = tmp("m14r_a.csv", b"id,name,junk\r\n2,Bo,x\r\n1,Al,y\r\n");
        let b = tmp("m14r_b.csv", b"id,name,junk\r\n9,Zed,q\r\n");
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        wait(&docs, ia);
        // Delete a column, undo it, then insert one: only the insert stays.
        docs.delete_cols(ia, 0, 1).unwrap();
        docs.undo(ia).unwrap();
        docs.insert_cols(ia, 3, 1).unwrap();
        docs.delete_cols(ia, 2, 1).unwrap();
        let r = docs.recipe_of(ia, true, "tidy").unwrap();
        assert_eq!(r.columns, vec!["id", "name", "junk"]);
        assert_eq!(r.steps.len(), 2);
        assert!(matches!(
            r.steps[0],
            RecipeOp::InsertCols { at: 3, count: 1 }
        ));
        // Undone steps leave the recipe.
        docs.undo(ia).unwrap();
        assert_eq!(docs.recipe_of(ia, true, "").unwrap().steps.len(), 1);
        docs.redo(ia).unwrap();
        assert_eq!(docs.recipe_of(ia, true, "").unwrap().steps.len(), 2);
        // Run it on another file with the same columns.
        let ib = docs.open_path(&b, false).unwrap().id;
        wait(&docs, ib);
        assert!(docs.recipe_problems(ib, &r).unwrap().is_empty());
        docs.run_recipe(ib, &r, 20_000.0, false).unwrap();
        let cell = |r: u64, c: u32| docs.cell_text(ib, r, c).unwrap().0;
        assert_eq!(
            (cell(0, 0), cell(0, 1), cell(0, 2)),
            ("id".into(), "name".into(), String::new())
        );
        assert_eq!(docs.info(ib).unwrap().cols, 3);
        // Each step is one undo step.
        docs.undo(ib).unwrap();
        assert_eq!(docs.info(ib).unwrap().cols, 4);
        // Different columns are refused unless forced.
        let mut other = r.clone();
        other.columns = vec!["x".into()];
        assert!(docs.run_recipe(ib, &other, 20_000.0, false).is_err());
        // Saved and read back; never over an open file.
        let path = std::env::temp_dir().join("m14r.recipe.json");
        assert!(docs.save_recipe(ia, &r, &a).is_err());
        docs.save_recipe(ia, &r, &path).unwrap();
        assert_eq!(read_recipe(&path).unwrap().steps.len(), 2);
    }

    #[test]
    fn privacy_tools() {
        use natendo_engine::clean::{Cleaner, Step};
        use natendo_engine::privacy::{ColumnAction, PrivateKind, SafeCopy, Secret};
        let a = tmp(
            "m13a.csv",
            b"name,ssn\r\nAnn,123-45-6789\r\nBob,987-65-4321\r\n",
        );
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        wait(&docs, ia);
        let f = docs
            .find_private(ia, true, ReadOptions::default(), 20_770.0)
            .unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].col, f[0].kind), (1, PrivateKind::Ssn));
        // Private mode: no recovery copy even with unsaved changes.
        docs.set_cells(ia, vec![(1, 0, "Anna".into())]).unwrap();
        docs.set_private(ia, true).unwrap();
        assert!(matches!(docs.autosave(ia).unwrap(), Autosaved::Skipped));
        docs.set_private(ia, false).unwrap();
        // A share-safe copy beside the file; the file itself is refused.
        let scramble = |p: &str| ColumnAction::Clean {
            cleaner: Cleaner {
                version: 1,
                name: String::new(),
                steps: vec![Step::Scramble {
                    phrase: Secret::new(p),
                    loose: false,
                }],
            },
        };
        let spec = SafeCopy {
            columns: vec![ColumnAction::Keep, scramble("a long enough phrase")],
            header: true,
            read: ReadOptions::default(),
        };
        let out = a.with_file_name("m13a (share-safe).csv");
        let rep = docs.save_safe_copy(ia, &out, &spec, 0.0).unwrap();
        assert_eq!((rep.rows, rep.changed), (2, 2));
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.starts_with("name,ssn\r\nAnna,"), "{text}");
        assert!(!text.contains("6789"));
        assert!(docs.save_safe_copy(ia, &a, &spec, 0.0).is_err());
        let short = SafeCopy {
            columns: vec![ColumnAction::Keep, scramble("short")],
            ..spec
        };
        assert!(docs.save_safe_copy(ia, &out, &short, 0.0).is_err());
        let _ = std::fs::remove_file(out);
    }

    /// Review M13 H2/M4: tabs made from a private tab, and reopened private
    /// files, stay private (no recovery copies, no earlier saves).
    #[test]
    fn private_mode_reaches_new_tabs_and_reopens() {
        use natendo_engine::tools::{Sample, SampleSize};
        let a = tmp("m13h2.csv", b"name,ssn\r\nAnn,1\r\nBob,2\r\nCy,3\r\n");
        let docs = Docs::default();
        let private = a.clone();
        docs.set_private_check(Arc::new(move |p: &Path| p == private.as_path()));
        // Opening (and reopening) a private file marks the tab.
        let ia = docs.open_path(&a, false).unwrap().id;
        wait(&docs, ia);
        assert!(docs.is_private(ia).unwrap());
        let again = docs.open_with(&a, false, None).unwrap().id;
        assert!(docs.is_private(again).unwrap());
        docs.close(again);
        // A random sample of it is private: autosave is skipped.
        let sample = docs
            .new_table(
                ia,
                TableOp::Sample {
                    spec: Sample {
                        size: SampleSize::Rows { n: 2 },
                        seed: 1,
                        keep_first: true,
                    },
                },
            )
            .unwrap()
            .id;
        assert!(docs.is_private(sample).unwrap());
        docs.set_cells(sample, vec![(1, 0, "x".into())]).unwrap();
        assert!(matches!(docs.autosave(sample).unwrap(), Autosaved::Skipped));
        // A tab of another file is not.
        let b = tmp("m13h2b.csv", b"a\r\n1\r\n");
        let ib = docs.open_path(&b, false).unwrap().id;
        wait(&docs, ib);
        assert!(!docs.is_private(ib).unwrap());
        // The list changes: open tabs follow.
        let both = (a.clone(), b.clone());
        docs.set_private_check(Arc::new(move |p: &Path| {
            p == both.0.as_path() || p == both.1.as_path()
        }));
        docs.private_changed();
        assert!(docs.is_private(ib).unwrap());
        docs.set_private_check(Arc::new(|_: &Path| false));
        docs.private_changed();
        assert!(!docs.is_private(ib).unwrap());
        // Review round 2, N3: rows brought in from a private tab make the
        // target private.
        let only_a = a.clone();
        docs.set_private_check(Arc::new(move |p: &Path| p == only_a.as_path()));
        docs.private_changed();
        assert!(docs.is_private(ia).unwrap());
        let c = tmp("m13n3.csv", b"name,ssn\r\nZed,9\r\n");
        let ic = docs.open_path(&c, false).unwrap().id;
        wait(&docs, ic);
        assert!(!docs.is_private(ic).unwrap());
        docs.data_op(
            ic,
            DataOp::Append {
                other: ia,
                spec: AppendSpec {
                    other_header: true,
                    keep_first: true,
                    ..Default::default()
                },
            },
        )
        .unwrap();
        assert!(docs.is_private(ic).unwrap());
        // Round 3, R1: switching private mode for other files keeps it.
        docs.set_private_check(Arc::new(|_: &Path| false));
        docs.private_changed();
        assert!(docs.is_private(ic).unwrap());
        assert!(!docs.is_private(ia).unwrap());
        // Only the user turns it off.
        docs.set_private(ic, false).unwrap();
        docs.private_changed();
        assert!(!docs.is_private(ic).unwrap());
        let only_a = a.clone();
        docs.set_private_check(Arc::new(move |p: &Path| p == only_a.as_path()));
        docs.private_changed();
        // N1: a split from a private tab reports each file it wrote.
        let out = std::env::temp_dir().join(format!("natendo-n1-{}", std::process::id()));
        std::fs::create_dir_all(&out).unwrap();
        let written = Mutex::new(Vec::new());
        let r = docs
            .split_to_files(
                ia,
                &natendo_engine::tools::SplitParts {
                    col: None,
                    every: 1,
                    keep_first: true,
                },
                &out,
                "n1",
                false,
                &|p| written.lock().unwrap().push(p.to_path_buf()),
            )
            .unwrap();
        assert_eq!(written.lock().unwrap().len() as u64, r.files);
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn editing_tools() {
        use natendo_engine::editing::{ExtractWhat, GroupFunc};
        let a = tmp(
            "m12a.csv",
            b"house,name,code
A,Ann Lee,ny
B,Bob Ray,ca
A,Cy Fox,NY
",
        );
        let b = tmp(
            "m12b.csv",
            b"old,new
ny,New York
ca,California
",
        );
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        let ib = docs.open_path(&b, false).unwrap().id;
        wait(&docs, ia);
        wait(&docs, ib);
        let t = docs
            .value_tally(
                ia,
                &TallySpec {
                    col: 2,
                    keep_first: true,
                },
            )
            .unwrap();
        assert_eq!(t.values.len(), 3);
        assert_eq!(t.look_alikes.len(), 1);
        // A translation list from another tab, not matching case.
        let r = docs
            .data_op(
                ia,
                DataOp::MapValues {
                    spec: MapValues {
                        cols: vec![2],
                        keep_first: true,
                        ..Default::default()
                    },
                    from: Some(MapFrom {
                        doc: ib,
                        old: 0,
                        new: 1,
                        header: true,
                    }),
                },
            )
            .unwrap();
        assert_eq!(r.cells, 3);
        let cell = |r: u64, c: u32| docs.cell_text(ia, r, c).unwrap().0;
        assert_eq!(cell(3, 2), "New York");
        assert!(docs
            .data_op(
                ia,
                DataOp::MapValues {
                    spec: MapValues {
                        cols: vec![2],
                        ..Default::default()
                    },
                    from: Some(MapFrom {
                        doc: ib,
                        old: 9,
                        new: 1,
                        header: true
                    }),
                },
            )
            .is_err());
        let r = docs
            .data_op(
                ia,
                DataOp::GroupCalc {
                    spec: GroupCalc {
                        keys: vec![0],
                        func: GroupFunc::Count,
                        title: "Household".into(),
                        keep_first: true,
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        assert_eq!((r.cols, r.groups), (1, 2));
        assert_eq!(cell(1, 3), "2");
        docs.data_op(
            ia,
            DataOp::Extract {
                spec: Extract {
                    col: 1,
                    what: ExtractWhat::LastWord,
                    remove: true,
                    keep_first: true,
                    title: "Last".into(),
                },
            },
        )
        .unwrap();
        assert_eq!((cell(1, 1), cell(1, 2)), ("Ann".into(), "Lee".into()));
        docs.data_op(
            ia,
            DataOp::Arrange {
                spec: Arrange {
                    order: vec![2, 1],
                    names: vec!["Surname".into(), String::new()],
                    keep_first: true,
                },
            },
        )
        .unwrap();
        assert_eq!(docs.info(ia).unwrap().cols, 2);
        assert_eq!(
            (cell(0, 0), cell(1, 0), cell(1, 1)),
            ("Surname".into(), "Lee".into(), "Ann".into())
        );
        for _ in 0..4 {
            docs.undo(ia).unwrap();
        }
        assert_eq!(cell(1, 2), "ny");
    }

    #[test]
    fn join_lists_previews_and_lookups() {
        let a = tmp("m11a.csv", b"id,name\r\n001,Ann\r\n2,Bob\r\n3,Cy\r\n");
        let b = tmp("m11 codes.csv", b"code,city\r\n1,Rome\r\n3,Oslo\r\n");
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        let ib = docs.open_path(&b, false).unwrap().id;
        wait(&docs, ia);
        wait(&docs, ib);
        let spec = JoinSpec {
            keys: vec![0],
            other_keys: vec![0],
            keep_first: true,
            other_header: true,
            loose: true,
            ..Default::default()
        };
        // The preview changes nothing and counts the whole table.
        let p = docs.join_preview(ia, Some(ib), None, None, &spec).unwrap();
        assert_eq!((p.report.matched, p.report.unmatched), (2, 1));
        assert_eq!(p.titles, vec!["id", "name", "city"]);
        assert_eq!(docs.info(ia).unwrap().cols, 2);
        // The indexed other table is kept for the next preview.
        let key = docs
            .joins
            .lock()
            .unwrap()
            .get(&ia)
            .unwrap()
            .source
            .as_ref()
            .unwrap()
            .0
            .clone();
        docs.join_preview(ia, Some(ib), None, None, &spec).unwrap();
        assert_eq!(
            docs.joins.lock().unwrap()[&ia].source.as_ref().unwrap().0,
            key
        );
        // Review round 2 N3: another tab's Join screen keeps its own.
        docs.join_preview(ib, Some(ia), None, None, &spec).unwrap();
        assert_eq!(
            docs.joins.lock().unwrap()[&ia].source.as_ref().unwrap().0,
            key
        );
        // A kept list, previewed and run by its number.
        let lid = docs.join_list(ia, "1\n3\n".into()).unwrap();
        let p = docs
            .join_preview(
                ia,
                None,
                None,
                Some(lid),
                &JoinSpec {
                    write: natendo_engine::join::JoinWrite::Nothing,
                    keep_rows: natendo_engine::join::KeepRows::Unmatched,
                    ..spec.clone()
                },
            )
            .unwrap();
        assert_eq!(p.report.removed, 2);
        // Long texts are refused.
        let long = JoinSpec {
            not_found: "x".repeat(MAX_EDIT_CHARS + 1),
            ..spec.clone()
        };
        assert!(docs.join_preview(ia, Some(ib), None, None, &long).is_err());
        docs.join_close(ia);
        assert!(!docs.joins.lock().unwrap().contains_key(&ia));
        assert!(docs.joins.lock().unwrap().contains_key(&ib));
        assert!(docs.join_preview(ia, None, None, Some(lid), &spec).is_err());
        // Keep only the rows on a pasted list.
        let keep = JoinSpec {
            write: natendo_engine::join::JoinWrite::Nothing,
            keep_rows: natendo_engine::join::KeepRows::Matched,
            ..spec.clone()
        };
        let r = docs
            .data_op(
                ia,
                DataOp::Join {
                    other: None,
                    list: Some("2\n3\n".into()),
                    list_id: None,
                    spec: keep.clone(),
                },
            )
            .unwrap();
        assert_eq!(r.join.unwrap().removed, 1);
        assert_eq!(docs.info(ia).unwrap().rows, 3);
        docs.undo(ia).unwrap();
        assert!(docs
            .data_op(
                ia,
                DataOp::Join {
                    other: None,
                    list: Some("x".repeat(MAX_JOIN_LIST_CHARS + 1)),
                    list_id: None,
                    spec: keep,
                },
            )
            .is_err());
        // LOOKUP finds the tab by its name without the extension.
        let cs = ComputeSpec {
            expr: "LOOKUP([id], \"M11 codes\", \"code\", \"city\", \"-\")".into(),
            title: "city".into(),
            ..Default::default()
        };
        let p = docs.compute_preview(ia, &cs).unwrap();
        let got: Vec<&str> = p.rows.iter().map(|r| r.1.as_str()).collect();
        assert_eq!(got, ["Rome", "-", "Oslo"]);
        docs.data_op(ia, DataOp::Compute { spec: cs.clone() })
            .unwrap();
        assert_eq!(docs.cell_text(ia, 3, 2).unwrap().0, "Oslo");
        let bad = ComputeSpec {
            expr: "LOOKUP([id], \"nope\", \"code\", \"city\")".into(),
            ..Default::default()
        };
        assert!(docs.compute_preview(ia, &bad).is_err());
        let bad = ComputeSpec {
            expr: "LOOKUP([id], \"m11 codes.csv\", \"code\", \"town\")".into(),
            ..Default::default()
        };
        assert!(docs.compute_preview(ia, &bad).is_err());
        // Review round 2 N2: LOOKUP tables go when the screen or the tab
        // they were read from closes.
        assert_eq!(docs.lookups.lock().unwrap().len(), 1);
        docs.lookup_close();
        assert!(docs.lookups.lock().unwrap().is_empty());
        docs.compute_preview(ia, &cs).unwrap();
        docs.join_preview(ia, Some(ib), None, None, &spec).unwrap();
        docs.close(ib);
        assert!(docs.lookups.lock().unwrap().is_empty());
        assert!(docs.joins.lock().unwrap()[&ia].source.is_none());
    }

    #[test]
    fn join_append_export_and_save_as() {
        let a = tmp("m5a.csv", b"id,name\r\n1,Ann\r\n2,Bob\r\n");
        let b = tmp("m5b.csv", b"id,city\r\n2,Rome\r\n3,Oslo\r\n");
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        let ib = docs.open_path(&b, false).unwrap().id;
        wait(&docs, ia);
        wait(&docs, ib);
        let r = docs
            .data_op(
                ia,
                DataOp::Join {
                    other: Some(ib),
                    list: None,
                    list_id: None,
                    spec: JoinSpec {
                        keys: vec![0],
                        other_keys: vec![0],
                        keep_first: true,
                        other_header: true,
                        match_case: true,
                        add_unmatched: true,
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        let j = r.join.unwrap();
        assert_eq!((j.matched, j.unmatched, j.added), (1, 1, 1));
        let cell = |r: u64, c: u32| docs.cell_text(ia, r, c).unwrap().0;
        assert_eq!(
            (cell(0, 2), cell(2, 2), cell(3, 0)),
            ("city".into(), "Rome".into(), "3".into())
        );
        docs.undo(ia).unwrap();
        // Append a document to itself (read once, never locked twice).
        docs.data_op(
            ia,
            DataOp::Append {
                other: ia,
                spec: AppendSpec {
                    other_header: true,
                    ..Default::default()
                },
            },
        )
        .unwrap();
        assert_eq!(docs.info(ia).unwrap().rows, 5);
        assert!(docs
            .data_op(
                ia,
                DataOp::Join {
                    other: Some(999),
                    list: None,
                    list_id: None,
                    spec: JoinSpec::default()
                }
            )
            .is_err());
        // Export preview and clipboard text.
        let o = ExportOptions {
            format: ExportFormat::Markdown,
            ..Default::default()
        };
        let p = docs.export_preview(ia, &o).unwrap();
        assert!(p.starts_with("| id | name |"));
        assert_eq!(docs.export_text(ia, &o).unwrap().lines().count(), 6);
        let long = ExportOptions {
            row: "x".repeat(MAX_EXPORT_OPTION_CHARS + 1),
            ..o.clone()
        };
        assert_eq!(
            docs.export_preview(ia, &long).unwrap_err().kind,
            "too_large"
        );
        let out = a.with_file_name("m5a.xlsx");
        let rep = docs
            .export_file(
                ia,
                &out,
                &ExportOptions {
                    format: ExportFormat::Xlsx,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!((rep.rows, rep.sheets), (4, 1));
        // Save As with options: semicolons, every value quoted, LF.
        let target = a.with_file_name("m5a-out.csv");
        let opts = SaveAs {
            delimiter: ";".into(),
            quote: Some("\"".into()),
            quoting: Quoting::All,
            encoding: "UTF-8".into(),
            bom: false,
            line_break: "LF".into(),
            convert_line_breaks: true,
        };
        let info = docs
            .save(ia, Some(target.clone()), false, false, Some(&opts))
            .unwrap();
        assert_eq!(info.delimiter, ";");
        assert!(std::fs::read_to_string(&target)
            .unwrap()
            .starts_with("\"id\";\"name\"\n\"1\";\"Ann\"\n"));
        let bad = SaveAs {
            quote: None,
            ..opts.clone()
        };
        assert_eq!(
            docs.save(ia, Some(target.clone()), false, false, Some(&bad))
                .unwrap_err()
                .kind,
            "bad_layout"
        );
        let bad = SaveAs {
            encoding: "nope".into(),
            ..opts
        };
        assert!(docs
            .save(ia, Some(target), false, false, Some(&bad))
            .is_err());
    }

    #[test]
    fn layout_choices_preview_and_reopen() {
        let p = tmp(
            "lay.txt",
            b"Stock list\n\nname;qty\nbolt;4\nnut;10\nwasher;25\n",
        );
        let docs = Docs::default();
        let info = docs.open_path(&p, false).unwrap();
        let id = info.id;
        assert_eq!((info.delimiter.as_str(), info.preamble_lines), (";", 2));
        assert!(info.layout_confidence.unwrap() > 0.5);
        let c = docs.layout_choices(id).unwrap();
        assert_eq!(c.current, c.choices[0].layout);
        assert_eq!(c.preamble, "Stock list\n\n");
        assert_eq!(c.choices[0].preview[0], vec!["name", "qty"]);
        // Read it as one column with nothing set aside instead.
        let other = Layout {
            delimiter: "|".into(),
            quote: None,
            escape: None,
            merge: false,
            skip_lines: 0,
        };
        assert_eq!(
            docs.preview_layout(id, &other).unwrap()[0],
            vec!["Stock list"]
        );
        let again = docs.open_with(&p, false, Some(&other)).unwrap();
        let again = wait(&docs, again.id);
        assert_eq!((again.rows, again.cols, again.preamble_lines), (6, 1, 0));
        assert_eq!(again.layout_confidence, None);
        // Bad input from the UI is refused.
        for bad in [
            Layout {
                delimiter: String::new(),
                ..other.clone()
            },
            Layout {
                delimiter: "abc".into(),
                ..other.clone()
            },
            Layout {
                delimiter: "é".into(),
                ..other.clone()
            },
            Layout {
                quote: Some("|".into()),
                ..other.clone()
            },
            Layout {
                escape: Some("\\".into()),
                ..other.clone()
            },
            Layout {
                skip_lines: MAX_SKIP_LINES + 1,
                ..other.clone()
            },
        ] {
            assert_eq!(
                docs.preview_layout(id, &bad).unwrap_err().kind,
                "bad_layout"
            );
        }
    }

    #[test]
    fn open_edit_copy_undo_save() {
        let p = tmp("a.csv", b"a,b\r\n1,\"x\ty\"\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        let info = wait(&docs, id);
        assert_eq!((info.rows, info.cols), (2, 2));
        assert_eq!(info.delimiter, ",");
        assert_eq!(
            docs.copy(id, 0, 2, 0, 2).unwrap(),
            "a\tb\r\n1\t\"x\ty\"\r\n"
        );
        let i = docs.set_cells(id, vec![(0, 0, "A".into())]).unwrap();
        assert!(i.modified && i.can_undo);
        let i = docs.fill(id, 1, 1, 0, 2, String::new()).unwrap();
        assert!(i.modified);
        assert_eq!(docs.cell_text(id, 1, 1).unwrap(), (String::new(), false));
        docs.undo(id).unwrap();
        docs.save(id, None, false, false, None).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"A,b\r\n1,\"x\ty\"\r\n");
        docs.close(id);
        assert!(docs.info(id).is_err());
    }

    #[test]
    fn changed_on_disk_is_reported_then_overwritable() {
        let p = tmp("b.csv", b"a\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        docs.set_cells(id, vec![(0, 0, "x".into())]).unwrap();
        std::fs::write(&p, b"other program\n").unwrap();
        let e = docs.save(id, None, false, false, None).unwrap_err();
        assert_eq!(e.kind, "changed_on_disk");
        docs.save(id, None, true, false, None).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"x\n");
    }

    #[test]
    fn caps_reads_and_writes() {
        let big = format!("\"{}\",b\n", "é".repeat(MAX_EDIT_CHARS + 10));
        let p = tmp("c.csv", big.as_bytes());
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        // Editor read is capped and flagged.
        let (text, cut) = docs.cell_text(id, 0, 0).unwrap();
        assert!(cut);
        assert_eq!(text.chars().count(), MAX_EDIT_CHARS);
        // Display read is capped at DISPLAY_CHARS, whatever the UI asks for.
        let bytes = docs.viewport(id, 0, 1, 0, 1, u32::MAX).unwrap();
        let head = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        assert_eq!(head & 1, 1);
        assert_eq!((head >> 1) as usize, DISPLAY_CHARS * 2); // 'é' is 2 bytes
        assert_eq!(
            docs.viewport(id, 0, 1000, 0, 1000, 10).unwrap_err().kind,
            "too_large"
        );
        // Copy refuses rather than silently cutting a long cell.
        assert_eq!(docs.copy(id, 0, 1, 0, 1).unwrap_err().kind, "too_large");
        // Writes are capped.
        assert_eq!(
            docs.fill(id, 0, 1_000, 0, 1_000, "x".into())
                .unwrap_err()
                .kind,
            "too_large"
        );
        let huge = "x".repeat(MAX_EDIT_CHARS * 4 + 1);
        assert_eq!(
            docs.set_cells(id, vec![(0, 0, huge)]).unwrap_err().kind,
            "too_large"
        );
        assert_eq!(
            docs.insert_rows(id, 0, MAX_INSERT_ROWS + 1)
                .unwrap_err()
                .kind,
            "too_large"
        );
        // Unknown ids are refused.
        assert_eq!(docs.info(9999).unwrap_err().kind, "no_document");
    }

    #[test]
    fn repairs_and_line_by_line_are_reported() {
        // Quotes are in use (the comma inside them), so the lone quote on
        // row 3 is an unclosed one.
        let p = tmp("d.csv", b"a,b\n\"q,1\",2\n\"open,1\nx,y\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        let i = wait(&docs, id);
        assert!(i.unclosed_quotes);
        let id2 = docs.open_path(&p, true).unwrap().id;
        let i2 = wait(&docs, id2);
        assert!(i2.line_by_line);
        assert_eq!(i2.rows, 4);
    }

    /// Review finding 6: after a save, a line-by-line document is a normal one.
    #[test]
    fn save_clears_line_by_line() {
        let p = tmp("f.csv", b"a,b\n\"open,1\nx,y\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, true).unwrap().id;
        assert!(wait(&docs, id).line_by_line);
        docs.set_cells(id, vec![(2, 0, "X".into())]).unwrap();
        let i = docs.save(id, None, false, false, None).unwrap();
        assert!(!i.line_by_line);
    }

    /// Review findings 4 and 5: copy memory per step and column inserts are bounded.
    #[test]
    fn copy_and_column_inserts_are_bounded() {
        // 40 cells of 1 MB each: a single cell copies, the band refuses
        // instead of holding all of them at full length.
        let cell = "x".repeat(1 << 20);
        let body: String = (0..40).map(|_| format!("{cell}\n")).collect();
        let p = tmp("g.csv", body.as_bytes());
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        assert_eq!(docs.copy(id, 0, 1, 0, 1).unwrap().len(), (1 << 20) + 2);
        // 40 MB fits the 64 MB limit: long cells are read one at a time.
        assert_eq!(
            docs.copy(id, 0, 40, 0, 1).unwrap().len(),
            40 * ((1 << 20) + 2)
        );
        assert_eq!(
            docs.insert_cols(id, 0, MAX_INSERT_COLS + 1)
                .unwrap_err()
                .kind,
            "too_large"
        );
        assert!(docs.insert_cols(id, 0, MAX_INSERT_COLS).is_ok());
        for _ in 0..15 {
            let _ = docs.insert_cols(id, 0, MAX_INSERT_COLS);
        }
        assert!(docs.info(id).unwrap().cols <= MAX_COLS_AFTER_INSERT);
    }

    /// Re-check finding: with column changes, a far-right edit is capped
    /// like inserting columns (without changes only that row grows).
    #[test]
    fn far_right_edit_is_capped_with_column_changes() {
        let docs = Docs::default();
        let p = tmp("h.csv", b"a,b\n1,2\n3,4\n");
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        assert!(docs.set_cells(id, vec![(0, 100_000, "x".into())]).is_ok());
        docs.undo(id).unwrap();
        docs.move_cols(id, 1, 1, 0).unwrap();
        let e = docs
            .set_cells(id, vec![(0, 1_000_000, "x".into())])
            .unwrap_err();
        assert_eq!(e.kind, "out_of_memory");
        let e = docs.fill(id, 0, 1, 20_000, 1, "x".into()).unwrap_err();
        assert_eq!(e.kind, "out_of_memory");
        assert!(docs.set_cells(id, vec![(0, 100, "x".into())]).is_ok());
        assert_eq!(docs.info(id).unwrap().cols, 101);
    }

    #[test]
    fn new_document_needs_a_path() {
        let docs = Docs::default();
        let id = docs.new_document().unwrap().id;
        assert_eq!(docs.info(id).unwrap().rows, 1);
        docs.set_cells(id, vec![(0, 0, "x".into())]).unwrap();
        assert_eq!(
            docs.save(id, None, false, false, None).unwrap_err().kind,
            "no_path"
        );
        let p = tmp("new.csv", b"");
        docs.save(id, Some(p.clone()), false, false, None).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"x\r\n");
        assert_eq!(docs.info(id).unwrap().name, "new.csv");
    }

    #[test]
    fn column_ops_and_moves() {
        let docs = Docs::default();
        let p = tmp("e.csv", b"a,b\n1,2\n");
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let i = docs.insert_cols(id, 1, 1).unwrap();
        assert_eq!(i.cols, 3);
        assert!(i.column_changes);
        docs.move_cols(id, 2, 1, 0).unwrap();
        docs.move_rows(id, 1, 1, 0).unwrap();
        assert_eq!(docs.copy(id, 0, 2, 0, 3).unwrap(), "2\t1\t\r\nb\ta\t\r\n");
        docs.delete_cols(id, 2, 1).unwrap();
        docs.delete_rows(id, 0, 1).unwrap();
        assert_eq!(docs.info(id).unwrap().rows, 1);
    }

    #[test]
    fn group_formats() {
        assert_eq!(group(100_000), "100,000");
        assert_eq!(group(1_000_000), "1,000,000");
        assert_eq!(group(12), "12");
    }

    #[test]
    fn pivots_preview_refresh_and_drill() {
        use natendo_engine::pivot::{Agg, PivotField, PivotSpec, ValueField};
        let p = tmp("pivot.csv", b"k,v\r\na,1\r\nb,2\r\na,3\r\n");
        let docs = Arc::new(Docs::default());
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let spec = PivotSpec {
            rows: vec![PivotField {
                col: 0,
                ..Default::default()
            }],
            values: vec![ValueField {
                col: 1,
                agg: Agg::Sum,
                ..Default::default()
            }],
            ..Default::default()
        };
        let g = docs.pivot_preview(id, &spec, None).unwrap();
        assert_eq!(g.rows[0].cells, ["4"]);
        // A quick look at the first row only.
        let g = docs.pivot_preview(id, &spec, Some(1)).unwrap();
        assert!(g.sampled);
        assert_eq!(g.rows.len(), 2);
        // Edits show on the next preview (the cube is read again).
        docs.set_cells(id, vec![(1, 1, "10".into())]).unwrap();
        let g = docs.pivot_preview(id, &spec, None).unwrap();
        assert_eq!(g.rows[0].cells, ["13"]);
        // New tab and drill-down tabs are new, unsaved documents.
        let t = docs.pivot_table(id, &spec).unwrap();
        assert!(t.modified);
        let d = docs
            .pivot_drill(id, &spec, &["a".to_string()], &[])
            .unwrap();
        assert_eq!(d.rows, 3);
        let text = docs.pivot_text(id, &spec).unwrap();
        assert!(text.starts_with("k\tSum of v\r\na\t13\r\n"), "{text}");
        let items = docs
            .pivot_items(
                id,
                &PivotField {
                    col: 0,
                    ..Default::default()
                },
                &spec,
            )
            .unwrap();
        assert_eq!(items.total, 2);
        // A pivot of a private tab is private.
        docs.set_private(id, true).unwrap();
        let t = docs.pivot_table(id, &spec).unwrap();
        assert!(docs.is_private(t.id).unwrap());
        docs.pivot_forget(id).unwrap();
        // A running pivot does not block a newer one.
        let entry = docs.get(id).unwrap();
        let job = docs.start_pivot(&entry).unwrap();
        let progress = job.progress.clone();
        let docs2 = docs.clone();
        let spec2 = spec.clone();
        let h = std::thread::spawn(move || docs2.pivot_preview(id, &spec2, None));
        while !progress.cancel.load(Ordering::Relaxed) {
            std::thread::yield_now();
        }
        drop(job);
        assert_eq!(h.join().unwrap().unwrap().rows[0].cells, ["13"]);
    }

    #[test]
    fn data_ops_and_new_tables() {
        use natendo_engine::ops::PasteOp;
        let p = tmp("m4b.csv", b"k,v\r\na,1\r\nb,2\r\na,3\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        // Paste Special: add, one undo step.
        let r = docs
            .data_op(
                id,
                DataOp::Paste {
                    spec: PasteSpec {
                        row: 1,
                        col: 1,
                        op: PasteOp::Add,
                        ..Default::default()
                    },
                    text: "10\r\n10\r\n".into(),
                },
            )
            .unwrap();
        assert_eq!(r.cells, 2);
        let cell = |r: u64, c: u32| docs.cell_text(id, r, c).unwrap().0;
        assert_eq!((cell(1, 1), cell(2, 1)), ("11".into(), "12".into()));
        docs.undo(id).unwrap();
        // Fill down past the end stops at the last row.
        let r = docs
            .data_op(
                id,
                DataOp::Fill {
                    spec: FillSpec {
                        row: 1,
                        col: 1,
                        rows: 2,
                        cols: 1,
                        count: 50,
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        assert_eq!((r.cells, cell(3, 1)), (1, "3".into()));
        // Show duplicates is a filter; remove keeps the first.
        let spec = Duplicates {
            cols: vec![0],
            keep_first: true,
            ..Default::default()
        };
        let r = docs
            .data_op(id, DataOp::ShowDuplicates { spec: spec.clone() })
            .unwrap();
        assert!(r.info.unwrap().filtered && r.rows == 2);
        docs.clear_filter(id).unwrap();
        let r = docs.data_op(id, DataOp::RemoveDuplicates { spec }).unwrap();
        assert_eq!((r.rows, r.info.unwrap().rows), (1, 3));
        // Pivot opens a new, unsaved document with no file.
        let info = docs
            .new_table(
                id,
                TableOp::Pivot {
                    spec: Pivot {
                        rows: vec![0],
                        col: 0,
                        value: 1,
                        keep_first: true,
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        assert!(info.modified && info.path.is_none() && info.id != id);
        assert_eq!(docs.data_edge(id, 0, 0, Dir::Down).unwrap(), (2, 0));
        // Bounds: too much text, bad separators, out-of-range columns.
        let big = DataOp::Paste {
            spec: PasteSpec::default(),
            text: "x".repeat(MAX_WRITE_CHARS * 4 + 1),
        };
        assert_eq!(docs.data_op(id, big).unwrap_err().kind, "too_large");
        let e = docs
            .data_op(
                id,
                DataOp::SplitColumn {
                    spec: SplitColumn {
                        col: 0,
                        separator: ";".repeat(5000),
                        ..Default::default()
                    },
                },
            )
            .unwrap_err();
        assert_eq!(e.kind, "invalid");
        let e = docs
            .data_op(
                id,
                DataOp::MergeColumns {
                    spec: MergeColumns {
                        col: 1,
                        count: 9,
                        ..Default::default()
                    },
                },
            )
            .unwrap_err();
        assert_eq!(e.kind, "invalid");
        assert!(docs.job(id).unwrap().is_none());
    }

    #[test]
    fn stale_plans_are_computed_again() {
        let p = tmp("retry.csv", b"a\r\nb\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let plans = AtomicU32::new(0);
        let applies = AtomicU32::new(0);
        // An edit landing between compute and apply: the first apply finds
        // the plan stale, the second plan goes through.
        docs.compute_apply(
            id,
            "Test",
            |_, _| Ok(plans.fetch_add(1, Ordering::Relaxed)),
            |_, n| {
                applies.fetch_add(1, Ordering::Relaxed);
                if n == 0 {
                    Err(Error::Changed)
                } else {
                    Ok(())
                }
            },
        )
        .unwrap();
        assert_eq!(plans.load(Ordering::Relaxed), 2);
        // Always stale: gives up with a clear message, job slot freed.
        let e = docs
            .compute_apply(id, "Test", |_, _| Ok(()), |_, _| Err(Error::Changed))
            .unwrap_err();
        assert_eq!(e.kind, "busy");
        assert!(docs.job(id).unwrap().is_none());
    }

    #[test]
    fn close_check_never_waits_for_a_busy_document() {
        let p = tmp("closecheck.csv", b"a\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        assert!(!docs.any_modified());
        let e = docs.get(id).unwrap();
        let held = e.write(); // as a save does
        assert!(docs.any_modified());
        drop(held);
        assert!(!docs.any_modified());
    }

    #[test]
    fn sort_filter_find_replace() {
        use natendo_engine::filter::{ColumnFilter, Condition, TextOp};
        use natendo_engine::find::Scope;
        use natendo_engine::sort::{SortKey, SortKind};
        let p = tmp("m4.csv", b"name,n\r\nb,2\r\na,10\r\nc,1\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let spec = SortSpec {
            keys: vec![SortKey {
                col: 1,
                descending: false,
                kind: SortKind::Auto,
                match_case: false,
            }],
            keep_first: true,
            read: Default::default(),
            lists: Vec::new(),
        };
        let info = docs.sort(id, spec).unwrap();
        assert!(info.modified && info.can_undo);
        let col0 = |docs: &Docs| -> Vec<String> {
            let e = docs.get(id).unwrap();
            let d = e.read();
            (0..d.row_count()).map(|r| d.cell(r, 0).unwrap()).collect()
        };
        assert_eq!(col0(&docs), ["name", "c", "b", "a"]);
        let filter = FilterSpec {
            columns: vec![ColumnFilter {
                col: 0,
                rule: Rule::Conditions {
                    first: Condition::Text {
                        op: TextOp::NotEquals,
                        text: "b".into(),
                        match_case: false,
                    },
                    second: None,
                    or: false,
                },
            }],
            keep_first: true,
            read: Default::default(),
        };
        let info = docs.filter(id, filter).unwrap();
        assert!(info.filtered);
        assert_eq!(
            (info.rows, info.total_rows, info.filter_cols.clone()),
            (3, 4, vec![0])
        );
        // Row numbers travel with the viewport while filtered.
        let bytes = docs.viewport(id, 0, 3, 0, 1, 10).unwrap();
        assert!(bytes.len() > 4 + 3 * 8);
        // Deleting the filtered column keeps its rows hidden and says so;
        // undo brings the column and its rule back.
        let info = docs.delete_cols(id, 0, 1).unwrap();
        assert!(info.filtered && info.filter_on_deleted && info.filter_cols.is_empty());
        let info = docs.undo(id).unwrap();
        assert!(!info.filter_on_deleted);
        assert_eq!(info.filter_cols, vec![0]);
        let find = FindSpec {
            pattern: Pattern {
                text: "a".into(),
                ..Default::default()
            },
            scope: Scope::All,
            skip_first: true,
        };
        let hit = docs
            .find_next(id, find.clone(), None, false)
            .unwrap()
            .unwrap();
        assert_eq!((hit.row, hit.col), (2, 0));
        assert_eq!(docs.find_all(id, find.clone()).unwrap().cells, 1);
        let pre = docs.replace_preview(id, find.clone(), "z".into()).unwrap();
        assert_eq!((pre.cells, pre.preview.len()), (1, 1));
        let done = docs.replace_all(id, find, "z".into()).unwrap();
        assert_eq!(done.cells, 1);
        assert_eq!(col0(&docs), ["name", "c", "z"]);
        let info = docs.clear_filter(id).unwrap();
        assert!(!info.filtered);
        assert_eq!(col0(&docs), ["name", "c", "b", "z"]);
        docs.undo(id).unwrap();
        docs.undo(id).unwrap();
        assert_eq!(col0(&docs), ["name", "b", "a", "c"]);
        assert!(docs.job(id).unwrap().is_none());
        let bad = FindSpec {
            pattern: Pattern {
                text: "(".into(),
                kind: natendo_engine::find::PatternKind::Regex,
                ..Default::default()
            },
            scope: Scope::All,
            skip_first: false,
        };
        assert_eq!(docs.find_all(id, bad).unwrap_err().kind, "bad_pattern");
        assert_eq!(
            find_in_text(
                "x\nxx",
                &Pattern {
                    text: "x".into(),
                    ..Default::default()
                }
            )
            .unwrap(),
            vec![(0, 1), (2, 3), (3, 4)]
        );
    }

    #[test]
    fn one_job_at_a_time_and_value_caps() {
        let p = tmp("busy.csv", b"a\r\nb\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let entry = docs.get(id).unwrap();
        let job = entry.start("Sorting").unwrap();
        assert_eq!(docs.job(id).unwrap().unwrap().label, "Sorting");
        let spec = SortSpec {
            keys: vec![natendo_engine::sort::SortKey {
                col: 0,
                descending: true,
                kind: Default::default(),
                match_case: false,
            }],
            ..Default::default()
        };
        assert_eq!(docs.sort(id, spec.clone()).unwrap_err().kind, "busy");
        docs.cancel_job(id).unwrap();
        assert!(job.progress.cancel.load(Ordering::Relaxed));
        drop(job);
        assert!(docs.sort(id, spec).is_ok());
        let huge = FilterSpec {
            columns: vec![natendo_engine::filter::ColumnFilter {
                col: 0,
                rule: Rule::Values {
                    values: vec!["x".into(); MAX_FILTER_VALUES + 1],
                    blanks: false,
                },
            }],
            ..Default::default()
        };
        assert_eq!(docs.filter(id, huge).unwrap_err().kind, "too_large");
    }

    #[test]
    fn move_cells_is_one_step_and_clears_what_it_leaves() {
        let p = tmp("move.csv", b"a,b,c\n1,2,3\n4,5,6\n7,8,9\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let all = |docs: &Docs| docs.copy(id, 0, 4, 0, 3).unwrap();
        let before = all(&docs);
        // 2x2 block at (1,0) moved one down and one right: overlaps itself.
        docs.move_cells(id, 1, 2, 0, 2, 2, 1, false).unwrap();
        assert_eq!(all(&docs), "a\tb\tc\r\n\t\t3\r\n\t1\t2\r\n7\t4\t5\r\n");
        docs.undo(id).unwrap();
        assert_eq!(all(&docs), before);
        // Moving back up-left over itself.
        docs.move_cells(id, 2, 2, 1, 2, 1, 0, false).unwrap();
        assert_eq!(all(&docs), "a\tb\tc\r\n5\t6\t3\r\n8\t9\t\r\n7\t\t\r\n");
        docs.undo(id).unwrap();
        assert_eq!(all(&docs), before);
        // Outside the table, or too many cells: refused, nothing changes.
        assert!(docs.move_cells(id, 0, 2, 0, 2, 3, 0, false).is_err());
        assert!(docs.move_cells(id, 0, 1, 0, 2, 0, 2, false).is_err());
        assert!(docs.move_cells(id, 0, u64::MAX, 0, 2, 0, 0, false).is_err());
        assert_eq!(
            docs.move_cells(id, 0, 60_000, 0, 1, 0, 0, false)
                .unwrap_err()
                .kind,
            "too_large"
        );
        assert_eq!(all(&docs), before);
        // Nothing to do.
        docs.move_cells(id, 1, 1, 1, 1, 1, 1, false).unwrap();
        assert_eq!(all(&docs), before);
        // Ctrl held: copy, the original stays.
        docs.move_cells(id, 1, 1, 0, 2, 3, 1, true).unwrap();
        assert_eq!(all(&docs), "a\tb\tc\r\n1\t2\t3\r\n4\t5\t6\r\n7\t1\t2\r\n");
        docs.undo(id).unwrap();
        assert_eq!(all(&docs), before);
        // A read-only tab refuses.
        docs.set_read_only(id, true).unwrap();
        assert!(docs.move_cells(id, 1, 1, 0, 1, 2, 0, false).is_err());
        assert_eq!(all(&docs), before);
    }

    #[test]
    fn move_cells_reads_long_cells_alone_and_copy_has_its_own_cap() {
        // 1,100 rows: the first band shares its budget among 1,024 cells,
        // so the 40,000-character cell is over its share and read alone.
        let mut body = String::from("a,b\n");
        for i in 0..1100 {
            if i == 5 {
                body.push_str(&"x".repeat(40_000));
                body.push_str(",\n");
            } else {
                body.push_str(&format!("{i},\n"));
            }
        }
        let p = tmp("long.csv", body.as_bytes());
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        docs.move_cells(id, 1, 1100, 0, 1, 1, 1, false).unwrap();
        let moved = docs.copy(id, 6, 1, 0, 2).unwrap();
        assert_eq!(moved, format!("\t{}\r\n", "x".repeat(40_000)));
        assert_eq!(docs.copy(id, 1100, 1, 0, 2).unwrap(), "\t1099\r\n");
        // Copying may write up to MAX_WRITE_CELLS; moving only half of that.
        assert_eq!(
            docs.move_cells(id, 0, 60_000, 0, 1, 0, 1, false)
                .unwrap_err()
                .kind,
            "too_large"
        );
        assert_ne!(
            docs.move_cells(id, 0, 60_000, 0, 1, 0, 1, true)
                .unwrap_err()
                .kind,
            "too_large"
        );
    }
}
