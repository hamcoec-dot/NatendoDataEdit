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

use natendo_engine::document::since::SincePlan;
use natendo_engine::gaps::{GapReport, GapsSpec};
use natendo_engine::relations::{RelationKind, RelationsReport, RelationsSpec};
use natendo_engine::sets::{
    report_table, slice_name, SetsBuilder, SetsReport, SetsResult, SetsSpec,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::recovery::Autosaved;
use natendo_engine::cellformula::CellFormula;
use natendo_engine::clean::{CleanMatrix, CleanPreview, CleanSpec, MatrixPreview, SplitName};
use natendo_engine::coltotals::{ColumnTotal, MAX_COLUMN_TOTALS};
use natendo_engine::disk::DiskMark;
use natendo_engine::document::repair::{Repair, RowText, SameShape};
use natendo_engine::document::report::{NoteCounts, Place, ReportNotes};
use natendo_engine::editing::{
    Arrange, Extract, GroupCalc, MapValues, TallySpec, ValueTally, MAX_PAIRS,
};
use natendo_engine::example::{ExampleSpec, Program};
use natendo_engine::export::{ExportFormat, ExportOptions, ExportReport};
use natendo_engine::expr::Lookups;
use natendo_engine::expr::{ComputePreview, ComputeSpec};
use natendo_engine::filter::{FilterSpec, Rule, ValueList};
use natendo_engine::find::{Change, FindAll, FindCount, FindSpec, Matcher, Pattern, ReplacePlan};
use natendo_engine::highlight::{Highlight, Mark};
use natendo_engine::join::{
    AppendSource, AppendSpec, JoinPreview, JoinReport, JoinSource, JoinSpec, LookupTable,
};
use natendo_engine::keyset::{KeyBinding, KeySet};
use natendo_engine::ops::{
    self, ColPlan, Dir, FillSpec, MergeColumns, PasteSpec, SplitColumn, MAX_BLOCK_CELLS,
};
use natendo_engine::pivot::{Cube, GridLimit, ItemList, PivotField, PivotGrid, PivotSpec};
use natendo_engine::recipe::{Recipe, RecipeOp};
use natendo_engine::reshape::{
    Blanks, Duplicates, MergeRows, Pivot, RowPlan, SplitRows, Table, Unpivot,
};
use natendo_engine::rows::{ShowRows, Totals};
use natendo_engine::rulecard::{LearnReport, Outliers, Preview};
use natendo_engine::sort::SortSpec;
use natendo_engine::stats::{ChartData, ChartSpec, ColType, SubtotalSpec, Summary};
use natendo_engine::tools::{
    Breaks, Combine, CompareIndex, CompareReport, CompareSpec, FillBlanks, Repeats, Reshape,
    Sample, SplitParts, Stack, MAX_COMBINE,
};
use natendo_engine::validate::{CheckKind, CheckMark, CheckReport, CheckSpec};
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
/// The filter panel's counting job (a newer count replaces it).
const FACET_JOB: &str = "Counting for the filter panel";
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
/// Most blocks one multi-block (Ctrl+click) request may carry.
pub const MAX_AREAS: usize = 10_000;
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

/// A document with unsaved changes, as the quit question lists it.
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct ModifiedDoc {
    pub id: u32,
    pub name: String,
    /// Counts every edit, undo and redo (it never goes back).
    pub generation: u64,
}

/// What stands between the app and a quit: unsaved documents (any window),
/// documents something holds right now (a save, a long change) and the long
/// jobs running.
#[derive(Debug, Serialize, Clone, PartialEq, Eq, Default)]
pub struct ModifiedReport {
    pub docs: Vec<ModifiedDoc>,
    pub busy: Vec<String>,
    pub jobs: Vec<String>,
}

impl ModifiedReport {
    /// Nothing to ask about: quit at once.
    pub fn is_clear(&self) -> bool {
        self.docs.is_empty() && self.busy.is_empty() && self.jobs.is_empty()
    }

    /// Has anything changed since the question was asked about `known`
    /// (document, generation)? A document modified now that was not listed, one
    /// edited since, or anything held: the person has not agreed to lose it.
    pub fn changed_since(&self, known: &[(u32, u64)]) -> bool {
        !self.busy.is_empty()
            || self
                .docs
                .iter()
                .any(|d| !known.contains(&(d.id, d.generation)))
    }
}

/// Most columns whose names are read at once (for rules that follow a
/// rename, and views): 20,000 names of at most 200 characters.
pub const MAX_NAME_COLS: usize = 20_000;

/// One open document.
pub struct Entry {
    /// Read/write lock: long operations compute under a read lock, so the
    /// grid keeps drawing, and apply under the write lock.
    doc: RwLock<Document>,
    /// The document's file, kept outside its lock (refreshed whenever the
    /// document is read), so "is this file open?" has an answer while a
    /// long operation holds the lock.
    path: Mutex<Option<PathBuf>>,
    job: Mutex<Option<Job>>,
    /// The last replace computed for a preview, reused by "Replace all"
    /// when nothing changed since (request as JSON, plan).
    replace: Mutex<Option<(String, ReplacePlan)>>,
    /// Rows (ids) the last Replace all changed ("Show changed rows").
    changed_rows: Mutex<(u64, Vec<u32>)>,
    /// Cells the last Fill blank cells left blank on purpose (its stops):
    /// row id -> logical columns, marked amber until undo, Esc or the next fill.
    fill_marks: Mutex<HashMap<u32, Vec<u32>>>,
    /// The Fill blank cells dialog's count being worked out, if any: a newer
    /// count, closing the dialog or any change stops it.
    fill_count: Mutex<Option<Arc<Progress>>>,
    /// The chip bar's counts being worked out, if any: a newer ask or any change (an edit, a filter
    /// change) stops it (C8 review R1).
    chip_count: Mutex<Option<Arc<Progress>>>,
    /// A check against a file contract being worked out, if any: a newer one, Cancel or any change
    /// stops it (C9).
    contract_count: Mutex<Option<Arc<Progress>>>,
    cancel: AtomicBool,
    /// Opened with quoted cells never spanning lines (cleared once a save
    /// has reloaded the file normally).
    pub line_by_line: AtomicBool,
    /// The JSON file this tab saves back to (C11), with its size and time when last read or written.
    json_target: Mutex<Option<JsonTarget>>,
    /// Indexing failed (shown once; the document stays usable as far as read).
    error: Mutex<Option<String>>,
    /// Name shown for a document with no file yet (imports).
    title: Mutex<Option<String>>,
    /// The last `DocInfo` read under the lock: what `Docs::info` answers with (marked stale) while a
    /// change holds the document, so the window never waits for the lock.
    last_info: Mutex<Option<DocInfo>>,
    /// Read-only: every change and every save over the file is refused.
    locked: AtomicBool,
    /// The steps panel is showing an earlier step (the later ones are in
    /// the redo list): changes and saving are refused until it goes back
    /// to the latest.
    peeking: AtomicBool,
    /// The last data-check run: the checks (as JSON), the document revision
    /// and, per unique check, the rows whose value repeats.
    checks: Mutex<Option<LastChecks>>,
    /// The last gap audit, with the revision it read (for "show the rows
    /// next to each gap").
    gaps: Mutex<Option<(u64, Arc<GapReport>)>>,
    /// The last relationship scan, with the revision it read (for "show the
    /// exceptions").
    relations: Mutex<Option<(u64, Arc<RelationsReport>)>>,
    /// The quick totals being worked out for the selection, if any: a newer
    /// selection or any change cancels them.
    totals: Mutex<Option<Arc<Progress>>>,
    /// The totals row's results for one revision of the document.
    column_totals: Mutex<ColumnTotalsCache>,
    /// The find bar's match count being worked out, if any: a newer count
    /// or any change cancels it.
    find_count: Mutex<Option<Arc<Progress>>>,
    /// The "Replace all would change N cells" count behind the ghost diff (one at a time per tab).
    ghost_count: Mutex<Option<Arc<Progress>>>,
    /// The last count's matches per block of rows: (search, revision,
    /// counts), so moving to the next match re-reads one block only.
    #[allow(clippy::type_complexity)]
    find_blocks: Mutex<Option<(String, u64, Vec<u64>)>>,
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
    /// The golden records last previewed (memory only).
    golden: Mutex<Option<Arc<natendo_engine::golden::GoldenPlan>>>,
    /// The patch of the last dry run on this tab (memory only).
    patch: Mutex<Option<patching::PatchKept>>,
    /// The encrypted file this tab was opened from or last saved to
    /// encrypted (Save keeps it encrypted there).
    encrypted_from: Mutex<Option<PathBuf>>,
    /// The steps done, for a recipe (see `Docs::recorded`).
    recipe: Mutex<RecipeLog>,
    /// Named moments in the undo history (see `Checkpoints`).
    checkpoints: Mutex<Checkpoints>,
    /// The last "what changed since this checkpoint" plan, kept while the document is as it was, so paging does not
    /// compare again.
    since_plan: Mutex<Option<KeptSince>>,
    /// The filter panel's column codes (see `FacetCache`).
    facets: Mutex<natendo_engine::filter::FacetCache>,
    /// The other list of the last "match people" run (for accepting).
    matched: Mutex<Option<MatchKept>>,
    /// The last comparison with a newer file (for accepting).
    updates: Mutex<Option<UpdatesKept>>,
    /// Notes and flags (loaded from beside the file when first asked).
    notes: Mutex<Option<natendo_engine::notes::NoteBook>>,
}

/// One encoding of the picker.
#[derive(Debug, Serialize, Clone)]
pub struct EncodingChoice {
    pub label: String,
    pub name: String,
    /// The file can switch to it without reopening (ASCII-compatible: the row offsets stay valid).
    pub direct: bool,
}

/// How much of the start of the file the scrub preview reads.
const PREVIEW_HEAD_BYTES: u64 = 512 * 1024;

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
    /// The document's file: read now when the lock is free, else the last
    /// one seen.
    fn current_path(&self) -> Option<PathBuf> {
        match self.doc.try_read() {
            Ok(d) => self.note_path(&d),
            Err(_) => self.path.lock().unwrap_or_else(|p| p.into_inner()).clone(),
        }
    }

    fn note_path(&self, d: &Document) -> Option<PathBuf> {
        let p = d.path().map(Path::to_path_buf);
        *self.path.lock().unwrap_or_else(|p| p.into_inner()) = p.clone();
        p
    }

    // A panic while holding the lock leaves the engine's own state
    // consistent (every op is applied whole), so keep going.
    fn read(&self) -> RwLockReadGuard<'_, Document> {
        self.doc.read().unwrap_or_else(|p| p.into_inner())
    }

    /// The read lock if it can be had now (`None` while a change holds the document).
    fn try_read(&self) -> Option<RwLockReadGuard<'_, Document>> {
        match self.doc.try_read() {
            Ok(d) => Some(d),
            Err(std::sync::TryLockError::Poisoned(p)) => Some(p.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
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
        if let Some(p) = self
            .chip_count
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            p.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(p) = self
            .contract_count
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            p.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(p) = self
            .fill_count
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            p.cancel.store(true, Ordering::Relaxed);
        }
        self.column_totals
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .stop();
        if let Some(p) = self
            .find_count
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            p.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(p) = self
            .ghost_count
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
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

/// Rules loaded from a `.ndchecks` file beside the table.
#[derive(Debug, Clone, Serialize)]
pub struct SidecarRules {
    pub checks: Vec<natendo_engine::validate::Check>,
    pub table_checks: Vec<natendo_engine::tablecheck::TableCheck>,
    /// Rules left out because their column or pattern did not fit this table.
    pub skipped: usize,
    pub keep_first: bool,
    /// The file's name, for the status line.
    pub file: String,
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
    /// Of the cells, those in hidden rows or columns (the preview says so).
    pub hidden_cells: u64,
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
    /// Explode cells to columns (keys, item positions or flags): every row.
    ExplodeColumns {
        spec: natendo_engine::explode::Explode,
    },
    /// Unnest list items to rows: shown rows.
    ExplodeRows {
        spec: natendo_engine::explode::Explode,
    },
    /// Show only the rows whose cell did not parse (a view).
    ShowExplodeBad {
        spec: natendo_engine::explode::Explode,
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
        /// Cells the person typed on the date review screen (shown row,
        /// column, text): part of the same undo step, never in a recipe.
        #[serde(default)]
        typed: Vec<(u64, u32, String)>,
    },
    /// A clean matrix: each ticked column with its steps, in one pass.
    CleanMatrix {
        spec: CleanMatrix,
        /// The moment of the run (days since 1970-01-01, local).
        now: f64,
        /// The preview's document revision: refused when the table changed
        /// since. `None` for a recipe, which has no preview.
        #[serde(default)]
        revision: Option<u64>,
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
    /// Show the rows behind one count of a file contract's drift report (C9).
    ShowContractRows {
        contract: Box<natendo_engine::contract::Contract>,
        what: natendo_engine::contract::DriftRows,
        opts: natendo_engine::contract::DriftOptions,
    },
    /// Show only the rows with a different number of fields than most.
    ShowRagged {
        #[serde(default)]
        pick: natendo_engine::document::ragged::RaggedPick,
        keep_first: bool,
    },
    /// Show only the rows whose value in a column has (or has not) a shape: the
    /// pattern pill's click. Reads the rows shown now; refused if the document
    /// changed since the pill was worked out.
    ShowShape {
        spec: natendo_engine::profile::ShapeFilter,
    },
    /// Show only the rows next to the gaps the last audit of `spec` found.
    ShowGapRows {
        spec: GapsSpec,
    },
    /// Show only the rows that break one line of the last relationship scan.
    ShowRelationExceptions {
        kind: RelationKind,
        a: u32,
        b: u32,
    },
    /// Put the rows that fail the checks first.
    SortProblems {
        spec: CheckSpec,
        #[serde(default)]
        scope: natendo_engine::sort::SortScope,
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
    /// One value into every cell of a block (clearing it, or Ctrl+Enter),
    /// any number of cells.
    FillRect {
        spec: natendo_engine::shift::RectFill,
    },
    /// Filter by formula: show only the rows where it is TRUE.
    FormulaFilter {
        spec: ComputeSpec,
    },
    /// Compare two columns: show only the rows where they differ.
    CompareShow {
        spec: natendo_engine::colcompare::CompareCols,
    },
    /// Compare two columns: a column saying Same, Different, Only in first
    /// or Only in second.
    CompareColumn {
        spec: natendo_engine::colcompare::CompareCols,
    },
    /// One value into every cell Go To Special picks (empty: clear them).
    FillSpecial {
        spec: natendo_engine::special::SpecialSpec,
        value: String,
    },
    /// Insert or delete cells, moving the others up, left, down or right.
    ShiftCells {
        spec: natendo_engine::shift::ShiftCells,
    },
}

/// Check a cleaner run from the webview: the moment (years 1900 to 9999,
/// as days since 1970) and the column list (the cleaner itself is checked
/// by the engine).
fn matrix_stale() -> Error {
    Error::Invalid("the table changed: preview again".into())
}

/// The moment a cleaning run says it is.
fn check_now(now: f64) -> CmdResult<()> {
    if !now.is_finite() || !(-25_567.0..=2_932_896.0).contains(&now) {
        return Err(CmdError::new(
            "bad_request",
            "the date and time of the run is not valid",
        ));
    }
    Ok(())
}

/// A recipe run's line about ambiguous dates a step left ("" when none).
fn left_note(what: &str, left: u64) -> String {
    if left == 0 {
        return String::new();
    }
    let dates = if left == 1 { "date" } else { "dates" };
    format!("{what}: {} ambiguous {dates} left as typed", group(left))
}

fn check_clean(spec: &CleanSpec, now: f64) -> CmdResult<()> {
    check_now(now)?;
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

/// A kept comparison of lists.
#[derive(Clone)]
struct SetsCache {
    /// Each list's tab and its revision when read.
    tabs: Vec<(u32, u64)>,
    spec: SetsSpec,
    exact: Vec<bool>,
    names: Vec<String>,
    result: Arc<SetsResult>,
}

/// What a comparison of lists found.
#[derive(Debug, Clone, Serialize)]
pub struct SetsRun {
    pub id: u64,
    pub report: SetsReport,
    pub names: Vec<String>,
}

fn lists_changed() -> CmdError {
    CmdError::new(
        "changed",
        "a list changed since it was compared: run the comparison again",
    )
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
    /// A gap audit's summary and missing ranges.
    Gaps {
        spec: GapsSpec,
    },
    /// Each value of one to three columns with its count (the counts tab).
    Counts {
        spec: natendo_engine::counts::CountsSpec,
    },
    /// This document compared with another open one.
    Compare {
        other: u32,
        spec: CompareSpec,
    },
}

/// What to do with the rows Find all found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FindRowsAction {
    Show,
    Delete,
    Copy,
    Flag { flag: u8 },
}

/// What a cell formula did.
#[derive(Debug, Clone, Serialize)]
pub struct FormulaReport {
    pub info: DocInfo,
    /// Cells written.
    pub written: u64,
    /// Cells left as they were because the formula gave an error there.
    pub errors: u64,
    pub first_error: Option<String>,
}

/// What acting on Find all's rows did.
#[derive(Debug, Clone, Serialize)]
pub struct FindRowsReport {
    pub info: DocInfo,
    /// Rows that had a flag of another colour (Flag).
    pub recolored: u64,
    /// Hidden columns left out of the new tab (Copy).
    pub left_out_cols: u32,
}

/// At most this many rows flagged at once from Find.
pub const MAX_FLAG_ROWS: usize = 100_000;

/// A new table and, for comparisons, what was found.
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct TableReport {
    pub info: DocInfo,
    pub compare: Option<CompareReport>,
    /// A sample's audit record, opened in a tab of its own.
    pub record: Option<DocInfo>,
}

/// A key-aware diff kept while its tabs are open: the index that draws it on the first table, the second
/// and the result, and which tabs those are.
struct DiffState {
    index: Arc<natendo_engine::diff::DiffIndex>,
    a: u32,
    b: u32,
    result: u32,
}

/// A diff a tab is in: its number, its shared index and which table of it the tab is.
type DiffOf = (
    u64,
    Arc<natendo_engine::diff::DiffIndex>,
    natendo_engine::diff::Side,
);

/// What the diff command gives back: the result tab, the counts and the report text.
#[derive(Debug, Serialize)]
pub struct DiffReportOut {
    pub info: DocInfo,
    pub report: natendo_engine::diff::DiffReport,
    pub diff: u64,
}

/// A window of marks for a tab, and whether the diff still fits the tab's rows.
#[derive(Debug, Serialize)]
pub struct DiffMarksOut {
    /// `None`: this tab is in no diff.
    pub diff: Option<u64>,
    /// The tab was edited after the compare: nothing is drawn, the UI says "compare again".
    pub out_of_date: bool,
    pub marks: Vec<natendo_engine::diff::DiffMark>,
}

/// Where a "next difference" landed.
#[derive(Debug, Serialize)]
pub struct DiffJump {
    pub row: u64,
    pub col: Option<u32>,
    pub wrapped: bool,
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
    /// Pairs above the score that were not listed.
    never_offered: u64,
}

/// A step noted for a recipe.
#[derive(Clone)]
enum Logged {
    Step(RecipeOp),
    /// Done but cannot be repeated by a recipe (in words).
    LeftOut(String),
    /// A step turned off in the steps panel (kept so it can be turned on).
    Off(RecipeOp),
}

/// A paste of this many cells or more gets an automatic checkpoint.
const BIG_PASTE_CELLS: usize = 1_000;

/// Most automatic checkpoints a tab keeps (the oldest goes first) and most named ones.
const MAX_AUTO_CHECKPOINTS: usize = 20;
const MAX_NAMED_CHECKPOINTS: usize = 200;

/// A named moment in the undo history: a mark (a few bytes, see `Document::step_mark`), no copy of the table.
#[derive(Clone)]
struct Checkpoint {
    id: u32,
    name: String,
    mark: u64,
    /// Made by the app before a risky change, not by the person.
    auto: bool,
    /// A save let it go: when (milliseconds since 1970) and why.
    gone: Option<(u64, &'static str)>,
}

/// A tab's checkpoints, in the order they were made. They are not saved to disk and go with the tab.
#[derive(Default)]
struct Checkpoints {
    next: u32,
    list: Vec<Checkpoint>,
}

impl Checkpoints {
    fn add(&mut self, name: String, mark: u64, auto: bool) -> u32 {
        self.next += 1;
        let id = self.next;
        self.list.push(Checkpoint {
            id,
            name,
            mark,
            auto,
            gone: None,
        });
        let cap = if auto {
            MAX_AUTO_CHECKPOINTS
        } else {
            MAX_NAMED_CHECKPOINTS
        };
        while self.list.iter().filter(|c| c.auto == auto).count() > cap {
            if let Some(i) = self.list.iter().position(|c| c.auto == auto) {
                self.list.remove(i);
            }
        }
        id
    }
}

/// A "since this checkpoint" plan and what it was made for.
struct KeptSince {
    checkpoint: u32,
    header: bool,
    revision: u64,
    plan: Arc<SincePlan>,
}

/// What changed since a checkpoint, as the screen asks for it.
#[derive(Debug, Clone, Serialize)]
pub struct SinceView {
    pub totals: natendo_engine::document::since::SinceTotals,
    pub cells: Vec<natendo_engine::document::since::SinceCell>,
}

/// One checkpoint as the panel shows it.
#[derive(Debug, Clone, Serialize)]
pub struct CheckpointInfo {
    pub id: u32,
    pub name: String,
    pub auto: bool,
    /// The undo history still reaches it (the engine's answer).
    pub reachable: bool,
    /// How many undos (negative) or redos (positive) get there; 0 when it is the state now or not reachable.
    pub steps: i64,
    /// The table is in exactly this state now.
    pub current: bool,
    /// Why "What changed since this?" cannot be answered ("" when it can).
    pub since_note: String,
    /// When a save let it go (milliseconds since 1970), and why ("a column change", ...).
    pub gone_at: Option<u64>,
    pub gone_why: Option<&'static str>,
}

/// What a bulk change is called in "Before ..." and whether it gets an automatic checkpoint. No wildcard: a new
/// kind of recipe step cannot be added without deciding here.
fn auto_label_op(op: &RecipeOp) -> Option<&'static str> {
    match op {
        RecipeOp::ReplaceAll { .. } => Some("Replace all"),
        RecipeOp::BatchReplace { .. } => Some("Batch replace"),
        RecipeOp::RemoveDuplicates { .. } => Some("Remove duplicates"),
        RecipeOp::RemoveBlankRows { .. } => Some("Remove blank rows"),
        RecipeOp::RemoveBlankCols { .. } => Some("Remove blank columns"),
        RecipeOp::SplitRows { .. } => Some("Split into rows"),
        RecipeOp::ExplodeRows { .. } => Some("Unnest to rows"),
        RecipeOp::MergeRows { .. } => Some("Merge rows"),
        RecipeOp::Clean { .. } | RecipeOp::CleanMatrix { .. } => Some("Clean-up"),
        RecipeOp::DeleteHidden => Some("Delete hidden rows"),
        RecipeOp::JoinFields { .. } => Some("Fix all like this"),
        RecipeOp::MapValues { .. } => Some("Change values from a list"),
        // Cheap to undo, or they change only the view: no checkpoint (and no crowding of the 20).
        RecipeOp::Sort { .. }
        | RecipeOp::SetAside { .. }
        | RecipeOp::Filter { .. }
        | RecipeOp::ClearFilter
        | RecipeOp::InsertCols { .. }
        | RecipeOp::DeleteCols { .. }
        | RecipeOp::MoveCols { .. }
        | RecipeOp::SplitColumn { .. }
        | RecipeOp::ExplodeColumns { .. }
        | RecipeOp::MergeColumns { .. }
        | RecipeOp::SortColumns { .. }
        | RecipeOp::SplitName { .. }
        | RecipeOp::FillExample { .. }
        | RecipeOp::Compute { .. }
        | RecipeOp::FillBlanks { .. }
        | RecipeOp::CountRepeats { .. }
        | RecipeOp::BlankRepeats { .. }
        | RecipeOp::Breaks { .. }
        | RecipeOp::GroupCalc { .. }
        | RecipeOp::Arrange { .. }
        | RecipeOp::Extract { .. }
        | RecipeOp::SortProblems { .. }
        | RecipeOp::CheckColumn { .. }
        | RecipeOp::RenameColumn { .. }
        | RecipeOp::TidyTitles { .. }
        | RecipeOp::CompareColumn { .. }
        | RecipeOp::ColumnTypes { .. } => None,
    }
}

/// The label of a change that is done but not repeatable by a recipe (it is named in words).
fn auto_label_left_out(words: &str) -> Option<&'static str> {
    const KINDS: [(&str, &str); 5] = [
        ("a join", "Join"),
        ("appending", "Append"),
        ("matching people", "Match people"),
        (
            "changes taken from a newer file",
            "Update from a newer file",
        ),
        ("changing values from a list", "Change values from a list"),
    ];
    KINDS
        .iter()
        .find(|(start, _)| words.starts_with(start))
        .map(|(_, label)| *label)
}

fn auto_label(logged: &Logged) -> Option<&'static str> {
    match logged {
        Logged::Step(op) | Logged::Off(op) => auto_label_op(op),
        Logged::LeftOut(words) => auto_label_left_out(words),
    }
}

/// One noted step: the history position before and after it.
#[derive(Clone)]
struct LogEntry {
    before: u64,
    pos: u64,
    item: Logged,
}

/// What a tab's recipe has so far: the first row before the first step
/// (the column titles), the steps with their place in the history, and
/// the questions the steps ask (see `step_ask`).
#[derive(Default)]
struct RecipeLog {
    first_row: Option<Vec<String>>,
    steps: Vec<LogEntry>,
    questions: Vec<natendo_engine::recipe::Question>,
}

/// One row of the steps panel.
#[derive(Debug, Clone, Serialize)]
pub struct StepRow {
    /// "step", "off" (turned off) or "left_out" (cannot be done again).
    pub kind: &'static str,
    pub text: String,
    /// The table is now after this step (else it was undone or peeked past).
    pub done: bool,
    /// The step's settings (steps that can run again).
    pub op: Option<RecipeOp>,
    /// Its text values, where a question can go: (place, value).
    pub texts: Vec<(String, String)>,
}

/// The steps panel: the tab's steps and where the table is among them.
#[derive(Debug, Clone, Serialize)]
pub struct StepsView {
    pub steps: Vec<StepRow>,
    /// The table is after this many steps (`steps.len()`: the latest).
    pub at: usize,
    /// Changes by hand after the last step (undone by a change in the
    /// panel, which asks first).
    pub hand_changes: bool,
    pub questions: Vec<natendo_engine::recipe::Question>,
}

/// A change made in the steps panel to step `k`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepChange {
    Remove,
    /// Turn off, or back on.
    Toggle,
    MoveUp,
    MoveDown,
    /// New settings.
    Replace {
        op: RecipeOp,
    },
}

/// A recipe of one step (to check or answer it on its own).
fn one_step_recipe(op: RecipeOp) -> Recipe {
    Recipe {
        version: natendo_engine::recipe::RECIPE_VERSION,
        name: String::new(),
        columns: Vec::new(),
        header: false,
        steps: vec![op],
        left_out: Vec::new(),
        questions: Vec::new(),
    }
}

/// Undo or redo until the history is at `target`; refused (and put back)
/// when the undo history no longer reaches it.
fn history_to(d: &mut Document, target: u64) -> Result<(), Error> {
    let start = d.history_pos();
    while d.history_pos() > target {
        if !d.undo() {
            while d.history_pos() < start && d.redo() {}
            return Err(Error::Invalid(
                "the undo history no longer reaches that step (it keeps a limited amount)".into(),
            ));
        }
    }
    while d.history_pos() < target {
        if !d.redo() {
            return Err(Error::Invalid("that step was undone and replaced".into()));
        }
    }
    Ok(())
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
        DataOp::ExplodeColumns { spec } => RecipeOp::ExplodeColumns { spec: spec.clone() },
        DataOp::ExplodeRows { spec } => RecipeOp::ExplodeRows { spec: spec.clone() },
        DataOp::ShowExplodeBad { .. } => {
            return Logged::LeftOut(
                "showing the cells that did not parse (a view, not a change)".into(),
            )
        }
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
        DataOp::Clean { spec, .. } => RecipeOp::Clean {
            spec: spec.recorded(),
        },
        DataOp::CleanMatrix { spec, .. } => RecipeOp::CleanMatrix {
            spec: spec.recorded(),
        },
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
        DataOp::FillRect { .. } => {
            return Logged::LeftOut("clearing or filling the selection".into())
        }
        DataOp::ShiftCells { .. } => return Logged::LeftOut("inserting or deleting cells".into()),
        DataOp::FillSpecial { .. } => {
            return Logged::LeftOut("filling the cells Go To Special picked".into())
        }
        DataOp::SortProblems { spec, scope } => RecipeOp::SortProblems {
            spec: spec.clone(),
            scope: *scope,
        },
        DataOp::CheckColumn { spec, title } => RecipeOp::CheckColumn {
            spec: spec.clone(),
            title: title.clone(),
        },
        DataOp::ShowDuplicates { .. } | DataOp::ShowProblems { .. } | DataOp::ShowShape { .. } => {
            return Logged::LeftOut(
                "showing duplicates, problems or a pattern (a view, not a change)".into(),
            )
        }
        DataOp::ShowRagged { .. } => {
            return Logged::LeftOut("showing ragged rows (a view, not a change)".into())
        }
        DataOp::ShowContractRows { .. } => {
            return Logged::LeftOut(
                "showing rows a file contract found (a view, not a change)".into(),
            )
        }
        DataOp::ShowRelationExceptions { .. } => {
            return Logged::LeftOut(
                "showing the exceptions to a relationship (a view, not a change)".into(),
            )
        }
        DataOp::ShowGapRows { .. } => {
            return Logged::LeftOut("showing the rows next to gaps (a view, not a change)".into())
        }
        DataOp::CompareShow { .. } => {
            return Logged::LeftOut(
                "showing rows where two columns differ (a view, not a change)".into(),
            )
        }
        DataOp::FormulaFilter { .. } => {
            return Logged::LeftOut("filtering by a formula (a view, not a change)".into())
        }
        DataOp::CompareColumn { spec } => RecipeOp::CompareColumn { spec: spec.clone() },
        DataOp::Join { .. } => return Logged::LeftOut("a join with another tab or a list".into()),
        DataOp::Append { .. } => return Logged::LeftOut("appending another tab".into()),
    };
    Logged::Step(s)
}

/// The recipe step for a set of column types: each typed column by its title (the first row; the position
/// when it has none), every other column left Auto.
fn types_step(d: &Document, types: &natendo_engine::coltype::ColumnTypes) -> RecipeOp {
    let titles = d.row_values(0).unwrap_or_default();
    let mut at = Vec::new();
    for c in 0..d
        .col_count()
        .min(natendo_engine::coltype::MAX_TYPED_COLS as u32)
    {
        let Some(p) = d.phys_of(c) else { continue };
        let t = types.get(p);
        if t != natendo_engine::coltype::ColumnType::Auto {
            at.push(natendo_engine::recipe::ColumnTypeAt {
                title: titles.get(c as usize).cloned().unwrap_or_default(),
                col: c,
                kind: t,
            });
        }
    }
    RecipeOp::ColumnTypes { types: at }
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
        RecipeOp::ExplodeColumns { spec } => DataOp::ExplodeColumns { spec },
        RecipeOp::ExplodeRows { spec } => DataOp::ExplodeRows { spec },
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
        RecipeOp::Clean { spec } => DataOp::Clean {
            typed: Vec::new(),
            spec,
            now,
        },
        RecipeOp::CleanMatrix { spec } => DataOp::CleanMatrix {
            spec,
            now,
            revision: None,
        },
        RecipeOp::SplitName { spec } => DataOp::SplitName { spec },
        RecipeOp::FillExample { spec } => DataOp::FillExample { spec },
        RecipeOp::Compute { spec } => DataOp::Compute { spec },
        RecipeOp::FillBlanks { spec } => DataOp::FillBlanks { spec },
        RecipeOp::CompareColumn { spec } => DataOp::CompareColumn { spec },
        RecipeOp::CountRepeats { spec } => DataOp::CountRepeats { spec },
        RecipeOp::BlankRepeats { spec } => DataOp::BlankRepeats { spec },
        RecipeOp::Breaks { spec } => DataOp::Breaks { spec },
        RecipeOp::DeleteHidden => DataOp::DeleteHidden,
        RecipeOp::MapValues { spec } => DataOp::MapValues { spec, from: None },
        RecipeOp::GroupCalc { spec } => DataOp::GroupCalc { spec },
        RecipeOp::Arrange { spec } => DataOp::Arrange { spec },
        RecipeOp::Extract { spec } => DataOp::Extract { spec },
        RecipeOp::SortProblems { mut spec, scope } => {
            spec.now = now;
            DataOp::SortProblems { spec, scope }
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
        | RecipeOp::ReplaceAll { .. }
        | RecipeOp::BatchReplace { .. }
        | RecipeOp::RenameColumn { .. }
        | RecipeOp::TidyTitles { .. }
        | RecipeOp::JoinFields { .. }
        | RecipeOp::SetAside { .. }
        | RecipeOp::ColumnTypes { .. } => unreachable!("handled by its own command"),
    }
}

/// Most ragged rows the repair bar steps through.
pub const MAX_RAGGED_LISTED: usize = 100_000;

/// The ragged rows for the repair bar.
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct RaggedList {
    pub ids: Vec<u32>,
    pub total: u64,
}

/// What a CSV report wrote.
#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
pub struct Written {
    /// Lines of the list.
    pub entries: u64,
    /// Cells given an apostrophe by the formula guard.
    pub guarded: u64,
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
    /// Cells given an apostrophe by the formula guard.
    pub guarded: u64,
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

/// The totals row's results kept for one state of a document.
#[derive(Default)]
struct ColumnTotalsCache {
    /// (revision, title row, how text is read).
    key: Option<(u64, bool, ReadOptions)>,
    found: HashMap<u32, ColumnTotal>,
    /// Stops the work for `key`.
    job: Option<Arc<Progress>>,
}

impl ColumnTotalsCache {
    /// Stop the work under way; finished results stay.
    fn stop(&mut self) {
        if let Some(p) = self.job.take() {
            p.cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ColumnTotalsOut {
    pub revision: u64,
    /// Data rows shown (and so totalled).
    pub rows: u64,
    /// Some rows are hidden or filtered out.
    pub filtered: bool,
    pub columns: Vec<ColumnTotal>,
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

/// Values a dictionary does not know.
#[derive(Debug, Serialize, Clone, PartialEq, Default)]
pub struct UnmatchedOut {
    /// The first 5,000, most common first.
    pub values: Vec<natendo_engine::editing::Tally>,
    /// How many such values, and the rows that hold them.
    pub count: u64,
    pub rows: u64,
    /// False when the column has more values than were counted.
    pub complete: bool,
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
    /// Paste, fill, clear: hidden rows between the rows written, left as
    /// they were.
    pub hidden_rows: u64,
    /// Fill: the custom order it followed, by name.
    pub order: Option<String>,
    /// Join and append: what matched.
    pub join: Option<JoinReport>,
    /// Cleaners: cells a step could not read (not a date or number);
    /// keep newest/oldest: copies whose date could not be read.
    pub left: u64,
    /// Cleaners: dates read the other way round from the chosen day order.
    pub other_order: u64,
    /// Cleaners: dates both day orders can read, left as typed.
    pub ambiguous_left: u64,
    /// Fill by example: the pattern used, in words.
    pub pattern: Option<String>,
    /// Fill by example: examples learned from.
    pub examples: u64,
    /// Computed columns: cells that came out as error codes.
    pub errors: u64,
    /// Fill blank cells with stops: cells marked amber (at most 100,000 of
    /// `skipped`).
    pub marked: u64,
    /// Explode cells: cells that did not parse, new and missing keys.
    pub explode: Option<natendo_engine::explode::ExplodeReport>,
    /// Fill by example: the rule it filled with and its examples' row ids (counting the odd
    /// results afterwards needs them).
    pub fill_rule: Option<FillRule>,
}

/// The rule a fill by example used.
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct FillRule {
    pub program: Program,
    pub example_ids: Vec<u32>,
}

/// Changes listed, and rows looked at, by a cleaner preview.
const CLEAN_PREVIEW_CHANGES: usize = 50;
const CLEAN_PREVIEW_ROWS: u64 = 20_000;
/// Rows the paged preview of a cleaner looks at (every count is exact within them).
const CLEAN_PAGE_ROWS: u64 = 5_000_000;

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

/// The reading counter behind `DocInfo::seq`.
static INFO_SEQ: AtomicU64 = AtomicU64::new(0);

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
    /// Moves when what the rows hold changes (not with filters).
    pub data_revision: u64,
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
    /// A mark of the state the table is in (see `Document::step_mark`): a checkpoint keeps it, a few bytes.
    pub step_mark: u64,
    /// What the person says the columns hold (by physical column, for views and Undo).
    pub column_types: natendo_engine::coltype::ColumnTypes,
    /// The typed columns as shown (logical column, type); Auto columns are left out.
    pub typed: Vec<(u32, natendo_engine::coltype::ColumnType)>,
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
    /// Looking at an earlier recipe step: changes and saving are off.
    pub peeking: bool,
    /// An answer made while a change held the document: the figures are from the last time they
    /// could be read (the live parts are current). The UI never takes a stale answer as the tab's
    /// state and asks again; see `ui/src/lib/infoguard.ts`.
    pub stale: bool,
    /// Rises with every reading made (every reply and every answer): a reply older than what the tab
    /// holds was built before it and is dropped. A stale answer keeps the number of its reading.
    pub seq: u64,
    /// Rows the user hid (still saved).
    pub hidden_rows: u64,
    /// Protected columns (changes pass them over).
    pub protected: Vec<u32>,
    /// Cells the last change left alone in protected columns.
    pub protected_skipped: u64,
    /// A filter is on (`filtered` is also true while rows are only hidden).
    pub has_filter: bool,
    pub line_break: &'static str,
    pub mixed_line_breaks: bool,
    /// Decoding replaced bytes: saving needs consent.
    pub lossy: bool,
    pub invalid_bytes: bool,
    /// The number of fields most rows of the file have (0 until indexed),
    /// the rows of the file with another number, and whether the first row is
    /// one of them. From counts kept while reading the file.
    /// What the steps of a recipe just run have to say (only in the answer
    /// to running one).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub run_notes: Vec<String>,
    /// Only in a save's answer: what the save did with the undo history.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_history: Option<natendo_engine::document::HistoryKept>,
    pub usual_fields: u32,
    pub ragged_rows: u64,
    pub first_row_ragged: bool,
    pub line_by_line: bool,
    pub column_changes: bool,
    pub repair_count: u64,
    pub unclosed_quotes: bool,
    pub long_quoted_cells: bool,
    /// First row read line by line after a quote that never closed.
    pub line_bound_from: Option<u64>,
    /// Report notes: lines above and below the table (C4).
    pub notes: NoteCounts,
    pub error: Option<String>,
    /// C11: the tab saves back to a JSON file: "jsonl" (JSON Lines) or "json" (an array).
    pub json_format: Option<&'static str>,
    /// C11: why the JSON file can only be saved as another file (a key twice in a record, lists of
    /// values). Title problems are found when saving (see `Docs::json_check`).
    pub json_refuse: Option<String>,
    /// C11: the JSON file it saves back to (shown in tooltips and used, hashed, to remember the blank-cell
    /// answer; never accepted back from the UI).
    pub json_file: Option<String>,
    /// Only on the answer to inserting or deleting rows: the document
    /// positions changed, so custom row heights can follow them while a
    /// filter is on (`None` past `MAX_ROWS_CHANGE` deleted rows).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows_change: Option<RowsChange>,
}

/// Rows inserted or deleted, by document position (see `DocInfo::rows_change`).
#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RowsChange {
    /// `count` rows were inserted starting at document position `at`.
    Insert { at: u64, count: u64 },
    /// These runs `(first, count)` were deleted, positions as before.
    Delete { runs: Vec<(u64, u64)> },
    /// These runs `(first, count)` (positions as before) were taken out and
    /// put together, in their order, starting at `to` (a position once they
    /// were out).
    Move { runs: Vec<(u64, u64)>, to: u64 },
}

/// Most rows a delete reports by position (more: the heights are cleared).
pub const MAX_ROWS_CHANGE: u64 = 100_000;

/// The JSON file a tab came from (C11), and its size and time when last read or written.
#[derive(Clone)]
struct JsonTarget {
    path: PathBuf,
    stamp: Option<natendo_engine::source::FileStamp>,
    /// How it is written (from the file as it came, or the last Save As).
    form: natendo_engine::jsonshape::JsonForm,
}

/// JSON Lines, LF, ending with a line break: how a tab with no JSON file of its own is saved as one.
fn lines_form() -> natendo_engine::jsonshape::JsonForm {
    natendo_engine::jsonshape::JsonForm {
        lines: true,
        indent: None,
        crlf: false,
        bom: false,
        final_newline: true,
    }
}

/// After a JSON save: the tab, and what the save wrote.
#[derive(Debug, Clone, Serialize)]
pub struct JsonSaved {
    pub info: DocInfo,
    pub report: natendo_engine::jsonshape::JsonReport,
    /// The file written was the tab's own (it is now saved), not a copy beside a file of its own.
    pub own: bool,
}

/// Largest file compared with its copy on disk (both are held at once).
const MAX_DISK_COMPARE: u64 = 512 << 20;

/// All open documents.
#[derive(Default)]
pub struct Docs {
    next: AtomicU32,
    map: Mutex<HashMap<u32, Arc<Entry>>>,
    /// Comparisons of lists (by number) kept while their screen is open.
    sets: Mutex<HashMap<u64, SetsCache>>,
    next_sets: std::sync::atomic::AtomicU64,
    /// Comparisons let go to save memory (to say so when one is asked for).
    sets_dropped: Mutex<std::collections::HashSet<u64>>,
    /// Kept while a Join screen is open, one per tab the screen is for
    /// (so two windows never share one): the indexed other table (or
    /// list) and pasted lists, so previews do not copy them again.
    joins: Mutex<HashMap<u32, JoinCache>>,
    /// Key-aware diffs (by number) kept while their tabs are open.
    diffs: Mutex<HashMap<u64, DiffState>>,
    next_diff: AtomicU64,
    /// Numbers given to pasted lists.
    next_list: std::sync::atomic::AtomicU64,
    /// LOOKUP tables by tab, its revision and the two columns.
    lookups: Mutex<Vec<(String, Arc<LookupTable>)>>,
    /// Value sets of "value must exist in" rules by tab, its revision, the column and the
    /// way of comparing (kept while that tab is unchanged; dropped when it closes).
    key_sets: Mutex<Vec<(String, Arc<KeySet>)>>,
    /// What the last save with a receipt said about its receipt, by tab (read once by the window).
    receipt_notes: Mutex<HashMap<u32, String>>,
    /// The files picked for Combine with a column map (their paths stay here; the window gets names and heads).
    combine: Mutex<Option<CombineSession>>,
    /// Whether a path is in private file mode (set when the app starts;
    /// the recent list keeps the private files and folders).
    private_check: Mutex<Option<PrivateCheck>>,
    /// The file contracts read from the contracts folder (C9), read once when first needed and
    /// again after a change.
    contracts: Mutex<Option<contracts::Store>>,
    /// The user's saved custom orders the fill handle may follow (set by
    /// the window when they change; never logged).
    fill_orders: Mutex<Arc<natendo_engine::fill::OrderBook>>,
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
/// Most value sets kept for "value must exist in" rules.
const MAX_KEY_SETS: usize = 8;

/// The one open tab called `table` (or that without its extension), the way LOOKUP finds
/// tabs: else why not ("open X", or two tabs with that name).
fn tab_named(names: &[(u32, String)], table: &str) -> Result<u32, String> {
    let want = table.trim().to_lowercase();
    let hits: Vec<u32> = names
        .iter()
        .filter(|(_, n)| {
            let n = n.to_lowercase();
            n == want || n.rsplit_once('.').is_some_and(|(stem, _)| stem == want)
        })
        .map(|(i, _)| *i)
        .collect();
    match hits.as_slice() {
        [one] => Ok(*one),
        [] => Err(format!("open {}", table.trim())),
        _ => Err(format!(
            "two open files are called {}: close one",
            table.trim()
        )),
    }
}

/// Memory all kept value sets may hold together: a quarter of the computer's memory, at most 2 GB.
fn key_sets_budget() -> u64 {
    natendo_engine::sysmem::installed_bytes()
        .map_or(1 << 30, |b| b / 4)
        .min(2 << 30)
}

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
    /// Every document with unsaved changes (in any window), the ones that
    /// cannot be looked at right now because something holds them (a save or
    /// a long change), and the long jobs running. `generation` counts every
    /// edit, undo and redo of the document (it never goes back), so a change
    /// made after a question was asked is seen even when the undo position is
    /// the same again.
    pub fn modified_report(&self) -> ModifiedReport {
        let entries: Vec<(u32, Arc<Entry>)> =
            self.map().iter().map(|(k, v)| (*k, v.clone())).collect();
        let mut report = ModifiedReport::default();
        for (id, e) in entries {
            let name = e
                .current_path()
                .as_deref()
                .and_then(Path::file_name)
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Untitled".into());
            if let Some(j) = e.jobs().as_ref() {
                report.jobs.push(j.label.to_string());
            }
            let seen = match e.doc.try_read() {
                Ok(d) => Some((d.is_modified(), d.revision())),
                Err(std::sync::TryLockError::Poisoned(p)) => {
                    let d = p.into_inner();
                    Some((d.is_modified(), d.revision()))
                }
                Err(std::sync::TryLockError::WouldBlock) => None,
            };
            match seen {
                Some((true, generation)) => report.docs.push(ModifiedDoc {
                    id,
                    name,
                    generation,
                }),
                Some((false, _)) => {}
                None => report.busy.push(name),
            }
        }
        report.docs.sort_by_key(|d| d.id);
        report.busy.sort();
        report.jobs.sort();
        report
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
        let path = doc.path().map(Path::to_path_buf);
        let entry = Arc::new(Entry {
            path: Mutex::new(path),
            doc: RwLock::new(doc),
            job: Mutex::new(None),
            replace: Mutex::new(None),
            changed_rows: Mutex::new((0, Vec::new())),
            fill_marks: Mutex::new(HashMap::new()),
            json_target: Mutex::new(None),
            fill_count: Mutex::new(None),
            chip_count: Mutex::new(None),
            contract_count: Mutex::new(None),
            cancel: AtomicBool::new(false),
            line_by_line: AtomicBool::new(line_by_line),
            error: Mutex::new(None),
            title: Mutex::new(None),
            last_info: Mutex::new(None),
            locked: AtomicBool::new(false),
            peeking: AtomicBool::new(false),
            checks: Mutex::new(None),
            gaps: Mutex::new(None),
            relations: Mutex::new(None),
            totals: Mutex::new(None),
            column_totals: Mutex::new(ColumnTotalsCache::default()),
            find_count: Mutex::new(None),
            ghost_count: Mutex::new(None),
            find_blocks: Mutex::new(None),
            typeahead: Mutex::new(None),
            writers: AtomicU32::new(0),
            closed: AtomicBool::new(false),
            changed_at: AtomicU64::new(0),
            autosaving: AtomicBool::new(false),
            private: AtomicBool::new(false),
            private_origin: AtomicBool::new(false),
            encrypted_from: Mutex::new(None),
            golden: Mutex::new(None),
            patch: Mutex::new(None),
            recipe: Mutex::new(RecipeLog::default()),
            checkpoints: Mutex::new(Checkpoints::default()),
            since_plan: Mutex::new(None),
            facets: Mutex::new(natendo_engine::filter::FacetCache::new()),
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
        self.open_with_encoding(path, line_by_line, layout, None)
    }

    /// `open_with`, read as the encoding given (`None`: detect it). A new document: the one it comes from stays
    /// open until the caller closes it, so a Cancel or an error keeps the old one.
    pub fn open_with_encoding(
        &self,
        path: &Path,
        line_by_line: bool,
        layout: Option<&Layout>,
        encoding: Option<Encoding>,
    ) -> CmdResult<DocInfo> {
        let mut opts = OpenOptions {
            line_by_line,
            encoding,
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
        self.diff_clear(id);
        let gone = self.map().remove(&id);
        if let Some(e) = &gone {
            e.cancel.store(true, Ordering::Relaxed);
            e.closed.store(true, Ordering::SeqCst);
        }
        self.sets
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|_, c| c.tabs.iter().all(|t| t.0 != id));
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
        self.key_sets
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(k, _)| !k.starts_with(&mine));
        crate::recovery::forget(id);
        // Freeing a big document (millions of rows of index and edits) takes a while: it is done on its own
        // thread, never on the window's.
        if let Some(e) = gone {
            std::thread::spawn(move || drop(e));
        }
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
            let Some(p) = e.current_path() else {
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
    pub fn save_change_report(&self, id: u32, path: &Path, guard: bool) -> CmdResult<Written> {
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
        let list = d.save_change_report(path, &titles, guard, &job.progress)?;
        Ok(Written {
            entries: list.cells_total + list.added_total + list.deleted_total,
            guarded: list.guarded,
        })
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
        self.refuse_peeking(id)?;
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
        // While looking back, the copy would hold the earlier version.
        if entry.jobs().is_some()
            || entry.private.load(Ordering::SeqCst)
            || entry.peeking.load(Ordering::Relaxed)
        {
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
    /// What to compare each open tab's file with (`None`: no file, or the
    /// tab is being changed or saved just now).
    pub fn disk_marks(&self) -> Vec<(u32, Option<DiskMark>)> {
        let mut ids = self.ids();
        ids.sort_unstable();
        ids.into_iter().map(|id| (id, self.disk_mark(id))).collect()
    }

    /// One tab's mark, without waiting for a change or a save to finish.
    pub fn disk_mark(&self, id: u32) -> Option<DiskMark> {
        let entry = self.get(id).ok()?;
        if entry.peeking.load(Ordering::Relaxed) || entry.change_waiting() {
            return None;
        }
        let d = match entry.doc.try_read() {
            Ok(d) => d,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return None,
        };
        d.disk_mark()
    }

    /// The disk's version of a tab's file and its differences from what the
    /// tab last read or saved (edits not saved are not part of it).
    pub fn disk_compare(&self, id: u32, header: bool) -> CmdResult<(DocInfo, DocInfo)> {
        let entry = self.get(id)?;
        let (disk, differences, path, name) = {
            let job = entry.start("Comparing with the file on disk")?;
            let d = entry.read();
            if d.source_bytes() > MAX_DISK_COMPARE {
                return Err(CmdError::new(
                    "too_large",
                    "this file is too large to hold twice; open the file on disk again and use Compare",
                ));
            }
            let path = d
                .path()
                .ok_or_else(|| CmdError::new("no_path", "this table has no file"))?
                .to_path_buf();
            if std::fs::metadata(&path)
                .map_err(natendo_engine::Error::from)?
                .len()
                > MAX_DISK_COMPARE
            {
                return Err(CmdError::new(
                    "too_large",
                    "the file on disk is too large to compare here",
                ));
            }
            let base = d.as_last_read()?;
            let disk = d.as_on_disk(&job.progress)?;
            let spec = CompareSpec {
                keep_first: header,
                ..Default::default()
            };
            let (t, _) = base.compare(&disk, &spec, &job.progress)?;
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            (disk, t.into_document()?, path, name)
        };
        // The disk's version has no path here: saving it can never replace
        // the file by accident.
        let mut copy = disk;
        copy.forget_path();
        let copy = self.insert_titled(copy, Some(format!("{name} (on disk)")))?;
        let report = self.insert_titled(differences, Some(format!("Changes in {name}")))?;
        for made in [copy.id, report.id] {
            self.private_if_from(made, &path);
            if entry.private.load(Ordering::SeqCst) {
                self.set_private(made, true)?;
            }
        }
        Ok((self.info(copy.id)?, self.info(report.id)?))
    }

    pub fn ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.map().keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// The open document showing this file, if any. A document busy with
    /// a long operation still counts (its path is kept outside its lock).
    pub fn find_open(&self, path: &Path) -> Option<u32> {
        self.find_open_many(std::slice::from_ref(&path.to_path_buf()))[0]
    }

    /// `find_open` for several files at once: each open document's path is
    /// resolved once, and paths are compared as text first; a file is only
    /// looked at on disk (same file under another name: a mapped drive and
    /// its network share) when the file names match.
    pub fn find_open_many(&self, paths: &[PathBuf]) -> Vec<Option<u32>> {
        let mut entries: Vec<(u32, Arc<Entry>)> =
            self.map().iter().map(|(k, v)| (*k, v.clone())).collect();
        entries.sort_unstable_by_key(|(id, _)| *id);
        let (ids, open): (Vec<u32>, Vec<(PathBuf, String)>) = entries
            .iter()
            .filter_map(|(id, e)| {
                let p = e.current_path()?;
                let k = path_key(&p);
                Some((*id, (p, k)))
            })
            .unzip();
        if open.is_empty() {
            return vec![None; paths.len()];
        }
        paths
            .iter()
            .map(|want| match_path(&open, want).map(|i| ids[i]))
            .collect()
    }

    /// For each remembered file at start: open already, or one of the files
    /// Windows passed this run (it may still be opening). Launch files are
    /// matched by the same rule as open ones.
    pub fn open_or_launching(&self, launch: &[PathBuf], paths: &[PathBuf]) -> Vec<bool> {
        let open = self.find_open_many(paths);
        let launch: Vec<(PathBuf, String)> =
            launch.iter().map(|p| (p.clone(), path_key(p))).collect();
        open.into_iter()
            .zip(paths)
            .map(|(o, p)| o.is_some() || (!launch.is_empty() && match_path(&launch, p).is_some()))
            .collect()
    }

    /// The file of each document (none for one without a file), read
    /// without waiting for a busy document.
    pub fn paths_of(&self, ids: &[u32]) -> Vec<Option<PathBuf>> {
        ids.iter()
            .map(|id| self.get(*id).ok().and_then(|e| e.current_path()))
            .collect()
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

    /// The tab's file; never waits for the document (while a change holds it, the path noted at
    /// the last reading is answered).
    pub fn path_of(&self, id: u32) -> CmdResult<Option<PathBuf>> {
        Ok(self.get(id)?.current_path())
    }

    /// The document's info as it is now: waits for a change to finish. This is the reply of every
    /// operation (save, move, recipe, find, ...), which already runs on the blocking pool, so what the
    /// tab takes from it is never behind the change it just made.
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
        Ok(self.info_waiting(&entry, id, error, title))
    }

    /// The document's info for a QUESTION (the `doc_info` command, read-only, the agent): never waits.
    /// While a change holds the document (or is waiting to take it) it answers from the last reading,
    /// marked stale, with the live parts current. Never use it as the reply of an operation.
    pub fn info_now(&self, id: u32) -> CmdResult<DocInfo> {
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
        let read_only = entry.locked.load(Ordering::Relaxed);
        let peeking = entry.peeking.load(Ordering::Relaxed);
        // A change holds the document: answer from the last reading (stale), with the parts that
        // do not need the lock brought up to date, instead of making the caller wait.
        let Some(d) = entry.try_read() else {
            let last = entry
                .last_info
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            if let Some(mut info) = last {
                info.read_only = read_only;
                info.peeking = peeking;
                info.stale = true;
                if let (None, Some(t)) = (&info.path, title) {
                    info.name = t;
                }
                return Ok(info);
            }
            // Nothing read yet (the very first question came during a change): wait for it. The
            // commands that call this run on the blocking pool, never on the window's thread.
            return Ok(self.info_waiting(&entry, id, error, title));
        };
        Ok(Self::info_from(&entry, id, &d, error, title))
    }

    /// `info`, waiting for the document's lock (for the first answer only).
    fn info_waiting(
        &self,
        entry: &Entry,
        id: u32,
        error: Option<String>,
        title: Option<String>,
    ) -> DocInfo {
        let d = entry.read();
        Self::info_from(entry, id, &d, error, title)
    }

    fn info_from(
        entry: &Entry,
        id: u32,
        d: &Document,
        error: Option<String>,
        title: Option<String>,
    ) -> DocInfo {
        entry.note_path(d);
        let mut info = describe(id, d, entry.line_by_line.load(Ordering::Relaxed), error);
        info.read_only = entry.locked.load(Ordering::Relaxed);
        info.peeking = entry.peeking.load(Ordering::Relaxed);
        if let Some(t) = &*entry.json_target.lock().unwrap_or_else(|p| p.into_inner()) {
            info.json_format = Some(if t.form.lines { "jsonl" } else { "json" });
            info.json_refuse = d.json_shape().and_then(|s| s.refuse.clone());
            info.json_file = Some(t.path.display().to_string());
        }
        if let (None, Some(t)) = (&info.path, title) {
            info.name = t;
        }
        *entry.last_info.lock().unwrap_or_else(|p| p.into_inner()) = Some(info.clone());
        info
    }

    /// The tab's name without its extension ("voters" for voters.csv), for
    /// naming tabs made from it.
    pub fn stem(&self, id: u32) -> String {
        let name = self.info(id).map(|i| i.name).unwrap_or_default();
        match name.rsplit_once('.') {
            Some((s, ext)) if !s.is_empty() && ext.len() <= 5 && !ext.contains(' ') => {
                s.to_string()
            }
            _ => name,
        }
    }

    fn set_title_of(&self, id: u32, title: String) {
        if let Ok(entry) = self.get(id) {
            *entry.title.lock().unwrap_or_else(|p| p.into_inner()) = Some(clean_title(&title));
        }
    }

    /// Protect these columns from changes (and no others). Not an undo step:
    /// a setting of the tab, like a column's width.
    pub fn set_protected(&self, id: u32, cols: Vec<u32>) -> CmdResult<DocInfo> {
        if cols.len() > 100_000 {
            return Err(CmdError::new("bad_request", "too many columns"));
        }
        let entry = self.get(id)?;
        entry.write().set_protected(&cols);
        drop(entry);
        self.info(id)
    }

    /// Name a tab that has no file yet (Rename on a new or tool-made tab);
    /// the name is also what Save suggests. Empty: back to the default.
    pub fn set_title(&self, id: u32, title: &str) -> CmdResult<DocInfo> {
        let entry = self.get(id)?;
        if entry.read().path().is_some() {
            return Err(CmdError::new(
                "has_path",
                "this tab is a saved file: use Rename file to rename it",
            ));
        }
        let t = clean_title(title);
        *entry.title.lock().unwrap_or_else(|p| p.into_inner()) = (!t.is_empty()).then_some(t);
        drop(entry);
        self.info(id)
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

    /// Column `col` of the table's first row (the header row), wherever
    /// filters or hidden rows leave it.
    pub fn first_row_cell(&self, id: u32, col: u32) -> CmdResult<String> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.first_row_cell(col)?)
    }

    /// The table's first row (the header row), at most `cols` values, each
    /// cut to `chars` characters: other columns' names, to warn of a repeat.
    pub fn first_row_names(&self, id: u32, cols: usize, chars: usize) -> CmdResult<Vec<String>> {
        // Bounded: at most MAX_NAME_COLS names of `chars` characters.
        let cols = cols.min(MAX_NAME_COLS);
        let entry = self.get(id)?;
        let d = entry.read();
        let mut v = d.first_row_values()?;
        v.truncate(cols);
        for s in &mut v {
            if let Some((i, _)) = s.char_indices().nth(chars) {
                s.truncate(i);
            }
        }
        Ok(v)
    }

    /// Rename a column: set its cell in the table's first row, one undo step
    /// (noted for the tab's recipe, so a recipe recorded across a rename
    /// replays it before the steps that use the new name).
    pub fn set_first_row_cell(&self, id: u32, col: u32, value: String) -> CmdResult<DocInfo> {
        check_write(1, std::iter::once(value.len()))?;
        let step = RecipeOp::RenameColumn {
            col,
            name: value.clone(),
        };
        self.recorded(id, Logged::Step(step), || {
            self.edit(id, |d| {
                check_width(d, col as u64 + 1)?;
                d.set_first_row_cell(col, value)
            })
        })
    }

    /// Copy column names: the names (the table's first row, in full) of
    /// columns `cols`, in that order, as one line split by `sep` and quoted
    /// where a name holds `sep`, a quote or a line break.
    pub fn column_names_line(&self, id: u32, cols: &[u32], sep: char) -> CmdResult<String> {
        let entry = self.get(id)?;
        let d = entry.read();
        let names = d.first_row_values()?;
        let mut out = String::new();
        for (i, c) in cols.iter().enumerate() {
            if i > 0 {
                out.push(sep);
            }
            let v = names.get(*c as usize).map(String::as_str).unwrap_or("");
            if v.contains([sep, '"', '\n', '\r']) {
                out.push('"');
                out.push_str(&v.replace('"', "\"\""));
                out.push('"');
            } else {
                out.push_str(v);
            }
            if out.len() > MAX_COPY_BYTES {
                return Err(CmdError::new("too_large", "the names are too long to copy"));
            }
        }
        Ok(out)
    }

    /// The ids of the ragged rows in table order (hidden ones too), at most
    /// [`MAX_RAGGED_LISTED`], and how many there are.
    pub fn ragged_list(&self, id: u32, keep_first: bool) -> CmdResult<RaggedList> {
        let entry = self.get(id)?;
        let job = entry.start("Looking for ragged rows")?;
        let d = entry.read();
        let found = d.ragged_rows(keep_first, &job.progress)?;
        let (ids, total) = d.ragged_ids(&found, MAX_RAGGED_LISTED);
        Ok(RaggedList { ids, total })
    }

    /// What the repair bar shows for a row, asked by its id or (to open the
    /// bar from the grid) by the shown row. Nothing is stored or logged.
    pub fn row_text(
        &self,
        id: u32,
        row: Option<u64>,
        row_id: Option<u32>,
        header: bool,
    ) -> CmdResult<RowText> {
        let entry = self.get(id)?;
        let mut found = {
            let d = entry.read();
            let wanted = row_id
                .or_else(|| row.and_then(|r| d.row_id(r)))
                .ok_or_else(|| CmdError::new("bad_request", "choose a row"))?;
            d.row_text(wanted, header)?
        };
        // (Notes that cannot be read do not stop the row from showing.)
        let keys = |wanted: u32| {
            self.with_notes(id, |d, book, _| {
                Ok(if d.row_noted(book, wanted) {
                    book.key_cols.clone()
                } else {
                    Vec::new()
                })
            })
            .unwrap_or_default()
        };
        found.note_keys = keys(found.id);
        for near in [&mut found.next, &mut found.previous].into_iter().flatten() {
            near.note_keys = keys(near.id);
            near.noted = !near.note_keys.is_empty();
        }
        Ok(found)
    }

    /// The fields of a shown row and the usual number (that row only).
    pub fn row_width(&self, id: u32, row: u64) -> CmdResult<(u64, u32)> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.row_width(row)?)
    }

    /// One repair of one row, one undo step. Left out of recipes: it is about
    /// this file's row.
    pub fn repair_row(
        &self,
        id: u32,
        row_id: u32,
        revision: u64,
        repair: &Repair,
    ) -> CmdResult<DocInfo> {
        self.recorded(id, Logged::LeftOut("repairing one row".into()), || {
            self.edit(id, |d| d.repair_row(row_id, revision, repair))
        })
    }

    /// What "Fix all like this" would change.
    pub fn same_shape(
        &self,
        id: u32,
        keep_first: bool,
        fields: u32,
        groups: &[u32],
    ) -> CmdResult<SameShape> {
        let entry = self.get(id)?;
        let job = entry.start("Looking for rows of the same shape")?;
        let d = entry.read();
        Ok(d.same_shape(keep_first, fields, groups, &job.progress)?)
    }

    /// "Fix all like this": one undo step, after the automatic checkpoint,
    /// recorded as a recipe step that holds counts only. The rows changed.
    pub fn fix_all(
        &self,
        id: u32,
        keep_first: bool,
        revision: u64,
        fields: u32,
        groups: Vec<u32>,
        skip: &[u32],
    ) -> CmdResult<(u64, DocInfo)> {
        let usual = self.get(id)?.read().usual_fields();
        let step = RecipeOp::JoinFields {
            usual,
            fields,
            groups: groups.clone(),
            header: keep_first,
            unticked: skip.len() as u32,
        };
        let mut rows = 0;
        let mut info = self.recorded(id, Logged::Step(step), || {
            let entry = self.get(id)?;
            let job = entry.start("Fixing rows")?;
            self.edit(id, |d| {
                rows = d.fix_all(keep_first, revision, fields, &groups, skip, &job.progress)?;
                Ok(())
            })
        })?;
        if !skip.is_empty() {
            info.run_notes = vec![natendo_engine::recipe::unticked_note(
                skip.len() as u32,
                fields,
            )];
        }
        Ok((rows, info))
    }

    /// A recorded "Fix all like this", run again.
    fn replay_join_fields(
        &self,
        id: u32,
        usual: u32,
        fields: u32,
        groups: Vec<u32>,
        header: bool,
    ) -> CmdResult<u64> {
        let step = RecipeOp::JoinFields {
            usual,
            fields,
            groups: groups.clone(),
            header,
            unticked: 0,
        };
        let mut rows = 0;
        self.recorded(id, Logged::Step(step), || {
            let entry = self.get(id)?;
            let job = entry.start("Fixing rows")?;
            self.edit(id, |d| {
                rows = d.replay_join_fields(usual, fields, &groups, header, &job.progress)?;
                Ok(())
            })
        })?;
        Ok(rows)
    }

    /// The rule card (Data › Show me the rule): the rule fill by example would use for
    /// `spec.col` (`spec.program` when given, checked), or the one example that breaks it.
    pub fn example_learn(&self, id: u32, spec: &ExampleSpec) -> CmdResult<LearnReport> {
        let entry = self.get(id)?;
        let _job = entry.start("Working out the rule")?;
        let d = entry.read();
        Ok(d.example_learn(spec)?)
    }

    /// The first empty cells a rule would fill, with what it reads and writes (the first
    /// 1,000 rows looked at, at most 20 listed).
    pub fn example_preview(
        &self,
        id: u32,
        spec: &ExampleSpec,
        program: &Program,
    ) -> CmdResult<Preview> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.example_preview(spec, program)?)
    }

    /// The cells a rule fills and the odd results among them (`after`: the examples' row ids
    /// of a fill that has run, to look at what it filled). A job: Cancel stops it.
    pub fn example_outliers(
        &self,
        id: u32,
        spec: &ExampleSpec,
        program: &Program,
        after: Option<Vec<u32>>,
    ) -> CmdResult<Outliers> {
        let entry = self.get(id)?;
        let job = entry.start("Looking for odd results")?;
        let d = entry.read();
        Ok(d.example_outliers(spec, program, after.as_deref(), &job.progress)?)
    }

    /// The rule as a formula for a computed column, or why it has none.
    pub fn example_formula(
        &self,
        id: u32,
        spec: &ExampleSpec,
        program: &Program,
        read: natendo_engine::value::ReadOptions,
    ) -> CmdResult<String> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.example_formula(spec, program, read)?)
    }

    /// How many empty, untitled columns past the usual number could be
    /// removed, and the revision they were looked at.
    pub fn empty_extra_columns(&self, id: u32, keep_first: bool) -> CmdResult<(u32, u64)> {
        let entry = self.get(id)?;
        let job = entry.start("Looking at the extra column")?;
        let d = entry.read();
        Ok((
            d.empty_extra_columns(keep_first, &job.progress),
            d.revision(),
        ))
    }

    /// Cut the empty cells past the usual number, after the automatic
    /// checkpoint. Left out of recipes.
    pub fn remove_empty_extra_columns(
        &self,
        id: u32,
        keep_first: bool,
        revision: u64,
    ) -> CmdResult<DocInfo> {
        self.recorded_auto(
            id,
            Logged::LeftOut("removing the empty column".into()),
            Some("removing the empty column".into()),
            || {
                let entry = self.get(id)?;
                let job = entry.start("Removing the empty column")?;
                self.edit(id, |d| {
                    d.remove_empty_extra_columns(keep_first, revision, &job.progress)
                        .map(|_| ())
                })
            },
        )
    }

    /// How many dates of each kind column `col` holds (the first `sample`
    /// shown rows, or all of them). A look: nothing changes.
    pub fn date_scan(
        &self,
        id: u32,
        col: u32,
        keep_first: bool,
        sample: Option<u64>,
    ) -> CmdResult<natendo_engine::datescan::DateScan> {
        let entry = self.get(id)?;
        let job = entry.start("Looking at the dates")?;
        let d = entry.read();
        Ok(d.date_scan(col, keep_first, sample, &job.progress)?)
    }

    /// The shown rows on screen whose date in `col` can be read two ways.
    pub fn date_marks(
        &self,
        id: u32,
        col: u32,
        keep_first: bool,
        row: u64,
        rows: u64,
    ) -> CmdResult<Vec<u64>> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.date_marks(col, keep_first, row, rows)?)
    }

    /// The first such rows with their text.
    pub fn date_list(
        &self,
        id: u32,
        col: u32,
        keep_first: bool,
    ) -> CmdResult<Vec<(u64, u64, String)>> {
        let entry = self.get(id)?;
        let job = entry.start("Listing the ambiguous dates")?;
        let d = entry.read();
        Ok(d.date_list(col, keep_first, 500, &job.progress)?)
    }

    /// The next or previous such row from `from`.
    pub fn date_find(
        &self,
        id: u32,
        col: u32,
        keep_first: bool,
        from: u64,
        forward: bool,
    ) -> CmdResult<Option<u64>> {
        let entry = self.get(id)?;
        let job = entry.start("Looking for the next date")?;
        let d = entry.read();
        Ok(d.date_find(col, keep_first, from, forward, &job.progress)?)
    }

    /// Paste as column names: set cells of the table's first row (column,
    /// name) as one undo step, shown or not (protected columns too, as a
    /// rename). Not a recipe step: the names are this file's.
    pub fn set_column_names(&self, id: u32, names: Vec<(u32, String)>) -> CmdResult<DocInfo> {
        check_write(names.len() as u64, names.iter().map(|c| c.1.len()))?;
        let widest = names.iter().map(|c| c.0 as u64 + 1).max().unwrap_or(0);
        self.recorded(id, Logged::LeftOut("Paste as column names".into()), || {
            self.edit(id, |d| {
                check_width(d, widest)?;
                d.set_first_row_cells(names)
            })
        })
    }

    /// Tidy column titles: what would change (column, now, after).
    pub fn tidy_titles_preview(
        &self,
        id: u32,
        spec: &natendo_engine::titles::TitleSpec,
    ) -> CmdResult<Vec<(u32, String, String)>> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.plan_tidy_titles(spec)?)
    }

    /// Tidy every column title, as one undo step (and a recipe step).
    pub fn tidy_titles(
        &self,
        id: u32,
        spec: natendo_engine::titles::TitleSpec,
    ) -> CmdResult<DocInfo> {
        let step = RecipeOp::TidyTitles { spec: spec.clone() };
        self.recorded(id, Logged::Step(step), || {
            self.edit(id, |d| d.tidy_titles(&spec).map(|_| ()))
        })
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
    #[cfg(test)]
    pub fn fill(
        &self,
        id: u32,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
        value: String,
    ) -> CmdResult<DocInfo> {
        self.fill_skipping(id, row, rows, col, cols, value, &[])
    }

    /// Rows hidden between shown rows `row` and `row + rows - 1` (left as
    /// they were by a fill or clear of that block).
    pub fn hidden_between(&self, id: u32, row: u64, rows: u64) -> CmdResult<u64> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.hidden_between(row..row.saturating_add(rows).min(d.row_count())))
    }

    /// [`Docs::fill`] leaving columns `skip` (hidden ones) as they are.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_skipping(
        &self,
        id: u32,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
        value: String,
        skip: &[u32],
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
            let keep: Vec<u32> = (col..c1).filter(|c| !skip.contains(c)).collect();
            let mut cells = Vec::with_capacity(n as usize);
            for r in row..r1 {
                for &c in &keep {
                    cells.push((r, c, value.clone()));
                }
            }
            d.set_cells(cells)
        })
    }

    /// One value into every cell of several blocks (Ctrl+click), as ONE
    /// undo step. The blocks are made non-overlapping here (never trust the
    /// window), the cells of all of them count against the per-change cap,
    /// hidden columns (`skip`) are left alone. An empty value clears them.
    pub fn fill_areas(
        &self,
        id: u32,
        areas: Vec<natendo_engine::areas::Area>,
        value: String,
        skip: &[u32],
    ) -> CmdResult<DocInfo> {
        if areas.len() > MAX_AREAS {
            return Err(CmdError::new("too_large", "too many selected blocks"));
        }
        let parts = natendo_engine::areas::disjoint(&areas);
        let n: u64 = parts.iter().fold(0u64, |n, a| n.saturating_add(a.cells()));
        check_write(n, std::iter::once(value.len()))?;
        if (value.len() as u64).saturating_mul(n) > MAX_WRITE_CHARS as u64 {
            return Err(too_much());
        }
        self.edit(id, |d| {
            let mut cells = Vec::with_capacity(n as usize);
            for a in &parts {
                let r1 = a.row.checked_add(a.rows).ok_or(Error::OutOfRange)?;
                let c1 = a.col.checked_add(a.cols).ok_or(Error::OutOfRange)?;
                check_width(d, c1 as u64)?;
                for r in a.row..r1 {
                    for c in (a.col..c1).filter(|c| !skip.contains(c)) {
                        cells.push((r, c, value.clone()));
                    }
                }
            }
            d.set_cells(cells)
        })
    }

    /// Selected cells as tab-separated text for the clipboard. Values with a
    /// tab, quote or line break are quoted, so pasting round-trips.
    #[cfg(test)]
    pub fn copy(&self, id: u32, row: u64, rows: u64, col: u32, cols: u32) -> CmdResult<String> {
        self.copy_skipping(id, row, rows, col, cols, &[])
    }

    /// [`Docs::copy`] leaving out columns `skip` (hidden ones), so what is
    /// not shown never reaches the clipboard.
    pub fn copy_skipping(
        &self,
        id: u32,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
        skip: &[u32],
    ) -> CmdResult<String> {
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
        // Which columns to leave out, worked out once (not per cell).
        let hidden: Vec<bool> = (0..cols).map(|c| skip.contains(&(col + c))).collect();
        let mut r = row;
        while r < r1 {
            let end = (r + band).min(r1);
            let v = d.viewport(r..end, col..col.saturating_add(cols), per_cell)?;
            for i in 0..v.rows {
                let mut first = true;
                for c in 0..v.cols {
                    if hidden[c as usize] {
                        continue;
                    }
                    let k = (i * v.cols + c) as usize;
                    if !first {
                        out.push('\t');
                    }
                    first = false;
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
            // Moving out of or into a protected column would lose or
            // half-move values: refused (a copy just skips them).
            if !copy && (col..col + cols).chain(to_col..to_col + cols).any(|c| d.is_protected(c)) {
                return Err(Error::Invalid(
                    "cells can't be moved out of or into a protected column (right-click its heading › Protect column to switch it off)".into(),
                ));
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
        let mut pos = None;
        let mut info = self.edit(id, |d| {
            pos = Some(d.insert_rows_at(at, vec![vec![String::new()]; count as usize])?);
            Ok(())
        })?;
        info.rows_change = pos.map(|at| RowsChange::Insert { at, count });
        Ok(info)
    }

    pub fn delete_rows(&self, id: u32, at: u64, count: u64) -> CmdResult<DocInfo> {
        let entry = self.get(id)?;
        // Several rows at once (or any while a filter hides others) get an automatic checkpoint, its mark taken
        // inside the change (see `recorded_auto`).
        let auto = count > 1 || entry.read().has_filter();
        if auto {
            natendo_engine::document::capture_mark_start(entry.read().uid());
        }
        let mut runs = None;
        let info = self.edit(id, |d| {
            runs = Some(d.delete_rows_at(at, count)?);
            Ok(())
        });
        let mark = natendo_engine::document::capture_mark_take();
        let mut info = info?;
        if count <= MAX_ROWS_CHANGE {
            info.rows_change = runs.map(|runs| RowsChange::Delete { runs });
        }
        if let (true, Some(m)) = (auto, mark) {
            self.auto_checkpoint(&entry, m, "deleting rows");
        }
        Ok(info)
    }

    /// Why shown rows `at..at + count` cannot be set aside as report notes at
    /// `place` (`None`: they can). Cheap, for greying menu entries.
    pub fn set_aside_check(
        &self,
        id: u32,
        at: u64,
        count: u64,
        place: Place,
    ) -> CmdResult<Option<String>> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.set_aside_check(at, count, place)
            .err()
            .map(|e| e.to_string()))
    }

    /// Set rows aside above (`Head`) or below the table as report notes (one
    /// undo step, the way deleting rows takes them out).
    pub fn set_aside(&self, id: u32, at: u64, count: u64, place: Place) -> CmdResult<DocInfo> {
        let n = u32::try_from(count).unwrap_or(u32::MAX);
        let step = match place {
            Place::Head => RecipeOp::SetAside { head: n, foot: 0 },
            Place::Foot => RecipeOp::SetAside { head: 0, foot: n },
        };
        self.recorded(id, Logged::Step(step), || {
            self.edit(id, |d| d.set_aside_rows(at, count, place))
        })
    }

    /// A total line among the last rows, and whether the rows above add up
    /// to it (`None`: none, or rows are hidden). Never waits behind a change.
    /// The Fill blank cells dialog's count line (with stops): how many blanks
    /// would be filled and how many left blank, in the first 200,000 shown
    /// rows. A newer count, `fill_blanks_count_stop` or any change stops it.
    pub fn fill_blanks_count(
        &self,
        id: u32,
        spec: &natendo_engine::tools::FillBlanks,
    ) -> CmdResult<natendo_engine::tools::FillCount> {
        let entry = self.get(id)?;
        let progress = Arc::new(Progress::new());
        {
            let mut slot = entry.fill_count.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(old) = slot.replace(progress.clone()) {
                old.cancel.store(true, Ordering::Relaxed);
            }
        }
        let d = match entry.doc.try_read() {
            Ok(d) => d,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(CmdError::new("busy", "the file is being changed"))
            }
        };
        if entry.change_waiting() {
            drop(d);
            return Err(CmdError::new("busy", "the file is being changed"));
        }
        let r = d.count_fill_blanks(spec, &progress);
        drop(d);
        let mut slot = entry.fill_count.lock().unwrap_or_else(|p| p.into_inner());
        if slot.as_ref().is_some_and(|p| Arc::ptr_eq(p, &progress)) {
            *slot = None;
        }
        Ok(r?)
    }

    /// Stop the dialog's count (the dialog closed).
    pub fn fill_blanks_count_stop(&self, id: u32) -> CmdResult<()> {
        let entry = self.get(id)?;
        if let Some(p) = entry
            .fill_count
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            p.cancel.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    /// The cells marked as left blank by the last fill, among shown rows
    /// `row..row + rows`.
    pub fn fill_marks(&self, id: u32, row: u64, rows: u64) -> CmdResult<Vec<(u64, u32)>> {
        let entry = self.get(id)?;
        let marks = entry
            .fill_marks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if marks.is_empty() {
            return Ok(Vec::new());
        }
        let d = entry.read();
        Ok(d.marked_cells(&marks, row, rows.min(10_000)))
    }

    /// The next (or previous) marked cell from a shown row and column.
    pub fn fill_marks_next(
        &self,
        id: u32,
        row: u64,
        col: u32,
        back: bool,
    ) -> CmdResult<Option<(u64, u32)>> {
        let entry = self.get(id)?;
        let marks = entry
            .fill_marks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let d = entry.read();
        Ok(d.next_marked(&marks, row, col, back))
    }

    /// Take the marks away (undo, Esc, a closed tab).
    pub fn fill_marks_clear(&self, id: u32) -> CmdResult<()> {
        self.get(id)?
            .fill_marks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        Ok(())
    }

    pub fn total_row(
        &self,
        id: u32,
        header: bool,
        read: ReadOptions,
    ) -> CmdResult<Option<natendo_engine::document::report::TotalRow>> {
        let entry = self.get(id)?;
        let d = match entry.doc.try_read() {
            Ok(d) => d,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(CmdError::new("busy", "the file is being changed"))
            }
        };
        Ok(d.find_total_row(header, read, &Progress::new())?)
    }

    /// The report notes, for the panel.
    pub fn report_notes(&self, id: u32) -> CmdResult<ReportNotes> {
        Ok(self.get(id)?.read().report_notes())
    }

    /// Put every report note back into the table (one undo step).
    pub fn put_back_notes(&self, id: u32) -> CmdResult<DocInfo> {
        self.recorded(
            id,
            Logged::LeftOut("putting report notes back into the table".into()),
            || self.edit(id, |d| d.put_back_notes().map(drop)),
        )
    }

    /// Save the report notes, or leave them out (one undo step).
    pub fn keep_notes(&self, id: u32, keep: bool) -> CmdResult<DocInfo> {
        self.edit(id, |d| {
            d.set_keep_notes(keep);
            Ok(())
        })
    }

    /// Hide shown rows (one undo step; hidden rows are still saved).
    pub fn hide_rows(&self, id: u32, at: u64, count: u64) -> CmdResult<DocInfo> {
        self.edit(id, |d| d.hide_rows(at, count).map(drop))
    }

    /// Rows hidden by hand in the gap just above shown row `row`.
    pub fn gap_hidden(&self, id: u32, row: u64) -> CmdResult<u64> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.hidden_by_hand_before(row))
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

    /// The totals row: each of `cols` over the shown rows. Results are kept
    /// until the document changes, so only columns not yet totalled are
    /// read. A change (or a request for a newer revision) stops older work.
    pub fn column_totals(
        &self,
        id: u32,
        cols: Vec<u32>,
        header: bool,
        read: ReadOptions,
    ) -> CmdResult<ColumnTotalsOut> {
        if cols.len() > MAX_COLUMN_TOTALS {
            return Err(CmdError::new("bad_request", "too many columns at once"));
        }
        let entry = self.get(id)?;
        let busy = || CmdError::new("busy", "the file is being changed");
        let d = match entry.doc.try_read() {
            Ok(d) => d,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(busy()),
        };
        if entry.change_waiting() {
            return Err(busy());
        }
        let key = (d.revision(), header, read);
        let cache = || {
            entry
                .column_totals
                .lock()
                .unwrap_or_else(|p| p.into_inner())
        };
        let (missing, progress) = {
            let mut c = cache();
            if c.key != Some(key) {
                c.stop();
                c.found.clear();
                c.key = Some(key);
            }
            let mut missing: Vec<u32> = cols
                .iter()
                .copied()
                .filter(|k| *k < d.col_count() && !c.found.contains_key(k))
                .collect();
            missing.sort_unstable();
            missing.dedup();
            // Two panes may ask for the same revision at once: one flag stops
            // both when the document changes.
            let progress = c.job.get_or_insert_with(Default::default).clone();
            (missing, progress)
        };
        if !missing.is_empty() {
            let found = d.column_totals(&missing, header, read, &progress)?;
            let mut c = cache();
            if c.key == Some(key) {
                c.found.extend(found.into_iter().map(|t| (t.col, t)));
            }
        }
        let rows = d.row_count().saturating_sub(u64::from(header));
        let c = cache();
        Ok(ColumnTotalsOut {
            revision: key.0,
            rows,
            filtered: d.row_count() < d.total_rows(),
            columns: cols
                .iter()
                .filter_map(|k| c.found.get(k).copied())
                .collect(),
        })
    }

    /// [`Docs::selection_totals`] over several blocks (Ctrl+click): made
    /// non-overlapping here (the window's list is not trusted), no distinct count.
    pub fn selection_totals_areas(
        &self,
        id: u32,
        areas: Vec<natendo_engine::areas::Area>,
        read: ReadOptions,
    ) -> CmdResult<TotalsOut> {
        if areas.len() > MAX_AREAS {
            return Err(CmdError::new("too_large", "too many selected blocks"));
        }
        let entry = self.get(id)?;
        let progress = Arc::new(Progress::new());
        {
            let mut slot = entry.totals.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(old) = slot.replace(progress.clone()) {
                old.cancel.store(true, Ordering::Relaxed);
            }
        }
        let d = match entry.doc.try_read() {
            Ok(d) => d,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(CmdError::new("busy", "the file is being changed"))
            }
        };
        if entry.change_waiting() {
            drop(d);
            return Err(CmdError::new("busy", "the file is being changed"));
        }
        let r = d
            .selection_totals_areas(&areas, read, &progress)
            .map(TotalsOut::from);
        drop(d);
        let mut slot = entry.totals.lock().unwrap_or_else(|p| p.into_inner());
        if slot.as_ref().is_some_and(|p| Arc::ptr_eq(p, &progress)) {
            *slot = None;
        }
        Ok(r?)
    }

    /// Move rows `at..at + count` to start at `to` (shown positions while a
    /// filter is on). `shown`, when given, is how many rows the window thought
    /// were shown: a move made from an older picture is refused.
    pub fn move_rows(
        &self,
        id: u32,
        at: u64,
        count: u64,
        to: u64,
        shown: Option<u64>,
    ) -> CmdResult<DocInfo> {
        let mut moved = None;
        let mut info = self.edit(id, |d| {
            if shown.is_some_and(|s| s != d.row_count()) {
                return Err(natendo_engine::Error::Invalid(
                    "the rows changed; try again".into(),
                ));
            }
            moved = d.move_rows_at(at, count, to)?;
            Ok(())
        })?;
        if count <= MAX_ROWS_CHANGE {
            info.rows_change = moved.map(|(runs, to)| RowsChange::Move { runs, to });
        }
        Ok(info)
    }

    pub fn insert_cols(&self, id: u32, at: u32, count: u32) -> CmdResult<DocInfo> {
        self.recorded(id, Logged::Step(RecipeOp::InsertCols { at, count }), || {
            self.insert_cols_now(id, at, count)
        })
    }

    /// Insert columns as part of something not a recipe step (Split into
    /// columns after a paste): not noted for the recipe, as the cells
    /// written into them are not.
    pub fn insert_cols_unrecorded(&self, id: u32, at: u32, count: u32) -> CmdResult<DocInfo> {
        self.insert_cols_now(id, at, count)
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
        if entry.peeking.load(Ordering::Relaxed) {
            return Err(peeking());
        }
        // A save over a locked file (this tab's or another's, however the
        // path is written) is refused before this document is locked for
        // writing.
        match path.as_deref() {
            None if entry.locked.load(Ordering::Relaxed) => return Err(read_only()),
            Some(p) => self.check_writable(p)?,
            None => {}
        }
        let saved;
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
            // Test builds only (feature `test-slow-save`, off in a release):
            // hold the document locked for a while, as a very big save would.
            #[cfg(feature = "test-slow-save")]
            if let Some(ms) = std::env::var("NATENDO_TEST_SAVE_DELAY_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
            {
                std::thread::sleep(std::time::Duration::from_millis(ms));
            }
            // "Show changed rows" keeps working across the save while it still describes the table (B4): its rows
            // get their ids in the saved file. One made before a later change stays stale, as before.
            let before = d.revision();
            let reachable: Vec<u32> = {
                let book = entry.checkpoints.lock().unwrap_or_else(|p| p.into_inner());
                book.list
                    .iter()
                    .filter(|c| d.step_reach(c.mark).is_some())
                    .map(|c| c.id)
                    .collect()
            };
            let changed = {
                let c = entry.changed_rows.lock().unwrap_or_else(|p| p.into_inner());
                (c.0 == before && !c.1.is_empty()).then(|| d.saved_ids_of(&c.1))
            };
            d.save(SaveOptions {
                path,
                overwrite_changed,
                allow_lossy,
                format,
                convert_line_breaks,
                quoting,
                ..Default::default()
            })?;
            if let Some(rows) = changed.filter(|_| d.revision() != before) {
                *entry.changed_rows.lock().unwrap_or_else(|p| p.into_inner()) =
                    (d.revision(), rows);
            }
            // Checkpoints the save let go say when and why (U4).
            if let Some(k) = d.last_history().filter(|_| d.revision() != before) {
                let why = if k.undo_cut_columns + k.redo_cut_columns > 0 {
                    "a column, encoding or width change"
                } else if k.undo_cut_memory + k.redo_cut_memory > 0 {
                    "not enough memory to keep the undo history"
                } else if k.undo_cut_work + k.redo_cut_work > 0 {
                    "the steps before it changed too many rows to carry across a save"
                } else {
                    "the undo history could not be kept"
                };
                let at = now_millis();
                let mut book = entry.checkpoints.lock().unwrap_or_else(|p| p.into_inner());
                for c in book.list.iter_mut() {
                    if c.gone.is_none()
                        && reachable.contains(&c.id)
                        && d.step_reach(c.mark).is_none()
                    {
                        c.gone = Some((at, why));
                    }
                }
            }
            saved = d.last_history().filter(|_| d.revision() != before);
            // Saved as a CSV file of its own: the tab no longer saves back to the JSON file (C11).
            if d.path().is_some() {
                d.set_json_shape(None);
                *entry.json_target.lock().unwrap_or_else(|p| p.into_inner()) = None;
            }
        }
        // The saved copy was reopened normally (quotes may span lines).
        entry.line_by_line.store(false, Ordering::Relaxed);
        // Nothing unsaved: the recovery copy goes.
        crate::recovery::forget(id);
        entry.autosaved.store(u64::MAX, Ordering::Relaxed);
        entry.too_large.store(0, Ordering::Relaxed);
        let mut info = self.info(id)?;
        info.saved_history = saved;
        Ok(info)
    }

    /// C11: the tab saves back to the JSON file at `path` (read with this size and time), written as
    /// `form` (`None`: as the file came, from the tab's JSON shape).
    pub fn set_json_target(
        &self,
        id: u32,
        path: PathBuf,
        stamp: Option<natendo_engine::source::FileStamp>,
        form: Option<natendo_engine::jsonshape::JsonForm>,
    ) {
        use natendo_engine::jsonshape::JsonForm;
        if let Ok(entry) = self.get(id) {
            let form = form
                .or_else(|| entry.read().json_shape().map(|s| JsonForm::from(&s.layout)))
                .unwrap_or_else(lines_form);
            *entry.json_target.lock().unwrap_or_else(|p| p.into_inner()) =
                Some(JsonTarget { path, stamp, form });
        }
    }

    /// C11: whether the tab can be saved as JSON, and whether a save needs the blank-cell question.
    pub fn json_check(&self, id: u32) -> CmdResult<natendo_engine::jsonshape::JsonCheck> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.json_check())
    }

    /// C11: save as JSON Lines or a JSON array. `path`: Save As (from the system dialog); `None`: back
    /// to the JSON file the tab came from. `form`: the layout to write (`None`: the file's own). With the
    /// same safety as Save: refused while peeking or read-only, stopped when another program changed
    /// the file (unless `overwrite_changed`), an earlier save kept, a temp file moved into place. A
    /// tab with a file of its own (a CSV tab) writes a copy and stays as it is.
    pub fn save_json(
        &self,
        id: u32,
        path: Option<PathBuf>,
        form: Option<natendo_engine::jsonshape::JsonForm>,
        blanks: natendo_engine::jsonshape::Blanks,
        overwrite_changed: bool,
    ) -> CmdResult<JsonSaved> {
        use natendo_engine::source::FileStamp;
        let entry = self.get(id)?;
        if entry.peeking.load(Ordering::Relaxed) {
            return Err(peeking());
        }
        let target = entry
            .json_target
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let back = path.is_none();
        let file = match (&path, &target) {
            (Some(p), _) => p.clone(),
            (None, Some(t)) => t.path.clone(),
            (None, None) => {
                return Err(CmdError::new(
                    "no_json_file",
                    "this tab did not come from a JSON file: use Save As",
                ))
            }
        };
        if back && entry.locked.load(Ordering::Relaxed) {
            return Err(read_only());
        }
        self.check_writable(&file)?;
        if back && !overwrite_changed {
            if let Some(t) = &target {
                if FileStamp::of(&t.path).ok() != t.stamp {
                    return Err(Error::ChangedOnDisk.into());
                }
            }
        }
        let private = entry.private.load(Ordering::SeqCst);
        let (report, own) = {
            let mut d = entry.write();
            if entry.locked.load(Ordering::Relaxed)
                && d.path().is_some_and(|own| same_file(&file, own))
            {
                return Err(read_only());
            }
            let form = form
                .or_else(|| target.as_ref().map(|t| t.form.clone()))
                .unwrap_or_else(lines_form);
            #[cfg(not(test))]
            if !private {
                crate::recovery::keep_version(&file);
            }
            let _ = private;
            // A tab with a CSV file of its own writes a copy; an imported tab takes the file as its own.
            let own = d.path().is_none();
            let progress = Progress::new();
            let report = if own {
                d.save_json(&file, &form, blanks, &progress)?
            } else {
                d.write_json(&file, &form, blanks, &progress)?
            };
            if own {
                // The file written is the one the tab saves back to, in the form just written.
                *entry.json_target.lock().unwrap_or_else(|p| p.into_inner()) = Some(JsonTarget {
                    path: file.clone(),
                    stamp: FileStamp::of(&file).ok(),
                    form: form.clone(),
                });
            }
            (report, own)
        };
        if own {
            if !back {
                if let Some(stem) = file.file_stem() {
                    self.set_title_of(id, stem.to_string_lossy().into_owned());
                }
            }
            crate::recovery::forget(id);
            entry.autosaved.store(u64::MAX, Ordering::Relaxed);
            entry.too_large.store(0, Ordering::Relaxed);
        }
        let info = self.info(id)?;
        Ok(JsonSaved { info, report, own })
    }

    /// [`Docs::save`] with a processing receipt written beside the saved file: the file the tab
    /// was read from (hashed over the bytes that were parsed, before the save replaces anything)
    /// and the file written. A private tab's receipt has no full paths. The save stands even
    /// when the receipt cannot be written; the note says so (see [`Docs::take_receipt_note`]).
    #[allow(clippy::too_many_arguments)]
    pub fn save_with_receipt(
        &self,
        id: u32,
        path: Option<PathBuf>,
        overwrite_changed: bool,
        allow_lossy: bool,
        options: Option<&SaveAs>,
        ask: crate::watch::ReceiptAsk,
    ) -> CmdResult<DocInfo> {
        use natendo_engine::receipt::{FileRecord, Receipt};
        let private = self.is_private(id)?;
        let full = ask.full_paths && !private;
        let mut r = Receipt::start(if path.is_some() { "save as" } else { "save" });
        {
            let entry = self.get(id)?;
            let d = entry.read();
            if let Some(src) = d.path().map(Path::to_path_buf) {
                if let Some(mut rec) = FileRecord::of_input(&d, full, &Progress::new())? {
                    rec.cols = Some(u64::from(d.col_count()));
                    r.add_input(rec, &src);
                }
            }
        }
        let info = self.save(id, path, overwrite_changed, allow_lossy, options)?;
        let written = self
            .path_of(id)?
            .ok_or_else(|| CmdError::new("invalid", "the file has no name"));
        let note = written.and_then(|out| {
            let mut rec = FileRecord::of_file(&out, full, &Progress::new())?;
            rec.rows = Some(info.total_rows);
            rec.cols = Some(u64::from(info.cols));
            r.outputs.push(rec);
            Ok(r.write_beside(&out)?)
        });
        let note = match note {
            Ok(p) => format!(
                "Receipt written: {}",
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
            Err(e) => format!("Saved, but the receipt was not written: {}", e.message),
        };
        self.receipt_notes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, note);
        Ok(info)
    }

    /// What the last save with a receipt on tab `id` said about it (once).
    pub fn take_receipt_note(&self, id: u32) -> Option<String> {
        self.receipt_notes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id)
    }

    /// Check a receipt (Tools > Verify a receipt): only the files the person picked are read.
    pub fn receipt_verify(
        &self,
        receipt: &Path,
        inputs: &[PathBuf],
        outputs: &[PathBuf],
        chain: bool,
    ) -> CmdResult<natendo_engine::receipt::VerifyReport> {
        Ok(natendo_engine::receipt::verify(
            receipt,
            inputs,
            outputs,
            chain,
            &Progress::new(),
        )?)
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

    /// Ask the running long operation to stop (nothing is changed). With a
    /// `label`, only that operation: another one that has started since (a
    /// sort, a count) is left alone.
    pub fn cancel_job(&self, id: u32, label: Option<&str>) -> CmdResult<()> {
        let entry = self.get(id)?;
        if let Some(j) = entry.jobs().as_ref() {
            if label.is_none_or(|l| l == j.label) {
                j.progress.cancel.store(true, Ordering::Relaxed);
            }
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

    // ---- key-aware diff ----

    /// Compare tab `a` (the first) with tab `b` (the second) by key and open the differences as a new tab.
    /// A job on `a` (progress and Cancel). Each step takes ONE table's read lock and lets it go (never both at
    /// once, so comparing a tab with itself works and nothing can deadlock); the result is private when
    /// either table is, and nothing about the values is logged.
    pub fn diff_files(
        &self,
        a: u32,
        b: u32,
        spec: natendo_engine::diff::DiffSpec,
    ) -> CmdResult<DiffReportOut> {
        use natendo_engine::document::Progress;
        let ea = self.get(a)?;
        let eb = self.get(b)?;
        let job = ea.start("Comparing")?;
        let progress: &Progress = &job.progress;
        let titles = ea.read().diff_titles(&spec);
        let index = Document::diff_index(&eb.read(), &titles, &spec, progress)?;
        let first = ea.read().diff_first(&index, &spec, progress)?;
        let out = eb
            .read()
            .diff_second(index, first, &spec, progress)
            .map_err(|e| {
                if matches!(e, Error::Changed) {
                    CmdError::new("changed", "a table changed while comparing; try again")
                } else {
                    e.into()
                }
            })?;
        drop(job);
        let natendo_engine::diff::DiffOutcome {
            table,
            report,
            index,
        } = out;
        let doc = table.into_document()?;
        let (rid, _) = self.insert(doc, false)?;
        self.inherit_private(rid, &[a, b], &[]);
        self.set_title_of(rid, format!("Diff: {} vs {}", self.stem(a), self.stem(b)));
        index.set_result_rev(self.get(rid)?.read().data_revision());
        let index = Arc::new(index);
        let id = self.next_diff.fetch_add(1, Ordering::Relaxed) + 1;
        {
            let mut m = self.diffs.lock().unwrap_or_else(|p| p.into_inner());
            // A newer diff of the same tabs replaces the older one.
            m.retain(|_, d| !(d.a == a && d.b == b));
            m.insert(
                id,
                DiffState {
                    index,
                    a,
                    b,
                    result: rid,
                },
            );
        }
        Ok(DiffReportOut {
            info: self.info(rid)?,
            report,
            diff: id,
        })
    }

    /// The differences in a window of tab `id` (a first table, a second table or a result), looked up by
    /// each shown row's key. Never waits behind a change ("busy").
    pub fn diff_marks(
        &self,
        id: u32,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
    ) -> CmdResult<DiffMarksOut> {
        let entry = self.get(id)?;
        let d = match entry.doc.try_read() {
            Ok(d) => d,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(CmdError::new("busy", "the file is being changed"))
            }
        };
        // The index is taken out and the map let go before anything is looked up: a question never holds
        // the mutex while it works (and so never waits behind a scan).
        let Some((did, index, side)) = self.diff_of(id) else {
            return Ok(DiffMarksOut {
                diff: None,
                out_of_date: false,
                marks: Vec::new(),
            });
        };
        if !index.current(side, d.data_revision()) {
            return Ok(DiffMarksOut {
                diff: Some(did),
                out_of_date: true,
                marks: Vec::new(),
            });
        }
        let marks = d.diff_marks(&index, side, row, rows, col, cols)?;
        Ok(DiffMarksOut {
            diff: Some(did),
            out_of_date: false,
            marks,
        })
    }

    /// The next (previous) difference after a row of tab `id` (a job: Cancel).
    pub fn diff_next(&self, id: u32, row: u64, backward: bool) -> CmdResult<Option<DiffJump>> {
        let entry = self.get(id)?;
        let job = entry.start("Looking for the next difference")?;
        let d = entry.read();
        // The index is taken out and the map let go before the scan (a long one must not hold the mutex).
        let Some((_, index, side)) = self.diff_of(id) else {
            return Ok(None);
        };
        let hit = d.diff_next(&index, side, row, backward, &job.progress)?;
        Ok(hit.map(|(row, wrapped, col)| DiffJump { row, col, wrapped }))
    }

    /// The newest diff tab `id` is in: its number, its index (shared; the map is let go at once) and which
    /// table of it the tab is.
    fn diff_of(&self, id: u32) -> Option<DiffOf> {
        use natendo_engine::diff::Side;
        let m = self.diffs.lock().unwrap_or_else(|p| p.into_inner());
        let (did, st) = m
            .iter()
            .filter(|(_, s)| s.a == id || s.b == id || s.result == id)
            .max_by_key(|(k, _)| **k)?;
        let side = if st.result == id {
            Side::Result
        } else if st.a == id {
            Side::A
        } else {
            Side::B
        };
        Some((*did, st.index.clone(), side))
    }

    /// Forget the diff tab `id` is in (its marks go from all its tabs).
    pub fn diff_clear(&self, id: u32) {
        self.diffs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|_, s| !(s.a == id || s.b == id || s.result == id));
    }

    /// A tab was only re-ordered (a sort): the diff still fits its rows, so it follows the new revision.
    fn diff_follow_sort(&self, id: u32, before: u64, after: u64) {
        use natendo_engine::diff::Side;
        for st in self
            .diffs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
        {
            for (tab, side) in [(st.a, Side::A), (st.b, Side::B), (st.result, Side::Result)] {
                if tab == id && st.index.current(side, before) {
                    st.index.follow_order(side, after);
                }
            }
        }
    }

    /// Sort every row (one undo step).
    pub fn sort(&self, id: u32, spec: SortSpec) -> CmdResult<DocInfo> {
        // A sort only puts the rows in another order, so a diff drawn on this tab still fits it. The
        // revisions before and after are read inside the apply, under the same write lock, so a change
        // that lands before or after the sort is never carried forward as if the marks still fit.
        let seen = std::cell::Cell::new(None);
        let info = self.sort_inner(id, spec, &seen)?;
        if let Some((b, a)) = seen.get() {
            self.diff_follow_sort(id, b, a);
        }
        Ok(info)
    }

    fn sort_inner(
        &self,
        id: u32,
        spec: SortSpec,
        seen: &std::cell::Cell<Option<(u64, u64)>>,
    ) -> CmdResult<DocInfo> {
        self.recorded(
            id,
            Logged::Step(RecipeOp::Sort { spec: spec.clone() }),
            || {
                self.compute_apply(
                    id,
                    "Sorting",
                    |d, p| d.plan_sort(&spec, p),
                    |d, plan| {
                        let before = d.data_revision();
                        d.apply_sort(plan)?;
                        seen.set(Some((before, d.data_revision())));
                        Ok(())
                    },
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

    /// The filter chips (C8): one per column rule, then one per tool.
    pub fn filter_chips(&self, id: u32) -> CmdResult<natendo_engine::chips::FilterChips> {
        Ok(self.get(id)?.read().filter_chips())
    }

    /// The chips with every column chip's own count (reads the rules not known since the last edit):
    /// only when the bar's menu opens or the pointer rests on the bar (one pass over the table at
    /// most). A newer ask, an edit or a filter change stops it; it never waits for a change.
    pub fn filter_chip_counts(&self, id: u32) -> CmdResult<natendo_engine::chips::FilterChips> {
        let entry = self.get(id)?;
        let progress = Arc::new(Progress::new());
        {
            let mut slot = entry.chip_count.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(old) = slot.replace(progress.clone()) {
                old.cancel.store(true, Ordering::Relaxed);
            }
        }
        let d = match entry.doc.try_read() {
            Ok(d) => d,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(CmdError::new("busy", "the file is being changed"))
            }
        };
        if entry.change_waiting() {
            drop(d);
            return Err(CmdError::new("busy", "the file is being changed"));
        }
        let r = d.filter_chip_counts(&progress);
        drop(d);
        let mut slot = entry.chip_count.lock().unwrap_or_else(|p| p.into_inner());
        if slot.as_ref().is_some_and(|p| Arc::ptr_eq(p, &progress)) {
            *slot = None;
        }
        Ok(r?)
    }

    /// Stop the chip bar's counts (the pointer left the bar before they came).
    pub fn filter_chip_counts_stop(&self, id: u32) -> CmdResult<()> {
        let entry = self.get(id)?;
        if let Some(p) = entry
            .chip_count
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            p.cancel.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Switch, invert, rename or remove a chip, or change Match all/any or
    /// Invert all (one undo step; it changes no data).
    pub fn filter_change(
        &self,
        id: u32,
        change: natendo_engine::chips::ChipChange,
    ) -> CmdResult<DocInfo> {
        let logged = {
            let entry = self.get(id)?;
            let d = entry.read();
            // Never reads the table (C8 review M1): the rules as they are, and whether a tool made a chip.
            let tools = d.tool_chip_count() > 0;
            let spec = d.filter_spec().filter(|_| !tools);
            match spec.and_then(|s| natendo_engine::chips::spec_after(&s, &change)) {
                Some(s) if s.columns.is_empty() => Logged::Step(RecipeOp::ClearFilter),
                Some(s) => Logged::Step(RecipeOp::Filter { spec: s }),
                None => Logged::LeftOut("a change to a filter chip made by a tool".into()),
            }
        };
        self.recorded(id, logged, || {
            self.compute_apply(
                id,
                "Filtering",
                |d, p| d.plan_filter_change(&change, p),
                |d, plan| d.apply_filter(plan),
            )
        })
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

    /// The find bar's count ("3 of 1,204"), worked out in the background: a
    /// newer count or a change cancels it ("cancelled"), and it never waits
    /// behind a change ("busy").
    pub fn find_count(
        &self,
        id: u32,
        spec: FindSpec,
        at: Option<(u64, u32)>,
    ) -> CmdResult<FindCount> {
        let entry = self.get(id)?;
        let progress = Arc::new(Progress::new());
        {
            let mut slot = entry.find_count.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(old) = slot.replace(progress.clone()) {
                old.cancel.store(true, Ordering::Relaxed);
            }
        }
        let d = match entry.doc.try_read() {
            Ok(d) => d,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(CmdError::new("busy", "the file is being changed"))
            }
        };
        if entry.change_waiting() {
            drop(d);
            return Err(CmdError::new("busy", "the file is being changed"));
        }
        let key = serde_json::to_string(&spec).unwrap_or_default();
        let rev = d.revision();
        let mut blocks = entry
            .find_blocks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .filter(|(k, r, _)| *k == key && *r == rev)
            .map(|(_, _, b)| b);
        let r = d.find_count_with(&spec, at, &mut blocks, &progress);
        drop(d);
        if let (Ok(_), Some(b)) = (&r, blocks) {
            *entry.find_blocks.lock().unwrap_or_else(|p| p.into_inner()) = Some((key, rev, b));
        }
        let mut slot = entry.find_count.lock().unwrap_or_else(|p| p.into_inner());
        if slot.as_ref().is_some_and(|p| Arc::ptr_eq(p, &progress)) {
            *slot = None;
        }
        Ok(r?)
    }

    /// Stop a count still running (the find bar closed).
    pub fn find_count_cancel(&self, id: u32) -> CmdResult<()> {
        let entry = self.get(id)?;
        if let Some(p) = entry
            .find_count
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            p.cancel.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    /// The matching cells on screen, to light them up.
    pub fn find_marks(
        &self,
        id: u32,
        spec: FindSpec,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
    ) -> CmdResult<Vec<(u64, u32)>> {
        let entry = self.get(id)?;
        let d = match entry.doc.try_read() {
            Ok(d) => d,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(CmdError::new("busy", "the file is being changed"))
            }
        };
        Ok(d.find_marks(&spec, row, rows, col, cols)?)
    }

    /// What Replace all would make of the cells on screen (the ghost diff). Read-only and quick; while
    /// a change holds the document it answers "busy" like the lit-up matches do, so nothing is drawn
    /// from a document that is about to change. Nothing about it is logged.
    #[allow(clippy::too_many_arguments)]
    pub fn replace_ghosts(
        &self,
        id: u32,
        spec: FindSpec,
        replacement: &str,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
    ) -> CmdResult<natendo_engine::find::GhostWindow> {
        let entry = self.get(id)?;
        let d = match entry.doc.try_read() {
            Ok(d) => d,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(CmdError::new("busy", "the file is being changed"))
            }
        };
        Ok(d.replace_ghosts(&spec, replacement, row, rows, col, cols)?)
    }

    /// How many cells Replace all would change (cancelled by a newer count, a change, or
    /// `replace_count_cancel`; never waits behind a change).
    pub fn replace_count(&self, id: u32, spec: FindSpec, replacement: &str) -> CmdResult<u64> {
        let entry = self.get(id)?;
        let progress = Arc::new(Progress::new());
        {
            let mut slot = entry.ghost_count.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(old) = slot.replace(progress.clone()) {
                old.cancel.store(true, Ordering::Relaxed);
            }
        }
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
        let r = d.replace_count(&spec, replacement, &progress);
        drop(d);
        let mut slot = entry.ghost_count.lock().unwrap_or_else(|p| p.into_inner());
        if slot.as_ref().is_some_and(|p| Arc::ptr_eq(p, &progress)) {
            *slot = None;
        }
        Ok(r?)
    }

    /// Stop the count behind the ghost diff (typing went on, Esc, the panel closed).
    pub fn replace_count_cancel(&self, id: u32) -> CmdResult<()> {
        let entry = self.get(id)?;
        if let Some(p) = entry
            .ghost_count
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            p.cancel.store(true, Ordering::Relaxed);
        }
        Ok(())
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
            hidden_cells: plan.hidden_cells,
        };
        *entry.replace.lock().unwrap_or_else(|p| p.into_inner()) = Some((key, plan));
        Ok(report)
    }

    /// A page of the kept Replace all preview (all changes, not just the
    /// first ones).
    pub fn replace_page(&self, id: u32, offset: u64, count: usize) -> CmdResult<Vec<Change>> {
        let entry = self.get(id)?;
        let kept = entry.replace.lock().unwrap_or_else(|p| p.into_inner());
        let Some((_, plan)) = kept.as_ref() else {
            return Err(CmdError::new("changed", "look for the matches again"));
        };
        let d = entry.read();
        Ok(d.replace_page(plan, offset, count.min(500))?)
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
            self.replace_all_now(id, spec, replacement, false)
        })
    }

    /// Act on the rows with a match: show only them, delete them (one undo
    /// step), copy them to a new tab, or flag them (a row note flag).
    /// `revision`: the one Find all ran at; the action refuses if the file
    /// changed since, so it never acts on rows the list didn't show.
    pub fn find_rows(
        &self,
        id: u32,
        spec: FindSpec,
        action: FindRowsAction,
        revision: Option<u64>,
    ) -> CmdResult<FindRowsReport> {
        let changed = || {
            CmdError::new(
                "changed",
                "the file changed since Find all, so nothing was done. Run Find all again",
            )
        };
        let same = |d: &natendo_engine::document::Document| -> Result<(), natendo_engine::Error> {
            match revision {
                Some(r) if r != d.revision() => Err(natendo_engine::Error::Changed),
                _ => Ok(()),
            }
        };
        if let Some(r) = revision {
            if self.get(id)?.read().revision() != r {
                return Err(changed());
            }
        }
        let report = |info: DocInfo| FindRowsReport {
            info,
            recolored: 0,
            left_out_cols: 0,
        };
        let fix = |e: CmdError| if e.kind == "changed" { changed() } else { e };
        match action {
            FindRowsAction::Show => self
                .compute_apply(
                    id,
                    "Filtering",
                    |d, p| {
                        same(d)?;
                        d.plan_show_matching(&spec, p).map(|mut c| {
                            c.name_chip("matching", "Rows found");
                            c
                        })
                    },
                    |d, plan| d.apply_rows(plan),
                )
                .map(report)
                .map_err(fix),
            FindRowsAction::Delete => self
                .compute_apply(
                    id,
                    "Deleting rows",
                    |d, p| {
                        same(d)?;
                        d.plan_delete_matching(&spec, p)
                    },
                    |d, plan| d.apply_rows(plan),
                )
                .map(report)
                .map_err(fix),
            FindRowsAction::Copy => {
                let entry = self.get(id)?;
                let (table, all) = {
                    let job = entry.start("Copying rows")?;
                    let d = entry.read();
                    same(&d).map_err(|_| changed())?;
                    (d.matching_table(&spec, &job.progress)?, d.col_count())
                };
                drop(entry);
                let left_out_cols = all.saturating_sub(table.cols);
                let doc = table.into_document()?;
                let (nid, _) = self.insert(doc, false)?;
                self.inherit_private(nid, &[id], &[]);
                self.set_title_of(nid, format!("Matches ({})", self.stem(id)));
                Ok(FindRowsReport {
                    info: self.info(nid)?,
                    recolored: 0,
                    left_out_cols,
                })
            }
            FindRowsAction::Flag { flag } => {
                use natendo_engine::notes::{Note, FLAGS};
                if flag == 0 || flag > FLAGS {
                    return Err(CmdError::new("invalid", "that flag is unknown"));
                }
                // A job, so a big file shows progress and can be cancelled.
                let rows = {
                    let entry = self.get(id)?;
                    let job = entry.start("Flagging rows")?;
                    let d = entry.read();
                    same(&d).map_err(|_| changed())?;
                    d.matching_rows(&spec, &job.progress)?
                };
                if rows.len() > MAX_FLAG_ROWS {
                    return Err(CmdError::new(
                        "too_large",
                        format!(
                            "at most {} rows can be flagged at once",
                            group(MAX_FLAG_ROWS as u64)
                        ),
                    ));
                }
                self.with_notes(id, |d, b, sidecar| {
                    if b.key_cols.is_empty() {
                        return Err(CmdError::new(
                            "no_keys",
                            "choose the columns that identify rows first (Notes › Columns that identify rows)",
                        ));
                    }
                    let before = b.notes.clone();
                    let mut recolored = 0u64;
                    for &r in &rows {
                        let key = d.note_key(b, r)?;
                        if key.is_empty() {
                            continue;
                        }
                        let old = b.notes.get(&key);
                        if old.is_some_and(|n| n.flag != 0 && n.flag != flag) {
                            recolored += 1;
                        }
                        let text = old.map(|n| n.text.clone()).unwrap_or_default();
                        b.notes.insert(key, Note { flag, text });
                    }
                    if let Err(e) = b.save(sidecar) {
                        b.notes = before;
                        return Err(e.into());
                    }
                    Ok(recolored)
                })
                .map(|recolored| (recolored, self.info(id)))
                .and_then(|(recolored, info)| {
                    Ok(FindRowsReport {
                        info: info?,
                        recolored,
                        left_out_cols: 0,
                    })
                })
            }
        }
    }

    /// Row notes that match a search (Find's "Look in: Notes").
    pub fn find_notes(
        &self,
        id: u32,
        pattern: Pattern,
    ) -> CmdResult<Vec<natendo_engine::notes::NoteMark>> {
        let m = Matcher::new(&pattern)?;
        let (all, _) = self.note_list(id, 100_000)?;
        Ok(all
            .into_iter()
            .filter(|n| m.is_match(&n.text))
            .take(MAX_FIND_HITS)
            .collect())
    }

    /// Column titles a replace would change (Find with "Include column
    /// titles"): (column, new title), applied as renames by the caller.
    pub fn replace_titles(
        &self,
        id: u32,
        spec: FindSpec,
        replacement: String,
    ) -> CmdResult<Vec<(u32, String)>> {
        let m = Matcher::new(&spec.pattern)?;
        m.check_replacement(&replacement)?;
        let entry = self.get(id)?;
        let d = entry.read();
        if d.row_count() == 0 {
            return Ok(Vec::new());
        }
        let cols: Vec<u32> = match &spec.scope {
            natendo_engine::find::Scope::Columns { cols } => cols.clone(),
            natendo_engine::find::Scope::Range { col, cols, row, .. } if *row == 0 => {
                (*col..col + cols).collect()
            }
            natendo_engine::find::Scope::Range { .. } => Vec::new(),
            natendo_engine::find::Scope::Areas { areas } => {
                // The columns of the blocks that reach the title row.
                let mut v: Vec<u32> = areas
                    .iter()
                    .filter(|a| a.row == 0 && a.rows > 0)
                    .flat_map(|a| a.col..a.col.saturating_add(a.cols).min(d.col_count()))
                    .collect();
                v.sort_unstable();
                v.dedup();
                v
            }
            natendo_engine::find::Scope::All => (0..d.col_count()).collect(),
        };
        let mut out = Vec::new();
        for c in cols
            .into_iter()
            .filter(|c| !spec.skip_cols.contains(c) && *c < d.col_count())
        {
            let t = d.cell(0, c).unwrap_or_default();
            if let Some((new, _)) = m.replace(&t, &replacement)? {
                if new != t {
                    out.push((c, new));
                }
            }
        }
        Ok(out)
    }

    /// Two columns of a tab as (find, replace with) pairs for a batch
    /// replace: shown rows, the first one left out when it holds titles,
    /// rows with an empty "find" skipped.
    pub fn column_pairs(
        &self,
        id: u32,
        find_col: u32,
        with_col: u32,
        skip_first: bool,
    ) -> CmdResult<Vec<(String, String)>> {
        use natendo_engine::find::{MAX_BATCH_BYTES, MAX_BATCH_PAIRS};
        let entry = self.get(id)?;
        let d = entry.read();
        if find_col >= d.col_count() || with_col >= d.col_count() {
            return Err(CmdError::new("invalid", "that column is not in the tab"));
        }
        let mut out = Vec::new();
        let mut bytes = 0usize;
        for r in u64::from(skip_first)..d.row_count() {
            let a = d.cell(r, find_col).unwrap_or_default();
            if a.is_empty() {
                continue;
            }
            let b = d.cell(r, with_col).unwrap_or_default();
            bytes += a.len() + b.len();
            out.push((a, b));
            if out.len() > MAX_BATCH_PAIRS || bytes > MAX_BATCH_BYTES {
                return Err(CmdError::new(
                    "too_large",
                    format!(
                        "a list can hold at most {} pairs and 16 MB of text",
                        group(MAX_BATCH_PAIRS as u64)
                    ),
                ));
            }
        }
        Ok(out)
    }

    /// What a batch replace (many pairs) would change; the plan is kept for
    /// [`Docs::batch_confirm`].
    pub fn batch_preview(
        &self,
        id: u32,
        spec: natendo_engine::find::BatchReplace,
    ) -> CmdResult<ReplaceReport> {
        let entry = self.get(id)?;
        let key = format!("batch:{}", serde_json::to_string(&spec).unwrap_or_default());
        let job = entry.start("Looking for matches")?;
        let plan = entry
            .read()
            .plan_batch_replace(&spec, PREVIEW_CHANGES, &job.progress)?;
        drop(job);
        let report = ReplaceReport {
            cells: plan.cells,
            hits: plan.hits,
            preview: plan.preview.clone(),
            info: None,
            hidden_cells: plan.hidden_cells,
        };
        *entry.replace.lock().unwrap_or_else(|p| p.into_inner()) = Some((key, plan));
        Ok(report)
    }

    /// Apply exactly the previewed batch replace (one undo step), or fail
    /// ("changed") if the list or the file changed since.
    pub fn batch_confirm(
        &self,
        id: u32,
        spec: natendo_engine::find::BatchReplace,
    ) -> CmdResult<ReplaceReport> {
        let step = RecipeOp::BatchReplace { spec: spec.clone() };
        self.recorded(id, Logged::Step(step), || self.batch_apply(id, spec, true))
    }

    fn batch_apply(
        &self,
        id: u32,
        spec: natendo_engine::find::BatchReplace,
        exact: bool,
    ) -> CmdResult<ReplaceReport> {
        let entry = self.get(id)?;
        let key = format!("batch:{}", serde_json::to_string(&spec).unwrap_or_default());
        let kept = entry
            .replace
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .filter(|(k, _)| *k == key)
            .map(|(_, plan)| plan);
        let plan = match kept {
            Some(p) => p,
            None if exact => {
                return Err(CmdError::new(
                    "changed",
                    "the list or the file changed since the preview; nothing was replaced. Look at the changes again",
                ))
            }
            None => {
                let job = entry.start("Replacing")?;
                let p = entry.read().plan_batch_replace(&spec, 0, &job.progress)?;
                drop(job);
                p
            }
        };
        let (cells, hits) = (plan.cells, plan.hits);
        let rows = plan.row_ids();
        let info = self.edit(id, |d| d.apply_replace(plan)).map_err(|e| {
            if e.kind == "changed" && exact {
                CmdError::new(
                    "changed",
                    "the file changed since the preview; nothing was replaced. Look at the changes again",
                )
            } else {
                e
            }
        })?;
        let rev = entry.read().revision();
        *entry.changed_rows.lock().unwrap_or_else(|p| p.into_inner()) = (rev, rows);
        Ok(ReplaceReport {
            cells,
            hits,
            preview: Vec::new(),
            info: Some(info),
            hidden_cells: 0,
        })
    }

    /// Show only the rows the last Replace all changed (a view, undoable).
    pub fn show_changed(&self, id: u32, keep_first: bool) -> CmdResult<DocInfo> {
        let entry = self.get(id)?;
        let (rev, ids) = entry
            .changed_rows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if ids.is_empty() {
            return Err(CmdError::new("invalid", "nothing was replaced yet"));
        }
        if rev != entry.read().revision() {
            return Err(CmdError::new(
                "changed",
                "the file changed since the last Replace all (or it was undone), so there are no changed rows to show",
            ));
        }
        drop(entry);
        self.compute_apply(
            id,
            "Filtering",
            |d, _| {
                let mut c = d.plan_show_ids(&ids, keep_first);
                c.name_chip("ids", "Rows picked");
                Ok(c)
            },
            |d, plan| d.apply_rows(plan),
        )
    }

    /// Replace in one cell (Find's Replace button): an edit by hand, not a
    /// recipe step.
    pub fn replace_cell(
        &self,
        id: u32,
        spec: FindSpec,
        replacement: String,
    ) -> CmdResult<ReplaceReport> {
        self.replace_all_now(id, spec, replacement, false)
    }

    /// The preview's "Replace all": applies exactly the plan the preview
    /// showed, or nothing. If the search, the replacement or the file
    /// changed since, it fails ("changed") instead of working out a new,
    /// unseen plan.
    #[cfg(test)]
    pub fn replace_confirm(
        &self,
        id: u32,
        spec: FindSpec,
        replacement: String,
    ) -> CmdResult<ReplaceReport> {
        self.replace_confirm_except(id, spec, replacement, Vec::new())
    }

    /// [`Docs::replace_confirm`] leaving out the changes the user unticked
    /// (row number in the document, column).
    pub fn replace_confirm_except(
        &self,
        id: u32,
        spec: FindSpec,
        replacement: String,
        exclude: Vec<(u64, u32)>,
    ) -> CmdResult<ReplaceReport> {
        if !exclude.is_empty() {
            let entry = self.get(id)?;
            let mut kept = entry.replace.lock().unwrap_or_else(|p| p.into_inner());
            if let Some((_, plan)) = kept.as_mut() {
                plan.exclude(&exclude);
            }
        }
        // With changes unticked by hand the step can't be repeated on
        // another file (or the same one) faithfully, so it is left out of
        // recipes rather than replayed as a full Replace all.
        let logged = if exclude.is_empty() {
            Logged::Step(RecipeOp::ReplaceAll {
                spec: spec.clone(),
                with: replacement.clone(),
            })
        } else {
            Logged::LeftOut(format!(
                "Replace all with {} change{} unticked by hand",
                exclude.len(),
                if exclude.len() == 1 { "" } else { "s" }
            ))
        };
        self.recorded(id, logged, || {
            self.replace_all_now(id, spec, replacement, true)
        })
    }

    fn replace_all_now(
        &self,
        id: u32,
        spec: FindSpec,
        replacement: String,
        exact: bool,
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
            let rows = plan.row_ids();
            let info = self.edit(id, |d| d.apply_replace(plan))?;
            let rev = entry.read().revision();
            *entry.changed_rows.lock().unwrap_or_else(|p| p.into_inner()) = (rev, rows);
            Ok(ReplaceReport {
                cells,
                hits,
                preview: Vec::new(),
                info: Some(info),
                hidden_cells: 0,
            })
        };
        let changed = || {
            CmdError::new(
                "changed",
                "the search or the file changed since the preview; nothing was replaced. Look at the changes again (Replace all…)",
            )
        };
        if let Some(plan) = kept {
            match attempt(plan) {
                Err(e) if e.kind == "changed" && exact => return Err(changed()),
                Err(e) if e.kind == "changed" => {}
                other => return other,
            }
        } else if exact {
            return Err(changed());
        }
        let job = entry.start("Replacing")?;
        let plan = entry
            .read()
            .plan_replace(&spec, &replacement, 0, &job.progress)?;
        drop(job);
        attempt(plan)
    }

    /// The user's saved custom orders, for fills (the window sends them when
    /// they change).
    /// Their lookup is built in the background, so fills find their
    /// starting values at once.
    pub fn set_fill_orders(&self, orders: Vec<natendo_engine::fill::FillOrder>) {
        let book = Arc::new(natendo_engine::fill::OrderBook::new(orders));
        *self.fill_orders.lock().unwrap_or_else(|p| p.into_inner()) = book.clone();
        if !book.orders().is_empty() {
            std::thread::spawn(move || book.prepare());
        }
    }

    /// Run a data operation as one undo step (or, for "show duplicates", a
    /// filter), computed under the read lock with progress and cancel.
    pub fn data_op(&self, id: u32, op: DataOp) -> CmdResult<OpReport> {
        // A fill that may follow the user's saved orders takes only the ones
        // it uses, so the step (and a recipe recorded from it) holds those.
        let op = match op {
            DataOp::Fill { mut spec } if spec.saved_orders => {
                spec.saved_orders = false;
                let saved = self
                    .fill_orders
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone();
                if !saved.orders().is_empty() {
                    let used =
                        self.get(id)?
                            .read()
                            .fill_orders_used(&spec, &saved, &Progress::new())?;
                    spec.orders = used
                        .into_iter()
                        .map(|j| saved.orders()[j].clone())
                        .collect();
                }
                DataOp::Fill { spec }
            }
            op => op,
        };
        // A fill's guard column is recorded with its title, so a recipe finds it by title.
        let op = match op {
            DataOp::FillBlanks { mut spec } if spec.guard.is_some() && spec.keep_first => {
                if let (Some(g), Ok(entry)) = (spec.guard, self.get(id)) {
                    spec.guard_title = entry.read().titles(&[g], true).into_iter().next();
                }
                DataOp::FillBlanks { spec }
            }
            op => op,
        };
        let logged = recipe_step(&op);
        // A paste of more than 1,000 cells also gets a checkpoint (a small one is as cheap to undo as to redo).
        let auto = match &op {
            DataOp::Paste { text, .. } if text.matches(['\t', '\n']).count() >= BIG_PASTE_CELLS => {
                Some("a big paste".to_string())
            }
            _ => auto_label(&logged).map(str::to_string),
        };
        self.recorded_auto(id, logged, auto, || self.data_op_now(id, op))
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
        use natendo_engine::fuzzy::{find_matches, MatchSide};
        if spec.fields.is_empty() || spec.fields.len() > natendo_engine::fuzzy::MAX_FIELDS {
            return Err(CmdError::new("invalid", "choose the columns to compare"));
        }
        // The same columns the command line's `match` builds its sides from.
        let (here, there, kinds) = spec.columns();
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
            never_offered: rep.total.saturating_sub(rep.listed.len() as u64),
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
            |d, p| {
                d.plan_show_noted(&book, flag, p).map(|mut c| {
                    c.name_chip("notes", "Rows with notes");
                    c
                })
            },
            |d, plan| d.apply_rows(plan),
        )
    }

    // ---- SQLite (Milestone 15) ----

    /// Open a table of a database file in a new tab.
    #[cfg(test)]
    pub fn open_sqlite(&self, path: &Path, table: &str) -> CmdResult<DocInfo> {
        self.open_sqlite_report(path, table).map(|r| r.0)
    }

    /// [`Docs::open_sqlite`], and one sentence about the columns declared NUMERIC/DECIMAL (SQLite keeps
    /// them as binary numbers, so trailing zeros were lost inside the database); empty when none.
    pub fn open_sqlite_report(&self, path: &Path, table: &str) -> CmdResult<(DocInfo, String)> {
        let progress = Progress::new();
        let opened = natendo_engine::sqlite::read_table_report(path, table, &progress)?;
        let (doc, decimal, geometry) = (opened.doc, opened.decimal, opened.geometry);
        let names = doc.row_values(0).unwrap_or_default();
        let note = if decimal.is_empty() {
            String::new()
        } else {
            let list: Vec<String> = decimal
                .iter()
                .take(5)
                .map(|c| {
                    match names
                        .get(*c as usize)
                        .map(|n| n.trim())
                        .filter(|n| !n.is_empty())
                    {
                        Some(n) => {
                            format!("\u{201c}{}\u{201d}", n.chars().take(40).collect::<String>())
                        }
                        None => format!("column {}", c + 1),
                    }
                })
                .collect();
            format!(
                "{} {} stored as binary numbers in the database, so trailing zeros were already lost there.",
                if decimal.len() == 1 { "Column" } else { "Columns" },
                list.join(", ")
            )
        };
        // Shapes left out are named, so nobody thinks the file has none.
        let note = if geometry.is_empty() {
            note
        } else {
            let shapes = format!(
                "Geometry column{} {} not shown.",
                if geometry.len() == 1 { "" } else { "s" },
                geometry.join(", ")
            );
            format!("{shapes} {note}").trim_end().to_string()
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let info = self.insert_titled(doc, Some(format!("{name}: {table}")))?;
        self.private_if_from(info.id, path);
        Ok((self.info(info.id)?, note))
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
        self.refuse_peeking(id)?;
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
        self.refuse_peeking(id)?;
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
        let auto = auto_label(&logged).map(str::to_string);
        self.recorded_auto(id, logged, auto, f)
    }

    /// `recorded`, with an automatic checkpoint "Before <label>" when `auto` names one and the change made a step.
    fn recorded_auto<T>(
        &self,
        id: u32,
        logged: Logged,
        auto: Option<String>,
        f: impl FnOnce() -> CmdResult<T>,
    ) -> CmdResult<T> {
        let entry = self.get(id)?;
        // The mark just before the change is noted by the document itself, under the write lock of the change (a
        // read here, before it, could be overtaken by another window's change).
        if auto.is_some() {
            natendo_engine::document::capture_mark_start(entry.read().uid());
        }
        let (first, before) = {
            let d = entry.read();
            (d.row_values(0).ok(), d.history_pos())
        };
        let out = f();
        let mark_before = natendo_engine::document::capture_mark_take();
        let out = out?;
        // A first step that sets the title lines aside: the recipe's column
        // titles are the row that is first after it (see
        // `Recipe::column_problems`).
        let first = match &logged {
            Logged::Step(RecipeOp::SetAside { head, .. }) if *head > 0 => {
                entry.read().row_values(0).ok()
            }
            _ => first,
        };
        if let (Some(label), Some(mark)) = (auto, mark_before) {
            self.auto_checkpoint(&entry, mark, &label);
        }
        let pos = entry.read().history_pos();
        let mut log = entry.recipe.lock().unwrap_or_else(|p| p.into_inner());
        // Steps past where this one started were undone and are now
        // replaced (filters are undo steps too, so this holds for them).
        log.steps.retain(|e| e.pos <= before);
        if log.steps.is_empty() {
            log.first_row = first;
        }
        if log.steps.len() < natendo_engine::recipe::MAX_STEPS {
            log.steps.push(LogEntry {
                before,
                pos,
                item: logged,
            });
        }
        Ok(out)
    }

    /// The noted steps the table can still reach (by undo or redo).
    fn live_steps(&self, id: u32) -> CmdResult<(Vec<LogEntry>, u64, u64)> {
        let entry = self.get(id)?;
        let (at, reach) = {
            let d = entry.read();
            (d.history_pos(), d.history_pos() + d.redo_count())
        };
        let log = entry.recipe.lock().unwrap_or_else(|p| p.into_inner());
        let steps: Vec<LogEntry> = log
            .steps
            .iter()
            .filter(|e| e.pos <= reach)
            .cloned()
            .collect();
        Ok((steps, at, reach))
    }

    /// The steps panel's view of tab `id`.
    pub fn steps_view(&self, id: u32) -> CmdResult<StepsView> {
        let (steps, at, _) = self.live_steps(id)?;
        let questions = {
            let entry = self.get(id)?;
            let log = entry.recipe.lock().unwrap_or_else(|p| p.into_inner());
            log.questions.clone()
        };
        let rows: Vec<StepRow> = steps
            .iter()
            .map(|e| {
                let (kind, text, op) = match &e.item {
                    Logged::Step(s) => ("step", s.describe(), Some(s.clone())),
                    Logged::Off(s) => ("off", s.describe(), Some(s.clone())),
                    Logged::LeftOut(w) => ("left_out", w.clone(), None),
                };
                let texts = op
                    .as_ref()
                    .map(|o| {
                        Recipe {
                            version: natendo_engine::recipe::RECIPE_VERSION,
                            name: String::new(),
                            columns: Vec::new(),
                            header: false,
                            steps: vec![o.clone()],
                            left_out: Vec::new(),
                            questions: Vec::new(),
                        }
                        .step_texts(0)
                    })
                    .unwrap_or_default();
                StepRow {
                    kind,
                    text,
                    done: e.pos <= at,
                    op,
                    texts,
                }
            })
            .collect();
        let done = steps.iter().filter(|e| e.pos <= at).count();
        let last = steps.last().map_or(0, |e| e.pos);
        Ok(StepsView {
            at: done,
            hand_changes: !steps.is_empty() && at > last,
            steps: rows,
            questions,
        })
    }

    /// Move the table to just after step `k` (`None`: before the first
    /// step; the number of steps: the latest, with any changes by hand
    /// after the last step), by undo or redo: a look at the table as it was
    /// then. Before the latest, changes and saving are refused.
    pub fn steps_goto(&self, id: u32, k: Option<usize>) -> CmdResult<DocInfo> {
        let (steps, _, reach) = self.live_steps(id)?;
        let target = match k {
            None => steps.first().map(|e| e.before),
            Some(k) if k == steps.len() => Some(reach),
            Some(k) => steps.get(k).map(|e| e.pos),
        }
        .ok_or_else(|| CmdError::new("invalid", "there is no such step"))?;
        let target = target.min(reach);
        let entry = self.get(id)?;
        let r = self.edit_any(id, true, |d| {
            history_to(d, target)?;
            Ok(())
        });
        let at = entry.read().history_pos();
        entry.peeking.store(at < reach, Ordering::Relaxed);
        let mut info = r?;
        info.peeking = at < reach;
        Ok(info)
    }

    /// The automatic checkpoint "Before <label>" at `mark`, when the change really made a step (the table is in a
    /// different state now) and no checkpoint already names that moment.
    fn auto_checkpoint(&self, entry: &Entry, mark: u64, label: &str) {
        if entry.read().step_mark() == mark {
            return;
        }
        let mut book = entry.checkpoints.lock().unwrap_or_else(|p| p.into_inner());
        if book.list.iter().any(|c| c.mark == mark) {
            return;
        }
        book.add(format!("Before {label}"), mark, true);
    }

    /// Name this moment: a checkpoint at the table's state now (a few bytes; nothing is copied).
    pub fn checkpoint_add(&self, id: u32, name: &str) -> CmdResult<Vec<CheckpointInfo>> {
        let name: String = name.trim().chars().take(100).collect();
        if name.is_empty() {
            return Err(CmdError::new("invalid", "give the checkpoint a name"));
        }
        let entry = self.get(id)?;
        let mark = entry.read().step_mark();
        {
            let mut book = entry.checkpoints.lock().unwrap_or_else(|p| p.into_inner());
            // Naming a moment that already has an automatic checkpoint turns that one into the person's own.
            if let Some(c) = book.list.iter_mut().find(|c| c.mark == mark && c.auto) {
                c.auto = false;
                c.name = name;
            } else {
                book.add(name, mark, false);
            }
        }
        self.checkpoint_list(id)
    }

    /// The tab's checkpoints, oldest first, each with the engine's word on whether it can still be reached.
    pub fn checkpoint_list(&self, id: u32) -> CmdResult<Vec<CheckpointInfo>> {
        let entry = self.get(id)?;
        let d = entry.read();
        let now = d.step_mark();
        let book = entry.checkpoints.lock().unwrap_or_else(|p| p.into_inner());
        Ok(book
            .list
            .iter()
            .map(|c| CheckpointInfo {
                id: c.id,
                name: c.name.clone(),
                auto: c.auto,
                reachable: d.step_reach(c.mark).is_some(),
                steps: d.step_reach(c.mark).unwrap_or(0),
                current: c.mark == now,
                gone_at: c.gone.map(|g| g.0),
                gone_why: c.gone.map(|g| g.1),
                since_note: match d.step_reach(c.mark) {
                    None if c.gone.is_some() => format!(
                        "No longer reachable: a save let it go ({}).",
                        c.gone.map_or("", |g| g.1)
                    ),
                    None => "No longer reachable: the undo history was trimmed past it, or a newer change replaced it.".to_string(),
                    Some(0) => "The table is in this state now: nothing has changed since.".to_string(),
                    Some(s) if s > 0 => "This checkpoint is ahead of the table (you went back past it): go to it first, then compare.".to_string(),
                    Some(_) => String::new(),
                },
            })
            .collect())
    }

    pub fn checkpoint_remove(&self, id: u32, cp: u32) -> CmdResult<Vec<CheckpointInfo>> {
        let entry = self.get(id)?;
        entry
            .checkpoints
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .list
            .retain(|c| c.id != cp);
        self.checkpoint_list(id)
    }

    /// Go back (or forward) to the state a checkpoint names, by undo or redo: one visible jump. The moment it
    /// leaves gets an automatic checkpoint ("Before restoring ...") so the way back is one click. Allowed while the
    /// steps panel looks back (it ends that look). Refused, and nothing changed, when the history no longer reaches
    /// it, or the file is locked.
    pub fn checkpoint_restore(&self, id: u32, cp: u32) -> CmdResult<DocInfo> {
        let entry = self.get(id)?;
        let (name, mark) = {
            let book = entry.checkpoints.lock().unwrap_or_else(|p| p.into_inner());
            let c = book
                .list
                .iter()
                .find(|c| c.id == cp)
                .ok_or_else(|| CmdError::new("invalid", "there is no such checkpoint"))?;
            (c.name.clone(), c.mark)
        };
        let leaving = entry.read().step_mark();
        let mut info = self.edit_any(id, true, |d| {
            if d.go_to_mark(mark) {
                Ok(())
            } else {
                Err(Error::Invalid(
                    "that checkpoint can no longer be reached: the undo history was trimmed past it, a newer change replaced it, or a save let it go (a column change, or not enough memory for the history)".into(),
                ))
            }
        })?;
        entry.peeking.store(false, Ordering::Relaxed);
        info.peeking = false;
        if leaving != mark {
            let mut book = entry.checkpoints.lock().unwrap_or_else(|p| p.into_inner());
            if !book.list.iter().any(|c| c.mark == leaving) {
                book.add(
                    format!("Before restoring \u{201c}{name}\u{201d}"),
                    leaving,
                    true,
                );
            }
        }
        Ok(info)
    }

    /// The checkpoint's mark.
    fn checkpoint_mark(&self, entry: &Entry, cp: u32) -> CmdResult<u64> {
        let book = entry.checkpoints.lock().unwrap_or_else(|p| p.into_inner());
        book.list
            .iter()
            .find(|c| c.id == cp)
            .map(|c| c.mark)
            .ok_or_else(|| CmdError::new("invalid", "there is no such checkpoint"))
    }

    /// Runs `f` with the plan of "what changed since this checkpoint" and the document it was made for, under ONE read
    /// lock: no change can come between the plan's check and the page read. The plan is made as a job with progress
    /// and Cancel the first time (and again after any change) and kept while the document stays as it is. Never run
    /// while a list is drawn: only when asked.
    fn with_since<T>(
        &self,
        entry: &Entry,
        cp: u32,
        header: bool,
        f: impl FnOnce(&Document, &SincePlan, u64) -> CmdResult<T>,
    ) -> CmdResult<T> {
        let mark = self.checkpoint_mark(entry, cp)?;
        let d = entry.read();
        let kept = entry
            .since_plan
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .filter(|k| k.checkpoint == cp && k.header == header && k.revision == d.revision())
            .map(|k| k.plan.clone());
        let plan = match kept {
            Some(plan) => plan,
            None => {
                let job = entry.start("Comparing with the checkpoint")?;
                let plan = Arc::new(d.since_plan(mark, header, &job.progress)?);
                *entry.since_plan.lock().unwrap_or_else(|p| p.into_inner()) = Some(KeptSince {
                    checkpoint: cp,
                    header,
                    revision: d.revision(),
                    plan: plan.clone(),
                });
                plan
            }
        };
        f(&d, &plan, mark)
    }

    /// What changed since a checkpoint: every count, and one page of the list (changed cells, rows taken out, rows
    /// made, cells of columns taken out).
    pub fn checkpoint_since(
        &self,
        id: u32,
        cp: u32,
        header: bool,
        offset: u64,
        want: usize,
    ) -> CmdResult<SinceView> {
        let entry = self.get(id)?;
        self.with_since(&entry, cp, header, |d, plan, mark| {
            Ok(SinceView {
                totals: plan.totals.clone(),
                cells: d.since_page(plan, mark, offset, want.min(500))?,
            })
        })
    }

    /// Write the whole list of what changed since a checkpoint as a CSV report to `path`; returns the lines written.
    /// Columns are named by the first row only when it holds titles (`header`): otherwise "Column N", so that no data
    /// value becomes a name.
    pub fn checkpoint_since_report(
        &self,
        id: u32,
        cp: u32,
        header: bool,
        path: &Path,
        guard: bool,
    ) -> CmdResult<Written> {
        use natendo_engine::document::since::SinceKind;
        use std::io::Write as _;
        self.check_writable(path)?;
        let entry = self.get(id)?;
        self.with_since(&entry, cp, header, |d, plan, mark| {
            if d.path()
                .is_some_and(|p| natendo_engine::save::same_path(p, path))
            {
                return Err(
                    Error::Invalid("choose another name: that is the open file".into()).into(),
                );
            }
            let named = |title: Option<String>, at: u32| {
                title
                    .filter(|t| header && !t.is_empty())
                    .unwrap_or_else(|| format!("Column {}", at + 1))
            };
            let titles: Vec<String> = (0..d.col_count())
                .map(|c| named(if header { d.cell(0, c) } else { None }, c))
                .collect();
            let removed: Vec<String> = plan
                .totals
                .removed_cols
                .iter()
                .map(|c| named(Some(c.title.clone()), c.at))
                .collect();
            let io = |e: std::io::Error| CmdError::new("io", e.to_string());
            let mut out = std::io::BufWriter::new(std::fs::File::create(path).map_err(io)?);
            let mut line = String::new();
            let mut guarded = 0u64;
            let mut put = |fields: &[&str], out: &mut dyn std::io::Write| -> CmdResult<()> {
                line.clear();
                for (i, f) in fields.iter().enumerate() {
                    if i > 0 {
                        line.push(',');
                    }
                    guarded += u64::from(natendo_engine::scan::write_field_guarded(
                        f,
                        natendo_engine::dialect::Dialect::CSV,
                        guard,
                        &mut line,
                    ));
                }
                line.push_str("\r\n");
                out.write_all(line.as_bytes()).map_err(io)
            };
            put(&["Row now", "Column", "Was", "Now"], &mut out)?;
            let mut offset = 0u64;
            loop {
                let page = d.since_page(plan, mark, offset, 5_000)?;
                if page.is_empty() {
                    break;
                }
                offset += page.len() as u64;
                for c in &page {
                    let row = c.row.map_or(String::new(), |r| (r + 1).to_string());
                    let column = match (c.kind, c.col, c.removed_col) {
                        (SinceKind::Cell, Some(col), _) => titles
                            .get(col as usize)
                            .cloned()
                            .unwrap_or_else(|| format!("Column {}", col + 1)),
                        (SinceKind::ColRemoved, _, Some(i)) => {
                            removed.get(i as usize).cloned().unwrap_or_default()
                        }
                        _ => "(whole row)".to_string(),
                    };
                    put(&[&row, &column, &c.old, &c.new], &mut out)?;
                }
            }
            out.flush().map_err(io)?;
            Ok(Written {
                entries: offset,
                guarded,
            })
        })
    }

    /// Refuse while the steps panel looks back.
    fn refuse_peeking(&self, id: u32) -> CmdResult<()> {
        if self.get(id)?.peeking.load(Ordering::Relaxed) {
            return Err(peeking());
        }
        Ok(())
    }

    /// Change step `k` (remove it, turn it off or on, move it, or give it
    /// new settings): the table goes back to before it and the steps from
    /// there run again. Changes by hand after it, and steps that cannot run
    /// again, would be lost: refused unless `force`.
    pub fn steps_change(
        &self,
        id: u32,
        k: usize,
        change: StepChange,
        now: f64,
        force: bool,
    ) -> CmdResult<DocInfo> {
        let (_, _, reach) = self.live_steps(id)?;
        // From the latest step.
        self.edit_any(id, true, |d| history_to(d, reach))?;
        self.get(id)?.peeking.store(false, Ordering::Relaxed);
        let (steps, at, _) = self.live_steps(id)?;
        if k >= steps.len() {
            return Err(CmdError::new("invalid", "there is no such step"));
        }
        let start = match change {
            StepChange::MoveUp if k == 0 => {
                return Err(CmdError::new("invalid", "the first step cannot move up"))
            }
            StepChange::MoveUp => k - 1,
            StepChange::MoveDown if k + 1 >= steps.len() => {
                return Err(CmdError::new("invalid", "the last step cannot move down"))
            }
            _ => k,
        };
        // What would be lost by going back.
        let mut lost = Vec::new();
        for (j, e) in steps.iter().enumerate().skip(start) {
            if let Logged::LeftOut(w) = &e.item {
                lost.push(format!("step {} ({w}) cannot be done again", j + 1));
            }
            if j > start && e.before != steps[j - 1].pos {
                lost.push(format!("changes made by hand after step {}", j));
            }
        }
        if at > steps.last().map_or(at, |e| e.pos) {
            lost.push("changes made by hand after the last step".into());
        }
        if !lost.is_empty() && !force {
            return Err(CmdError::new(
                "steps_lost",
                format!("going back would undo {}", lost.join("; ")),
            ));
        }
        let mut items: Vec<Logged> = steps[start..]
            .iter()
            .map(|e| e.item.clone())
            .filter(|l| !matches!(l, Logged::LeftOut(_)))
            .collect();
        let original = items.clone();
        let rel = k - start;
        match change {
            StepChange::Remove => {
                items.remove(rel);
            }
            StepChange::Toggle => {
                items[rel] = match items[rel].clone() {
                    Logged::Step(s) => Logged::Off(s),
                    Logged::Off(s) => Logged::Step(s),
                    l => l,
                };
            }
            StepChange::MoveUp => items.swap(0, 1),
            StepChange::MoveDown => items.swap(rel, rel + 1),
            StepChange::Replace { op } => {
                let r = one_step_recipe(op.clone());
                r.check()?;
                items[rel] = Logged::Step(op);
            }
        }
        let questions = {
            let entry = self.get(id)?;
            let log = entry.recipe.lock().unwrap_or_else(|p| p.into_inner());
            log.questions.clone()
        };
        let back = steps[start].before;
        self.edit(id, |d| {
            history_to(d, back)?;
            d.drop_redo();
            Ok(())
        })?;
        // The steps from `start` on are noted again as they run.
        let kept = {
            let entry = self.get(id)?;
            let mut log = entry.recipe.lock().unwrap_or_else(|p| p.into_inner());
            let first = &steps[start];
            if let Some(i) = log
                .steps
                .iter()
                .position(|e| e.before == first.before && e.pos == first.pos)
            {
                log.steps.truncate(i);
            }
            log.steps.len()
        };
        // All or nothing: when a step fails (or the run is cancelled), the
        // table goes back and the steps as they were run again.
        if let Err(e) = self.replay_steps(id, start, items, &questions, now) {
            let restore = self
                .edit_any(id, true, |d| {
                    history_to(d, back)?;
                    d.drop_redo();
                    Ok(())
                })
                .and_then(|_| {
                    let entry = self.get(id)?;
                    entry
                        .recipe
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .steps
                        .truncate(kept);
                    self.replay_steps(id, start, original, &questions, now)
                });
            let what = if e.kind == "cancelled" {
                "running the steps again was cancelled".to_string()
            } else {
                e.message
            };
            return Err(CmdError::new(
                "invalid",
                match restore {
                    Ok(()) => format!("{what}; nothing was changed"),
                    Err(r) => format!(
                        "{what}; putting the steps back as they were also failed: {}",
                        r.message
                    ),
                },
            ));
        }
        self.info(id)
    }

    /// Run `items` (steps from number `start + 1` on) and note them.
    fn replay_steps(
        &self,
        id: u32,
        start: usize,
        items: Vec<Logged>,
        questions: &[natendo_engine::recipe::Question],
        now: f64,
    ) -> CmdResult<()> {
        for (i, item) in items.into_iter().enumerate() {
            match item {
                Logged::Step(op) => {
                    // Questions take their last answers; the step keeps
                    // its question.
                    let mut r = one_step_recipe(op.clone());
                    r.questions = questions.to_vec();
                    let run = r.answered(&[])?.steps.remove(0);
                    self.run_recipe_op(id, run, now).map_err(|mut e| {
                        e.message =
                            format!("step {} ({}): {}", start + i + 1, op.describe(), e.message);
                        e
                    })?;
                    let entry = self.get(id)?;
                    let mut log = entry.recipe.lock().unwrap_or_else(|p| p.into_inner());
                    if let Some(last) = log.steps.last_mut() {
                        last.item = Logged::Step(op);
                    }
                }
                Logged::Off(op) => {
                    let entry = self.get(id)?;
                    let pos = entry.read().history_pos();
                    let mut log = entry.recipe.lock().unwrap_or_else(|p| p.into_inner());
                    log.steps.retain(|e| e.pos <= pos);
                    log.steps.push(LogEntry {
                        before: pos,
                        pos,
                        item: Logged::Off(op),
                    });
                }
                Logged::LeftOut(_) => {}
            }
        }
        Ok(())
    }

    /// Make the text at `path` in step `k` a question asked each time the
    /// recipe runs (the text there now is its first answer).
    pub fn steps_ask(
        &self,
        id: u32,
        k: usize,
        path: &str,
        name: &str,
        prompt: &str,
    ) -> CmdResult<StepsView> {
        let (steps, _, _) = self.live_steps(id)?;
        let e = steps
            .get(k)
            .ok_or_else(|| CmdError::new("invalid", "there is no such step"))?;
        let (op, off) = match &e.item {
            Logged::Step(s) => (s.clone(), false),
            Logged::Off(s) => (s.clone(), true),
            Logged::LeftOut(_) => {
                return Err(CmdError::new("invalid", "that step cannot be done again"))
            }
        };
        {
            let entry = self.get(id)?;
            let mut log = entry.recipe.lock().unwrap_or_else(|p| p.into_inner());
            let mut r = one_step_recipe(op);
            r.questions = log.questions.clone();
            r.ask(0, path, name, prompt)?;
            let new = r.steps.remove(0);
            // The same entry in the full log (by position).
            if let Some(le) = log
                .steps
                .iter_mut()
                .find(|x| x.pos == e.pos && x.before == e.before)
            {
                le.item = if off {
                    Logged::Off(new)
                } else {
                    Logged::Step(new)
                };
            }
            log.questions = r.questions;
        }
        self.steps_view(id)
    }

    /// The tab's steps so far (those not undone) as a recipe.
    pub fn recipe_of(&self, id: u32, header: bool, name: &str) -> CmdResult<Recipe> {
        let entry = self.get(id)?;
        let pos = entry.read().history_pos();
        let log = entry.recipe.lock().unwrap_or_else(|p| p.into_inner());
        let mut steps = Vec::new();
        let mut left_out = Vec::new();
        for e in &log.steps {
            if e.pos > pos {
                continue;
            }
            match &e.item {
                Logged::Step(s) => steps.push(s.clone()),
                Logged::LeftOut(w) => left_out.push(w.clone()),
                Logged::Off(_) => {}
            }
        }
        // A table with no title row has data in its first row: a column type is then kept by position only, never
        // by the first person's name or ID.
        if !header {
            for st in steps.iter_mut() {
                if let RecipeOp::ColumnTypes { types } = st {
                    for t in types.iter_mut() {
                        t.title.clear();
                    }
                }
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
            questions: log.questions.clone(),
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
        answers: &[(String, String)],
        now: f64,
        force: bool,
    ) -> CmdResult<DocInfo> {
        let recipe = &recipe.answered(answers)?;
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
        let mut notes = Vec::new();
        for (i, s) in recipe.steps.iter().enumerate() {
            let note = self.run_recipe_op(id, s.clone(), now).map_err(|mut e| {
                e.message = format!("step {} ({}): {}", i + 1, s.describe(), e.message);
                e
            })?;
            if !note.is_empty() {
                notes.push(note);
            }
        }
        let mut info = self.info(id)?;
        info.run_notes = notes;
        Ok(info)
    }

    /// One recipe step, through the same command as when done by hand.
    /// Gives what a step has to say about its run ("" for most).
    fn run_recipe_op(&self, id: u32, s: RecipeOp, now: f64) -> CmdResult<String> {
        if let RecipeOp::Clean { spec } = &s {
            check_clean(spec, now)?;
        }
        if let RecipeOp::Filter { spec } = &s {
            check_filter(spec)?;
        }
        let done: CmdResult<()> = match s {
            RecipeOp::Sort { spec } => self.sort(id, spec).map(|_| ()),
            RecipeOp::Filter { spec } => self.filter(id, spec).map(|_| ()),
            RecipeOp::ClearFilter => self.clear_filter(id).map(|_| ()),
            RecipeOp::InsertCols { at, count } => self.insert_cols(id, at, count).map(|_| ()),
            RecipeOp::DeleteCols { at, count } => self.delete_cols(id, at, count).map(|_| ()),
            RecipeOp::MoveCols { at, count, to } => self.move_cols(id, at, count, to).map(|_| ()),
            RecipeOp::ReplaceAll { spec, with } => self.replace_all(id, spec, with).map(|_| ()),
            RecipeOp::BatchReplace { spec } => self.batch_apply(id, spec, false).map(|_| ()),
            RecipeOp::RenameColumn { col, name } => {
                self.set_first_row_cell(id, col, name).map(|_| ())
            }
            RecipeOp::TidyTitles { spec } => self.tidy_titles(id, spec).map(|_| ()),
            RecipeOp::JoinFields {
                usual,
                fields,
                groups,
                header,
                ..
            } => {
                let rows = self.replay_join_fields(id, usual, fields, groups, header)?;
                return Ok(format!(
                    "Join fields in every row of {fields} fields: {} {}",
                    group(rows),
                    if rows == 1 { "row" } else { "rows" }
                ));
            }
            RecipeOp::ColumnTypes { types } => {
                let set = {
                    let entry = self.get(id)?;
                    let d = entry.read();
                    d.types_from_step(&types)?
                };
                self.set_column_types(id, set).map(|_| ())
            }
            RecipeOp::SetAside { head, foot } => {
                let step = RecipeOp::SetAside { head, foot };
                self.recorded(id, Logged::Step(step), || {
                    self.edit(id, |d| d.set_aside_ends(u64::from(head), u64::from(foot)))
                })?;
                let n = self.get(id)?.read().note_counts();
                return Ok(format!(
                    "Set aside {} above and {} below the table as report notes (View › Report notes shows them)",
                    lines_word(n.above_set),
                    lines_word(n.below_set)
                ));
            }
            RecipeOp::CleanMatrix { spec } => {
                // Columns are found by title in the file the recipe runs on.
                let spec = self.get(id)?.read().matrix_by_title(&spec)?;
                let op = DataOp::CleanMatrix {
                    spec,
                    now,
                    revision: None,
                };
                let left = self.data_op(id, op)?.ambiguous_left;
                return Ok(left_note("Clean matrix", left));
            }
            RecipeOp::Clean { spec } => {
                let what = spec
                    .cleaner
                    .steps
                    .iter()
                    .find_map(|s| match s {
                        natendo_engine::clean::Step::Dates { format, .. } => {
                            Some(format!("Dates as {format}"))
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| "Clean".into());
                let left = self
                    .data_op(id, recipe_data_op(RecipeOp::Clean { spec }, now))?
                    .ambiguous_left;
                return Ok(left_note(&what, left));
            }
            RecipeOp::ExplodeColumns { spec } | RecipeOp::ExplodeRows { spec } => {
                // New and missing keys, and cells that did not parse, are named.
                let title = self
                    .get(id)?
                    .read()
                    .titles(&[spec.col], spec.keep_first)
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                let rows = spec.how == natendo_engine::explode::ExplodeHow::Rows;
                let op = if rows {
                    DataOp::ExplodeRows { spec }
                } else {
                    DataOp::ExplodeColumns { spec }
                };
                let r = self.data_op(id, op)?;
                return Ok(r
                    .explode
                    .map(|e| e.note(&format!("\u{201c}{title}\u{201d}")))
                    .unwrap_or_default());
            }
            other => self.data_op(id, recipe_data_op(other, now)).map(|_| ()),
        };
        done.map(|()| String::new())
    }

    fn data_op_now(&self, id: u32, op: DataOp) -> CmdResult<OpReport> {
        if let DataOp::ShowProblems { spec, .. }
        | DataOp::SortProblems { spec, .. }
        | DataOp::CheckColumn { spec, .. } = &op
        {
            natendo_engine::validate::check_spec(spec)?;
        }
        if let DataOp::ShowContractRows { contract, .. } = &op {
            contract.validate()?;
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
            DataOp::ExplodeColumns { .. } | DataOp::ExplodeRows { .. } => "Exploding cells",
            DataOp::ShowExplodeBad { .. } => "Filtering",
            DataOp::MergeColumns { .. } | DataOp::MergeRows { .. } => "Merging",
            DataOp::SortColumns { .. } => "Sorting columns",
            DataOp::Join { .. } => "Joining",
            DataOp::Append { .. } => "Appending",
            DataOp::Clean { .. } | DataOp::CleanMatrix { .. } => "Cleaning",
            DataOp::SplitName { .. } => "Splitting names",
            DataOp::FillExample { .. } => "Filling by example",
            DataOp::Compute { .. } => "Computing",
            // Showing rows changes no data, so a locked file allows it.
            DataOp::ShowProblems { .. }
            | DataOp::ShowContractRows { .. }
            | DataOp::ShowRagged { .. }
            | DataOp::ShowShape { .. }
            | DataOp::ShowGapRows { .. }
            | DataOp::ShowRelationExceptions { .. } => "Filtering",
            DataOp::CompareShow { .. } | DataOp::CompareColumn { .. } => "Comparing",
            DataOp::FormulaFilter { .. } => "Filtering",
            DataOp::SortProblems { .. } | DataOp::CheckColumn { .. } => "Checking",
            DataOp::FillBlanks { .. } => "Filling blanks",
            DataOp::CountRepeats { .. } | DataOp::BlankRepeats { .. } => "Looking for repeats",
            DataOp::Breaks { .. } => "Adding breaks",
            DataOp::DeleteHidden => "Deleting hidden rows",
            DataOp::MapValues { .. } => "Changing values",
            DataOp::GroupCalc { .. } => "Working out group figures",
            DataOp::Arrange { .. } => "Arranging columns",
            DataOp::Extract { .. } => "Extracting",
            DataOp::FillRect { spec } if spec.value.is_empty() => "Clearing",
            DataOp::FillRect { .. } => "Filling",
            DataOp::FillSpecial { value, .. } if value.is_empty() => "Clearing",
            DataOp::FillSpecial { .. } => "Filling",
            DataOp::ShiftCells { .. } => "Moving cells",
        };
        if let DataOp::Clean { spec, now, .. } = &op {
            check_clean(spec, *now)?;
        }
        if let DataOp::CleanMatrix {
            spec,
            now,
            revision,
        } = &op
        {
            check_now(*now)?;
            spec.check()?;
            // Before anything else, so a refusal leaves no checkpoint.
            if revision.is_some_and(|r| self.get(id).is_ok_and(|e| e.read().revision() != r)) {
                return Err(matrix_stale().into());
            }
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
        // The tables "value must exist in" rules read, the same way.
        let keys = match &op {
            DataOp::ShowProblems { spec, .. }
            | DataOp::SortProblems { spec, .. }
            | DataOp::CheckColumn { spec, .. } => self.keys_in_job(id, spec)?,
            _ => Vec::new(),
        };
        // LOOKUP tables of a computed column, copied out the same way.
        let mut read_tabs: Vec<u32> = Vec::new();
        let tables = match &op {
            DataOp::Compute { spec } | DataOp::FormulaFilter { spec } => {
                self.lookup_tables_from(id, spec, Some(&mut read_tabs))?
            }
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
        let last_gaps = match &op {
            DataOp::ShowGapRows { .. } => self
                .get(id)?
                .gaps
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
            _ => None,
        };
        let last_relations = match &op {
            DataOp::ShowRelationExceptions { .. } => self
                .get(id)?
                .relations
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
            _ => None,
        };
        let join_report = std::cell::RefCell::new(None);
        let explode_report = std::cell::RefCell::new(None);
        // Fill blank cells: the cells its stops left blank, to mark.
        let fill_marks = std::cell::RefCell::new(None::<Vec<(u32, u32)>>);
        // Cleaners and fill by example: (left or missed, pattern, examples).
        let extra = std::cell::RefCell::new((0u64, None::<String>, 0u64));
        let fill_rule = std::cell::RefCell::new(None::<FillRule>);
        let errors = std::cell::Cell::new(0u64);
        let groups = std::cell::Cell::new(0u64);
        let other_order = std::cell::Cell::new(0u64);
        let ambiguous = std::cell::Cell::new(0u64);
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
                DataOp::ExplodeColumns { spec } => {
                    let (plan, rep) = d.plan_explode_columns(spec, p)?;
                    *explode_report.borrow_mut() = Some(rep);
                    Planned::Cols(plan)
                }
                DataOp::ExplodeRows { spec } => {
                    let (plan, rep) = d.plan_explode_rows(spec, p)?;
                    *explode_report.borrow_mut() = Some(rep);
                    Planned::Rows(plan)
                }
                DataOp::ShowExplodeBad { spec } => Planned::Rows(d.plan_explode_bad(spec, p)?),
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
                DataOp::Clean { spec, now, typed } => {
                    let mut plan = d.plan_clean(spec, *now, seed, 0, p)?;
                    plan.set_typed(d, typed)?;
                    *extra.borrow_mut() = (plan.left, None, 0);
                    other_order.set(plan.other_order);
                    ambiguous.set(plan.ambiguous_left);
                    Planned::Clean(plan)
                }
                DataOp::CleanMatrix {
                    spec,
                    now,
                    revision,
                } => {
                    // Again under the lock the plan is made with.
                    if revision.is_some_and(|r| d.revision() != r) {
                        return Err(matrix_stale());
                    }
                    let plan = d.plan_clean_matrix(spec, *now, seed, p)?.plan;
                    *extra.borrow_mut() = (plan.left, None, 0);
                    other_order.set(plan.other_order);
                    ambiguous.set(plan.ambiguous_left);
                    Planned::Clean(plan)
                }
                DataOp::SplitName { spec } => Planned::Cols(d.plan_split_name(spec, p)?),
                DataOp::FillExample { spec } => {
                    let (plan, rep) = d.plan_fill_example(spec, p)?;
                    *extra.borrow_mut() = (rep.missed, Some(rep.pattern), rep.examples);
                    *fill_rule.borrow_mut() = Some(FillRule {
                        program: rep.program,
                        example_ids: rep.example_ids,
                    });
                    Planned::Cells(plan)
                }
                DataOp::Compute { spec } => {
                    let mut spec = spec.clone();
                    spec.tables = tables.clone();
                    let (plan, rep) = d.plan_compute(&spec, p)?;
                    errors.set(rep.errors);
                    Planned::Cols(plan)
                }
                DataOp::FormulaFilter { spec } => {
                    let mut spec = spec.clone();
                    spec.tables = tables.clone();
                    let (plan, n) = d.plan_formula_filter(&spec, p)?;
                    errors.set(n);
                    Planned::Rows(plan)
                }
                DataOp::ShowProblems { spec, passing } => {
                    let spec = &CheckSpec {
                        keys: keys.clone(),
                        ..spec.clone()
                    };
                    let rep = d.run_checks(spec, p)?;
                    Planned::Rows(d.plan_show_problems(&rep, spec.keep_first, *passing)?)
                }
                DataOp::ShowShape { spec } => Planned::Rows(d.plan_shape_filter(spec, p)?),
                DataOp::ShowContractRows {
                    contract,
                    what,
                    opts,
                } => {
                    let pairing = if opts.keep_first {
                        natendo_engine::contract::pair(contract, &d.rule_titles(true))
                    } else {
                        natendo_engine::contract::pair_in_order(contract, d.col_count())
                    };
                    Planned::Rows(d.plan_drift_rows(contract, &pairing, *what, opts, p)?)
                }
                DataOp::ShowRelationExceptions { kind, a, b } => {
                    let report = last_relations
                        .as_ref()
                        .filter(|(rev, _)| *rev == d.revision())
                        .map(|(_, r)| r)
                        .filter(|r| {
                            r.findings
                                .iter()
                                .any(|f| (f.kind, f.a, f.b) == (*kind, *a, *b))
                        })
                        .ok_or_else(|| {
                            Error::Invalid("the table changed since the scan: scan again".into())
                        })?;
                    Planned::Rows(d.plan_relation_exceptions(report, *kind, *a, *b, p)?)
                }
                DataOp::ShowGapRows { spec } => {
                    let report = last_gaps
                        .as_ref()
                        .filter(|(rev, r)| *rev == d.revision() && r.spec() == spec)
                        .ok_or_else(|| {
                            Error::Invalid(
                                "the table changed since the audit: run the audit again".into(),
                            )
                        })?;
                    Planned::Rows(d.plan_show_gaps(&report.1)?)
                }
                DataOp::ShowRagged { pick, keep_first } => {
                    let found = d.ragged_rows(*keep_first, p)?;
                    groups.set(found.short);
                    Planned::Rows(d.plan_show_ragged(&found, *keep_first, *pick)?)
                }
                DataOp::SortProblems { spec, scope } => {
                    let spec = &CheckSpec {
                        keys: keys.clone(),
                        ..spec.clone()
                    };
                    let rep = d.run_checks(spec, p)?;
                    groups.set(rep.rows);
                    Planned::Sort(d.plan_sort_problems(&rep, spec.keep_first, *scope)?)
                }
                DataOp::CheckColumn { spec, title } => {
                    let spec = &CheckSpec {
                        keys: keys.clone(),
                        ..spec.clone()
                    };
                    let rep = d.run_checks(spec, p)?;
                    groups.set(rep.rows);
                    Planned::Cols(d.plan_check_column(spec, &rep, title, p)?)
                }
                DataOp::FillBlanks { spec } => {
                    let mut plan = d.plan_fill_blanks(spec, p)?;
                    *fill_marks.borrow_mut() = Some(plan.take_marks());
                    Planned::Rows(plan)
                }
                DataOp::CompareShow { spec } => Planned::Rows(d.plan_compare_show(spec, p)?),
                DataOp::CompareColumn { spec } => Planned::Cols(d.plan_compare_column(spec, p)?),
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
                DataOp::FillRect { spec } => match d.plan_clear_columns(spec)? {
                    Some(c) => Planned::Cols(c),
                    None => Planned::Cells(d.plan_fill_rect(spec, p)?),
                },
                DataOp::ShiftCells { spec } => Planned::Cells(d.plan_shift_cells(spec, p)?),
                DataOp::FillSpecial { spec, value } => {
                    Planned::Cells(d.plan_fill_special(spec, value, p)?)
                }
            };
            let mut planned = planned;
            if let Planned::Rows(c) = &mut planned {
                let (kind, label) = chip_words(&op);
                c.name_chip(kind, label);
            }
            let mut r = report.borrow_mut();
            *r = OpReport::default();
            match &planned {
                Planned::Cells(c) => {
                    r.cells = c.cells;
                    r.skipped = c.skipped;
                    r.hidden_rows = c.hidden_rows;
                    r.order = c.order.clone();
                }
                Planned::Cols(c) => {
                    r.cells = c.cells;
                    r.cols = c.cols;
                }
                Planned::Rows(c) => {
                    r.rows = c.rows;
                    r.groups = c.groups;
                    r.left = c.unreadable;
                    r.skipped = c.skipped;
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
            // Shifting right can widen rows by the block's width.
            DataOp::ShiftCells { .. } => u64::MAX,
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
        if let Some(e) = explode_report.into_inner() {
            r.cols = r.cols.max(e.cols);
            r.explode = Some(e);
        }
        if let Some(marks) = fill_marks.into_inner() {
            r.marked = marks.len() as u64;
            let mut by_id: HashMap<u32, Vec<u32>> = HashMap::new();
            for (id, c) in marks {
                by_id.entry(id).or_default().push(c);
            }
            *self
                .get(id)?
                .fill_marks
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = by_id;
        }
        let (left, pattern, examples) = extra.into_inner();
        r.left = r.left.max(left);
        r.other_order = other_order.get();
        r.ambiguous_left = ambiguous.get();
        r.pattern = pattern;
        r.examples = examples;
        r.fill_rule = fill_rule.into_inner();
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
    /// The column insight strip (a quick sample; no job, so it never
    /// blocks other work).
    pub fn column_insights(
        &self,
        id: u32,
        spec: &natendo_engine::insight::InsightSpec,
    ) -> CmdResult<natendo_engine::insight::Insights> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.column_insights(spec)?)
    }

    pub fn column_profile(
        &self,
        id: u32,
        spec: &natendo_engine::profile::ProfileSpec,
    ) -> CmdResult<natendo_engine::profile::Profile> {
        let entry = self.get(id)?;
        let job = entry.start("Profiling the column")?;
        let d = entry.read();
        Ok(d.column_profile(spec, &job.progress)?)
    }

    /// Group the look-alikes of a column by sound, spelling or both (the
    /// Value editor), as a job with progress and Cancel.
    pub fn lookalike_groups(
        &self,
        id: u32,
        spec: &natendo_engine::editing::LookAlikeSpec,
    ) -> CmdResult<natendo_engine::editing::LookAlikes> {
        let entry = self.get(id)?;
        let job = entry.start("Looking for look-alikes")?;
        let d = entry.read();
        Ok(d.lookalike_groups(spec, &job.progress)?)
    }

    /// Whether a dictionary made from tab `id` holds values from a private
    /// or an encrypted file (it then asks before saving).
    pub fn dictionary_sensitive(&self, id: u32) -> CmdResult<bool> {
        let e = self.get(id)?;
        Ok(e.private.load(Ordering::SeqCst)
            || e.encrypted_from
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_some())
    }

    /// The column's values (with counts) a dictionary does not know: neither
    /// a variant nor a canonical value ("new since last time"); at most
    /// 5,000 listed, all counted.
    pub fn dictionary_unmatched(
        &self,
        id: u32,
        col: u32,
        keep_first: bool,
        dict: &natendo_engine::dictionary::Dictionary,
    ) -> CmdResult<UnmatchedOut> {
        let dict = dict.clone().checked()?.dict;
        let entry = self.get(id)?;
        let job = entry.start("Looking for values not in the dictionary")?;
        let d = entry.read();
        let tally = d.value_tally(
            &TallySpec {
                col,
                keep_first,
                limit: None,
            },
            &job.progress,
        )?;
        let all = dict.unmatched(&tally.values);
        Ok(UnmatchedOut {
            count: all.len() as u64,
            rows: all.iter().map(|t| t.count).sum(),
            values: all.into_iter().take(5_000).collect(),
            complete: tally.complete,
        })
    }

    /// Explode cells: what the column looks like, from a sample (quick).
    pub fn explode_detect(
        &self,
        id: u32,
        col: u32,
        keep_first: bool,
    ) -> CmdResult<natendo_engine::explode::ExplodeDetect> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.explode_detect(col, keep_first)?)
    }

    /// Explode cells: read the whole column (a job with Cancel).
    pub fn explode_scan(
        &self,
        id: u32,
        spec: &natendo_engine::explode::Explode,
    ) -> CmdResult<natendo_engine::explode::ExplodeScan> {
        let entry = self.get(id)?;
        let job = entry.start("Reading the cells")?;
        let d = entry.read();
        Ok(d.explode_scan(spec, &job.progress)?)
    }

    /// Explode cells: before and after for the first 20 shown rows.
    pub fn explode_preview(
        &self,
        id: u32,
        spec: &natendo_engine::explode::Explode,
    ) -> CmdResult<natendo_engine::explode::ExplodePreview> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.explode_preview(spec)?)
    }

    pub fn value_tally(&self, id: u32, spec: &TallySpec) -> CmdResult<ValueTally> {
        let entry = self.get(id)?;
        let job = entry.start("Counting values")?;
        let d = entry.read();
        Ok(d.value_tally(spec, &job.progress)?)
    }

    /// Run a formula typed in a cell (or filled over a block with
    /// Ctrl+Enter): its results are written as one undo step and the formula
    /// is remembered beside them.
    pub fn cell_formula(&self, id: u32, spec: &CellFormula) -> CmdResult<FormulaReport> {
        let mut spec = spec.clone();
        let expr = {
            let entry = self.get(id)?;
            let d = entry.read();
            d.cell_formula_expr(&spec)
        };
        if let Ok(expr) = expr {
            let probe = ComputeSpec {
                expr,
                keep_first: spec.keep_first,
                ..Default::default()
            };
            spec.tables = self.lookup_tables(id, &probe)?;
        }
        // A job: a long fill shows progress and can be cancelled. The results
        // are written only if the table is still as it was read (see
        // `formula_plan`).
        let res = {
            let entry = self.get(id)?;
            let job = entry.start("Running the formula")?;
            let d = entry.read();
            d.run_cell_formula(&spec, &job.progress)?
        };
        let (written, errors, first_error) =
            (res.cells.len() as u64, res.errors, res.first_error.clone());
        let info = if res.cells.is_empty() {
            self.info(id)?
        } else {
            check_write(res.cells.len() as u64, res.cells.iter().map(|c| c.2.len()))?;
            let info = self.edit(id, |d| {
                let plan = d.formula_plan(&res).map_err(|e| match e {
                    Error::Changed => Error::Invalid(
                        "the file changed while the formula ran, so nothing was written; try again"
                            .into(),
                    ),
                    e => e,
                })?;
                d.apply_cells(plan)
            })?;
            let entry = self.get(id)?;
            entry.write().remember_formulas(&res);
            info
        };
        Ok(FormulaReport {
            info,
            written,
            errors,
            first_error,
        })
    }

    /// The formula a cell's value came from (while the value is unchanged).
    pub fn cell_formula_of(&self, id: u32, row: u64, col: u32) -> CmdResult<Option<String>> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.cell_formula(row, col))
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

    /// For each rule of `spec` (by number), the table a "value must exist in" rule reads:
    /// the open tab with that name (or that name without its extension), never two (then the
    /// rule does not run and says to close one), its column by title. Read from the tab's
    /// shown data rows and kept while the tab is unchanged. A rule on this tab's own column
    /// reads this tab (call before taking this tab's lock).
    /// Each "value must exist in" rule's set: kept ones from the cache, others read now with
    /// `build`'s progress (cancelled: nothing is kept). With `build` `None` (screen marks)
    /// nothing is read: a rule without a kept set is left out.
    pub fn key_bindings(
        &self,
        id: u32,
        spec: &CheckSpec,
        build: Option<&Progress>,
    ) -> CmdResult<Vec<KeyBinding>> {
        if !spec
            .checks
            .iter()
            .any(|c| matches!(c.kind, CheckKind::ForeignKey { .. }))
        {
            return Ok(Vec::new());
        }
        let names = self.tab_names();
        let mut out = Vec::with_capacity(spec.checks.len());
        for c in &spec.checks {
            let CheckKind::ForeignKey {
                table,
                column,
                match_case,
                trim,
                ..
            } = &c.kind
            else {
                out.push(KeyBinding::None);
                continue;
            };
            let other = match tab_named(&names, table) {
                Ok(i) => i,
                Err(why) => {
                    out.push(KeyBinding::Missing(why));
                    continue;
                }
            };
            let entry = self.get(other)?;
            let d = entry.read();
            // The other table's first row holds titles; this tab's own column follows its setting.
            let keep_first = if other == id { spec.keep_first } else { true };
            let titles = d.expr_names(keep_first);
            let want_col = column.trim().to_lowercase();
            let Some(col) = titles
                .iter()
                .position(|t| t.trim().to_lowercase() == want_col)
            else {
                out.push(KeyBinding::Missing(format!(
                    "{} has no column {}",
                    table.trim(),
                    column.trim()
                )));
                continue;
            };
            let name = format!(
                "{other}:{}:{col}:{}:{}:{}",
                d.revision(),
                u8::from(keep_first),
                u8::from(*match_case),
                u8::from(*trim)
            );
            let hit = self
                .key_sets
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, s)| s.clone());
            let set = match (hit, build) {
                (Some(s), _) => s,
                (None, None) => {
                    out.push(KeyBinding::None);
                    continue;
                }
                (None, Some(progress)) => {
                    let budget = key_sets_budget();
                    let mut s = KeySet::build(
                        &d,
                        col as u32,
                        keep_first,
                        *match_case,
                        *trim,
                        budget,
                        progress,
                    )?;
                    s.source.doc = other;
                    s.source.table = table.trim().to_string();
                    s.source.column = titles[col].clone();
                    let s = Arc::new(s);
                    let mut cache = self.key_sets.lock().unwrap_or_else(|p| p.into_inner());
                    // Sets of an earlier state of the same tab are no use any more.
                    let now = format!("{other}:{}:", d.revision());
                    let mine = format!("{other}:");
                    cache.retain(|(k, _)| !k.starts_with(&mine) || k.starts_with(&now));
                    cache.push((name, s.clone()));
                    // The oldest go first while there are too many or they hold too much memory.
                    while cache.len() > MAX_KEY_SETS
                        || (cache.len() > 1
                            && cache.iter().map(|(_, s)| s.bytes()).sum::<u64>() > budget)
                    {
                        cache.remove(0);
                    }
                    s
                }
            };
            out.push(KeyBinding::Ready(set));
        }
        Ok(out)
    }

    /// Each open tab's id and name.
    fn tab_names(&self) -> Vec<(u32, String)> {
        self.ids()
            .into_iter()
            .filter_map(|i| self.info(i).ok().map(|n| (i, n.name)))
            .collect()
    }

    /// For each "as many rows as" table check, the shown data rows of the open tab it names
    /// (its first row holds titles; tab `id` itself follows its own setting).
    pub fn table_rows(&self, id: u32, spec: &CheckSpec) -> Vec<Option<Result<u64, String>>> {
        use natendo_engine::tablecheck::TableCheckKind;
        if !spec
            .table_checks
            .iter()
            .any(|c| matches!(c.kind, TableCheckKind::RowCountSameAs { .. }))
        {
            return Vec::new();
        }
        let names = self.tab_names();
        spec.table_checks
            .iter()
            .map(|c| {
                let TableCheckKind::RowCountSameAs { table } = &c.kind else {
                    return None;
                };
                Some(tab_named(&names, table).and_then(|other| {
                    let entry = self.get(other).map_err(|e| e.message.clone())?;
                    let d = entry.read();
                    let keep_first = if other == id { spec.keep_first } else { true };
                    let n = d.row_count();
                    Ok(n - u64::from(keep_first && n > 0))
                }))
            })
            .collect()
    }

    /// `spec` with its "value must exist in" rules' tables and "as many rows as" counts filled in (see [`Docs::key_bindings`]).
    fn with_keys(
        &self,
        id: u32,
        spec: &CheckSpec,
        build: Option<&Progress>,
    ) -> CmdResult<CheckSpec> {
        let mut s = spec.clone();
        s.keys = self.key_bindings(id, spec, build)?;
        s.table_rows = self.table_rows(id, spec);
        Ok(s)
    }

    /// [`Docs::key_bindings`] in a job of its own on tab `id` (progress and Cancel), for
    /// steps whose own job starts later.
    fn keys_in_job(&self, id: u32, spec: &CheckSpec) -> CmdResult<Vec<KeyBinding>> {
        if !spec
            .checks
            .iter()
            .any(|c| matches!(c.kind, CheckKind::ForeignKey { .. }))
        {
            return Ok(Vec::new());
        }
        let entry = self.get(id)?;
        let job = entry.start("Reading the other table")?;
        self.key_bindings(id, spec, Some(&job.progress))
    }

    /// Run data checks over the shown rows (a job: progress and Cancel).
    pub fn run_checks(&self, id: u32, spec: &CheckSpec) -> CmdResult<CheckReport> {
        let key = serde_json::to_string(&spec.checks).unwrap_or_default();
        let entry = self.get(id)?;
        let job = entry.start("Checking")?;
        // Other tables are read inside the job (before this tab's read lock is taken).
        let spec = &self.with_keys(id, spec, Some(&job.progress))?;
        let d = entry.read();
        let r = d.run_checks(spec, &job.progress)?;
        *entry.checks.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((key, d.revision(), r.repeats.clone()));
        Ok(r)
    }

    /// Compare 2 to 5 lists (a job on tab `owner`). The result is kept, for
    /// opening its slices, until [`Docs::sets_close`] or a tab closes.
    pub fn sets_run(&self, owner: u32, tabs: &[u32], spec: &SetsSpec) -> CmdResult<SetsRun> {
        spec.check()?;
        if tabs.len() != spec.lists.len() {
            return Err(CmdError::new("bad_request", "one tab for each list"));
        }
        for i in 0..tabs.len() {
            for j in 0..i {
                if tabs[i] == tabs[j] && spec.lists[i] == spec.lists[j] {
                    return Err(CmdError::new(
                        "bad_request",
                        "the same tab is in twice: give it different key columns",
                    ));
                }
            }
        }
        let owner = self.get(owner)?;
        let job = owner.start("Comparing lists")?;
        let entries = tabs
            .iter()
            .map(|&t| self.get(t))
            .collect::<CmdResult<Vec<_>>>()?;
        // Key parts a list types Id or Text are exact in every list, and
        // each table is as it was then when it is read.
        let mut exact = vec![false; spec.lists[0].cols.len()];
        let mut revisions = Vec::with_capacity(tabs.len());
        for (e, l) in entries.iter().zip(&spec.lists) {
            let d = e.read();
            if l.cols.iter().any(|&c| c >= d.col_count()) {
                return Err(Error::OutOfRange.into());
            }
            for (a, b) in exact.iter_mut().zip(d.exact_key_parts(&l.cols)) {
                *a |= b;
            }
            revisions.push(d.revision());
        }
        let mut builder = SetsBuilder::new(spec, exact.clone())?;
        for (e, &rev) in entries.iter().zip(&revisions) {
            let d = e.read();
            if d.revision() != rev {
                return Err(lists_changed());
            }
            builder.add(&d, &job.progress)?;
        }
        let result = builder.finish(&job.progress)?;
        let names = self.list_names(tabs, spec)?;
        let id = self.next_sets.fetch_add(1, Ordering::SeqCst) + 1;
        let report = result.report.clone();
        let mut kept = self.sets.lock().unwrap_or_else(|p| p.into_inner());
        // A few screens (one a window) at most.
        while kept.len() >= 4 {
            let oldest = *kept.keys().min().expect("not empty");
            kept.remove(&oldest);
            self.sets_dropped
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(oldest);
        }
        kept.insert(
            id,
            SetsCache {
                tabs: tabs.iter().copied().zip(revisions).collect(),
                spec: spec.clone(),
                exact,
                names: names.clone(),
                result: Arc::new(result),
            },
        );
        Ok(SetsRun { id, report, names })
    }

    /// Each list's name: its tab's, with the key columns when two lists
    /// would read the same.
    fn list_names(&self, tabs: &[u32], spec: &SetsSpec) -> CmdResult<Vec<String>> {
        let stems: Vec<String> = tabs.iter().map(|&t| self.stem(t)).collect();
        let mut out = Vec::with_capacity(tabs.len());
        for (i, (&t, l)) in tabs.iter().zip(&spec.lists).enumerate() {
            if stems.iter().filter(|s| **s == stems[i]).count() < 2 {
                out.push(stems[i].clone());
                continue;
            }
            let titles = self.get(t)?.read().titles(&l.cols, l.header).join(" + ");
            out.push(format!("{} ({titles})", stems[i]));
        }
        // Still alike (two files of one name, the same key titles): the
        // list's letter tells them apart.
        let alike = out.clone();
        for (i, name) in out.iter_mut().enumerate() {
            if alike.iter().filter(|n| **n == alike[i]).count() > 1 {
                *name = format!("{name} [list {}]", char::from(b'A' + i as u8));
            }
        }
        Ok(out)
    }

    /// The kept comparison, refused when any of its tables changed (an edit,
    /// a sort, a filter, hidden rows).
    fn sets_kept(&self, id: u64) -> CmdResult<SetsCache> {
        let kept = self
            .sets
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id)
            .cloned();
        let Some(c) = kept else {
            let dropped = self.sets_dropped.lock().unwrap_or_else(|p| p.into_inner());
            return Err(if dropped.contains(&id) {
                CmdError::new(
                    "changed",
                    "this comparison was closed to save memory: run it again",
                )
            } else {
                lists_changed()
            });
        };
        for &(tab, rev) in &c.tabs {
            if !self.get(tab).is_ok_and(|e| e.read().revision() == rev) {
                return Err(lists_changed());
            }
        }
        Ok(c)
    }

    /// A slice in a new tab: list `list`'s rows, or with `keys` the
    /// different keys as that list spells them. Private when any compared
    /// tab is: being in the slice or not comes from every list.
    pub fn sets_open(&self, id: u64, mask: u8, list: usize, keys: bool) -> CmdResult<DocInfo> {
        let c = self.sets_kept(id)?;
        if list >= c.tabs.len() || mask == 0 || mask & (1 << list) == 0 {
            return Err(CmdError::new(
                "bad_request",
                "that list is not in the slice",
            ));
        }
        let (tab, rev) = c.tabs[list];
        let table = {
            let entry = self.get(tab)?;
            let job = entry.start("Opening the slice")?;
            let d = entry.read();
            if d.revision() != rev {
                return Err(lists_changed());
            }
            let masks = &c.result.row_masks[list];
            if keys {
                d.sets_keys_table(&c.spec, &c.exact, list, masks, mask, &job.progress)?
            } else {
                d.sets_slice_table(&c.spec.lists[list], masks, mask, &job.progress)?
            }
        };
        let slice = slice_name(mask, &c.names);
        let rules = c.spec.rules_text();
        let title = if keys {
            format!("Keys in {slice}, as in {} ({rules})", c.names[list])
        } else {
            format!("{slice}, rows of {} ({rules})", c.names[list])
        };
        self.sets_tab(table, &c, title)
    }

    /// The counts as a table in a new tab.
    pub fn sets_table(&self, id: u64) -> CmdResult<DocInfo> {
        let c = self.sets_kept(id)?;
        let table = report_table(&c.result.report, &c.names);
        let title = format!("Compared lists ({})", c.spec.rules_text());
        self.sets_tab(table, &c, title)
    }

    fn sets_tab(&self, table: Table, c: &SetsCache, title: String) -> CmdResult<DocInfo> {
        let (nid, _) = self.insert(table.into_document()?, false)?;
        let from: Vec<u32> = c.tabs.iter().map(|t| t.0).collect();
        self.inherit_private(nid, &from, &[]);
        self.set_title_of(nid, title);
        self.info(nid)
    }

    /// The comparison's screen closed.
    pub fn sets_close(&self, id: u64) {
        self.sets
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id);
    }

    /// What a clean matrix would change, column by column (a job).
    pub fn clean_matrix_preview(
        &self,
        id: u32,
        spec: &CleanMatrix,
        now: f64,
    ) -> CmdResult<MatrixPreview> {
        check_now(now)?;
        let entry = self.get(id)?;
        let job = entry.start("Cleaning preview")?;
        let d = entry.read();
        Ok(d.preview_clean_matrix(spec, now, 1, &job.progress)?)
    }

    /// Look for relationships among columns (a job: progress and Cancel).
    pub fn find_relations(&self, id: u32, spec: &RelationsSpec) -> CmdResult<RelationsReport> {
        let entry = self.get(id)?;
        let job = entry.start("Looking for relationships")?;
        let d = entry.read();
        let r = d.find_relations(spec, &job.progress)?;
        *entry.relations.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((d.revision(), Arc::new(r.clone())));
        Ok(r)
    }

    /// Find the numbers missing from a column (a job: progress and Cancel).
    pub fn gap_audit(&self, id: u32, spec: &GapsSpec) -> CmdResult<Arc<GapReport>> {
        let entry = self.get(id)?;
        let job = entry.start("Looking for gaps")?;
        let d = entry.read();
        let r = Arc::new(d.gap_audit(spec, &job.progress)?);
        *entry.gaps.lock().unwrap_or_else(|p| p.into_inner()) = Some((d.revision(), r.clone()));
        Ok(r)
    }

    /// X-ray the shown rows of `cols` (none: every column) for gremlins: a job with progress and Cancel.
    /// Only counts and row numbers come back, never a value.
    pub fn xray_report(
        &self,
        id: u32,
        cols: &[u32],
        keep_first: bool,
    ) -> CmdResult<natendo_engine::xray::XrayReport> {
        let entry = self.get(id)?;
        let job = entry.start("Looking for look-alike characters")?;
        let d = entry.read();
        Ok(d.xray_report(cols, keep_first, &job.progress)?)
    }

    /// The encodings the picker offers (label, shown name, whether the file can switch to it without reopening).
    pub fn encoding_choices() -> Vec<EncodingChoice> {
        natendo_engine::encscan::picker_encodings()
            .into_iter()
            .filter_map(|(label, name)| {
                Encoding::from_label(label).map(|e| EncodingChoice {
                    label: label.to_string(),
                    name: name.to_string(),
                    direct: e.is_direct(),
                })
            })
            .collect()
    }

    /// The file's bytes on disk are what this document was opened from (nothing else changed them): the
    /// original bytes of a decoded file are gone, so every re-reading of the file starts from this check.
    fn disk_unchanged(&self, id: u32) -> CmdResult<PathBuf> {
        let entry = self.get(id)?;
        let path = entry
            .path
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| CmdError::new("no_path", "this document has no file yet"))?;
        if entry.read().changed_on_disk() {
            return Err(CmdError::new(
                "changed_on_disk",
                "The file changed on disk since it was opened. Reopen it first.",
            ));
        }
        Ok(path)
    }

    /// The first rows of the file read as `label`, laid out as this document is (the scrub preview). A throwaway
    /// read of the head of the file: the document does not change.
    pub fn encoding_preview(
        &self,
        id: u32,
        label: &str,
        rows: usize,
    ) -> CmdResult<Vec<Vec<String>>> {
        use std::io::Read;
        let enc = Encoding::from_label(label)
            .ok_or_else(|| CmdError::new("invalid", "that encoding is not known"))?;
        let path = self.disk_unchanged(id)?;
        let mut head = Vec::new();
        std::fs::File::open(&path)
            .and_then(|f| f.take(PREVIEW_HEAD_BYTES).read_to_end(&mut head))
            .map_err(|e| CmdError::new("io", e.to_string()))?;
        if natendo_engine::source::is_gzip(&head) {
            return Err(CmdError::new(
                "invalid",
                "a compressed file has no encoding of its own to preview: open the unpacked file instead",
            ));
        }
        let entry = self.get(id)?;
        let d = entry.read();
        let shown = d.preview_decoded(&head, enc, rows.min(40))?;
        // A preview never carries more than a screen of text out.
        Ok(shown
            .into_iter()
            .map(|r| {
                r.into_iter()
                    .take(60)
                    .map(|c| c.chars().take(200).collect())
                    .collect()
            })
            .collect())
    }

    /// Read the file's bytes as `label` without reopening (one undo step): only when both the file's reading and
    /// `label` are ASCII-compatible. Refused with unsaved changes, while the Steps panel looks back, on a locked
    /// file, and when the file changed on disk.
    pub fn encoding_apply(&self, id: u32, label: &str) -> CmdResult<DocInfo> {
        let enc = Encoding::from_label(label)
            .ok_or_else(|| CmdError::new("invalid", "that encoding is not known"))?;
        self.disk_unchanged(id)?;
        self.edit(id, |d| d.set_encoding(enc))
    }

    /// Scan the shown rows' raw bytes for rows in another encoding: a job with progress and Cancel. Only counts
    /// and row numbers come back, never a value.
    pub fn scan_encodings(
        &self,
        id: u32,
        keep_first: bool,
    ) -> CmdResult<natendo_engine::encscan::EncodingScan> {
        let entry = self.get(id)?;
        let job = entry.start("Looking for rows in another encoding")?;
        let d = entry.read();
        Ok(d.scan_encodings(keep_first, &job.progress)?)
    }

    /// Show only the rows the scan flagged (a view: nothing in the data changes).
    pub fn encoding_show_rows(&self, id: u32, keep_first: bool) -> CmdResult<DocInfo> {
        let scan = self.scan_encodings(id, keep_first)?;
        self.edit_as(id, false, |d| {
            let mut plan = d.plan_show_encoding_rows(&scan, keep_first)?;
            plan.name_chip("encoding", "Rows with characters that could not be read");
            d.apply_rows(plan)
        })
    }

    /// Show only the rows an X-ray finds gremlins in (a view: nothing in the data changes).
    pub fn xray_show(&self, id: u32, cols: &[u32], keep_first: bool) -> CmdResult<DocInfo> {
        let report = self.xray_report(id, cols, keep_first)?;
        self.edit_as(id, false, |d| {
            let mut plan = d.plan_show_xray(&report, keep_first)?;
            plan.name_chip("xray", "Rows X-ray picked");
            d.apply_rows(plan)
        })
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
        // Drawn often: only sets already kept are used, never read here.
        let spec = &self.with_keys(id, spec, None)?;
        let entry = self.get(id)?;
        let d = entry.read();
        let last = entry.checks.lock().unwrap_or_else(|p| p.into_inner());
        let repeats: &[(u32, Vec<u64>)] = match last.as_ref() {
            Some((k, rev, reps)) if *k == key && *rev == d.revision() => reps,
            _ => &[],
        };
        Ok(d.check_marks(row, rows, col, cols, spec, repeats)?)
    }

    /// The next (or previous) problem cell after a cell, in the order the table is shown
    /// (a job: progress and Cancel). Columns in `skip_cols` (hidden ones) are passed over.
    /// Unique rules are known from the last run while the data has not changed since.
    #[allow(clippy::too_many_arguments)]
    pub fn next_problem(
        &self,
        id: u32,
        row: u64,
        col: u32,
        backward: bool,
        spec: &CheckSpec,
        skip_cols: &[u32],
    ) -> CmdResult<natendo_engine::validate::JumpResult> {
        let key = serde_json::to_string(&spec.checks).unwrap_or_default();
        let entry = self.get(id)?;
        let job = entry.start("Looking for the next problem")?;
        let spec = &self.with_keys(id, spec, Some(&job.progress))?;
        let d = entry.read();
        let repeats: Vec<(u32, Vec<u64>)> = {
            let last = entry.checks.lock().unwrap_or_else(|p| p.into_inner());
            match last.as_ref() {
                Some((k, rev, reps)) if *k == key && *rev == d.revision() => reps.clone(),
                _ => Vec::new(),
            }
        };
        Ok(d.next_problem(row, col, backward, spec, &repeats, skip_cols, &job.progress)?)
    }

    /// Rules a column's own values suggest (a job: progress and Cancel).
    pub fn suggest_rules(
        &self,
        id: u32,
        col: u32,
        keep_first: bool,
    ) -> CmdResult<Vec<natendo_engine::lintsuggest::Suggestion>> {
        let entry = self.get(id)?;
        let job = entry.start("Suggesting rules")?;
        let d = entry.read();
        Ok(d.suggest_rules(col, keep_first, &job.progress)?)
    }

    /// The rules file beside tab `id`'s file, if there is one (`None` when not): its rules that
    /// fit this table, how many did not, and the file's name. Reading it runs nothing.
    pub fn checks_sidecar_load(&self, id: u32) -> CmdResult<Option<SidecarRules>> {
        use natendo_engine::checkfile::{checks_path, ChecksFile};
        let Some(path) = self.path_of(id)? else {
            return Ok(None);
        };
        let side = checks_path(&path);
        let Some(file) = ChecksFile::load(&side)? else {
            return Ok(None);
        };
        let entry = self.get(id)?;
        let loaded = file.resolve(&entry.read());
        Ok(Some(SidecarRules {
            checks: loaded.checks,
            table_checks: loaded.table_checks,
            skipped: loaded.skipped,
            keep_first: loaded.keep_first,
            file: side
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }))
    }

    /// Keep tab `id`'s rules in a file beside its file (only when asked; no rules removes the
    /// file). Refused for a private tab (private file or folder), an encrypted file, a file not
    /// saved yet, and a folder that cannot be written. Returns the file's name.
    pub fn checks_sidecar_keep(
        &self,
        id: u32,
        checks: Vec<natendo_engine::validate::Check>,
        table_checks: Vec<natendo_engine::tablecheck::TableCheck>,
        keep_first: bool,
    ) -> CmdResult<String> {
        use natendo_engine::checkfile::{checks_path, ChecksFile};
        if self.is_private(id)? {
            return Err(CmdError::new(
                "invalid",
                "rules are not kept beside a file in private mode",
            ));
        }
        if self.encrypted_from(id)?.is_some() {
            return Err(CmdError::new(
                "invalid",
                "rules are not kept beside an encrypted file",
            ));
        }
        let path = self.path_of(id)?.ok_or_else(|| {
            CmdError::new("invalid", "save the file first: rules are kept beside it")
        })?;
        if self.is_private_path(&path) {
            return Err(CmdError::new(
                "invalid",
                "rules are not kept beside a file in private mode",
            ));
        }
        let side = checks_path(&path);
        let entry = self.get(id)?;
        let file = ChecksFile::for_checks(&entry.read(), &checks, &table_checks, keep_first);
        file.save(&side).map_err(|e| {
            CmdError::new(
                "invalid",
                format!(
                    "the rules could not be saved beside the file (is the folder read-only?): {e}"
                ),
            )
        })?;
        Ok(side
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default())
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
        let mut sample_record: Option<Vec<u8>> = None;
        let mut sample_seed: Option<u64> = None;
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
                TableOp::Sample { spec } => {
                    let (t, r) = d.sample_report(spec, &job.progress)?;
                    // The file's fingerprint, when the tab holds it unchanged.
                    let print = match d.path() {
                        Some(_) if d.changed_on_disk() => {
                            "(the file changed on disk since it was opened; open it again for a fingerprint)"
                                .into()
                        }
                        Some(p) if !d.is_modified() => {
                            natendo_engine::tools::file_sha256(p, &job.progress)?
                        }
                        Some(_) => {
                            "(the tab has changes not saved; save it first for a fingerprint)"
                                .into()
                        }
                        None => "(not saved to a file)".into(),
                    };
                    let source = d
                        .path()
                        .map_or_else(|| self.stem(id), |p| p.display().to_string());
                    let column = spec.per.map(|c| {
                        d.expr_names(spec.keep_first)
                            .get(c as usize)
                            .cloned()
                            .unwrap_or_default()
                    });
                    sample_record = Some(natendo_engine::tools::sample_record(
                        &r,
                        &source,
                        column.as_deref(),
                        &print,
                    ));
                    sample_seed = Some(r.seed);
                    t
                }
                TableOp::Counts { spec } => d.counts_table(spec, &job.progress)?.0,
                TableOp::Gaps { spec } => {
                    let names = d.expr_names(spec.keep_first);
                    let titles: Vec<String> = spec
                        .series
                        .iter()
                        .map(|&c| names.get(c as usize).cloned().unwrap_or_default())
                        .collect();
                    d.gap_audit(spec, &job.progress)?.table(&titles)?
                }
                TableOp::Compare { .. } => unreachable!("handled above"),
            }
        };
        let doc = table.into_document()?;
        let (nid, _) = self.insert(doc, false)?;
        let record = match sample_record {
            Some(bytes) => {
                let rd = Document::from_bytes(bytes, OpenOptions::default(), Some("csv"))?;
                let (rid, _) = self.insert(rd, false)?;
                self.inherit_private(rid, &[id], &[]);
                self.set_title_of(rid, format!("Sample record ({})", self.stem(id)));
                Some(self.info(rid)?)
            }
            None => None,
        };
        // A sample, comparison or pivot of a private tab is private too.
        let mut from = vec![id];
        if let TableOp::Compare { other, .. } = &op {
            from.push(*other);
        }
        self.inherit_private(nid, &from, &[]);
        let stem = self.stem(id);
        let title = match &op {
            TableOp::Pivot { .. } => format!("Pivot ({stem})"),
            TableOp::Unpivot { .. } => format!("Unpivot ({stem})"),
            TableOp::Subtotals { .. } => format!("Subtotals ({stem})"),
            TableOp::Reshape { .. } => format!("Reshaped ({stem})"),
            TableOp::Stack { .. } => format!("Stacked ({stem})"),
            TableOp::Sample { .. } => match sample_seed {
                Some(seed) => format!("Sample ({stem}, seed {seed})"),
                None => format!("Sample ({stem})"),
            },
            TableOp::Counts { .. } => format!("Counts ({stem})"),
            TableOp::Gaps { spec } => {
                // The column's title, never a value.
                let column = self
                    .get(id)?
                    .read()
                    .expr_names(spec.keep_first)
                    .get(spec.col as usize)
                    .map(|t| t.chars().take(60).collect::<String>())
                    .unwrap_or_default();
                format!("Gaps in {column} ({stem})")
            }
            TableOp::Compare { other, .. } => format!("Compare {stem} with {}", self.stem(*other)),
        };
        self.set_title_of(nid, title);
        Ok(TableReport {
            info: self.info(nid)?,
            compare,
            record,
        })
    }

    /// Start a pivot job: a pivot already running on this tab (an older
    /// preview) is cancelled first; anything else makes this "busy".
    fn start_pivot<'a>(&self, entry: &'a Entry) -> CmdResult<JobGuard<'a>> {
        self.start_replacing(entry, PIVOT_JOB)
    }

    /// Start job `label`, cancelling an older one of the same label first
    /// (a newer pivot preview, or filter-panel count, replaces it).
    fn start_replacing<'a>(
        &self,
        entry: &'a Entry,
        label: &'static str,
    ) -> CmdResult<JobGuard<'a>> {
        for _ in 0..200 {
            match entry.start(label) {
                Ok(j) => return Ok(j),
                Err(e) => {
                    let ours = entry.jobs().as_ref().is_some_and(|j| j.label == label);
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
        Err(CmdError::new("busy", "still working; try again"))
    }

    /// The filter panel's counts (an older count still running is
    /// cancelled: each click asks again).
    pub fn facet_counts(
        &self,
        id: u32,
        cards: &[natendo_engine::filter::FacetCard],
        spec: &FilterSpec,
    ) -> CmdResult<natendo_engine::filter::Facets> {
        let entry = self.get(id)?;
        let job = self.start_replacing(&entry, FACET_JOB)?;
        let d = entry.read();
        let mut cache = entry.facets.lock().unwrap_or_else(|p| p.into_inner());
        Ok(d.facet_counts_cached(&mut cache, cards, spec, &job.progress)?)
    }

    /// Free what the filter panel worked out for a tab (the panel closed,
    /// or its last card went). A count still running stops first.
    pub fn facet_forget(&self, id: u32) -> CmdResult<()> {
        let entry = self.get(id)?;
        if let Some(j) = entry.jobs().as_ref() {
            if j.label == FACET_JOB {
                j.progress.cancel.store(true, Ordering::Relaxed);
            }
        }
        entry
            .facets
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        Ok(())
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
        let titles = Self::pivot_titles(&entry, spec.keep_first);
        let names = |fields: &[natendo_engine::pivot::PivotField]| -> String {
            fields
                .iter()
                .map(|f| {
                    titles
                        .get(f.col as usize)
                        .cloned()
                        .unwrap_or_else(|| format!("column {}", f.col + 1))
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        let (rows, cols) = (names(&spec.rows), names(&spec.cols));
        let what = match (rows.is_empty(), cols.is_empty()) {
            (false, false) => format!(" – {rows} by {cols}"),
            (false, true) => format!(" – {rows}"),
            (true, false) => format!(" – by {cols}"),
            (true, true) => String::new(),
        };
        self.set_title_of(nid, format!("Pivot{what} ({})", self.stem(id)));
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
        // Named after the fields, never their values: a value can be a
        // person's name, and the tab name becomes the suggested file name.
        let titles = Self::pivot_titles(&entry, spec.keep_first);
        let fields: Vec<String> = spec
            .rows
            .iter()
            .take(row_path.len())
            .chain(spec.cols.iter().take(col_path.len()))
            .map(|f| {
                titles
                    .get(f.col as usize)
                    .cloned()
                    .unwrap_or_else(|| format!("column {}", f.col + 1))
            })
            .collect();
        let what = if fields.is_empty() {
            "the grand total".to_string()
        } else {
            format!("one {}", fields.join(" and "))
        };
        self.set_title_of(nid, format!("Rows for {what} ({})", self.stem(id)));
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
                d.write_part(&part.ids, spec.keep_first, spec.guard, &mut std::io::sink())?;
            }
        }
        job.progress.start(parts.len() as u64);
        let mut rows = 0u64;
        let mut guarded = 0u64;
        for (n, (part, path)) in parts.iter().zip(&paths).enumerate() {
            let done = crate::extras::write_atomic_with(path, |f| {
                let mut w = std::io::BufWriter::with_capacity(1 << 20, f);
                guarded += d
                    .write_part(&part.ids, spec.keep_first, spec.guard, &mut w)?
                    .1;
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
            guarded,
        })
    }

    /// Combine with a column map, step 1: read each picked file's head (its first rows only), and the automatic
    /// column map (today's Combine) with suggestions. The paths stay here, under a token.
    pub fn combine_start(&self, paths: Vec<PathBuf>, spec: &Combine) -> CmdResult<CombineStart> {
        use natendo_engine::combine::{auto_plan, head_of, suggest};
        if paths.is_empty() || paths.len() > MAX_COMBINE {
            return Err(CmdError::new(
                "bad_request",
                format!("combine 1 to {MAX_COMBINE} files"),
            ));
        }
        if !spec.header || !spec.by_title {
            return Err(CmdError::new(
                "bad_request",
                "the column map is for files with titles, matched by title",
            ));
        }
        combine_size_check(&paths)?;
        let mut heads = Vec::with_capacity(paths.len());
        let mut stamps = Vec::with_capacity(paths.len());
        for p in &paths {
            stamps.push(file_stamp(p)?);
            heads.push(head_of(p, true).map_err(|e| {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                CmdError::new("invalid", format!("{name}: {e}"))
            })?);
        }
        let rows: Vec<Vec<String>> = heads.iter().map(|h| h.titles.clone()).collect();
        let plan = auto_plan(&rows, true);
        let suggestions = suggest(&plan, &heads);
        static TOKENS: AtomicU64 = AtomicU64::new(1);
        let token = TOKENS.fetch_add(1, Ordering::Relaxed);
        let start = CombineStart {
            token,
            files: heads.clone(),
            plan,
            suggestions,
        };
        *self.combine.lock().unwrap_or_else(|p| p.into_inner()) = Some(CombineSession {
            token,
            paths,
            stamps,
            heads,
            spec: spec.clone(),
        });
        Ok(start)
    }

    fn combine_session<T>(
        &self,
        token: u64,
        f: impl FnOnce(&CombineSession) -> CmdResult<T>,
    ) -> CmdResult<T> {
        let s = self.combine.lock().unwrap_or_else(|p| p.into_inner());
        match s.as_ref() {
            Some(s) if s.token == token => f(s),
            _ => Err(CmdError::new(
                "invalid",
                "those files are no longer picked: choose them again",
            )),
        }
    }

    /// Check a column map changed by hand (empty columns taken out) and suggest merges for it.
    pub fn combine_check(&self, token: u64, mut plan: CombinePlan) -> CmdResult<CombineChecked> {
        self.combine_session(token, |s| {
            plan.compact();
            let widths: Vec<usize> = s.heads.iter().map(|h| h.cols as usize).collect();
            plan.check(&widths)?;
            let suggestions = natendo_engine::combine::suggest(&plan, &s.heads);
            Ok(CombineChecked { plan, suggestions })
        })
    }

    /// Read the picked files' heads again (after a file changed), keeping the token.
    pub fn combine_refresh(&self, token: u64) -> CmdResult<CombineStart> {
        let (paths, spec) =
            self.combine_session(token, |s| Ok((s.paths.clone(), s.spec.clone())))?;
        let mut start = self.combine_start(paths, &spec)?;
        // Same picked files, same token.
        if let Some(s) = self
            .combine
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
        {
            s.token = token;
        }
        start.token = token;
        Ok(start)
    }

    /// A saved column map of `plan` (titles only, never paths, names or values).
    pub fn combine_mapping(
        &self,
        token: u64,
        plan: &CombinePlan,
        extras: natendo_engine::combine::Extras,
    ) -> CmdResult<Vec<u8>> {
        self.combine_session(token, |s| {
            let widths: Vec<usize> = s.heads.iter().map(|h| h.cols as usize).collect();
            plan.check(&widths)?;
            let rows: Vec<Vec<String>> = s.heads.iter().map(|h| h.titles.clone()).collect();
            Ok(
                natendo_engine::combine::SavedMapping::of_plan(plan, &rows, extras, s.spec.source)
                    .to_json(),
            )
        })
    }

    /// Fit a saved column map to the picked files.
    pub fn combine_apply_mapping(&self, token: u64, bytes: &[u8]) -> CmdResult<CombineFit> {
        let m = natendo_engine::combine::SavedMapping::from_json(bytes)?;
        self.combine_session(token, |s| {
            let rows: Vec<Vec<String>> = s.heads.iter().map(|h| h.titles.clone()).collect();
            let mut fit = m.apply(&rows);
            fit.plan.compact();
            let suggestions = natendo_engine::combine::suggest(&fit.plan, &s.heads);
            Ok(CombineFit {
                fit,
                suggestions,
                file_name_column: m.file_name_column,
                extras_mode: m.extras,
            })
        })
    }

    pub fn combine_cancel(&self, token: u64) {
        let mut s = self.combine.lock().unwrap_or_else(|p| p.into_inner());
        if s.as_ref().is_some_and(|s| s.token == token) {
            *s = None;
        }
    }

    /// Combine with a column map, last step: open each file fully, check it is still what its head showed (M1:
    /// the same file, the same titles, no wider row), and stack the rows as `plan` says into a new tab. A file
    /// that changed gives `combine_stale` and nothing is combined: look at the map again.
    pub fn combine_run(&self, token: u64, plan: CombinePlan, source: bool) -> CmdResult<DocInfo> {
        let (paths, stamps, heads, mut spec) = self.combine_session(token, |s| {
            Ok((
                s.paths.clone(),
                s.stamps.clone(),
                s.heads.clone(),
                s.spec.clone(),
            ))
        })?;
        spec.source = source;
        let widths: Vec<usize> = heads.iter().map(|h| h.cols as usize).collect();
        plan.check(&widths)?;
        let stale = |m: String| CmdError::new("combine_stale", m);
        combine_size_check(&paths)?;
        let mut docs = Vec::with_capacity(paths.len());
        for ((p, stamp), head) in paths.iter().zip(&stamps).zip(&heads) {
            if file_stamp(p)? != *stamp {
                return Err(stale(format!(
                    "{} changed after it was picked: look at the column map again",
                    head.name
                )));
            }
            let mut d = Document::open(p, OpenOptions::default())?;
            while !d.index_step(64 << 20)? {}
            natendo_engine::combine::still_fits(head, &d, true, &Progress::new())
                .map_err(|e| stale(e.to_string()))?;
            if file_stamp(p)? != *stamp {
                return Err(stale(format!(
                    "{} changed while it was read: look at the column map again",
                    head.name
                )));
            }
            docs.push((head.name.clone(), d));
        }
        let refs: Vec<(String, &Document)> = docs.iter().map(|(n, d)| (n.clone(), d)).collect();
        let table =
            natendo_engine::tools::combine_with_plan(&refs, &plan, &spec, &Progress::new())?;
        drop(refs);
        drop(docs);
        let (nid, _) = self.insert(table.into_document()?, false)?;
        self.set_title_of(nid, format!("Combined ({} files)", paths.len()));
        let ps: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
        self.inherit_private(nid, &[], &ps);
        self.combine_cancel(token);
        self.info(nid)
    }

    /// Stack several files (picked in a native dialog) into a new tab.
    pub fn combine_files(&self, paths: &[PathBuf], spec: &Combine) -> CmdResult<DocInfo> {
        if paths.is_empty() || paths.len() > MAX_COMBINE {
            return Err(CmdError::new(
                "bad_request",
                format!("combine 1 to {MAX_COMBINE} files"),
            ));
        }
        combine_size_check(paths)?;
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
        // Only the tables are combined: lines above or below them (report
        // notes) are not.
        let with_notes = docs.iter().filter(|(_, d)| d.has_report_notes()).count();
        let refs: Vec<(String, &Document)> = docs.iter().map(|(n, d)| (n.clone(), d)).collect();
        let table = natendo_engine::tools::combine(&refs, spec, &Progress::new())?;
        drop(refs);
        drop(docs);
        let (nid, _) = self.insert(table.into_document()?, false)?;
        self.set_title_of(nid, format!("Combined ({} files)", paths.len()));
        let paths: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
        self.inherit_private(nid, &[], &paths);
        let mut info = self.info(nid)?;
        if with_notes > 0 {
            info.run_notes.push(format!(
                "Report notes of {} are not included (the lines above their tables).",
                if with_notes == 1 {
                    "1 file".to_string()
                } else {
                    format!("{with_notes} files")
                }
            ));
        }
        Ok(info)
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

    /// Compare two columns: counts, and the rows where they differ.
    pub fn compare_cols(
        &self,
        id: u32,
        spec: &natendo_engine::colcompare::CompareCols,
    ) -> CmdResult<natendo_engine::colcompare::CompareColsReport> {
        let entry = self.get(id)?;
        let job = entry.start("Comparing")?;
        let d = entry.read();
        Ok(d.compare_cols(spec, &job.progress)?)
    }

    /// Go To Special: the cells of an area that are blank, numbers, text or
    /// differ from another column of their row.
    pub fn special_cells(
        &self,
        id: u32,
        mut spec: natendo_engine::special::SpecialSpec,
    ) -> CmdResult<natendo_engine::special::SpecialCells> {
        let entry = self.get(id)?;
        let job = entry.start("Looking")?;
        if let natendo_engine::special::SpecialKind::Problems { checks } = &mut spec.kind {
            checks.keys = self.key_bindings(id, checks, Some(&job.progress))?;
        }
        let d = entry.read();
        // Failing cells: the repeats of the last check run, when it ran the
        // same checks on this version of the table.
        if let natendo_engine::special::SpecialKind::Problems { checks } = &spec.kind {
            let key = serde_json::to_string(&checks.checks).unwrap_or_default();
            if let Some((k, rev, reps)) = entry
                .checks
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
            {
                if *k == key && *rev == d.revision() {
                    spec.repeats = reps.clone();
                }
            }
        }
        Ok(d.special_cells(&spec, &job.progress)?)
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

    /// A page of what a cleaner would change, with exact counts over the whole selection (a read job: the
    /// Clean-up dialog pages through every changed cell before Apply).
    pub fn clean_preview_page(
        &self,
        id: u32,
        spec: &CleanSpec,
        now: f64,
        offset: u64,
        want: usize,
    ) -> CmdResult<CleanPreview> {
        check_clean(spec, now)?;
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.preview_clean_page(
            spec,
            now,
            run_seed(),
            offset,
            want.min(500),
            CLEAN_PAGE_ROWS,
            true,
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
        // Parts in files of their own: none may land on a file open here.
        if opts.format == ExportFormat::Xlsx && opts.split_files {
            let n = d.xlsx_parts(opts).len();
            if n > 1 {
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                for i in 1..=n {
                    self.check_writable(&path.with_file_name(format!("{stem} ({i} of {n}).xlsx")))?;
                }
            }
        }
        Ok(d.export_file(path, opts, &job.progress)?)
    }

    /// Before parts go to files of their own beside `path`: the part names
    /// already there (they would be replaced), and parts of an earlier
    /// export in another number of parts ("voters (3 of 12).xlsx").
    pub fn xlsx_part_clash(
        &self,
        id: u32,
        path: &Path,
        opts: &ExportOptions,
    ) -> CmdResult<(Vec<String>, Vec<String>)> {
        if opts.format != ExportFormat::Xlsx || !opts.split_files {
            return Ok((Vec::new(), Vec::new()));
        }
        let n = self.get(id)?.read().xlsx_parts(opts).len();
        if n < 2 {
            return Ok((Vec::new(), Vec::new()));
        }
        Ok(part_clash(path, n))
    }

    /// Several tabs as the sheets of one Excel workbook (each tab named
    /// after it; a tab longer than a sheet goes on over more sheets).
    /// `widths`: each tab's on-screen column widths, in the order of `ids`.
    pub fn export_book(
        &self,
        ids: &[u32],
        path: &Path,
        opts: &ExportOptions,
        widths: &[Vec<u32>],
    ) -> CmdResult<ExportReport> {
        check_export(opts)?;
        self.check_writable(path)?;
        // The same tab twice goes in once.
        let mut seen = std::collections::HashSet::new();
        let order: Vec<usize> = (0..ids.len()).filter(|&k| seen.insert(ids[k])).collect();
        let widths: Vec<Vec<u32>> = order
            .iter()
            .map(|&k| widths.get(k).cloned().unwrap_or_default())
            .collect();
        let ids: Vec<u32> = order.iter().map(|&k| ids[k]).collect();
        let ids = ids.as_slice();
        if ids.is_empty() || ids.len() > 256 {
            return Err(CmdError::new("invalid", "choose 1 to 256 tabs"));
        }
        let entries: Vec<_> = ids.iter().map(|&i| self.get(i)).collect::<CmdResult<_>>()?;
        let job = entries[0].start("Exporting")?;
        let names: Vec<String> = ids
            .iter()
            .map(|&i| {
                self.info(i).map(|x| {
                    x.name
                        .rsplit_once('.')
                        .map_or(x.name.as_str(), |(b, _)| b)
                        .to_string()
                })
            })
            .collect::<CmdResult<_>>()?;
        let docs: Vec<_> = entries.iter().map(|e| e.read()).collect();
        let tab_opts: Vec<ExportOptions> = widths
            .into_iter()
            .map(|w| ExportOptions {
                widths: w,
                ..opts.clone()
            })
            .collect();
        let mut sheets = Vec::new();
        for ((d, name), opts) in docs.iter().zip(&names).zip(&tab_opts) {
            let parts = d.xlsx_parts(opts);
            let n = parts.len();
            for (k, rows) in parts.into_iter().enumerate() {
                sheets.push(natendo_engine::xlsxbook::BookSheet {
                    doc: d,
                    opts,
                    name: natendo_engine::xlsxbook::part_sheet_name(name, k + 1, n),
                    rows,
                });
            }
        }
        Ok(natendo_engine::export::write_atomic(path, |w| {
            natendo_engine::xlsxbook::write_book(w, &sheets, &job.progress)
        })?)
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

    /// The shown row for a row number of the whole file (Go To while a
    /// filter is on), and whether that exact row is shown.
    /// For each document position, its shown row (`None`: hidden), so custom
    /// row heights draw on the rows a filter shows. At most `MAX_ROWS_CHANGE`.
    pub fn shown_rows_at(&self, id: u32, places: &[u64]) -> CmdResult<Vec<Option<u64>>> {
        if places.len() as u64 > MAX_ROWS_CHANGE {
            return Err(CmdError::new("too_large", "too many rows at once"));
        }
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.shown_rows_at(places))
    }

    /// Document positions of shown rows `first..first + count` (at most
    /// `MAX_ROWS_CHANGE` + 1 of them, so a caller can tell it passed the cap).
    pub fn doc_rows(&self, id: u32, first: u64, count: u64) -> CmdResult<Vec<u64>> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.doc_rows(first, count.min(MAX_ROWS_CHANGE + 1)))
    }

    pub fn shown_row_of(&self, id: u32, number: u64) -> CmdResult<Option<(u64, bool)>> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.shown_row_of(number))
    }

    /// The block of data around a cell (Ctrl+A's first press).
    pub fn current_region(
        &self,
        id: u32,
        row: u64,
        col: u32,
        first: u64,
    ) -> CmdResult<Option<(u64, u64, u32, u32)>> {
        let entry = self.get(id)?;
        let d = entry.read();
        Ok(d.current_region(row, col, first)?)
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
        // Any step (a filter too) would clear the redo list, where the
        // steps after the one looked at are kept.
        if self.get(id)?.peeking.load(Ordering::Relaxed) {
            return Err(peeking());
        }
        self.edit_any(id, changes, f)
    }

    /// As [`Docs::edit_as`], also while the steps panel looks back (for
    /// its own moves).
    fn edit_any(
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
        // A filter-panel count still running gives way to the edit (the
        // panel counts again after it).
        if let Some(j) = entry.jobs().as_ref() {
            if j.label == FACET_JOB {
                j.progress.cancel.store(true, Ordering::Relaxed);
            }
        }
        let title = entry
            .title
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let mut d = entry.write();
        f(&mut d)?;
        // The same reading as a question gets (a tab with no file keeps its name, a JSON tab its file);
        // every fresh reading is kept, so the stale answer of a later question is never older than this edit.
        Ok(Self::info_from(&entry, id, &d, error, title))
    }

    /// Say what column `col` (as shown) holds. Not an edit of the data: allowed on a read-only file, one
    /// undo step on the look side (see the tab's look steps), and nothing in the saved bytes changes.
    pub fn set_column_type(
        &self,
        id: u32,
        col: u32,
        ty: natendo_engine::coltype::ColumnType,
    ) -> CmdResult<DocInfo> {
        // The new set of types, worked out first so a recipe records the state it makes.
        let types = {
            let entry = self.get(id)?;
            let d = entry.read();
            let Some(phys) = (col < d.col_count()).then(|| d.phys_of(col)).flatten() else {
                return Err(Error::OutOfRange.into());
            };
            d.type_overlay().with(phys, ty)
        };
        self.set_column_types(id, types)
    }

    /// Put all the column types back as they were (Undo, a saved view, a session).
    pub fn set_column_types(
        &self,
        id: u32,
        types: natendo_engine::coltype::ColumnTypes,
    ) -> CmdResult<DocInfo> {
        // A recipe records the whole set by column title (a watched folder or the command line opens a fresh
        // file with every column Auto, so the recipe has to say what the columns hold).
        let step = {
            let entry = self.get(id)?;
            let d = entry.read();
            types_step(&d, &types)
        };
        self.recorded(id, Logged::Step(step), || {
            self.edit_any(id, false, |d| {
                d.set_column_types(types);
                Ok(())
            })
        })
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
        self.info_now(id)
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
    same_file::is_same_file(a, b).unwrap_or(false) || path_key(a) == path_key(b)
}

/// Which of `have` (a path and its `path_key`) is the file `want`: compared
/// as text first; looked at on disk (same file under another name: a mapped
/// drive and its network share) only when the file names match.
fn match_path(have: &[(PathBuf, String)], want: &Path) -> Option<usize> {
    let k = path_key(want);
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_lowercase());
    have.iter().position(|(_, hk)| *hk == k).or_else(|| {
        have.iter().position(|(p, _)| {
            name(p) == name(want) && same_file::is_same_file(p, want).unwrap_or(false)
        })
    })
}

/// A path as text for matching: resolved where it exists, without the
/// `\\?\` form, and (on Windows) in lower case with `\`.
pub(crate) fn path_key(p: &Path) -> String {
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
    norm(p)
}

fn peeking() -> CmdError {
    CmdError::new(
        "peeking",
        "looking at an earlier recipe step: go back to the latest step to change or save",
    )
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
/// The filter chip a "show these rows" tool makes (C8): its kind and words.
fn chip_words(op: &DataOp) -> (&'static str, &'static str) {
    match op {
        DataOp::ShowDuplicates { .. } => ("duplicates", "Duplicates"),
        DataOp::ShowExplodeBad { .. } => ("explode", "Cells that could not be split"),
        DataOp::FormulaFilter { .. } => ("formula", "Formula"),
        DataOp::ShowProblems { passing: true, .. } => ("problems", "Rows without problems"),
        DataOp::ShowProblems { .. } => ("problems", "Rows with problems"),
        DataOp::ShowShape { .. } => ("shape", "Pattern"),
        DataOp::ShowRelationExceptions { .. } => ("relations", "Relation exceptions"),
        DataOp::ShowGapRows { .. } => ("gaps", "Gaps"),
        DataOp::ShowRagged { .. } => ("ragged", "Ragged rows"),
        DataOp::ShowContractRows { .. } => ("contract", "Rows a contract found"),
        DataOp::CompareShow { .. } => ("compare", "Compared rows"),
        _ => ("rows", "Rows a tool picked"),
    }
}

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

/// Part files of `n` already beside `path`, and parts of another count
/// ("stem (k of m).xlsx", m not n) left by an earlier export.
fn part_clash(path: &Path, n: usize) -> (Vec<String>, Vec<String>) {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    let same: Vec<String> = (1..=n)
        .map(|i| format!("{stem} ({i} of {n}).xlsx"))
        .filter(|f| dir.join(f).exists())
        .collect();
    let mut other = Vec::new();
    let head = format!("{stem} (").to_lowercase();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten().take(100_000) {
            let f = e.file_name().to_string_lossy().to_string();
            let low = f.to_lowercase();
            let Some(rest) = low
                .strip_prefix(&head)
                .and_then(|r| r.strip_suffix(").xlsx"))
            else {
                continue;
            };
            let Some((k, m)) = rest.split_once(" of ") else {
                continue;
            };
            if let (Ok(k), Ok(m)) = (k.parse::<usize>(), m.parse::<usize>()) {
                if k >= 1 && k <= m && m != n {
                    other.push(f);
                }
            }
        }
    }
    other.sort();
    (same, other)
}

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
/// "1 line", "4 lines".
fn lines_word(n: u32) -> String {
    if n == 1 {
        "1 line".into()
    } else {
        format!("{} lines", group(u64::from(n)))
    }
}

pub(crate) fn group(n: u64) -> String {
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
        data_revision: d.data_revision(),
        filter_on_deleted: d.filter_on_deleted_column(),
        cols: d.col_count(),
        indexed: d.is_indexed(),
        progress: d.index_progress(),
        modified: d.is_modified(),
        can_undo: d.can_undo(),
        can_redo: d.can_redo(),
        history_pos: d.history_pos(),
        step_mark: d.step_mark(),
        column_types: d.type_overlay().clone(),
        typed: {
            let ctx = d.read_ctx(Default::default());
            (0..d
                .col_count()
                .min(natendo_engine::coltype::MAX_TYPED_COLS as u32))
                .filter_map(|c| {
                    let t = ctx.type_of(c);
                    (t != natendo_engine::coltype::ColumnType::Auto).then_some((c, t))
                })
                .collect()
        },
        encoding: f.encoding.name(),
        bom: f.bom,
        delimiter: f.dialect.delimiter_str().to_string(),
        quote: f.dialect.quote().map(byte_label),
        escape: f.dialect.escape().map(byte_label),
        merge: f.dialect.merges(),
        preamble_lines: d.preamble_lines(),
        layout_confidence: d.detection().map(|x| x.confidence),
        read_only: false,
        peeking: false,
        stale: false,
        seq: INFO_SEQ.fetch_add(1, Ordering::SeqCst) + 1,
        hidden_rows: d.hidden_rows(),
        protected: d.protected_cols(),
        protected_skipped: d.protected_skipped(),
        has_filter: d.has_filter(),
        line_break: line_break_name(f.line_break),
        mixed_line_breaks: stats.mixed_line_breaks(),
        lossy: d.is_lossy(),
        invalid_bytes: d.has_invalid_bytes(),
        run_notes: Vec::new(),
        saved_history: None,
        usual_fields: d.ragged_at_open(false).map_or(0, |r| r.0),
        ragged_rows: d.ragged_at_open(false).map_or(0, |r| r.1),
        first_row_ragged: d.first_row_ragged(),
        line_by_line,
        column_changes: d.has_column_changes(),
        repair_count: stats.repair_count,
        unclosed_quotes: flags & scan::UNCLOSED_QUOTE != 0 || d.index().line_bound_from().is_some(),
        long_quoted_cells: flags & scan::LONG_QUOTED_CELL != 0,
        line_bound_from: d.index().line_bound_from(),
        notes: d.note_counts(),
        error,
        json_format: None,
        json_refuse: None,
        json_file: None,
        rows_change: None,
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

/// A tab name as typed: no line breaks or control characters, at most
/// 120 characters, trimmed.
pub fn clean_title(t: &str) -> String {
    let one: String = t
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(120)
        .collect();
    one.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A tab name as a file name: characters Windows refuses become "-".
pub fn file_name_of(t: &str) -> String {
    let s: String = t
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let s = s.trim_end_matches(['.', ' ']).trim().to_string();
    if s.is_empty() {
        return "Untitled".into();
    }
    // Names Windows keeps for devices (CON, NUL, COM1 ...), with or without
    // an extension, cannot be file names.
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
        format!("{s}_file")
    } else {
        s
    }
}

#[path = "docs_golden.rs"]
pub mod golden;

#[path = "docs_patch.rs"]
pub mod patching;

#[path = "docs_contracts.rs"]
pub mod contracts;

#[path = "docs_xfer.rs"]
pub mod xfer;

use natendo_engine::combine::{CombinePlan, FileHead, Suggestion};

/// Picked files for Combine with a column map.
struct CombineSession {
    token: u64,
    paths: Vec<PathBuf>,
    /// Each file's size and time at the pick (M1).
    stamps: Vec<(u64, Option<std::time::SystemTime>)>,
    heads: Vec<FileHead>,
    spec: Combine,
}

/// What the mapper opens with.
#[derive(Debug, Clone, Serialize)]
pub struct CombineStart {
    pub token: u64,
    /// Names, titles and first rows (never paths).
    pub files: Vec<FileHead>,
    pub plan: CombinePlan,
    pub suggestions: Vec<Suggestion>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CombineChecked {
    pub plan: CombinePlan,
    pub suggestions: Vec<Suggestion>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CombineFit {
    #[serde(flatten)]
    pub fit: natendo_engine::combine::Fit,
    pub suggestions: Vec<Suggestion>,
    pub file_name_column: bool,
    /// What the map does with columns it does not name.
    pub extras_mode: natendo_engine::combine::Extras,
}

fn file_stamp(p: &Path) -> CmdResult<(u64, Option<std::time::SystemTime>)> {
    let m = std::fs::metadata(p).map_err(natendo_engine::Error::from)?;
    Ok((m.len(), m.modified().ok()))
}

/// The result cannot be larger than a new table may be, so files that together
/// are larger are refused before any is read (review M1 of item 1).
fn combine_size_check(paths: &[PathBuf]) -> CmdResult<()> {
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str, body: &[u8]) -> PathBuf {
        // Tests run in parallel: two tests writing one temp file break each
        // other now and then. Each name belongs to the first test (thread)
        // that uses it.
        static OWNERS: std::sync::Mutex<Option<HashMap<String, String>>> =
            std::sync::Mutex::new(None);
        let me = std::thread::current().name().unwrap_or("").to_string();
        let mut owners = OWNERS.lock().unwrap_or_else(|p| p.into_inner());
        let owner = owners
            .get_or_insert_with(HashMap::new)
            .entry(name.to_string())
            .or_insert_with(|| me.clone());
        assert_eq!(
            *owner, me,
            "temp file {name} is used by two tests; give one another name"
        );
        drop(owners);
        let dir = crate::testdir::path(&format!("nde-docs-{name}"));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn combine_with_a_column_map_checks_the_files_again_before_combining() {
        let a = tmp("cmap-a-Zqx.csv", b"Cust_ID,Name\r\n1,Ann\r\n");
        let b = tmp("cmap-b-Zqx.csv", b"Name,CustID\r\nBo,2\r\n");
        let docs = Docs::default();
        let spec = Combine {
            header: true,
            by_title: true,
            source: false,
        };
        let start = docs
            .combine_start(vec![a.clone(), b.clone()], &spec)
            .unwrap();
        // The window gets names and heads, never a folder.
        let json = serde_json::to_string(&start).unwrap();
        let folder = a.parent().unwrap().to_string_lossy().replace('\\', "\\\\");
        assert!(!json.contains(&folder), "{json}");
        assert_eq!(start.plan.titles, ["Cust_ID", "Name", "CustID"]);
        assert_eq!(start.suggestions.len(), 1);
        let mut plan = start.plan.clone();
        plan.merge(0, 2).unwrap();
        let checked = docs.combine_check(start.token, plan.clone()).unwrap();
        assert!(checked.suggestions.is_empty());
        // A plan putting two columns of one file together is refused.
        let mut bad = plan.clone();
        bad.files[1] = vec![Some(0), Some(0)];
        assert!(docs.combine_check(start.token, bad).is_err());
        // The saved map holds titles only.
        let saved = docs
            .combine_mapping(start.token, &plan, natendo_engine::combine::Extras::Add)
            .unwrap();
        let text = String::from_utf8(saved.clone()).unwrap();
        assert!(!text.contains("Zqx") && !text.contains("Ann"), "{text}");
        let fit = docs.combine_apply_mapping(start.token, &saved).unwrap();
        assert_eq!(fit.fit.plan, plan);
        // A file changed after the pick: refused, nothing combined, then fine after a refresh.
        std::fs::write(&b, b"Name,CustID,Extra\r\nBo,2,x\r\nCy,3,y\r\n").unwrap();
        let e = docs
            .combine_run(start.token, plan.clone(), true)
            .unwrap_err();
        assert_eq!(e.kind, "combine_stale", "{}", e.message);
        assert!(e.message.contains("cmap-b-Zqx.csv"), "{}", e.message);
        let again = docs.combine_refresh(start.token).unwrap();
        assert_eq!(again.token, start.token);
        let mut plan = again.plan.clone();
        let ci = plan.titles.iter().position(|t| t == "CustID").unwrap() as u32;
        plan.merge(0, ci).unwrap();
        let info = docs.combine_run(start.token, plan, true).unwrap();
        wait(&docs, info.id);
        let rows: Vec<Vec<String>> = (0..4)
            .map(|r| docs.get(info.id).unwrap().read().row_values(r).unwrap())
            .collect();
        assert_eq!(rows[0], ["Source file", "Cust_ID", "Name", "Extra"]);
        assert_eq!(rows[2], ["cmap-b-Zqx.csv", "2", "Bo", "x"]);
        // The picked files are let go after combining.
        assert!(docs.combine_check(start.token, again.plan).is_err());
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
            guard: false,
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

    /// Renaming writes the table's first row even when hidden rows leave
    /// another row at the top, and undo puts the old name back.
    #[test]
    fn rename_writes_the_real_first_row() {
        let p = tmp("rename.csv", b"name,age\r\nAda,30\r\nBo,40\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        docs.hide_rows(id, 0, 1).unwrap();
        assert_eq!(docs.cell_text(id, 0, 0).unwrap().0, "Ada");
        assert_eq!(docs.first_row_cell(id, 0).unwrap(), "name");
        assert_eq!(docs.first_row_names(id, 10, 2).unwrap(), vec!["na", "ag"]);
        docs.set_first_row_cell(id, 0, "given".into()).unwrap();
        assert_eq!(docs.first_row_cell(id, 0).unwrap(), "given");
        assert_eq!(docs.cell_text(id, 0, 0).unwrap().0, "Ada");
        // A new column past the last one.
        docs.set_first_row_cell(id, 2, "city".into()).unwrap();
        assert_eq!(docs.first_row_cell(id, 2).unwrap(), "city");
        docs.undo(id).unwrap();
        docs.undo(id).unwrap();
        assert_eq!(docs.first_row_cell(id, 0).unwrap(), "name");
    }

    /// A fill follows a saved custom order only when it should, and says which.
    /// Gap #4: while filtered, an insert or delete answers with the document
    /// positions it changed, and the two read-only lookups map shown rows and
    /// document rows both ways (capped).
    #[test]
    fn filtered_row_changes_report_their_document_positions() {
        let docs = Docs::default();
        let p = tmp(
            "heights-filtered.csv",
            b"n,k
r1,y
r2,x
r3,x
r4,y
r5,x
r6,y
r7,x
",
        );
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        // Unfiltered: the positions asked for.
        let ins = docs.insert_rows(id, 2, 1).unwrap();
        assert_eq!(
            ins.rows_change,
            Some(RowsChange::Insert { at: 2, count: 1 })
        );
        docs.undo(id).unwrap();
        let spec: FilterSpec = serde_json::from_str(
            r#"{"columns":[{"col":1,"rule":{"type":"values","values":["x"],"blanks":false}}],"keep_first":true}"#,
        )
        .unwrap();
        docs.filter(id, spec).unwrap();
        // Shown: 0 (titles), 2, 3, 5, 7.
        assert_eq!(docs.doc_rows(id, 0, 10).unwrap(), [0, 2, 3, 5, 7]);
        assert_eq!(
            docs.shown_rows_at(id, &[2, 4, 7]).unwrap(),
            [Some(1), None, Some(4)]
        );
        assert!(docs
            .shown_rows_at(id, &vec![0; MAX_ROWS_CHANGE as usize + 1])
            .is_err());
        let ins = docs.insert_rows(id, 3, 2).unwrap();
        assert_eq!(
            ins.rows_change,
            Some(RowsChange::Insert { at: 5, count: 2 })
        );
        docs.undo(id).unwrap();
        let del = docs.delete_rows(id, 2, 2).unwrap();
        assert_eq!(
            del.rows_change,
            Some(RowsChange::Delete {
                runs: vec![(3, 1), (5, 1)]
            })
        );
        // Other answers carry no report.
        assert_eq!(docs.undo(id).unwrap().rows_change, None);
        // It reaches the window as JSON with a "kind".
        let json = serde_json::to_value(&del).unwrap();
        assert_eq!(json["rows_change"]["kind"], "delete");
        assert!(serde_json::to_value(&ins).unwrap()["rows_change"]["at"] == 5);
        // A move: shown rows 2..4 (document rows 3 and 5, row 4 hidden between)
        // to the end, after the last shown row (7).
        let mv = docs.move_rows(id, 2, 2, 3, Some(5)).unwrap();
        assert_eq!(
            mv.rows_change,
            Some(RowsChange::Move {
                runs: vec![(3, 1), (5, 1)],
                to: 6
            })
        );
        assert_eq!(docs.doc_rows(id, 0, 10).unwrap(), [0, 2, 5, 6, 7]);
        assert_eq!(
            serde_json::to_value(&mv).unwrap()["rows_change"]["kind"],
            "move"
        );
        // Made from an older picture of the shown rows: refused, nothing moves.
        let e = docs.move_rows(id, 1, 1, 2, Some(9)).unwrap_err();
        assert!(e.message.contains("rows changed"), "{}", e.message);
        assert_eq!(docs.doc_rows(id, 0, 10).unwrap(), [0, 2, 5, 6, 7]);
    }

    #[test]
    fn a_fill_takes_only_the_saved_orders_it_uses() {
        use natendo_engine::fill::{FillMode, FillOrder};
        let p = tmp(
            "fill-orders.csv",
            b"level,who\r\nLow,Smith\r\nx,y\r\nx,y\r\n",
        );
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let o = |name: &str, items: &[&str], one_cell: bool| FillOrder {
            name: name.into(),
            items: items.iter().map(|s| s.to_string()).collect(),
            one_cell,
        };
        docs.set_fill_orders(vec![
            o("Names", &["Smith", "Jones", "Brown"], false),
            o("Priority", &["Low", "Medium", "High"], true),
        ]);
        let fill = |col: u32| DataOp::Fill {
            spec: FillSpec {
                row: 1,
                col,
                rows: 1,
                cols: 1,
                dir: natendo_engine::ops::Dir::Down,
                count: 2,
                mode: FillMode::Auto,
                saved_orders: true,
                ..Default::default()
            },
        };
        let r = docs.data_op(id, fill(0)).unwrap();
        assert_eq!(r.order.as_deref(), Some("Priority"));
        assert_eq!(docs.cell_text(id, 3, 0).unwrap().0, "High");
        // One name on an order not ticked for one cell: copied.
        let r = docs.data_op(id, fill(1)).unwrap();
        assert_eq!(r.order, None);
        assert_eq!(docs.cell_text(id, 3, 1).unwrap().0, "Smith");
    }

    /// A rename is a recipe step: a recipe recorded across it replays the
    /// rename on a fresh copy of the file.
    #[test]
    fn rename_is_a_recipe_step() {
        let p = tmp("rename-recipe.csv", b"name,age\r\nAda,30\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        docs.set_first_row_cell(id, 1, "years".into()).unwrap();
        let recipe = docs.recipe_of(id, true, "r").unwrap();
        assert!(matches!(
            recipe.steps.as_slice(),
            [RecipeOp::RenameColumn { col: 1, name }] if name == "years"
        ));
        let q = tmp("rename-recipe-2.csv", b"name,age\r\nBo,40\r\n");
        let id2 = docs.open_path(&q, false).unwrap().id;
        wait(&docs, id2);
        docs.run_recipe(id2, &recipe, &[], 0.0, true).unwrap();
        assert_eq!(docs.first_row_cell(id2, 1).unwrap(), "years");
        // An undone rename is not part of the recipe.
        docs.undo(id).unwrap();
        assert!(docs.recipe_of(id, true, "r").unwrap().steps.is_empty());
    }

    /// The steps panel: look at the table after a step, then change a step
    /// and the steps after it run again; a value can become a question.
    #[test]
    fn steps_panel_peeks_and_replays() {
        let p = tmp(
            "steps.csv",
            b"name,city,junk\r\nbo,reno,x\r\nann,oslo,y\r\n",
        );
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        docs.delete_cols(id, 2, 1).unwrap();
        docs.set_first_row_cell(id, 1, "town".into()).unwrap();
        docs.insert_cols(id, 0, 1).unwrap();
        let v = docs.steps_view(id).unwrap();
        assert_eq!((v.steps.len(), v.at), (3, 3));
        assert!(!v.hand_changes);
        // Peek after step 1, then back to the latest.
        docs.steps_goto(id, Some(0)).unwrap();
        assert_eq!(docs.first_row_cell(id, 1).unwrap(), "city");
        assert_eq!(docs.steps_view(id).unwrap().at, 1);
        docs.steps_goto(id, Some(2)).unwrap();
        assert_eq!(docs.first_row_cell(id, 2).unwrap(), "town");
        // While looking back, changes and saving are refused and the later
        // steps stay.
        let info = docs.steps_goto(id, Some(0)).unwrap();
        assert!(info.peeking);
        assert_eq!(docs.delete_cols(id, 0, 1).unwrap_err().kind, "peeking");
        assert_eq!(
            docs.set_cells(id, vec![(1, 0, "x".into())])
                .unwrap_err()
                .kind,
            "peeking"
        );
        assert_eq!(docs.undo(id).unwrap_err().kind, "peeking");
        assert_eq!(
            docs.save(id, None, true, true, None).unwrap_err().kind,
            "peeking"
        );
        assert!(matches!(docs.autosave(id).unwrap(), Autosaved::Skipped));
        let info = docs.steps_goto(id, Some(3)).unwrap();
        assert!(!info.peeking);
        assert_eq!(docs.first_row_cell(id, 0).unwrap(), "");
        assert_eq!(docs.steps_view(id).unwrap().steps.len(), 3);
        // A change whose steps fail to run again leaves everything as it
        // was: here the new settings name a column that is not there.
        let e = docs
            .steps_change(
                id,
                1,
                StepChange::Replace {
                    op: RecipeOp::DeleteCols { at: 40, count: 1 },
                },
                0.0,
                false,
            )
            .unwrap_err();
        assert!(e.message.ends_with("nothing was changed"), "{}", e.message);
        assert_eq!(docs.first_row_cell(id, 2).unwrap(), "town");
        let v = docs.steps_view(id).unwrap();
        assert_eq!((v.steps.len(), v.at), (3, 3));
        assert!(v.steps.iter().all(|s| s.kind == "step"));
        // Turn step 1 off: the junk column comes back, the later steps
        // run again.
        docs.steps_change(id, 0, StepChange::Toggle, 0.0, false)
            .unwrap();
        assert_eq!(docs.first_row_cell(id, 3).unwrap(), "junk");
        assert_eq!(docs.first_row_cell(id, 2).unwrap(), "town");
        let v = docs.steps_view(id).unwrap();
        assert_eq!(
            v.steps.iter().map(|s| s.kind).collect::<Vec<_>>(),
            ["off", "step", "step"]
        );
        assert_eq!(docs.recipe_of(id, true, "").unwrap().steps.len(), 2);
        // Back on, then remove the inserted column.
        docs.steps_change(id, 0, StepChange::Toggle, 0.0, false)
            .unwrap();
        docs.steps_change(id, 2, StepChange::Remove, 0.0, false)
            .unwrap();
        assert_eq!(docs.first_row_cell(id, 0).unwrap(), "name");
        assert_eq!(docs.steps_view(id).unwrap().steps.len(), 2);
        // New settings for the rename.
        docs.steps_change(
            id,
            1,
            StepChange::Replace {
                op: RecipeOp::RenameColumn {
                    col: 1,
                    name: "place".into(),
                },
            },
            0.0,
            false,
        )
        .unwrap();
        assert_eq!(docs.first_row_cell(id, 1).unwrap(), "place");
        // A change by hand after the last step is not lost without asking.
        docs.set_cells(id, vec![(1, 0, "Bo".into())]).unwrap();
        let e = docs
            .steps_change(id, 0, StepChange::MoveDown, 0.0, false)
            .unwrap_err();
        assert_eq!(e.kind, "steps_lost");
        // The rename's name becomes a question.
        let v = docs
            .steps_ask(id, 1, "name", "column", "Name the column?")
            .unwrap();
        assert_eq!(v.questions[0].default, "place");
        let r = docs.recipe_of(id, true, "").unwrap();
        assert_eq!(r.questions.len(), 1);
        assert!(matches!(&r.steps[1], RecipeOp::RenameColumn { name, .. } if name == "{{column}}"));
        let q = tmp("steps-2.csv", b"name,city,junk\r\ncy,rome,z\r\n");
        let id2 = docs.open_path(&q, false).unwrap().id;
        wait(&docs, id2);
        docs.run_recipe(id2, &r, &[("column".into(), "where".into())], 0.0, false)
            .unwrap();
        assert_eq!(docs.first_row_cell(id2, 1).unwrap(), "where");
    }

    /// Clearing a whole column (any number of rows) and shifting cells are
    /// data operations of their own, one undo step each.
    #[test]
    fn fill_rect_and_shift_cells() {
        use natendo_engine::shift::{RectFill, Shift, ShiftCells};
        let p = tmp("fill-rect.csv", b"a,b\r\n1,2\r\n3,4\r\n5,6\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let cell = |r: u64, c: u32| docs.cell_text(id, r, c).unwrap().0;
        let r = docs
            .data_op(
                id,
                DataOp::FillRect {
                    spec: RectFill {
                        row: 1,
                        rows: 3,
                        col: 1,
                        cols: 1,
                        value: String::new(),
                        skip_cols: Vec::new(),
                    },
                },
            )
            .unwrap();
        assert_eq!(r.cells, 3);
        assert_eq!((cell(0, 1), cell(3, 1)), ("b".into(), String::new()));
        docs.undo(id).unwrap();
        assert_eq!(cell(3, 1), "6");
        docs.data_op(
            id,
            DataOp::ShiftCells {
                spec: ShiftCells {
                    row: 1,
                    rows: 1,
                    col: 0,
                    cols: 1,
                    shift: Shift::Up,
                },
            },
        )
        .unwrap();
        assert_eq!((cell(1, 0), cell(3, 0)), ("3".into(), String::new()));
        docs.undo(id).unwrap();
        assert_eq!(cell(1, 0), "1");
        // Neither goes into a recipe.
        assert!(docs.recipe_of(id, true, "r").unwrap().steps.is_empty());
    }

    /// An undone step is not kept in the recipe when a filter comes next
    /// (filters are undo steps).
    #[test]
    fn undone_step_then_filter_leaves_the_recipe() {
        use natendo_engine::filter::{ColumnFilter, Condition, TextOp};
        let p = tmp("undone-filter.csv", b"name,age\r\nAda,30\r\nBo,40\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        docs.set_first_row_cell(id, 1, "years".into()).unwrap();
        docs.undo(id).unwrap();
        let spec = FilterSpec {
            combine: Default::default(),
            invert: false,
            columns: vec![ColumnFilter {
                enabled: true,
                negate: false,
                name: None,
                col: 0,
                rule: Rule::Conditions {
                    first: Condition::Text {
                        op: TextOp::NotEquals,
                        text: "Bo".into(),
                        match_case: false,
                    },
                    second: None,
                    or: false,
                },
            }],
            keep_first: true,
            read: Default::default(),
        };
        docs.filter(id, spec).unwrap();
        let recipe = docs.recipe_of(id, true, "r").unwrap();
        assert!(
            matches!(recipe.steps.as_slice(), [RecipeOp::Filter { .. }]),
            "{:?}",
            recipe.steps
        );
        // Undoing the filter leaves nothing; redo brings it back.
        docs.undo(id).unwrap();
        assert!(docs.recipe_of(id, true, "r").unwrap().steps.is_empty());
        docs.redo(id).unwrap();
        assert_eq!(docs.recipe_of(id, true, "r").unwrap().steps.len(), 1);
    }

    /// A tab busy with a long operation (its lock held) still counts as
    /// showing its file, so the file is never opened a second time.
    #[test]
    fn busy_document_still_counts_as_open() {
        let p = tmp("busy-open.csv", b"a,b\r\n1,2\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let entry = docs.get(id).unwrap();
        let _held = entry.write();
        assert_eq!(docs.find_open(&p), Some(id));
        let other = tmp("busy-other.csv", b"a\r\n");
        assert_eq!(
            docs.find_open_many(&[other, p.clone()]),
            vec![None, Some(id)]
        );
        assert_eq!(docs.paths_of(&[id]), vec![Some(p.clone())]);
    }

    /// What the start-up offer leaves out: a file open already, and a file
    /// Windows passed at start written another way; not other files.
    #[test]
    fn open_or_launching_marks() {
        let a = tmp("launch-open.csv", b"a,b\r\n1,2\r\n");
        let b = tmp("launch-passed.csv", b"a\r\n1\r\n");
        let c = tmp("launch-neither.csv", b"a\r\n1\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&a, false).unwrap().id;
        wait(&docs, id);
        let dir = b.parent().unwrap();
        let _ = std::fs::create_dir_all(dir.join("x"));
        // The launch file written another way: through `..`, and (Windows)
        // in other letter case with `/`.
        let mut other = dir.join("x").join("..").join(b.file_name().unwrap());
        if cfg!(windows) {
            other = PathBuf::from(other.to_string_lossy().to_uppercase().replace('\\', "/"));
        }
        let paths = [a.clone(), b.clone(), c.clone()];
        assert_eq!(
            docs.open_or_launching(&[other], &paths),
            vec![true, true, false]
        );
        assert_eq!(
            docs.open_or_launching(&[], &paths),
            vec![true, false, false]
        );
    }

    /// Copy column names and Paste as column names.
    #[test]
    fn column_names_copy_and_paste() {
        let p = tmp(
            "names-copy.csv",
            b"id,\"last, first\",city\r\n1,\"Stone, Ada\",Dayton\r\n",
        );
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        assert_eq!(
            docs.column_names_line(id, &[0, 1, 2], '\t').unwrap(),
            "id\tlast, first\tcity"
        );
        assert_eq!(docs.column_names_line(id, &[2, 0], ',').unwrap(), "city,id");
        assert_eq!(
            docs.column_names_line(id, &[1], ',').unwrap(),
            "\"last, first\""
        );
        // Past the last column: an empty name.
        assert_eq!(docs.column_names_line(id, &[0, 9], ';').unwrap(), "id;");
        let info = docs
            .set_column_names(id, vec![(0, "key".into()), (2, "town".into())])
            .unwrap();
        assert!(info.can_undo);
        assert_eq!(
            docs.column_names_line(id, &[0, 1, 2], ',').unwrap(),
            "key,\"last, first\",town"
        );
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
                name: None,
                col: Some(0),
                kind: CheckKind::Unique {
                    match_case: false,
                    also: vec![],
                },
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
                    scope: Default::default(),
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
                    typed: Vec::new(),
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
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        assert_eq!((r.cells, r.examples), (1, 1));
        assert!(r.pattern.is_some());
        assert_eq!(cell(1, 1), "AL");
        // The rule card: the fill's rule, its odd results counted after, and the card's calls.
        let rule = r.fill_rule.clone().expect("the rule it used");
        let ex = ExampleSpec {
            col: 1,
            skip_first: true,
            ..Default::default()
        };
        let o = docs
            .example_outliers(id, &ex, &rule.program, Some(rule.example_ids.clone()))
            .unwrap();
        assert_eq!((o.fills, o.odd_rows), (1, 0));
        let learned = docs.example_learn(id, &ex).unwrap();
        assert_eq!(learned.program.as_ref(), Some(&rule.program));
        assert!(docs
            .example_preview(id, &ex, &rule.program)
            .unwrap()
            .rows
            .is_empty());
        assert!(docs
            .example_formula(id, &ex, &rule.program, Default::default())
            .is_ok());
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

    /// A1a: the Open-a-table command on a database with a NUMERIC REAL 1.50, a TEXT 00123 and a 19-digit
    /// INTEGER: text and integers come through exactly, the REAL is what the database holds (1.5), and the
    /// note names the column (never a value).
    #[test]
    fn open_a_table_keeps_text_and_integers_exact_and_notes_decimal_columns() {
        let db = crate::testdir::path("nd-a1a-open.sqlite");
        let _ = std::fs::remove_file(&db);
        {
            let c = rusqlite::Connection::open(&db).unwrap();
            c.execute_batch(
                "CREATE TABLE t(price NUMERIC, zip TEXT, big INTEGER);
                 INSERT INTO t VALUES(1.50, '00123', 1234567890123456789);",
            )
            .unwrap();
        }
        let docs = Docs::default();
        let (info, note) = docs.open_sqlite_report(&db, "t").unwrap();
        let cell = |r: u64, c: u32| docs.cell_text(info.id, r, c).unwrap().0;
        assert_eq!(cell(1, 0), "1.5");
        assert_eq!(cell(1, 1), "00123");
        assert_eq!(cell(1, 2), "1234567890123456789");
        assert!(
            note.contains("\u{201c}price\u{201d}")
                && note.contains("trailing zeros were already lost"),
            "{note}"
        );
        assert!(
            !note.contains("1.5")
                && !note.contains("00123")
                && !note.contains("1234567890123456789")
        );
        let _ = std::fs::remove_file(&db);
    }

    /// A1b review: a recipe recorded on a file with NO title row keeps no cell value (the first row is data).
    #[test]
    fn a_recipe_of_a_headerless_file_holds_no_cell_value() {
        let p = tmp(
            "a1b_noheader.csv",
            b"Ann Lee,00123
Bob Ray,00456
",
        );
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        docs.set_column_type(id, 1, natendo_engine::coltype::ColumnType::Id)
            .unwrap();
        let r = docs.recipe_of(id, false, "types").unwrap();
        let json = String::from_utf8(r.to_json().unwrap()).unwrap();
        assert!(
            json.contains("column_types")
                || json.contains("ColumnTypes")
                || json.contains("\"type\""),
            "{json}"
        );
        for v in ["Ann", "Lee", "00123", "Bob", "00456"] {
            assert!(
                !json.contains(v),
                "the recipe holds a cell value ({v}): {json}"
            );
        }
        // With a title row the title is kept (it is the column's name, not data).
        let p2 = tmp(
            "a1b_header.csv",
            b"name,zip
Ann Lee,00123
",
        );
        let id2 = docs.open_path(&p2, false).unwrap().id;
        wait(&docs, id2);
        docs.set_column_type(id2, 1, natendo_engine::coltype::ColumnType::Id)
            .unwrap();
        let json2 =
            String::from_utf8(docs.recipe_of(id2, true, "t").unwrap().to_json().unwrap()).unwrap();
        assert!(
            json2.contains("\"zip\"") && !json2.contains("Ann"),
            "{json2}"
        );
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

    /// U3/B4: a note (by key), a checkpoint and "Show changed rows" survive sort, save and undo; "Show changed rows"
    /// made before a later edit stays stale across a save.
    #[test]
    fn notes_checkpoints_and_changed_rows_survive_sort_save_undo() {
        let p = tmp(
            "u3.csv",
            b"id,name,town
1,Zed,velmont
2,Amy,tarrow
3,Bo,velmont
4,Cy,kessel
",
        );
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let sort = SortSpec {
            keys: vec![natendo_engine::sort::SortKey {
                col: 1,
                descending: false,
                kind: natendo_engine::sort::SortKind::Text,
                match_case: false,
            }],
            keep_first: true,
            ..Default::default()
        };
        docs.sort(id, sort).unwrap();
        // Amy, Bo, Cy, Zed. A note on Bo, a checkpoint, then Replace all velmont -> Velmont (Bo and Zed).
        docs.note_set(id, 2, 3, "moved").unwrap();
        let cps = docs.checkpoint_add(id, "sorted").unwrap();
        let cp = cps[0].id;
        let spec = FindSpec {
            pattern: Pattern {
                text: "velmont".into(),
                match_case: true,
                ..Default::default()
            },
            scope: natendo_engine::find::Scope::All,
            skip_first: true,
            skip_cols: Vec::new(),
            ..Default::default()
        };
        docs.replace_preview(id, spec.clone(), "Velmont".into())
            .unwrap();
        docs.replace_confirm(id, spec, "Velmont".into()).unwrap();
        docs.save(id, None, false, false, None).unwrap();
        let names = |docs: &Docs| -> Vec<String> {
            let n = docs.info(id).unwrap().rows;
            (1..n)
                .map(|r| docs.cell_text(id, r, 1).unwrap().0)
                .collect()
        };
        // Show changed rows after the save: the same people.
        docs.show_changed(id, true).unwrap();
        assert_eq!(names(&docs), ["Bo", "Zed"]);
        docs.undo(id).unwrap();
        // The note is still on Bo, the checkpoint still reachable and the replace undoes across the save.
        let marks = docs.note_marks(id, 0, 10).unwrap();
        assert_eq!(marks.len(), 1);
        assert_eq!(docs.cell_text(id, marks[0].row, 1).unwrap().0, "Bo");
        assert!(docs.checkpoint_list(id).unwrap()[0].reachable);
        docs.undo(id).unwrap();
        assert_eq!(docs.cell_text(id, 2, 2).unwrap().0, "velmont");
        docs.undo(id).unwrap();
        assert_eq!(
            names(&docs),
            ["Zed", "Amy", "Bo", "Cy"],
            "the sort undoes across the save"
        );
        docs.checkpoint_restore(id, cp).unwrap();
        assert_eq!(
            names(&docs),
            ["Amy", "Bo", "Cy", "Zed"],
            "back to the checkpoint"
        );
        // Replace all, then an edit, then a save: the changed rows no longer describe the table.
        let spec = FindSpec {
            pattern: Pattern {
                text: "kessel".into(),
                ..Default::default()
            },
            scope: natendo_engine::find::Scope::All,
            skip_first: true,
            skip_cols: Vec::new(),
            ..Default::default()
        };
        docs.replace_preview(id, spec.clone(), "K".into()).unwrap();
        docs.replace_confirm(id, spec, "K".into()).unwrap();
        docs.set_cells(id, vec![(1, 2, "edited".into())]).unwrap();
        docs.save(id, None, false, false, None).unwrap();
        assert_eq!(docs.show_changed(id, true).unwrap_err().kind, "changed");
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
            cells_skip_hidden: false,
            removed_skip_hidden: false,
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

    /// Holds the document's write lock (as a running change does) for `ms`, then lets go.
    fn hold_write(docs: &Docs, id: u32, ms: u64) -> std::thread::JoinHandle<()> {
        let entry = docs.get(id).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let h = std::thread::spawn(move || {
            let _w = entry.doc.write().unwrap();
            tx.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(ms));
        });
        rx.recv().unwrap();
        h
    }

    #[test]
    fn an_operations_reply_is_fresh_even_while_another_thread_waits_to_write() {
        let a = tmp(
            "lock_c.csv",
            b"a,b
1,2
",
        );
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        let before = wait(&docs, ia);
        let entry = docs.get(ia).unwrap();
        // One reader holds the document, a writer queues behind it (an agent or second window making a
        // change): a plain try_read would now fail, so a question is answered stale, but the reply of an
        // operation must still be the real, current figures.
        let (tx, rx) = std::sync::mpsc::channel();
        let e1 = entry.clone();
        let reader = std::thread::spawn(move || {
            let _r = e1.doc.read().unwrap();
            tx.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(400));
        });
        rx.recv().unwrap();
        let e2 = entry.clone();
        let writer = std::thread::spawn(move || {
            let mut d = e2.doc.write().unwrap();
            d.set_cell(1, 0, "changed".to_string()).unwrap();
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let question = docs.info_now(ia).unwrap();
        assert!(
            question.stale,
            "a question is answered at once, from the last reading"
        );
        let reply = docs.info(ia).unwrap();
        assert!(!reply.stale, "an operation's reply is never stale");
        assert!(
            reply.seq > question.seq,
            "the reply is a newer reading than the stale answer"
        );
        // (Whether the queued writer got in before or after this reply depends on the system's lock; what
        // must hold is that the reply is a real reading, and that after the change a reply shows it.)
        reader.join().unwrap();
        writer.join().unwrap();
        let after = docs.info(ia).unwrap();
        assert!(!after.stale);
        assert!(
            after.data_revision > before.data_revision,
            "a reply after the change shows it"
        );
    }

    #[test]
    fn every_fresh_reading_is_kept_and_readings_only_count_up() {
        let a = tmp(
            "lock_d.csv",
            b"a
1
",
        );
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        let first = wait(&docs, ia);
        // An edit's own reply (built without info()) is what a stale answer falls back to afterwards.
        let edited = docs.set_cells(ia, vec![(1, 0, "x".into())]).unwrap();
        assert!(edited.seq > first.seq);
        let h = hold_write(&docs, ia, 300);
        let stale = docs.info_now(ia).unwrap();
        assert!(stale.stale);
        assert_eq!(
            (stale.seq, stale.history_pos),
            (edited.seq, edited.history_pos),
            "not older than the last edit"
        );
        h.join().unwrap();
        let later = docs.info(ia).unwrap();
        assert!(later.seq > stale.seq);
    }

    #[test]
    fn commands_that_ask_about_a_document_never_wait_for_a_change_to_finish() {
        let a = tmp(
            "lock_a.csv",
            b"a,b
1,2
3,4
",
        );
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        let fresh = wait(&docs, ia);
        assert!(!fresh.stale);
        let t0 = std::time::Instant::now();
        let h = hold_write(&docs, ia, 600);
        // info: at once, stale, same figures; the live parts (read-only) are current.
        docs.set_read_only(ia, true)
            .unwrap_or_else(|_| fresh.clone());
        let got = docs.info_now(ia).unwrap();
        assert!(got.stale, "answered from the last reading");
        assert_eq!(
            (got.rows, got.cols, got.data_revision),
            (fresh.rows, fresh.cols, fresh.data_revision)
        );
        assert!(got.read_only, "live parts are current");
        // path_of and private_changed never wait either.
        assert_eq!(docs.path_of(ia).unwrap().as_deref(), Some(a.as_path()));
        docs.private_changed();
        docs.set_private(ia, true).unwrap();
        docs.set_private(ia, false).unwrap();
        assert!(
            t0.elapsed() < std::time::Duration::from_millis(300),
            "no waiting: {:?}",
            t0.elapsed()
        );
        h.join().unwrap();
        // After the change let go: a fresh answer follows.
        let after = docs.info_now(ia).unwrap();
        assert!(!after.stale);
    }

    #[test]
    fn the_first_answer_during_a_change_waits_on_its_own_thread_and_is_fresh() {
        // A document whose first reading never happened: the stored info is cleared as if new.
        let a = tmp(
            "lock_b.csv",
            b"a
1
",
        );
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        wait(&docs, ia);
        *docs.get(ia).unwrap().last_info.lock().unwrap() = None;
        let h = hold_write(&docs, ia, 300);
        let t0 = std::time::Instant::now();
        let got = docs.info_now(ia).unwrap();
        assert!(!got.stale, "with nothing stored it waits for a real answer");
        assert!(t0.elapsed() >= std::time::Duration::from_millis(200));
        h.join().unwrap();
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
        docs.run_recipe(ib, &r, &[], 20_000.0, false).unwrap();
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
        assert!(docs.run_recipe(ib, &other, &[], 20_000.0, false).is_err());
        // Saved and read back; never over an open file.
        let path = crate::testdir::path("m14r.recipe.json");
        assert!(docs.save_recipe(ia, &r, &a).is_err());
        docs.save_recipe(ia, &r, &path).unwrap();
        assert_eq!(read_recipe(&path).unwrap().steps.len(), 2);
    }

    #[test]
    fn rules_are_kept_beside_a_file_only_when_asked_and_never_for_private_or_encrypted_files() {
        use natendo_engine::validate::{Check, CheckKind};
        let rules = vec![Check::new(Some(0), CheckKind::Required)];
        let docs = Docs::default();
        // A plain file: kept, read back (and the file holds the rule, not the cells).
        let a = tmp(
            "lint_a.csv",
            b"name
Ann

",
        );
        let ia = docs.open_path(&a, false).unwrap().id;
        wait(&docs, ia);
        assert!(docs.checks_sidecar_load(ia).unwrap().is_none());
        let name = docs
            .checks_sidecar_keep(ia, rules.clone(), vec![], true)
            .unwrap();
        assert_eq!(name, "lint_a.csv.ndchecks");
        let got = docs.checks_sidecar_load(ia).unwrap().unwrap();
        assert_eq!((got.checks.len(), got.skipped), (1, 0));
        let text = std::fs::read_to_string(a.with_file_name(&name)).unwrap();
        assert!(!text.contains("Ann"));
        // No rules removes it.
        docs.checks_sidecar_keep(ia, vec![], vec![], true).unwrap();
        assert!(docs.checks_sidecar_load(ia).unwrap().is_none());
        // A private tab (private file mode) is refused.
        docs.set_private(ia, true).unwrap();
        let e = docs
            .checks_sidecar_keep(ia, rules.clone(), vec![], true)
            .unwrap_err();
        assert!(e.message.contains("private"), "{}", e.message);
        assert!(!a.with_file_name("lint_a.csv.ndchecks").exists());
        docs.set_private(ia, false).unwrap();
        // A file in a private folder is refused even when the tab itself is not marked.
        let b = tmp(
            "lint_b.csv",
            b"name
Bob
",
        );
        let ib = docs.open_path(&b, false).unwrap().id;
        wait(&docs, ib);
        let folder = b.clone();
        docs.set_private_check(Arc::new(move |p: &Path| p == folder.as_path()));
        let e = docs
            .checks_sidecar_keep(ib, rules.clone(), vec![], true)
            .unwrap_err();
        assert!(e.message.contains("private"), "{}", e.message);
        docs.set_private_check(Arc::new(|_: &Path| false));
        // A tab opened from an encrypted file is private and has no plain file beside it: refused.
        let enc = b.with_file_name("lint_b.ndenc");
        docs.save_encrypted(ib, &enc, "a long enough password")
            .unwrap();
        let ie = docs
            .open_encrypted(&enc, "a long enough password")
            .unwrap()
            .id;
        wait(&docs, ie);
        assert!(docs
            .checks_sidecar_keep(ie, rules.clone(), vec![], true)
            .is_err());
        assert!(!b.with_file_name("lint_b.ndenc.ndchecks").exists());
        // An unsaved tab has no place to keep them.
        let n = docs.new_document().unwrap().id;
        assert!(docs
            .checks_sidecar_keep(n, rules.clone(), vec![], true)
            .is_err());
        // A place that cannot be written gives a clear message, not a crash.
        let c = tmp(
            "lint_c.csv",
            b"name
Cy
",
        );
        let ic = docs.open_path(&c, false).unwrap().id;
        wait(&docs, ic);
        std::fs::create_dir(c.with_file_name("lint_c.csv.ndchecks")).unwrap();
        let e = docs
            .checks_sidecar_keep(ic, rules, vec![], true)
            .unwrap_err();
        assert!(e.message.contains("could not be saved"), "{}", e.message);
    }

    #[test]
    fn a_diff_opens_a_result_tab_that_is_private_when_a_source_is_and_follows_a_sort() {
        use natendo_engine::diff::DiffSpec;
        use natendo_engine::sort::{SortKey, SortKind, SortSpec};
        let a = tmp("diff_a.csv", b"id,name\r\n1,Ann\r\n2,Bo\r\n3,Cy\r\n");
        let b = tmp("diff_b.csv", b"id,name\r\n1,Ann\r\n2,Bob\r\n4,Di\r\n");
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        let ib = docs.open_path(&b, false).unwrap().id;
        wait(&docs, ia);
        wait(&docs, ib);
        let spec = DiffSpec {
            keys: vec![0],
            keep_first: true,
            ..Default::default()
        };
        let out = docs.diff_files(ia, ib, spec.clone()).unwrap();
        assert_eq!(
            (
                out.report.same,
                out.report.changed,
                out.report.added,
                out.report.removed
            ),
            (1, 1, 1, 1)
        );
        assert!(!docs.is_private(out.info.id).unwrap());
        // The marks on the first table: Bo's name changed (row 2), Cy removed (row 3).
        let m = docs.diff_marks(ia, 0, 10, 0, 10).unwrap();
        assert_eq!(
            m.marks.iter().map(|x| (x.row, x.col)).collect::<Vec<_>>(),
            vec![(2, Some(1)), (3, None)]
        );
        // A sort only re-orders the rows: the marks follow, by key.
        let sort = SortSpec {
            keys: vec![SortKey {
                col: 1,
                descending: true,
                kind: SortKind::Text,
                match_case: false,
            }],
            keep_first: true,
            ..Default::default()
        };
        docs.sort(ia, sort.clone()).unwrap();
        let m = docs.diff_marks(ia, 0, 10, 0, 10).unwrap();
        assert!(!m.out_of_date);
        assert_eq!(
            m.marks.iter().map(|x| (x.row, x.col)).collect::<Vec<_>>(),
            vec![(1, None), (2, Some(1))]
        );
        // An edit makes them out of date.
        docs.set_cells(ia, vec![(1, 1, "Zed".into())]).unwrap();
        let m = docs.diff_marks(ia, 0, 10, 0, 10).unwrap();
        assert!(m.out_of_date && m.marks.is_empty());
        // A sort after the edit does not bring the marks back: the edit is not carried forward as current.
        docs.sort(ia, sort).unwrap();
        let m = docs.diff_marks(ia, 0, 10, 0, 10).unwrap();
        assert!(m.out_of_date && m.marks.is_empty());
        // Next difference, and clearing.
        let again = docs.diff_files(ia, ib, spec.clone()).unwrap();
        assert!(again.diff > out.diff);
        assert!(docs.diff_next(ib, 0, false).unwrap().is_some());
        docs.diff_clear(ia);
        assert!(docs.diff_marks(ib, 0, 10, 0, 10).unwrap().diff.is_none());
        // A table compared with itself: all the same.
        let me = docs.diff_files(ib, ib, spec.clone()).unwrap();
        assert_eq!(
            (me.report.changed, me.report.added, me.report.removed),
            (0, 0, 0)
        );
        // A private source makes the result private.
        docs.set_private(ib, true).unwrap();
        let p = docs.diff_files(ia, ib, spec).unwrap();
        assert!(
            docs.is_private(p.info.id).unwrap(),
            "the result of a private tab is private"
        );
    }

    #[test]
    #[ignore]
    fn info_cost_on_a_million_rows() {
        let mut csv = String::from("a,b,c\r\n");
        for i in 0..1_000_000 {
            csv.push_str(&format!("{i},x{},y\r\n", i % 97));
        }
        let a = tmp("infocost.csv", csv.as_bytes());
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        wait(&docs, ia);
        let mut best = f64::MAX;
        for _ in 0..7 {
            let t = std::time::Instant::now();
            for _ in 0..20_000 {
                std::hint::black_box(docs.info(ia).unwrap());
            }
            best = best.min(t.elapsed().as_secs_f64() * 1e6 / 20_000.0);
        }
        println!("INFOCOST {best:.2} microseconds per info() call (best of 7 x 20,000)");
    }

    /// The app's share-safe copy and encrypted save write what the engine
    /// calls `natendo-cli share-safe` and `encrypt` make write (the CLI tests
    /// compare the CLI's output with those engine calls byte for byte).
    #[test]
    fn share_safe_and_encrypt_match_the_engine_calls_the_cli_makes() {
        use natendo_engine::privacy::{ColumnAction, SafeCopy, Secret};
        let a = tmp(
            "cli-parity.csv",
            b"id,name,dob\r\n1,Ann Lee,1980-01-02\r\n2,Bob Ray,=1+2\r\n",
        );
        let docs = Docs::default();
        let id = docs.open_path(&a, false).unwrap().id;
        wait(&docs, id);
        let mut scramble: natendo_engine::clean::Cleaner =
            serde_json::from_str(r#"{"steps":[{"tool":"scramble"}]}"#).unwrap();
        if let natendo_engine::clean::Step::Scramble { phrase, .. } = &mut scramble.steps[0] {
            *phrase = Secret::new("correct horse battery staple");
        }
        let spec = SafeCopy {
            columns: vec![
                ColumnAction::Keep,
                ColumnAction::Clean { cleaner: scramble },
                ColumnAction::Keep,
            ],
            header: true,
            read: Default::default(),
            guard: true,
        };
        let app = a.with_file_name("cli-parity-app.csv");
        let eng = a.with_file_name("cli-parity-engine.csv");
        docs.save_safe_copy(id, &app, &spec, 20_000.0).unwrap();
        let d = natendo_engine::Document::open_indexed(&a, Default::default(), None).unwrap();
        d.save_safe_copy(&eng, &spec, 20_000.0, &natendo_engine::Progress::new())
            .unwrap();
        let bytes = std::fs::read(&app).unwrap();
        assert_eq!(bytes, std::fs::read(&eng).unwrap());
        assert!(!String::from_utf8_lossy(&bytes).contains("Ann Lee"));
        // Encrypted: the salt differs, the plain text is the same.
        let pw = "a long test password";
        let app_enc = a.with_file_name("cli-parity-app.ndenc");
        let eng_enc = a.with_file_name("cli-parity-engine.ndenc");
        docs.save_encrypted(id, &app_enc, pw).unwrap();
        d.save_encrypted(&eng_enc, &Secret::new(pw), &natendo_engine::Progress::new())
            .unwrap();
        let plain = |p: &std::path::Path| {
            natendo_engine::crypt::decrypt(&std::fs::read(p).unwrap(), &Secret::new(pw)).unwrap()
        };
        assert_eq!(plain(&app_enc), plain(&eng_enc));
        for f in [app, eng, app_enc, eng_enc] {
            let _ = std::fs::remove_file(f);
        }
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
            guard: false,
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
                        ..Default::default()
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
        let out = crate::testdir::path("natendo-n1");
        std::fs::create_dir_all(&out).unwrap();
        let written = Mutex::new(Vec::new());
        let r = docs
            .split_to_files(
                ia,
                &natendo_engine::tools::SplitParts {
                    guard: false,
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
                    limit: None,
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

    /// C8 review R1: the chip bar's counts are stopped by an edit, a filter change or a newer ask.
    #[test]
    fn chip_counts_stop_on_a_change() {
        let p = tmp(
            "chipstop.csv",
            b"a,b
1,x
2,y
",
        );
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let entry = docs.get(id).unwrap();
        let running = Arc::new(Progress::new());
        *entry.chip_count.lock().unwrap() = Some(running.clone());
        docs.set_cells(id, vec![(1, 1, "z".into())]).unwrap();
        assert!(running.cancel.load(Ordering::Relaxed), "an edit stops it");
        let running = Arc::new(Progress::new());
        *entry.chip_count.lock().unwrap() = Some(running.clone());
        docs.filter_chip_counts_stop(id).unwrap();
        assert!(
            running.cancel.load(Ordering::Relaxed),
            "the stop command stops it"
        );
        // A count after that works (no filter: no chips).
        assert!(docs.filter_chip_counts(id).unwrap().chips.is_empty());
    }

    /// C11: a JSON file opened from Import, as the app does it (the tab saves back to it).
    fn json_tab(docs: &Docs, name: &str, body: &[u8]) -> (u32, PathBuf) {
        let p = tmp(name, body);
        let imports = crate::imports::Imports::default();
        let start = imports.open_path(&p).unwrap();
        let info = imports
            .finish(
                start.id,
                &natendo_engine::import::ImportOptions::default(),
                docs,
            )
            .unwrap();
        (info.info.id, p)
    }

    /// C11: Ctrl+S on a JSON tab writes the file back in its own format and kinds; the tab is saved and
    /// keeps its undo history; DocInfo says so.
    #[test]
    fn json_tab_saves_back_to_its_file() {
        use natendo_engine::jsonshape::Blanks;
        let docs = Docs::default();
        let body = b"{\"id\":1,\"price\":1.50,\"ok\":true}
{\"id\":2,\"price\":3}
";
        let (id, p) = json_tab(&docs, "back.jsonl", body);
        let i = wait(&docs, id);
        assert_eq!(i.json_format, Some("jsonl"));
        assert_eq!(i.json_refuse, None);
        assert!(i.json_file.is_some() && i.path.is_none());
        // No edits: the same bytes.
        let r = docs
            .save_json(id, None, None, Blanks::LeaveOut, false)
            .unwrap();
        assert!(r.own);
        assert_eq!(r.info.name, "back");
        assert_eq!(r.report.records, 2);
        assert_eq!(std::fs::read(&p).unwrap(), body);
        let e = docs.set_cells(id, vec![(2, 1, "4.25".into())]).unwrap();
        assert_eq!(e.name, "back");
        assert!(docs.info(id).unwrap().modified);
        let r = docs
            .save_json(id, None, None, Blanks::LeaveOut, false)
            .unwrap();
        assert!(!r.info.modified && r.info.can_undo);
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "{\"id\":1,\"price\":1.50,\"ok\":true}
{\"id\":2,\"price\":4.25}
"
        );
    }

    /// An edit's reply keeps the name of a tab imported from a file that is not JSON (it once said
    /// "Untitled" after any edit), and such a tab has no JSON file.
    #[test]
    fn an_imported_tab_keeps_its_name_after_an_edit() {
        let docs = Docs::default();
        let (id, _) = json_tab(
            &docs,
            "people.md",
            b"| a | b |
| --- | --- |
| 1 | x |
",
        );
        let i = wait(&docs, id);
        assert_eq!(i.name, "people");
        assert!(i.json_format.is_none());
        let e = docs.set_cells(id, vec![(1, 1, "y".into())]).unwrap();
        assert_eq!(e.name, "people");
        assert!(e.json_format.is_none() && e.path.is_none());
    }

    /// C11: the same safety as Save: another program's change stops the save (until agreed), a
    /// read-only tab and a peeking one are refused, and a failed write leaves the file as it was.
    #[test]
    fn json_save_back_is_as_safe_as_save() {
        use natendo_engine::jsonshape::Blanks;
        let docs = Docs::default();
        let body = b"{\"a\":\"x\"}
";
        let (id, p) = json_tab(&docs, "safe.jsonl", body);
        wait(&docs, id);
        docs.set_cells(id, vec![(1, 0, "y".into())]).unwrap();
        // Another program changes the file.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            &p,
            b"{\"a\":\"theirs\"}
",
        )
        .unwrap();
        let e = docs
            .save_json(id, None, None, Blanks::LeaveOut, false)
            .unwrap_err();
        assert_eq!(e.kind, "changed_on_disk");
        assert_eq!(
            std::fs::read(&p).unwrap(),
            b"{\"a\":\"theirs\"}
"
        );
        docs.save_json(id, None, None, Blanks::LeaveOut, true)
            .unwrap();
        assert_eq!(
            std::fs::read(&p).unwrap(),
            b"{\"a\":\"y\"}
"
        );
        // Read-only.
        docs.set_read_only(id, true).unwrap();
        assert_eq!(
            docs.save_json(id, None, None, Blanks::LeaveOut, false)
                .unwrap_err()
                .kind,
            "read_only"
        );
        docs.set_read_only(id, false).unwrap();
        // A write that cannot happen (the folder is gone) leaves no file and an error.
        let gone = p.parent().unwrap().join("no-such-folder").join("x.jsonl");
        assert!(docs
            .save_json(id, Some(gone.clone()), None, Blanks::LeaveOut, false)
            .is_err());
        assert!(!gone.exists());
        assert_eq!(
            std::fs::read(&p).unwrap(),
            b"{\"a\":\"y\"}
"
        );
    }

    /// C11: a tab with a CSV file of its own writes a JSON copy (all text) and stays as it is; a JSON tab
    /// saved as CSV no longer saves back to the JSON file.
    #[test]
    fn json_copies_and_csv_save_as() {
        use natendo_engine::jsonshape::{Blanks, JsonForm};
        let docs = Docs::default();
        let csv = tmp(
            "own.csv",
            b"n,t
1,x
",
        );
        let id = docs.open_path(&csv, false).unwrap().id;
        wait(&docs, id);
        docs.set_cells(id, vec![(1, 1, "y".into())]).unwrap();
        let out = csv.with_file_name("own-copy.jsonl");
        let form = JsonForm {
            lines: true,
            indent: None,
            crlf: false,
            bom: false,
            final_newline: true,
        };
        let r = docs
            .save_json(id, Some(out.clone()), Some(form), Blanks::LeaveOut, false)
            .unwrap();
        assert!(!r.own && r.info.modified && r.info.json_format.is_none());
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "{\"n\":\"1\",\"t\":\"y\"}
"
        );
        // A JSON tab saved as CSV.
        let (jid, _) = json_tab(
            &docs,
            "tocsv.jsonl",
            b"{\"a\":1}
",
        );
        wait(&docs, jid);
        let target = csv.with_file_name("tocsv.csv");
        let i = docs.save(jid, Some(target), false, false, None).unwrap();
        assert!(i.json_format.is_none() && i.path.is_some());
        assert_eq!(
            docs.save_json(jid, None, None, Blanks::LeaveOut, false)
                .unwrap_err()
                .kind,
            "no_json_file"
        );
    }

    /// C11: a record with the same key twice cannot be saved back, and DocInfo says why.
    #[test]
    fn json_key_twice_is_refused_with_the_reason() {
        use natendo_engine::jsonshape::Blanks;
        let docs = Docs::default();
        let (id, p) = json_tab(
            &docs,
            "twice.jsonl",
            b"{\"a\":1}
{\"a\":2,\"a\":3}
",
        );
        let i = wait(&docs, id);
        assert!(
            i.json_refuse
                .as_deref()
                .is_some_and(|w| w.contains("twice")),
            "{:?}",
            i.json_refuse
        );
        assert!(docs
            .save_json(id, None, None, Blanks::LeaveOut, false)
            .is_err());
        assert_eq!(
            std::fs::read(&p).unwrap(),
            b"{\"a\":1}
{\"a\":2,\"a\":3}
"
        );
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
    fn hidden_columns_are_not_copied_or_filled() {
        let docs = Docs::default();
        let p = tmp("hc.csv", b"a,b,c\n1,2,3\n");
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        assert_eq!(
            docs.copy_skipping(id, 0, 2, 0, 3, &[1]).unwrap(),
            "a\tc\r\n1\t3\r\n"
        );
        docs.fill_skipping(id, 1, 1, 0, 3, "x".into(), &[1])
            .unwrap();
        assert_eq!(docs.copy(id, 1, 1, 0, 3).unwrap(), "x\t2\tx\r\n");
    }

    /// B2: the picker's preview, Apply and scan on a real file: the preview reads the head as each encoding, Apply is
    /// one undo step, both are refused when the file changed on disk (R9b, direct encodings too), Apply is refused
    /// with unsaved edits, and a needs-reopen encoding is refused in place.
    #[test]
    fn encoding_preview_apply_scan_and_the_disk_check() {
        let docs = Docs::default();
        let bytes = b"name,city\nJos\xe9,Z\xfcrich\nAnn,Oslo\n";
        let p = tmp("enc.csv", bytes);
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let first = docs.info(id).unwrap();
        assert_eq!(
            first.encoding, "windows-1252",
            "detected as the legacy encoding"
        );
        // The preview: the head of the file read as each choice.
        let utf8 = docs.encoding_preview(id, "utf-8", 5).unwrap();
        assert!(utf8[1][0].contains('\u{fffd}'), "{utf8:?}");
        let latin = docs.encoding_preview(id, "windows-1252", 5).unwrap();
        assert_eq!(latin[1], ["Jos\u{e9}", "Z\u{fc}rich"]);
        assert!(docs.encoding_preview(id, "no-such", 5).is_err());
        // Apply (UTF-8 here: ASCII-compatible, no reopen): one undo step, the data revision moves.
        let before = docs.info(id).unwrap();
        let info = docs.encoding_apply(id, "utf-8").unwrap();
        assert_eq!(info.history_pos, before.history_pos + 1);
        assert!(info.data_revision > before.data_revision);
        assert_eq!(info.encoding, "UTF-8");
        assert!(docs.copy(id, 1, 1, 0, 2).unwrap().contains('\u{fffd}'));
        // A needs-reopen encoding is refused in place, whatever the file is.
        let e = docs.encoding_apply(id, "shift_jis").unwrap_err();
        assert!(e.message.contains("reopen"), "{}", e.message);
        // Edits make Apply refuse (they were made against the old reading).
        docs.set_cells(id, vec![(2, 0, "Bob".into())]).unwrap();
        let e = docs.encoding_apply(id, "windows-1252").unwrap_err();
        assert!(
            e.message.contains("Save or undo your changes first"),
            "{}",
            e.message
        );
        docs.undo(id).unwrap();
        // The file changed on disk: preview and Apply both refuse, even for a direct encoding.
        std::fs::write(
            &p,
            b"name,city\nJos\xe9,Z\xfcrich\nAnn,Oslo\nplus,one more\n",
        )
        .unwrap();
        for e in [
            docs.encoding_preview(id, "windows-1252", 5).unwrap_err(),
            docs.encoding_apply(id, "windows-1252").unwrap_err(),
        ] {
            assert_eq!(e.kind, "changed_on_disk");
            assert!(e.message.contains("changed on disk"), "{}", e.message);
        }
        // Reading the changed file again as another encoding is a new document (the old one is untouched).
        let again = docs
            .open_with_encoding(&p, false, None, Encoding::from_label("windows-1252"))
            .unwrap();
        assert_eq!(again.encoding, "windows-1252");
        assert_eq!(wait(&docs, again.id).rows, 4);
        assert_ne!(again.id, id);
    }

    #[test]
    fn the_scan_marks_rows_and_shows_only_them() {
        let docs = Docs::default();
        let mut text = b"name\n".to_vec();
        text.extend_from_slice(b"ok\n");
        text.extend_from_slice("caf\u{e9}\n".as_bytes());
        text.extend_from_slice(b"Jos\xe9\n");
        text.extend_from_slice(b"fine\n");
        let p = tmp("scan.csv", &text);
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        // Detection calls a mixed file Windows-1252: read it as UTF-8 first, as a person comparing would.
        let info = docs
            .open_with_encoding(&p, false, None, Some(Encoding::Utf8))
            .unwrap();
        wait(&docs, info.id);
        let scan = docs.scan_encodings(info.id, true).unwrap();
        assert_eq!(
            (scan.ascii, scan.utf8, scan.invalid, scan.mixed),
            (2, 1, 1, true)
        );
        let shown = docs.encoding_show_rows(info.id, true).unwrap();
        assert_eq!(shown.rows, 2, "the title row and the one invalid row");
    }

    fn names(docs: &Docs, id: u32) -> Vec<String> {
        docs.checkpoint_list(id)
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect()
    }

    /// B4: named checkpoints restore by mark; restoring leaves a way back; a newer change, a lock and a save
    /// each end or refuse them, and say so.
    #[test]
    fn checkpoints_restore_and_say_when_they_are_gone() {
        let docs = Docs::default();
        let p = tmp("marks.csv", b"a,b\n1,2\n3,4\n");
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let at = |docs: &Docs| docs.copy(id, 0, 3, 0, 2).unwrap();
        docs.checkpoint_add(id, "start").unwrap();
        docs.set_cells(id, vec![(1, 0, "x".into())]).unwrap();
        docs.checkpoint_add(id, "one").unwrap();
        docs.set_cells(id, vec![(2, 0, "y".into())]).unwrap();
        let list = docs.checkpoint_add(id, "two").unwrap();
        assert_eq!(names(&docs, id), ["start", "one", "two"]);
        assert!(list.iter().all(|c| c.reachable && !c.auto));
        assert!(list[2].current && !list[0].current);
        let (start, one, two) = (list[0].id, list[1].id, list[2].id);
        // Back to "one": one jump, and the moment it left is kept so the way back is one click.
        let info = docs.checkpoint_restore(id, one).unwrap();
        assert_eq!(at(&docs), "a\tb\r\nx\t2\r\n3\t4\r\n");
        assert!(!info.peeking);
        let list = docs.checkpoint_list(id).unwrap();
        assert!(list[1].current);
        // "two" is where we were: it already names that moment, so no duplicate is made.
        assert_eq!(list.len(), 3);
        docs.checkpoint_restore(id, two).unwrap();
        assert_eq!(at(&docs), "a\tb\r\nx\t2\r\ny\t4\r\n");
        docs.checkpoint_restore(id, start).unwrap();
        assert_eq!(at(&docs), "a\tb\r\n1\t2\r\n3\t4\r\n");
        // Restoring from an unnamed moment keeps that moment as an automatic checkpoint.
        docs.checkpoint_restore(id, two).unwrap();
        docs.set_cells(id, vec![(1, 1, "z".into())]).unwrap();
        docs.checkpoint_restore(id, start).unwrap();
        let list = docs.checkpoint_list(id).unwrap();
        assert!(
            list.last().unwrap().auto && list.last().unwrap().name.starts_with("Before restoring")
        );
        assert!(list
            .iter()
            .all(|c| c.reachable || c.id == one || c.id == two));
        // The change after the restore replaced what was undone: the old branch is gone, and asking says so.
        docs.set_cells(id, vec![(1, 1, "q".into())]).unwrap();
        let list = docs.checkpoint_list(id).unwrap();
        let gone: Vec<&str> = list
            .iter()
            .filter(|c| !c.reachable)
            .map(|c| c.name.as_str())
            .collect();
        assert!(gone.contains(&"one") && gone.contains(&"two"), "{gone:?}");
        let before = at(&docs);
        let e = docs.checkpoint_restore(id, two).unwrap_err();
        assert!(e.message.contains("no longer be reached"), "{}", e.message);
        assert_eq!(at(&docs), before, "a refused restore changes nothing");
        // A lock refuses it like any change.
        docs.set_read_only(id, true).unwrap();
        assert_eq!(
            docs.checkpoint_restore(id, start).unwrap_err().kind,
            "read_only"
        );
        docs.set_read_only(id, false).unwrap();
        // A save keeps the history, so the checkpoint before the edits is still there.
        docs.save(id, None, false, true, None).unwrap();
        assert!(docs
            .checkpoint_list(id)
            .unwrap()
            .iter()
            .any(|c| c.reachable));
        assert!(docs.checkpoint_restore(id, start).is_ok());
        // Forgetting one leaves the table alone.
        let n = docs.checkpoint_list(id).unwrap().len();
        assert_eq!(docs.checkpoint_remove(id, start).unwrap().len(), n - 1);
    }

    /// B4 part 2: "what changed since this checkpoint" through the shell: counts and pages by row id, a plain refusal
    /// for a checkpoint ahead of the table (and the note that greys it), and the automatic "Before Split into rows"
    /// checkpoint, whose rows count once.
    #[test]
    fn checkpoint_since_counts_pages_and_refuses_plainly() {
        let docs = Docs::default();
        let p = tmp("since.csv", b"name,tags\nann,a|b|c\nbob,x\ncat,p|q\n");
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        docs.checkpoint_add(id, "begin").unwrap();
        let begin = docs.checkpoint_list(id).unwrap()[0].id;
        let info = docs.checkpoint_list(id).unwrap();
        assert_eq!(
            info[0].since_note,
            "The table is in this state now: nothing has changed since."
        );
        docs.set_cells(id, vec![(1, 0, "Anna".into()), (3, 1, "pq".into())])
            .unwrap();
        let v = docs.checkpoint_since(id, begin, true, 0, 10).unwrap();
        assert_eq!(
            (
                v.totals.rows_changed,
                v.totals.cells_changed,
                v.totals.steps
            ),
            (2, 2, 1)
        );
        assert_eq!(v.cells.len(), 2);
        assert_eq!(
            (
                v.cells[0].old.as_str(),
                v.cells[0].new.as_str(),
                v.cells[0].row
            ),
            ("ann", "Anna", Some(1))
        );
        // A page past the first: the same plan, the next cells.
        let second = docs.checkpoint_since(id, begin, true, 1, 10).unwrap();
        assert_eq!(second.cells.len(), 1);
        assert_eq!(second.cells[0].new, "pq");
        // The list says it can be compared.
        assert_eq!(docs.checkpoint_list(id).unwrap()[0].since_note, "");
        // The report: a CSV with a line per cell.
        let out = crate::testdir::path("since-report.csv");
        assert_eq!(
            docs.checkpoint_since_report(id, begin, true, &out, false)
                .unwrap()
                .entries,
            2
        );
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.starts_with("Row now,Column,Was,Now"), "{text}");
        assert!(text.contains("ann,"), "{text}");
        let _ = std::fs::remove_file(&out);
        // Split into rows: an automatic checkpoint is made inside the change; the rows it adds are added once.
        let spec = natendo_engine::reshape::SplitRows {
            col: 1,
            separator: "|".into(),
            trim: true,
            skip_empty: true,
            keep_first: true,
        };
        docs.data_op(id, DataOp::SplitRows { spec }).unwrap();
        let list = docs.checkpoint_list(id).unwrap();
        let before = list
            .iter()
            .find(|c| c.name == "Before Split into rows")
            .expect("the automatic checkpoint");
        let v = docs.checkpoint_since(id, before.id, true, 0, 10).unwrap();
        // a|b|c makes two more rows (p|q was edited to pq above, so it does not split).
        assert_eq!(v.totals.rows_added, 2, "added once");
        // Going back past the checkpoint leaves it ahead of the table: refused, with a plain note.
        docs.checkpoint_restore(id, begin).unwrap();
        let list = docs.checkpoint_list(id).unwrap();
        let ahead = list
            .iter()
            .find(|c| c.name == "Before Split into rows")
            .unwrap();
        assert!(
            ahead.since_note.contains("ahead of the table"),
            "{}",
            ahead.since_note
        );
        let e = docs
            .checkpoint_since(id, ahead.id, true, 0, 10)
            .unwrap_err();
        assert!(e.message.contains("ahead of the table"), "{}", e.message);
    }

    /// The CSV formula guard through the shell: each of the four outputs with it on and off. Off writes the text as
    /// it is; a save of the file itself never changes it.
    #[test]
    fn formula_guard_on_reports_safe_copy_and_split_never_on_save() {
        let docs = Docs::default();
        let body: &[u8] = b"name,note\nann,=1+1\n@bob,-5\ncat,+1 555 0100\n";
        let p = tmp("guard.csv", body);
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let begin = docs.checkpoint_add(id, "begin").unwrap()[0].id;
        let dir = crate::testdir::path("nd-guard");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let read = |p: &Path| std::fs::read_to_string(p).unwrap();
        // Safe copy: on is 3 cells (the number stays), off is the file's text.
        let spec = |guard| natendo_engine::privacy::SafeCopy {
            columns: vec![
                natendo_engine::privacy::ColumnAction::Keep,
                natendo_engine::privacy::ColumnAction::Keep,
            ],
            header: true,
            read: Default::default(),
            guard,
        };
        let out = dir.join("safe.csv");
        let r = docs.save_safe_copy(id, &out, &spec(true), 0.0).unwrap();
        assert_eq!(r.guarded, 3);
        assert_eq!(
            read(&out),
            "name,note\r\nann,'=1+1\r\n'@bob,-5\r\ncat,'+1 555 0100\r\n"
        );
        let r = docs.save_safe_copy(id, &out, &spec(false), 0.0).unwrap();
        assert_eq!(r.guarded, 0);
        assert_eq!(
            read(&out),
            "name,note\r\nann,=1+1\r\n@bob,-5\r\ncat,+1 555 0100\r\n"
        );
        // Split: off unless asked.
        let split = |guard| natendo_engine::tools::SplitParts {
            col: None,
            every: 10,
            keep_first: true,
            guard,
        };
        let r = docs
            .split_to_files(id, &split(false), &dir, "off", false, &|_| {})
            .unwrap();
        assert_eq!(r.guarded, 0);
        assert_eq!(read(&dir.join("off - 1.csv")).as_bytes(), body);
        let r = docs
            .split_to_files(id, &split(true), &dir, "on", false, &|_| {})
            .unwrap();
        assert_eq!(r.guarded, 3);
        assert!(read(&dir.join("on - 1.csv")).contains("\nann,'=1+1\n'@bob,-5\n"));
        // The two reports.
        docs.set_cells(id, vec![(1, 0, "=cmd".into())]).unwrap();
        let out = dir.join("changes.csv");
        let w = docs.save_change_report(id, &out, true).unwrap();
        assert_eq!((w.entries, w.guarded), (1, 1));
        assert!(read(&out).contains(",ann,'=cmd"), "{}", read(&out));
        let w = docs.save_change_report(id, &out, false).unwrap();
        assert_eq!(w.guarded, 0);
        assert!(read(&out).contains(",ann,=cmd"), "{}", read(&out));
        let out = dir.join("since.csv");
        let w = docs
            .checkpoint_since_report(id, begin, true, &out, true)
            .unwrap();
        assert_eq!((w.entries, w.guarded), (1, 1));
        assert!(read(&out).contains(",ann,'=cmd"), "{}", read(&out));
        let w = docs
            .checkpoint_since_report(id, begin, true, &out, false)
            .unwrap();
        assert_eq!(w.guarded, 0);
        assert!(read(&out).contains(",ann,=cmd"), "{}", read(&out));
        // Saving the file itself never adds an apostrophe.
        docs.save(id, None, false, true, None).unwrap();
        let saved = read(&p);
        assert!(
            !saved.contains('\'') && saved.contains("=cmd,=1+1"),
            "{saved}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// C1a: the open notice's counts come with the file's facts, and the filter shows the short and long rows; after
    /// a column change it is refused with the reason.
    #[test]
    fn ragged_rows_are_counted_at_open_and_shown() {
        let docs = Docs::default();
        let p = tmp(
            "ragged.csv",
            b"a,b,c
1,2,3
4,Smith, John,6
7
8,9,10
",
        );
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let i = docs.info(id).unwrap();
        assert_eq!(
            (i.usual_fields, i.ragged_rows, i.first_row_ragged),
            (3, 2, false)
        );
        let show = |pick| DataOp::ShowRagged {
            pick,
            keep_first: true,
        };
        use natendo_engine::document::ragged::RaggedPick;
        let r = docs.data_op(id, show(RaggedPick::All)).unwrap();
        assert_eq!((r.rows, r.groups, docs.info(id).unwrap().rows), (2, 1, 3));
        let r = docs.data_op(id, show(RaggedPick::Long)).unwrap();
        assert_eq!((r.rows, docs.info(id).unwrap().rows), (1, 2));
        docs.insert_cols(id, 0, 1).unwrap();
        let e = docs.data_op(id, show(RaggedPick::All)).unwrap_err();
        assert!(
            e.message.contains("added, removed or moved"),
            "{}",
            e.message
        );
    }

    /// C1b: the repair bar's calls: the ragged rows by id, one row's text, and a repair that is one undo step, left
    /// out of recipes, refused when stale and on a locked file.
    #[test]
    fn the_repair_bar_lists_reads_and_repairs_a_row() {
        let docs = Docs::default();
        let p = tmp(
            "repair.csv",
            b"id,name,city\n1,Smith, John,Rome\n2,ann,Oslo\n3\n",
        );
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let list = docs.ragged_list(id, true).unwrap();
        assert_eq!((list.ids.as_slice(), list.total), (&[1u32, 3][..], 2));
        let t = docs.row_text(id, None, Some(1), true).unwrap();
        assert_eq!(
            (t.row, t.file_row, t.raw.as_str()),
            (Some(1), 2, "1,Smith, John,Rome")
        );
        assert_eq!(docs.row_text(id, Some(3), None, true).unwrap().id, 3);
        let join = Repair::Join { groups: vec![1, 2] };
        let info = docs.repair_row(id, 1, t.revision, &join).unwrap();
        assert!(info.modified && info.can_undo);
        assert!(docs.repair_row(id, 1, t.revision, &join).is_err());
        assert_eq!(docs.ragged_list(id, true).unwrap().ids, [3]);
        let book = docs.recipe_of(id, true, "r").unwrap();
        assert!(
            format!("{book:?}").contains("repairing one row"),
            "{book:?}"
        );
        docs.undo(id).unwrap();
        assert_eq!(docs.ragged_list(id, true).unwrap().ids, [1, 3]);
        // A note on a row says which columns match it, for the bar's warning.
        docs.note_set(id, 2, 0, "check").unwrap();
        let t = docs.row_text(id, None, Some(3), true).unwrap();
        assert!(t.note_keys.is_empty() && t.previous.as_ref().unwrap().noted);
        assert_eq!(t.previous.unwrap().note_keys, [0]);
    }

    /// C1b b2: "Fix all like this" and removing the empty column through the shell: automatic checkpoints, a recipe
    /// step with counts only, replay on a second file and its refusal.
    #[test]
    fn fix_all_checkpoints_and_records_counts_only() {
        let docs = Docs::default();
        let body: &[u8] = b"a,b,c\n1,Smith, John,x\n2,Doe, Jane,y\n3,u,v\n4,u,v\n5,u,v\n";
        let p = tmp("fixall.csv", body);
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let same = docs.same_shape(id, true, 4, &[1, 2]).unwrap();
        assert_eq!(same.total, 2);
        let (n, _) = docs
            .fix_all(id, true, same.revision, 4, vec![1, 2], &[])
            .unwrap();
        assert_eq!(n, 2);
        let names: Vec<String> = docs
            .checkpoint_list(id)
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["Before Fix all like this"]);
        let book = docs.recipe_of(id, true, "r").unwrap();
        let json = serde_json::to_string(&book).unwrap();
        assert!(
            json.contains("join_fields") && !json.contains("Smith"),
            "{json}"
        );
        let (cols, revision) = docs.empty_extra_columns(id, true).unwrap();
        assert_eq!(cols, 1);
        let info = docs.remove_empty_extra_columns(id, true, revision).unwrap();
        assert_eq!(info.cols, 3);
        assert!(docs
            .checkpoint_list(id)
            .unwrap()
            .iter()
            .any(|c| c.name == "Before removing the empty column"));
        // The step on a second file, and on one of another width.
        let q = tmp("fixall2.csv", b"a,b,c\n9,Roe, Bo,z\n8,u,v\n7,u,v\n");
        let two = docs.open_path(&q, false).unwrap().id;
        wait(&docs, two);
        docs.run_recipe_op(two, book.steps[0].clone(), 0.0).unwrap();
        assert_eq!(docs.ragged_list(two, true).unwrap().total, 0);
        let r = tmp("fixall3.csv", b"a,b\n1,2\n3,4\n");
        let three = docs.open_path(&r, false).unwrap().id;
        wait(&docs, three);
        let e = docs
            .run_recipe_op(three, book.steps[0].clone(), 0.0)
            .unwrap_err();
        assert!(
            e.message.contains("made for files with 3 fields"),
            "{}",
            e.message
        );
    }

    /// A dates step saved by 0.6.0 says nothing about ambiguous dates: it
    /// replays to the same bytes as it did then. The same step recorded now
    /// writes the choice, and its recipe does the same again.
    #[test]
    fn an_older_dates_recipe_replays_the_same_and_a_new_one_records_the_choice() {
        let docs = Docs::default();
        let body: &[u8] = b"when,n\r\n03/04/2026,1\r\n25/04/2026,2\r\nsoon,3\r\n";
        let old = br#"{"version":1,"name":"old","columns":["when","n"],"header":true,"steps":[{"op":"clean","spec":{"cleaner":{"version":1,"name":"","steps":[{"tool":"dates","format":"yyyy-mm-dd"}]},"scope":{"kind":"columns","cols":[0]},"skip_first":true,"read":{}}}]}"#;
        let old = Recipe::from_json(old).unwrap();
        assert!(!String::from_utf8(old.to_json().unwrap())
            .unwrap()
            .contains("ambiguous"));
        // What 0.6.0 wrote: month first, and the day-first date read anyway.
        let want: &[u8] = b"when,n\r\n2026-03-04,1\r\n2026-04-25,2\r\nsoon,3\r\n";
        let replay = |name: &str, book: &Recipe| {
            let p = tmp(name, body);
            let id = docs.open_path(&p, false).unwrap().id;
            wait(&docs, id);
            docs.run_recipe(id, book, &[], 20_000.0, true).unwrap();
            let new = docs.recipe_of(id, true, "r").unwrap();
            docs.save(id, None, false, false, None).unwrap();
            (std::fs::read(&p).unwrap(), new)
        };
        let (bytes, recorded) = replay("dates-old.csv", &old);
        assert_eq!(bytes, want);
        let json = String::from_utf8(recorded.to_json().unwrap()).unwrap();
        assert!(json.contains("\"ambiguous\": \"mdy\""), "{json}");
        let again = Recipe::from_json(json.as_bytes()).unwrap();
        assert_eq!(replay("dates-new.csv", &again).0, want);
        // A step told to leave ambiguous dates says how many it left.
        let leave = json.replace("\"ambiguous\": \"mdy\"", "\"ambiguous\": \"leave\"");
        let leave = Recipe::from_json(leave.as_bytes()).unwrap();
        let p = tmp("dates-leave.csv", body);
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let info = docs.run_recipe(id, &leave, &[], 20_000.0, true).unwrap();
        assert_eq!(
            info.run_notes,
            ["Dates as yyyy-mm-dd: 1 ambiguous date left as typed"]
        );
    }

    /// C1b b2 review: a recorded fix says that unticked rows are not in the recipe, and running it reports how many
    /// rows it changed, none included.
    #[test]
    fn a_recorded_fix_says_what_it_leaves_out_and_reports_its_count() {
        let docs = Docs::default();
        let body: &[u8] = b"a,b,c\n1,Smith, John,x\n2,Doe, Jane,y\n3,u,v\n4,u,v\n5,u,v\n";
        let p = tmp("unticked.csv", body);
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let same = docs.same_shape(id, true, 4, &[1, 2]).unwrap();
        let left = same.first[1].id;
        let (_, done) = docs
            .fix_all(id, true, same.revision, 4, vec![1, 2], &[left])
            .unwrap();
        let note =
            "1 row you unticked is not in the recipe; running it fixes every row of 4 fields";
        assert_eq!(done.run_notes, [note]);
        assert!(docs.steps_view(id).unwrap().steps[0].text.contains(note));
        let book = docs.recipe_of(id, true, "r").unwrap();
        assert!(book.steps[0].describe().contains(note));
        // Run on a fresh copy: both rows, the count said; run again: none.
        let q = tmp("unticked2.csv", body);
        let two = docs.open_path(&q, false).unwrap().id;
        wait(&docs, two);
        let info = docs.run_recipe(two, &book, &[], 0.0, true).unwrap();
        assert_eq!(
            info.run_notes,
            ["Join fields in every row of 4 fields: 2 rows"]
        );
        assert!(!docs.steps_view(two).unwrap().steps[0]
            .text
            .contains("unticked"));
        let info = docs.run_recipe(two, &book, &[], 0.0, true).unwrap();
        assert_eq!(
            info.run_notes,
            ["Join fields in every row of 4 fields: 0 rows"]
        );
    }

    /// B4: risky bulk changes name a moment first; cheap ones do not; the automatic ones are capped at 20 and
    /// never push out a named one; a big paste and a delete of several rows count too.
    /// B4 part 2, review fixes: the report names columns from the first row only when it holds titles (no data value
    /// becomes a column name); rows taken out and columns taken out are listed; a page is read against the table its
    /// plan was made for, also after an edit between two pages.
    #[test]
    fn since_report_titles_lists_and_a_fresh_plan_after_an_edit() {
        let docs = Docs::default();
        let p = tmp(
            "since2.csv",
            b"Secret Person,Elm Street\nann,Oslo\nbob,Rome\ncat,Lyon\n",
        );
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let begin = docs.checkpoint_add(id, "begin").unwrap()[0].id;
        docs.set_cells(id, vec![(1, 0, "Anna".into())]).unwrap();
        docs.delete_rows(id, 2, 1).unwrap();
        docs.delete_cols(id, 1, 1).unwrap();
        let out = crate::testdir::path("since-titles.csv");
        // Without a header row: "Column N", never a value of the first row.
        let n = docs
            .checkpoint_since_report(id, begin, false, &out, false)
            .unwrap()
            .entries;
        // One changed cell, one row taken out, three cells of the column taken out (the rows still there).
        assert_eq!(n, 5);
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(
            text.contains("Column 1,ann,Anna") && text.contains("Column 2,Oslo,(column taken out)"),
            "{text}"
        );
        assert!(
            text.contains(",(whole row),bob | Rome,(taken out)"),
            "{text}"
        );
        assert!(
            !text.contains(",Secret Person,ann") && !text.contains(",Elm Street,Oslo"),
            "{text}"
        );
        // With a header row the titles name the columns, the taken-out one by its title at the checkpoint.
        docs.checkpoint_since_report(id, begin, true, &out, false)
            .unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(
            text.contains("Secret Person,ann,Anna")
                && text.contains("Elm Street,Oslo,(column taken out)"),
            "{text}"
        );
        let _ = std::fs::remove_file(&out);
        // A page, then an edit, then the next page: compared again, totals and lines of the table as it is now.
        let first = docs.checkpoint_since(id, begin, true, 0, 2).unwrap();
        assert_eq!(first.totals.cells_changed, 1);
        docs.set_cells(id, vec![(2, 0, "Katja".into())]).unwrap();
        let second = docs.checkpoint_since(id, begin, true, 0, 10).unwrap();
        assert_eq!(second.totals.cells_changed, 2);
        assert_eq!(second.cells.len() as u64, second.totals.entries);
        assert!(second.cells.iter().any(|c| c.new == "Katja"));
    }

    #[test]
    fn risky_changes_make_automatic_checkpoints() {
        let docs = Docs::default();
        let mut text = String::from("a,b\n");
        for i in 0..40 {
            text.push_str(&format!("{},{}\n", i % 3, i));
        }
        let p = tmp("auto.csv", text.as_bytes());
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        docs.checkpoint_add(id, "mine").unwrap();
        // Cheap: a cell edit and a sort make none.
        docs.set_cells(id, vec![(1, 0, "9".into())]).unwrap();
        assert_eq!(names(&docs, id), ["mine"]);
        // Deleting several rows does.
        docs.delete_rows(id, 5, 3).unwrap();
        assert_eq!(names(&docs, id), ["mine", "Before deleting rows"]);
        // A paste of a handful of cells does not; one of 1,000 or more does.
        let paste = |text: String| DataOp::Paste {
            spec: natendo_engine::ops::PasteSpec {
                row: 1,
                col: 0,
                ..Default::default()
            },
            text,
        };
        docs.data_op(id, paste("x\ty\r\n".into())).unwrap();
        assert_eq!(names(&docs, id).len(), 2);
        let big = "p\tq\r\n".repeat(600);
        docs.data_op(id, paste(big)).unwrap();
        assert_eq!(names(&docs, id).last().unwrap(), "Before a big paste");
        // Twenty-five more: only the newest 20 automatic ones stay, and "mine" is kept.
        for i in 0..25 {
            docs.delete_rows(id, 1, 2).unwrap();
            let _ = i;
        }
        let list = docs.checkpoint_list(id).unwrap();
        assert_eq!(list.iter().filter(|c| c.auto).count(), 20);
        assert_eq!(list.iter().filter(|c| !c.auto).count(), 1);
        assert_eq!(list[0].name, "mine");
    }

    /// B4: every kind of recipe step is classified (the match has no wildcard, so adding one without deciding
    /// does not compile); the risky ones are the ones this lists.
    #[test]
    fn the_risky_steps_are_the_ones_listed() {
        use natendo_engine::recipe::RecipeOp;
        assert_eq!(
            auto_label_op(&RecipeOp::DeleteHidden),
            Some("Delete hidden rows")
        );
        assert_eq!(auto_label_op(&RecipeOp::ClearFilter), None);
        assert_eq!(
            auto_label_left_out("a join with another tab or a list"),
            Some("Join")
        );
        assert_eq!(
            auto_label_left_out("changes taken from a newer file"),
            Some("Update from a newer file")
        );
        assert_eq!(auto_label_left_out("a paste"), None);
    }

    #[test]
    fn several_blocks_fill_each_cell_once_as_one_undo_step() {
        use natendo_engine::areas::Area;
        let docs = Docs::default();
        let p = tmp("areas.csv", b"a,b,c\n1,2,3\n4,5,6\n7,8,9\n");
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let a = |row, rows, col, cols| Area {
            row,
            rows,
            col,
            cols,
        };
        let pos = docs.info(id).unwrap().history_pos;
        // Two blocks that overlap at one cell, and a hidden column in the way.
        let info = docs
            .fill_areas(id, vec![a(1, 2, 0, 2), a(2, 2, 1, 2)], "x".into(), &[1])
            .unwrap();
        assert_eq!(info.history_pos, pos + 1, "one undo step");
        assert_eq!(
            docs.copy(id, 0, 4, 0, 3).unwrap(),
            "a\tb\tc\r\nx\t2\t3\r\nx\t5\tx\r\n7\t8\tx\r\n"
        );
        docs.undo(id).unwrap();
        assert_eq!(
            docs.copy(id, 0, 4, 0, 3).unwrap(),
            "a\tb\tc\r\n1\t2\t3\r\n4\t5\t6\r\n7\t8\t9\r\n"
        );
        // The cap counts every block's cells.
        let many: Vec<Area> = (0..2u32).map(|i| a(0, 60_000, i, 1)).collect();
        assert_eq!(
            docs.fill_areas(id, many, "y".into(), &[]).unwrap_err().kind,
            "too_large"
        );
        let too_many = vec![a(0, 1, 0, 1); super::MAX_AREAS + 1];
        assert_eq!(
            docs.fill_areas(id, too_many, "y".into(), &[])
                .unwrap_err()
                .kind,
            "too_large"
        );
    }

    #[test]
    fn totals_over_several_blocks_count_each_cell_once() {
        use natendo_engine::areas::Area;
        let docs = Docs::default();
        let p = tmp("areatotals.csv", b"a,b\n1,2\n3,4\n5,6\n");
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let a = |row, rows, col, cols| Area {
            row,
            rows,
            col,
            cols,
        };
        // a2:a3, a3:b4 and b2 overlap: column a rows 2..4 and column b rows 2..4 are six cells.
        let t = docs
            .selection_totals_areas(
                id,
                vec![a(1, 2, 0, 1), a(2, 2, 0, 2), a(1, 1, 1, 1)],
                natendo_engine::value::ReadOptions::default(),
            )
            .unwrap();
        assert_eq!(t.totals.cells, 6);
        assert_eq!(t.totals.sum, Some(21.0));
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
    fn find_acts_on_rows_and_titles() {
        let docs = Docs::default();
        let p = tmp("fr.csv", b"name,x city\nx1,a\nb,c\n");
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let spec = FindSpec {
            pattern: Pattern {
                text: "x".into(),
                ..Default::default()
            },
            scope: natendo_engine::find::Scope::All,
            skip_first: true,
            skip_cols: Vec::new(),
            ..Default::default()
        };
        let t = docs
            .find_rows(id, spec.clone(), FindRowsAction::Copy, None)
            .unwrap()
            .info;
        assert_eq!(t.rows, 2);
        assert!(t.name.starts_with("Matches (fr"));
        // A stale Find all is refused: nothing is done.
        let rev = docs.find_all(id, spec.clone()).unwrap().revision;
        docs.set_cells(id, vec![(2, 0, "x2".into())]).unwrap();
        assert_eq!(
            docs.find_rows(id, spec.clone(), FindRowsAction::Delete, Some(rev))
                .unwrap_err()
                .kind,
            "changed"
        );
        let rev = docs.find_all(id, spec.clone()).unwrap().revision;
        docs.undo(id).unwrap();
        assert_eq!(
            docs.find_rows(id, spec.clone(), FindRowsAction::Show, Some(rev))
                .unwrap_err()
                .kind,
            "changed"
        );
        let rev = docs.find_all(id, spec.clone()).unwrap().revision;
        assert_eq!(
            docs.find_rows(id, spec.clone(), FindRowsAction::Show, Some(rev))
                .unwrap()
                .info
                .rows,
            2
        );
        assert_eq!(
            docs.replace_titles(id, spec.clone(), "y".into()).unwrap(),
            vec![(1, "y city".to_string())]
        );
        docs.find_rows(id, spec.clone(), FindRowsAction::Flag { flag: 1 }, None)
            .unwrap();
        let r = docs
            .find_rows(id, spec.clone(), FindRowsAction::Flag { flag: 2 }, None)
            .unwrap();
        assert_eq!(r.recolored, 1);
        docs.find_rows(id, spec.clone(), FindRowsAction::Flag { flag: 1 }, None)
            .unwrap();
        let (noted, _) = docs.note_list(id, 10).unwrap();

        assert_eq!(noted.iter().filter(|n| n.flag == 1).count(), 1);
        docs.clear_filter(id).unwrap();
        assert_eq!(
            docs.column_pairs(id, 0, 1, true).unwrap(),
            vec![
                ("x1".to_string(), "a".to_string()),
                ("b".to_string(), "c".to_string())
            ]
        );
        assert!(docs.show_changed(id, true).is_err());
        docs.replace_preview(id, spec.clone(), "y".into()).unwrap();
        docs.replace_confirm(id, spec, "y".into()).unwrap();
        assert_eq!(docs.show_changed(id, true).unwrap().rows, 2);
        // After undo there are no changed rows to show.
        docs.undo(id).unwrap();
        docs.undo(id).unwrap();
        assert_eq!(docs.show_changed(id, true).unwrap_err().kind, "changed");
    }

    #[test]
    fn batch_replace_preview_then_confirm() {
        let docs = Docs::default();
        let p = tmp("br.csv", b"code\nA\nB\nC\n");
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let spec = natendo_engine::find::BatchReplace {
            pairs: vec![("A".into(), "Alpha".into()), ("B".into(), "Beta".into())],
            whole_cell: true,
            skip_first: true,
            ..Default::default()
        };
        let pv = docs.batch_preview(id, spec.clone()).unwrap();
        assert_eq!(pv.cells, 2);
        let mut other = spec.clone();
        other.pairs[0].1 = "WIPED".into();
        assert_eq!(docs.batch_confirm(id, other).unwrap_err().kind, "changed");
        docs.batch_preview(id, spec.clone()).unwrap();
        assert_eq!(docs.batch_confirm(id, spec).unwrap().cells, 2);
        assert_eq!(docs.copy(id, 1, 3, 0, 1).unwrap(), "Alpha\r\nBeta\r\nC\r\n");
    }

    #[test]
    fn replace_confirm_applies_only_the_previewed_plan() {
        let docs = Docs::default();
        let p = tmp("rc.csv", b"a\nzzq\nzzq\nkeep\n");
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let spec = |t: &str| FindSpec {
            pattern: Pattern {
                text: t.into(),
                ..Default::default()
            },
            scope: natendo_engine::find::Scope::All,
            skip_first: false,
            skip_cols: Vec::new(),
            ..Default::default()
        };
        // Previewed one thing, confirmed another: refused, nothing changes.
        docs.replace_preview(id, spec("zzq"), "yy".into()).unwrap();
        assert_eq!(
            docs.replace_confirm(id, spec("zzq"), "WIPED".into())
                .unwrap_err()
                .kind,
            "changed"
        );
        assert_eq!(
            docs.replace_confirm(id, spec("zzq"), "yy".into())
                .unwrap_err()
                .kind,
            "changed"
        );
        assert_eq!(docs.find_all(id, spec("zzq")).unwrap().cells, 2);
        // The file changed after the preview: refused too.
        docs.replace_preview(id, spec("zzq"), "yy".into()).unwrap();
        docs.set_cells(id, vec![(3, 0, "zzq".into())]).unwrap();
        assert_eq!(
            docs.replace_confirm(id, spec("zzq"), "yy".into())
                .unwrap_err()
                .kind,
            "changed"
        );
        assert_eq!(docs.find_all(id, spec("zzq")).unwrap().cells, 3);
        // The previewed plan itself goes through.
        docs.replace_preview(id, spec("zzq"), "yy".into()).unwrap();
        let r = docs.replace_confirm(id, spec("zzq"), "yy".into()).unwrap();
        assert_eq!(r.cells, 3);
    }

    #[test]
    fn unsaved_tabs_can_be_named() {
        let docs = Docs::default();
        let id = docs.new_document().unwrap().id;
        assert_eq!(docs.info(id).unwrap().name, "Untitled");
        let i = docs
            .set_title(
                id,
                "  Pivot – city
by party ",
            )
            .unwrap();
        assert_eq!(i.name, "Pivot – city by party");
        assert_eq!(docs.set_title(id, "").unwrap().name, "Untitled");
        assert_eq!(file_name_of("Pivot: a/b (voters)?"), "Pivot- a-b (voters)-");
        assert_eq!(file_name_of(" .. "), "Untitled");
        assert_eq!(file_name_of("con"), "con_file");
        assert_eq!(file_name_of("NUL.txt"), "NUL.txt_file");
        assert_eq!(file_name_of("COM1"), "COM1_file");
        assert_eq!(file_name_of("Console"), "Console");
        assert_eq!(clean_title(&"x".repeat(300)).len(), 120);
        // A saved file keeps its file name; Rename file renames it.
        let p = tmp(
            "named.csv",
            b"a
1
",
        );
        let f = docs.open_path(&p, false).unwrap().id;
        wait(&docs, f);
        assert_eq!(docs.set_title(f, "x").unwrap_err().kind, "has_path");
    }

    #[test]
    fn audit_sample_opens_its_record_and_tabs_go_to_one_workbook() {
        use natendo_engine::tools::{Sample, SampleSize};
        let p = tmp(
            "audit.csv",
            b"id,precinct
1,A
2,B
3,A
4,B
5,A
6,B
",
        );
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let rep = docs
            .new_table_report(
                id,
                TableOp::Sample {
                    spec: Sample {
                        size: SampleSize::Rows { n: 2 },
                        seed: 7,
                        keep_first: true,
                        per: Some(1),
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        assert!(rep.info.name.contains("seed 7"), "{}", rep.info.name);
        assert_eq!(rep.info.rows, 5);
        let rec = rep.record.expect("a record tab");
        assert!(rec.name.starts_with("Sample record"));
        let cell = |r: u64, c: u32| docs.cell_text(rec.id, r, c).unwrap().0;
        assert_eq!(cell(3, 1), "7", "the seed");
        // The file's own SHA-256 (the tab is unchanged).
        let want = natendo_engine::tools::file_sha256(&p, &Progress::new()).unwrap();
        assert_eq!(want.len(), 64);
        assert_eq!(cell(2, 1), want);
        assert_eq!(cell(4, 1), "precinct");
        // Two tabs as the sheets of one workbook.
        let out = p.with_file_name("audit-book.xlsx");
        let r = docs
            .export_book(
                &[id, rep.info.id, id],
                &out,
                &ExportOptions {
                    format: ExportFormat::Xlsx,
                    ..Default::default()
                },
                &[vec![300], vec![], vec![]],
            )
            .unwrap();
        assert_eq!((r.sheets, r.rows), (2, 10));
        let bytes = std::fs::read(&out).unwrap();
        let book = natendo_engine::sheet::Book::open(&bytes).unwrap();
        let names: Vec<String> = book.sheets().iter().map(|s| s.name.clone()).collect();
        assert_eq!(names[0], "audit");
        assert!(names[1].starts_with("Sample (audit"), "{names:?}");
        let _ = std::fs::remove_file(&out);
        // Another program changes the file: the record says so instead of
        // fingerprinting bytes that are not the rows sampled.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            &p,
            b"id,precinct
9,Z
9,Z
9,Z
",
        )
        .unwrap();
        let rep = docs
            .new_table_report(
                id,
                TableOp::Sample {
                    spec: Sample {
                        size: SampleSize::Rows { n: 1 },
                        seed: 7,
                        keep_first: true,
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        let rec = rep.record.expect("a record tab");
        let v = docs.cell_text(rec.id, 2, 1).unwrap().0;
        assert!(v.contains("changed on disk"), "{v}");
    }

    #[test]
    fn part_files_already_there_are_found() {
        let dir = crate::testdir::path("nde-clash");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in [
            "voters (2 of 3).xlsx",
            "Voters (7 of 10).xlsx",
            "voters (1 of 3).csv",
            "other (1 of 2).xlsx",
        ] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let (same, other) = part_clash(&dir.join("voters.xlsx"), 3);
        assert_eq!(same, vec!["voters (2 of 3).xlsx".to_string()]);
        assert_eq!(other, vec!["Voters (7 of 10).xlsx".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pivot_tabs_are_named_after_their_fields() {
        let docs = Docs::default();
        let p = tmp(
            "voters.csv",
            b"city,party
A,x
B,y
",
        );
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let spec = natendo_engine::pivot::PivotSpec {
            rows: vec![natendo_engine::pivot::PivotField {
                col: 0,
                ..Default::default()
            }],
            cols: vec![natendo_engine::pivot::PivotField {
                col: 1,
                ..Default::default()
            }],
            ..Default::default()
        };
        let t = docs.pivot_table(id, &spec).unwrap();
        assert_eq!(t.name, "Pivot – city by party (voters)");
        // A drill-down is named after the fields, never their values.
        let d = docs
            .pivot_drill(id, &spec, &["a".into()], &["x".into()])
            .unwrap();
        assert_eq!(d.name, "Rows for one city and party (voters)");
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
        docs.move_rows(id, 1, 1, 0, None).unwrap();
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
    fn gap_audit_shows_rows_and_opens_a_table() {
        let docs = Docs::default();
        let path = tmp("gaps.csv", b"N,x\r\n1,a\r\n2,b\r\n5,c\r\n6,d\r\n");
        let id = docs.open_path(&path, false).unwrap().id;
        wait(&docs, id);
        let spec = GapsSpec {
            keep_first: true,
            ..Default::default()
        };
        let show = || DataOp::ShowGapRows { spec: spec.clone() };
        assert!(docs.data_op(id, show()).is_err(), "no audit yet");
        assert_eq!(docs.gap_audit(id, &spec).unwrap().missing, "2");
        let table = docs
            .new_table(id, TableOp::Gaps { spec: spec.clone() })
            .unwrap();
        assert!(table.name.starts_with("Gaps in N ("), "{}", table.name);
        let other = GapsSpec {
            start: "0".into(),
            ..spec.clone()
        };
        let report = docs.data_op(id, DataOp::ShowGapRows { spec: other });
        assert!(report.is_err(), "another spec than the one audited");
        assert_eq!(docs.data_op(id, show()).unwrap().rows, 2);
        // The table changed: the old audit is not used.
        assert!(docs.data_op(id, show()).is_err());
    }

    #[test]
    fn a_clean_matrix_previews_applies_and_is_recorded() {
        use natendo_engine::clean::{MatrixEntry, Spaces, Step};
        let docs = Docs::default();
        let p = tmp(
            "matrix.csv",
            b"name,town
 ann , york 
",
        );
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        let entry = |col, title: &str| MatrixEntry {
            col,
            title: title.into(),
            steps: vec![Step::Spaces { how: Spaces::Trim }],
            from_file: String::new(),
        };
        let spec = CleanMatrix {
            entries: vec![entry(0, "name"), entry(1, "town")],
            skip_first: true,
            read: Default::default(),
        };
        let preview = docs.clean_matrix_preview(id, &spec, 20_000.0).unwrap();
        assert_eq!(preview.columns[1].changed, 1);
        assert!(docs.clean_matrix_preview(id, &spec, f64::NAN).is_err());
        let op = |revision| DataOp::CleanMatrix {
            spec: spec.clone(),
            now: 20_000.0,
            revision,
        };
        // A change after the preview (another window, the agent): refused,
        // nothing applied or recorded.
        docs.set_cells(id, vec![(1, 1, "leeds".into())]).unwrap();
        let why = docs.data_op(id, op(Some(preview.revision))).unwrap_err();
        assert!(why.message.contains("preview again"), "{}", why.message);
        assert!(docs.recipe_of(id, true, "r").unwrap().steps.len() <= 1);
        let again = docs.clean_matrix_preview(id, &spec, 20_000.0).unwrap();
        assert_eq!(docs.data_op(id, op(Some(again.revision))).unwrap().cells, 1);
        let logged = docs.recipe_of(id, true, "r").unwrap().steps;
        assert_eq!(logged.last(), Some(&RecipeOp::CleanMatrix { spec }));
    }

    #[test]
    fn compared_lists_open_slices_until_a_list_changes() {
        use natendo_engine::sets::SetList;
        let docs = Docs::default();
        let open = |name: &str, body: &[u8]| {
            let id = docs.open_path(&tmp(name, body), false).unwrap().id;
            wait(&docs, id);
            id
        };
        let a = open("sets-a.csv", b"id,n\r\n1,x\r\n2,y\r\n");
        let b = open("sets-b.csv", b"id,n\r\n3,z\r\n2,w\r\n");
        let list = SetList {
            cols: vec![0],
            header: true,
        };
        let spec = SetsSpec {
            lists: vec![list.clone(), list.clone()],
            ..Default::default()
        };
        assert!(
            docs.sets_run(a, &[a, a], &spec).is_err(),
            "the same list twice"
        );
        let run = docs.sets_run(a, &[a, b], &spec).unwrap();
        assert_eq!(run.names, ["sets-a", "sets-b"]);
        assert_eq!(run.report.slices[2].keys, 1);
        let slice = docs.sets_open(run.id, 3, 1, false).unwrap();
        assert_eq!(slice.rows, 2);
        assert_eq!(
            slice.name,
            "sets-a and sets-b, rows of sets-b (capitals ignored, spaces trimmed)"
        );
        assert!(
            docs.sets_open(run.id, 2, 0, false).is_err(),
            "not in the slice"
        );
        assert!(docs.sets_table(run.id).unwrap().rows > 3);
        // Two files of one name: told apart by key titles, then by letter.
        let again = open("sets-a.csv", b"id,n\r\n9,x\r\n");
        let other = SetList {
            cols: vec![1],
            header: true,
        };
        let named = docs
            .sets_run(
                a,
                &[a, again, a],
                &SetsSpec {
                    lists: vec![list.clone(), list.clone(), other],
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            named.names,
            ["sets-a (id) [list A]", "sets-a (id) [list B]", "sets-a (n)"]
        );
        docs.close(again);
        let run = docs.sets_run(a, &[a, b], &spec).unwrap();
        // A sort of the second list alone makes the result stale.
        let sort = SortSpec {
            keys: vec![natendo_engine::sort::SortKey {
                col: 0,
                descending: false,
                kind: Default::default(),
                match_case: false,
            }],
            keep_first: true,
            ..Default::default()
        };
        docs.sort(b, sort).unwrap();
        let why = docs.sets_open(run.id, 3, 0, false).unwrap_err();
        assert!(
            why.message.contains("run the comparison again"),
            "{}",
            why.message
        );
        assert!(docs.sets_table(run.id).is_err());
        let run = docs.sets_run(a, &[a, b], &spec).unwrap();
        docs.hide_rows(a, 1, 1).unwrap();
        assert!(docs.sets_open(run.id, 3, 0, false).is_err(), "hidden rows");
        let run = docs.sets_run(a, &[a, b], &spec).unwrap();
        docs.close(b);
        assert!(
            docs.sets_open(run.id, 1, 0, false).is_err(),
            "a list closed"
        );
    }

    #[test]
    fn a_comparison_let_go_for_memory_says_so() {
        use natendo_engine::sets::SetList;
        let docs = Docs::default();
        let open = |name: &str| {
            let id = docs
                .open_path(&tmp(name, b"id\r\n1\r\n"), false)
                .unwrap()
                .id;
            wait(&docs, id);
            id
        };
        let (a, b) = (open("sets-old-a.csv"), open("sets-old-b.csv"));
        let list = SetList {
            cols: vec![0],
            header: true,
        };
        let spec = SetsSpec {
            lists: vec![list.clone(), list],
            ..Default::default()
        };
        let runs: Vec<u64> = (0..5)
            .map(|_| docs.sets_run(a, &[a, b], &spec).unwrap().id)
            .collect();
        let why = docs.sets_open(runs[0], 3, 0, false).unwrap_err();
        assert_eq!(
            why.message,
            "this comparison was closed to save memory: run it again"
        );
        assert!(docs.sets_open(runs[1], 3, 0, false).is_ok());
        // One the screen closed itself is not said to be for memory.
        docs.sets_close(runs[4]);
        let why = docs.sets_table(runs[4]).unwrap_err();
        assert!(why.message.contains("run the comparison again"));
    }

    #[test]
    fn relationship_exceptions_follow_the_last_scan() {
        let docs = Docs::default();
        let mut body = String::from("zip,state\r\n");
        for i in 0..200 {
            let state = if i == 7 { 9 } else { i % 10 % 3 };
            body.push_str(&format!("z{},s{state}\r\n", i % 10));
        }
        let id = docs
            .open_path(&tmp("relations.csv", body.as_bytes()), false)
            .unwrap()
            .id;
        wait(&docs, id);
        let show = |a, b| DataOp::ShowRelationExceptions {
            kind: RelationKind::Depends,
            a,
            b,
        };
        assert!(docs.data_op(id, show(0, 1)).is_err(), "no scan yet");
        let spec = RelationsSpec {
            cols: vec![0, 1],
            ..Default::default()
        };
        let r = docs.find_relations(id, &spec).unwrap();
        assert_eq!(r.findings.len(), 1, "{:?}", r.findings);
        assert!(docs.data_op(id, show(1, 0)).is_err(), "not a line found");
        assert_eq!(docs.data_op(id, show(0, 1)).unwrap().rows, 1);
        // The view changed the table: the old scan is not used.
        assert!(docs.data_op(id, show(0, 1)).is_err());
        assert!(docs.recipe_of(id, true, "r").unwrap().steps.is_empty());
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
        assert!(docs.modified_report().is_clear());
        let e = docs.get(id).unwrap();
        let held = e.write(); // as a save does
        let r = docs.modified_report();
        assert_eq!(
            r.busy,
            vec!["closecheck.csv".to_string()],
            "held: not looked inside, named"
        );
        assert!(r.docs.is_empty() && !r.is_clear());
        drop(held);
        assert!(docs.modified_report().is_clear());
    }

    /// The quit question's list: a document edited, then undone, then edited
    /// again has the same undo position as after the first edit, but its
    /// generation is new, so a change made while the question was open is seen.
    #[test]
    fn generation_goes_up_on_every_edit_undo_and_redo() {
        let p = tmp("generation.csv", b"a,b\r\n1,2\r\n");
        let docs = Docs::default();
        let id = docs.open_path(&p, false).unwrap().id;
        wait(&docs, id);
        assert!(docs.modified_report().is_clear());
        docs.set_cells(id, vec![(0, 0, "x".into())]).unwrap();
        let first = docs.modified_report();
        assert_eq!(first.docs.len(), 1);
        assert_eq!(first.docs[0].name, "generation.csv");
        let pos_first = docs.info(id).unwrap().history_pos;
        let g1 = first.docs[0].generation;
        // The question was asked now: it knows (id, g1).
        let known = [(id, g1)];
        assert!(!first.changed_since(&known), "nothing new yet");
        // Undo (the document is clean again), then a different edit: the
        // undo position is the same as after the first edit.
        docs.undo(id).unwrap();
        let after_undo = docs.modified_report();
        assert!(after_undo.docs.is_empty());
        docs.set_cells(id, vec![(0, 1, "y".into())]).unwrap();
        assert_eq!(docs.info(id).unwrap().history_pos, pos_first);
        let now = docs.modified_report();
        let g3 = now.docs[0].generation;
        assert!(
            g3 > g1,
            "generation {g3} must be past {g1} though the undo position repeats"
        );
        assert!(
            now.changed_since(&known),
            "the new edit is not the one the person agreed to lose"
        );
        // Redo counts too.
        docs.undo(id).unwrap();
        docs.redo(id).unwrap();
        let g4 = docs.modified_report().docs[0].generation;
        assert!(g4 > g3);
    }

    #[test]
    fn a_document_modified_after_the_question_is_not_agreed_to() {
        let a = tmp("quit-a.csv", b"a\r\n1\r\n");
        let b = tmp("quit-b.csv", b"a\r\n1\r\n");
        let docs = Docs::default();
        let ia = docs.open_path(&a, false).unwrap().id;
        let ib = docs.open_path(&b, false).unwrap().id;
        wait(&docs, ia);
        wait(&docs, ib);
        docs.set_cells(ia, vec![(0, 0, "x".into())]).unwrap();
        let r = docs.modified_report();
        let known = [(ia, r.docs[0].generation)];
        assert!(!r.changed_since(&known));
        // An edit in the other file (the other window), after the question.
        docs.set_cells(ib, vec![(0, 0, "y".into())]).unwrap();
        let r2 = docs.modified_report();
        assert_eq!(r2.docs.len(), 2);
        assert!(
            r2.changed_since(&known),
            "quit_app must not exit: it asks again"
        );
        assert!(!r2.changed_since(&[(ia, r2.docs[0].generation), (ib, r2.docs[1].generation)]));
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
            scope: Default::default(),
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
            combine: Default::default(),
            invert: false,
            columns: vec![ColumnFilter {
                enabled: true,
                negate: false,
                name: None,
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
            skip_cols: Vec::new(),
            ..Default::default()
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
        // Clearing the filter, the replace, the filter and the sort: four steps.
        for _ in 0..4 {
            docs.undo(id).unwrap();
        }
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
            skip_cols: Vec::new(),
            ..Default::default()
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
        // A cancel aimed at another operation leaves this one running.
        docs.cancel_job(id, Some("Looking for look-alikes"))
            .unwrap();
        assert!(!job.progress.cancel.load(Ordering::Relaxed));
        docs.cancel_job(id, Some("Sorting")).unwrap();
        assert!(job.progress.cancel.load(Ordering::Relaxed));
        job.progress.cancel.store(false, Ordering::Relaxed);
        // No label: whatever is running.
        docs.cancel_job(id, None).unwrap();
        assert!(job.progress.cancel.load(Ordering::Relaxed));
        drop(job);
        assert!(docs.sort(id, spec).is_ok());
        let huge = FilterSpec {
            columns: vec![natendo_engine::filter::ColumnFilter {
                enabled: true,
                negate: false,
                name: None,
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

#[cfg(test)]
#[path = "audit.rs"]
mod audit;

#[cfg(test)]
mod key_binding_tests {
    use super::*;
    use natendo_engine::validate::{Check, CheckKind};

    fn file(dir: &str, name: &str, body: &str) -> PathBuf {
        let d = crate::testdir::path(&format!("nde-fk-{dir}"));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    fn open(docs: &Docs, p: &Path) -> u32 {
        let id = docs.open_path(p, false).unwrap().id;
        for _ in 0..500 {
            if docs.info(id).unwrap().indexed {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        id
    }

    fn rule(table: &str) -> CheckSpec {
        CheckSpec {
            checks: vec![Check::new(
                Some(1),
                CheckKind::ForeignKey {
                    table: table.into(),
                    column: "code".into(),
                    match_case: false,
                    trim: true,
                    allow_blank: true,
                },
            )],
            keep_first: true,
            ..Default::default()
        }
    }

    #[test]
    fn table_checks_count_the_rows_of_the_open_tab_they_name_and_are_kept_beside_the_file() {
        use natendo_engine::tablecheck::{Compare, TableCheck, TableCheckKind};
        let docs = Docs::default();
        let votes = open(
            &docs,
            &file(
                "tc",
                "votes.csv",
                "p,early,day,total\r\nA,1,2,3\r\nB,4,5,9\r\n",
            ),
        );
        let same = |table: &str| TableCheck {
            name: None,
            kind: TableCheckKind::RowCountSameAs {
                table: table.into(),
            },
        };
        let spec = CheckSpec {
            keep_first: true,
            table_checks: vec![
                same("roster"),
                same("votes"),
                TableCheck {
                    name: Some("Adds up".into()),
                    kind: TableCheckKind::Aggregate {
                        expr: "SUM([early]) + SUM([day]) = SUM([total])".into(),
                        tolerance: Default::default(),
                    },
                },
            ],
            ..Default::default()
        };
        // Not open: that one does not run; its own file counts its own rows.
        let r = docs.run_checks(votes, &spec).unwrap();
        let oks: Vec<Option<bool>> = r.table.iter().map(|x| x.ok).collect();
        assert_eq!(oks, vec![None, Some(true), Some(true)]);
        assert_eq!(r.table[0].why.as_deref(), Some("open roster"));
        let roster = open(&docs, &file("tc", "roster.csv", "p\r\nA\r\nB\r\nC\r\n"));
        let r = docs.run_checks(votes, &spec).unwrap();
        assert_eq!(r.table[0].ok, Some(false));
        assert_eq!(r.table[0].expected, "3 rows, as roster has");
        // Kept beside the file with the rules, and read back.
        let tc = vec![TableCheck {
            name: None,
            kind: TableCheckKind::RowCount {
                compare: Compare::AtLeast,
                value: 1,
            },
        }];
        docs.checks_sidecar_keep(votes, vec![], tc.clone(), true)
            .unwrap();
        let back = docs.checks_sidecar_load(votes).unwrap().unwrap();
        assert_eq!(back.table_checks, tc);
        assert!(back.checks.is_empty());
        docs.checks_sidecar_keep(votes, vec![], vec![], true)
            .unwrap();
        assert!(docs.checks_sidecar_load(votes).unwrap().is_none());
        docs.close(roster);
        docs.close(votes);
    }

    #[test]
    fn save_as_with_a_receipt_records_the_file_read_and_the_file_written() {
        use natendo_engine::receipt::verify;
        let docs = Docs::default();
        let src = file("rc", "voters.csv", "name,n\r\nAnn,1\r\n");
        let id = open(&docs, &src);
        docs.set_cells(id, vec![(1, 1, "2".into())]).unwrap();
        let out = src.with_file_name("voters-fixed.csv");
        docs.save_with_receipt(
            id,
            Some(out.clone()),
            false,
            false,
            None,
            Default::default(),
        )
        .unwrap();
        let note = docs.take_receipt_note(id).unwrap();
        assert!(note.contains("voters-fixed.csv.ndreceipt.json"), "{note}");
        assert!(docs.take_receipt_note(id).is_none(), "said once");
        let receipt = natendo_engine::receipt::receipt_path(&out);
        let v = verify(
            &receipt,
            std::slice::from_ref(&src),
            std::slice::from_ref(&out),
            false,
            &Progress::new(),
        )
        .unwrap();
        assert!(v.ok, "{:?}", v.lines);
        assert_eq!(v.summary["action"], "save as");
        docs.set_cells(id, vec![(1, 1, "3".into())]).unwrap();
        // The receipt cannot be written (a folder is in its place): the save stands, the note says so.
        let blocked = out.with_file_name("blocked.csv");
        std::fs::create_dir_all(natendo_engine::receipt::receipt_path(&blocked)).unwrap();
        docs.save_with_receipt(
            id,
            Some(blocked.clone()),
            false,
            false,
            None,
            Default::default(),
        )
        .unwrap();
        assert!(std::fs::read_to_string(&blocked).unwrap().contains('3'));
        let note = docs.take_receipt_note(id).unwrap();
        assert!(
            note.starts_with("Saved, but the receipt was not written"),
            "{note}"
        );
        docs.close(id);
    }

    #[test]
    fn rules_read_the_open_tab_by_name_once_and_refuse_when_it_is_not_there() {
        let docs = Docs::default();
        let voters = open(
            &docs,
            &file(
                "a",
                "voters.csv",
                "name,precinct\r\nAnn,P1\r\nBo,P9\r\nCy,P2\r\n",
            ),
        );
        let spec = rule("precincts");
        // Not open: the run says so, and every path that would show it as clean refuses.
        let r = docs.run_checks(voters, &spec).unwrap();
        assert_eq!(r.not_run.len(), 1);
        assert!(r.not_run[0].why.contains("open precincts"));
        for op in [
            DataOp::ShowProblems {
                spec: spec.clone(),
                passing: false,
            },
            DataOp::SortProblems {
                spec: spec.clone(),
                scope: Default::default(),
            },
            DataOp::CheckColumn {
                spec: spec.clone(),
                title: String::new(),
            },
        ] {
            let e = docs.data_op(voters, op).unwrap_err();
            assert!(format!("{e:?}").contains("did not run"), "{e:?}");
        }
        assert!(docs.next_problem(voters, 1, 0, false, &spec, &[]).is_err());
        // Open it (by its name without the extension): one problem, P9.
        let prec = open(
            &docs,
            &file(
                "a",
                "precincts.csv",
                "code,area\r\nP1,North\r\nP2,South\r\n",
            ),
        );
        let r = docs.run_checks(voters, &spec).unwrap();
        assert!(r.not_run.is_empty());
        assert_eq!(r.per_check, vec![1]);
        assert_eq!(r.tables[0].source.doc, prec);
        // The set is kept while the other tab is unchanged, and read again after an edit.
        let go = Progress::new();
        let a = docs.key_bindings(voters, &spec, Some(&go)).unwrap();
        let b = docs.key_bindings(voters, &spec, Some(&go)).unwrap();
        assert_eq!(a, b, "the same set, not read twice");
        assert!(!docs
            .check_marks(voters, 0, 4, 0, 2, &spec)
            .unwrap()
            .is_empty());
        docs.set_cells(prec, vec![(2, 0, "P9".into())]).unwrap();
        // Screen marks never read the changed table: the rule is left out until Check now.
        assert_eq!(
            docs.key_bindings(voters, &spec, None).unwrap(),
            vec![KeyBinding::None]
        );
        let marks = docs.check_marks(voters, 0, 4, 0, 2, &spec).unwrap();
        assert!(marks.is_empty(), "{marks:?}");
        assert_eq!(
            docs.key_sets.lock().unwrap().len(),
            1,
            "nothing read for marks"
        );
        // A cancelled read fails and keeps nothing.
        let stop = Progress::new();
        stop.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let e = docs.key_bindings(voters, &spec, Some(&stop)).unwrap_err();
        assert!(format!("{e:?}").to_lowercase().contains("cancel"), "{e:?}");
        assert_eq!(
            docs.key_sets.lock().unwrap().len(),
            1,
            "a cancelled read is not kept"
        );
        let c = docs.key_bindings(voters, &spec, Some(&go)).unwrap();
        assert_ne!(a, c);
        assert_eq!(
            docs.key_sets.lock().unwrap().len(),
            1,
            "the old set is dropped"
        );
        let r = docs.run_checks(voters, &spec).unwrap();
        assert_eq!(r.per_check, vec![1], "now P2 is missing");
        let rep = docs
            .data_op(
                voters,
                DataOp::ShowProblems {
                    spec: spec.clone(),
                    passing: false,
                },
            )
            .unwrap();
        assert_eq!(rep.rows, 1);
        // A second tab with the same name: refused until one is closed.
        let twin = open(
            &docs,
            &file("b", "precincts.csv", "code,area\r\nP1,North\r\n"),
        );
        let r = docs.run_checks(voters, &spec).unwrap();
        assert!(
            r.not_run[0].why.contains("two open files"),
            "{:?}",
            r.not_run
        );
        docs.close(twin);
        // A column that is not there.
        let r = docs.run_checks(voters, &rule("precincts.csv")).unwrap();
        assert!(r.not_run.is_empty());
        let mut bad = rule("precincts");
        if let CheckKind::ForeignKey { column, .. } = &mut bad.checks[0].kind {
            *column = "zone".into();
        }
        let r = docs.run_checks(voters, &bad).unwrap();
        assert!(r.not_run[0].why.contains("no column zone"));
        // Closing the tab lets go of its sets.
        docs.close(prec);
        assert!(docs.key_sets.lock().unwrap().is_empty());
        docs.close(voters);
    }

    #[test]
    fn a_rule_can_read_its_own_tab() {
        let docs = Docs::default();
        let staff = open(
            &docs,
            &file("c", "staff.csv", "code,boss\r\n1,\r\n2,1\r\n3,8\r\n"),
        );
        let mut spec = rule("staff");
        spec.checks[0].col = Some(1);
        let r = docs.run_checks(staff, &spec).unwrap();
        assert_eq!(r.per_check, vec![1]);
        assert_eq!(r.listed[0].value, "8");
        docs.close(staff);
    }
}
